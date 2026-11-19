import Foundation

/// Transcreve os blocos de uma reunião pela costura de cada fonte e geração. Ao vivo, na pausa, no
/// fim e na recuperação o caminho é o mesmo; só muda se a cauda é fechada. O estado está no banco:
/// outro processo (ou uma nova abertura) continua de onde a última confirmação parou.
public actor ProcessadorDeReuniao {
    public typealias AoConfirmar = @Sendable (_ trechos: [TrechoNovo], _ atraso: Double) async -> Void

    public enum Falha: LocalizedError {
        case concorrencia, textoIncompleto
        public var errorDescription: String? {
            switch self {
            case .concorrencia: return "Outra tentativa já está transcrevendo esta reunião."
            case .textoIncompleto: return "Ainda há áudio sem transcrição nesta reunião."
            }
        }
    }

    /// Contexto antes do ponto confirmado, margem antes do fim disponível, passo mínimo e janela máxima.
    public static let contexto = taxaLancamento
    public static let margem = taxaLancamento * 8 / 10
    public static let passoMinimo = taxaLancamento * 2
    public static let janelaMaxima = taxaLancamento * 15

    private struct Chave: Hashable { let fonte: FonteDeAudio; let geracao: Int }

    public let documento: String
    private let armazem: ArmazemLancamento
    private let disco: DiscoDeBlocos
    private let reconhecedor: ReconhecedorDeFala
    private let preparar: PreparadorDeTexto
    private let dono = UUID().uuidString
    private var aoConfirmar: AoConfirmar?
    private var tomadas = Set<Chave>()
    private var seladoEm: [Chave: Date] = [:]
    private var trabalhando = false
    private var pedidoPendente = false

    public init(documento: String, armazem: ArmazemLancamento, disco: DiscoDeBlocos,
                reconhecedor: ReconhecedorDeFala, preparar: @escaping PreparadorDeTexto = { $0 }) {
        self.documento = documento
        self.armazem = armazem
        self.disco = disco
        self.reconhecedor = reconhecedor
        self.preparar = preparar
    }

    public func definirAoConfirmar(_ acao: @escaping AoConfirmar) { aoConfirmar = acao }

    /// Aviso do coletor: há um bloco novo durável. Processa o que já dá para decidir, sem fechar a cauda.
    public func blocoSelado(fonte: FonteDeAudio, geracao: Int, fim: Int) async {
        seladoEm[Chave(fonte: fonte, geracao: geracao)] = Date()
        if trabalhando { pedidoPendente = true; return }
        trabalhando = true
        defer { trabalhando = false }
        repeat {
            pedidoPendente = false
            do { try await avancar(fechamento: false) }
            catch { ultimoErro = error; return }
        } while pedidoPendente
    }

    public private(set) var ultimoErro: Error?

    /// Fecha as caudas de todas as fontes (pausa, fim ou recuperação). Espera um passo ao vivo em curso.
    public func fechar() async throws {
        while trabalhando { try await Task.sleep(for: .milliseconds(20)) }
        trabalhando = true
        defer { trabalhando = false }
        try await avancar(fechamento: true)
    }

    /// Recuperar é transcrever: fecha as costuras abertas e tenta de novo cada faixa pendente.
    public func recuperar() async throws {
        try await fechar()
        try await repetirPendencias()
    }

    private func avancar(fechamento: Bool) async throws {
        let costuras = try armazem.costuras(documento: documento)
        let blocos = try armazem.blocos(documento: documento)
        for costura in costuras where !costura.fechada {
            let chave = Chave(fonte: costura.fonte, geracao: costura.geracao)
            let daFonte = blocos.filter { $0.id.fonte == costura.fonte && $0.id.geracao == costura.geracao }
            guard let disponivel = daFonte.map(\.fim).max() else { continue }
            if !tomadas.contains(chave) {
                try armazem.tomarCostura(documento: documento, fonte: costura.fonte, geracao: costura.geracao, dono: dono)
                tomadas.insert(chave)
            }
            var confirmado = costura.confirmadoAte
            while true {
                try Task.checkCancellation()
                let fimJanela = min(disponivel, confirmado + Self.janelaMaxima)
                let fecha = fechamento && fimJanela == disponivel
                let limite = fecha ? fimJanela : fimJanela - Self.margem
                guard limite > confirmado, fecha || limite - confirmado >= Self.passoMinimo else { break }
                let atual = try armazem.blocos(documento: documento)
                    .filter { $0.id.fonte == costura.fonte && $0.id.geracao == costura.geracao }
                guard let novo = try await passo(chave, blocos: atual, confirmado: confirmado, fimJanela: fimJanela,
                                                 limite: limite, fechamento: fecha) else { break }
                if novo == confirmado { break }
                confirmado = novo
            }
        }
    }

    /// Um passo: lê a janela com contexto, reconhece, decide o corte e confirma numa transação.
    private func passo(_ chave: Chave, blocos: [BlocoRegistrado], confirmado: Int, fimJanela: Int,
                       limite: Int, fechamento: Bool) async throws -> Int? {
        let inicioGeracao = blocos.map(\.inicio).min() ?? confirmado
        let de = max(inicioGeracao, confirmado - Self.contexto)
        let (audio, quebrados) = try disco.faixa(blocos, de: de, ate: fimJanela)
        for b in quebrados { try armazem.marcarLacuna(b.id, motivo: DiscoDeBlocos.Falha.incompleto(b.id.texto).localizedDescription) }
        let taxa = Double(taxaLancamento)
        var palavras = AtividadeDeVoz.tem(audio)
            ? try await reconhecedor.palavras(audio, inicio: Double(de) / taxa,
                                               fimDaFala: fechamento ? Double(fimJanela - de) / taxa : nil)
            : []
        var caudaPendente: Range<Int>?
        if fechamento {
            var trechoFinal = Array(audio.suffix(CaudaDaFala.capacidade))
            var inicioCauda = fimJanela - trechoFinal.count
            let ultimoFim = palavras.last?.fim
            let corteAtual = max(Double(confirmado) / taxa, ultimoFim ?? 0)
            var recuperada = try await ProcessadorDeArquivo.recuperarCauda(trechoFinal, base: Double(inicioCauda) / taxa,
                                                                           ultimoFim: corteAtual, reconhecedor: reconhecedor)
            if recuperada == nil, Double(fimJanela) / taxa - corteAtual >= 0.28 {
                let deExpandido = max(inicioGeracao, fimJanela - CaudaDaFala.capacidade)
                if deExpandido < de {
                    let (audioExpandido, _) = try disco.faixa(blocos, de: deExpandido, ate: fimJanela)
                    trechoFinal = audioExpandido
                    inicioCauda = deExpandido
                    recuperada = try await ProcessadorDeArquivo.recuperarCauda(trechoFinal, base: Double(inicioCauda) / taxa,
                                                                               ultimoFim: corteAtual, reconhecedor: reconhecedor)
                }
            }
            if let recuperada {
                palavras.removeAll { $0.inicio >= recuperada.limite }
                palavras += recuperada.palavras
            } else if let fala = CaudaDaFala.localizar(trechoFinal), let ultimoFim,
                      inicioCauda + fala.inicio >= max(confirmado, Int((ultimoFim + 0.16) * taxa)) {
                caudaPendente = (inicioCauda + fala.inicio)..<(inicioCauda + fala.fim)
            }
        }
        let decisao = Costura.decidir(palavras: palavras, confirmadoAte: confirmado, limite: limite, fechamento: fechamento) { faixa in
            let a = max(0, faixa.lowerBound - de), z = min(audio.count, faixa.upperBound - de)
            return a < z && AtividadeDeVoz.tem(audio[a..<z])
        }
        guard decisao.novoConfirmado > confirmado else { return confirmado }
        var trechos: [TrechoNovo] = []
        for t in Costura.trechos(decisao.confirmadas, documento: documento, fonte: chave.fonte, geracao: chave.geracao) {
            let editado = await preparar(t.texto)
            let texto = editado.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? t.texto : editado
            trechos.append(TrechoNovo(id: t.id, fonte: chave.fonte, inicio: t.inicio, fim: t.fim, texto: texto, voz: chave.fonte.vozAoVivo))
        }
        let comAtividade = Set(blocos.filter { $0.estado == .capturado && $0.fim <= decisao.novoConfirmado }.filter { b in
            let a = max(0, b.inicio - de), z = min(audio.count, b.fim - de)
            return a < z && AtividadeDeVoz.tem(audio[a..<z])
        }.map(\.id.texto))
        let pendencia = (decisao.pendente ?? caudaPendente).map { (inicio: $0.lowerBound, fim: $0.upperBound,
                                                 motivo: "Havia voz, mas o reconhecedor não devolveu texto. O áudio continua guardado.") }
        // O evento ao vivo leva o ID com que cada trecho ficou no banco (sufixo em colisão).
        guard let persistidos = try armazem.confirmarPersistindo(.init(documento: documento, fonte: chave.fonte, geracao: chave.geracao, dono: dono,
                                                                    esperado: confirmado, novo: decisao.novoConfirmado, trechos: trechos,
                                                                    comAtividade: comAtividade, pendencia: pendencia, fechar: false))
        else { throw Falha.concorrencia }
        if !persistidos.isEmpty, let aoConfirmar {
            await aoConfirmar(persistidos, seladoEm[chave].map { Date().timeIntervalSince($0) } ?? 0)
        }
        return decisao.novoConfirmado
    }

    /// Nova tentativa de cada faixa com voz sem texto. Vazio de novo: continua pendente e repetível.
    public func repetirPendencias() async throws {
        let blocos = try armazem.blocos(documento: documento)
        for p in try armazem.pendencias(documento: documento) where p.estado == "aberta" {
            try Task.checkCancellation()
            let daFonte = blocos.filter { $0.id.fonte == p.fonte && $0.id.geracao == p.geracao }
            let de = max(daFonte.map(\.inicio).min() ?? p.inicio, p.inicio - Self.contexto)
            let ate = min(daFonte.map(\.fim).max() ?? p.fim, p.fim + Self.contexto)
            let (audio, quebrados) = try disco.faixa(daFonte, de: de, ate: ate)
            for b in quebrados { try armazem.marcarLacuna(b.id, motivo: DiscoDeBlocos.Falha.incompleto(b.id.texto).localizedDescription) }
            let semCobertura = quebrados.contains { $0.inicio < p.fim && $0.fim > p.inicio }
                || daFonte.contains { $0.estado == .lacuna && $0.inicio < p.fim && $0.fim > p.inicio }
            let taxa = Double(taxaLancamento)
            // Pendência sobre um intervalo que já tem texto gravado da mesma fonte: não há como saber,
            // sem adivinhar, o que falta e onde entra. No fluxo normal isso não ocorre (pendência nasce
            // num passo sem palavras ou depois da última); vem de áudio adotado depois de uma queda ou
            // de estado inconsistente. Decide antes de reconhecer: nada se resolve sozinho, o áudio fica
            // guardado, o motivo aparece na reunião e a decisão é de quem usa (Descartar áudio preserva
            // o texto já confirmado). As outras pendências seguem sendo tentadas.
            // Áudio adotado depois de uma queda: qualquer sobreposição com texto gravado retém.
            // Pendência normal: só sobreposição de verdade (centro do trecho dentro dela, ou trecho que a
            // atravessa de ponta a ponta); palavra confirmada pelo centro que só encosta na borda não conta.
            let (pi, pf) = (Double(p.inicio) / taxa, Double(p.fim) / taxa)
            let adocao = p.id.hasPrefix("adocao|")
            if try brutos(documento: documento).contains(where: { t in
                guard t.fonte == p.fonte, t.inicio < pf, t.fim > pi else { return false }
                let centro = (t.inicio + t.fim) / 2
                return adocao || (centro >= pi && centro < pf) || (t.inicio <= pi && t.fim >= pf)
            }) {
                let motivo = "Áudio guardado cobre um trecho que já tem texto. Para não duplicar nem perder fala, ele continua guardado até você decidir."
                guard try armazem.resolver(pendencia: p.id, documento: documento, tentativas: p.tentativas, desfecho: .continua(motivo)) else {
                    throw Falha.concorrencia
                }
                continue
            }
            let palavras = try await reconhecedor.palavras(audio, inicio: Double(de) / taxa, fimDaFala: Double(ate - de) / taxa)
                .filter { $0.centro >= Double(p.inicio) / taxa && $0.centro < Double(p.fim) / taxa }
            let desfecho: ArmazemLancamento.DesfechoPendencia
            if palavras.isEmpty {
                let a = max(0, p.inicio - de), z = min(audio.count, p.fim - de)
                desfecho = semCobertura ? .continua("Parte do áudio foi perdida. Confira a lacuna na transcrição.")
                    : a < z && AtividadeDeVoz.tem(audio[a..<z])
                    ? .continua("Havia voz, mas o reconhecedor não devolveu texto em \(p.tentativas + 1) tentativas. O áudio continua guardado.")
                    : .semFala
            } else {
                var trechos: [TrechoNovo] = []
                for t in Costura.trechos(palavras, documento: documento, fonte: p.fonte, geracao: p.geracao) {
                    let editado = await preparar(t.texto)
                    let texto = editado.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? t.texto : editado
                    trechos.append(TrechoNovo(id: t.id, fonte: p.fonte, inicio: t.inicio, fim: t.fim, texto: texto, voz: p.fonte.vozAoVivo))
                }
                desfecho = .resolvida(trechos)
            }
            guard try armazem.resolver(pendencia: p.id, documento: documento, tentativas: p.tentativas, desfecho: desfecho) else {
                throw Falha.concorrencia
            }
        }
    }

    /// Fecha as costuras de vez no fim da reunião (nenhuma geração recebe mais áudio).
    public func encerrarCosturas() throws {
        for c in try armazem.costuras(documento: documento) where !c.fechada {
            try armazem.escrever { conexao in
                try conexao.rodar("UPDATE lt_costuras SET fechada = 1 WHERE documento = ? AND fonte = ? AND geracao = ? AND confirmado_ate >= ?",
                                  [.texto(documento), .texto(c.fonte.rawValue), .inteiro(Int64(c.geracao)), .inteiro(Int64(c.confirmadoAte))])
            }
        }
    }

    /// Identifica as pessoas da chamada sobre o áudio retido e grava a atribuição por ID de trecho.
    /// Só depois do commit o áudio resolvido é liberado. Falha mantém o áudio para nova tentativa.
    @discardableResult
    public func passadaFinal(_ identificador: IdentificadorDeFalantes?) async throws -> Bool {
        var blocos = try armazem.blocos(documento: documento)
        for i in blocos.indices where blocos[i].arquivo != nil && !blocos[i].audioLiberado && blocos[i].estado != .lacuna {
            do { _ = try disco.ler(blocos[i]) }
            catch {
                try armazem.marcarLacuna(blocos[i].id, motivo: error.localizedDescription)
                blocos[i].estado = .lacuna
            }
        }
        guard !blocos.contains(where: { $0.estado == .capturado || $0.estado == .pendente }),
              try !armazem.pendencias(documento: documento).contains(where: { $0.estado == "aberta" })
        else { throw Falha.textoIncompleto }
        let doc = try armazem.documento(documento)
        let daChamada = doc?.trechos.filter { $0.origem != "microfone" } ?? []
        var atribuicao: [String: String] = [:]
        var identificados = true
        if !daChamada.isEmpty {
            if let identificador {
                // Dentro da pasta privada da reunião (0700, arquivo 0600): sobra de uma saída no meio
                // sai na próxima abertura, junto com os temporários dos blocos.
                let wav = try disco.pasta(documento: documento).appendingPathComponent("\(DiscoDeBlocos.prefixoWAV)\(UUID().uuidString).wav")
                defer { try? FileManager.default.removeItem(at: wav) }
                do {
                    try disco.escreverWAV(blocos.filter { $0.id.fonte == .mac }, em: wav)
                    let faixas = try await identificador.faixas(wav: wav)
                    try Task.checkCancellation()
                    // Só rótulos provisórios: "chamada" ao vivo e "fluxo-N" de diários antigos. Voz de histórico migrado fica.
                    for t in try brutos(documento: documento) where t.fonte == .mac && (t.voz == "chamada" || t.voz.hasPrefix("fluxo-")) {
                        atribuicao[t.id] = AlinhamentoDeVoz.identificar(inicio: t.inicio, fim: t.fim, faixas: faixas)
                    }
                } catch is CancellationError { throw CancellationError() }
                catch { identificados = false }
            } else { identificados = false }
        }
        guard try armazem.gravarFalantes(documento: documento, atribuicao: identificados ? atribuicao : [:],
                                         identificados: identificados, liberarAudio: identificados) else { throw Falha.textoIncompleto }
        if identificados { limparArquivos() }
        return identificados
    }

    private func brutos(documento: String) throws -> [(id: String, fonte: FonteDeAudio, inicio: Double, fim: Double, voz: String)] {
        try armazem.ler { c in
            try c.consultar("SELECT id, fonte, inicio, fim, voz FROM lt_trechos WHERE documento = ? AND versao = ''", [.texto(documento)]) {
                ($0.texto(0), FonteDeAudio(rawValue: $0.texto(1)) ?? .mac, $0.real(2), $0.real(3), $0.texto(4))
            }
        }
    }

    /// Remove os arquivos já liberados no banco. Repetida na abertura se for interrompida.
    public func limparArquivos() {
        guard let lista = try? armazem.arquivosParaLimpar() else { return }
        for (id, arquivo) in lista where id.documento == documento {
            do {
                // Referência que não é um bloco desta reunião não é seguida: só ela sai do banco.
                if DiscoDeBlocos.arquivoValido(arquivo, documento: documento) { try disco.apagar(arquivo) }
                try armazem.esquecerArquivo(id)
            } catch {
                // A referência continua para a limpeza na próxima abertura.
                continue
            }
        }
    }
}

extension DiscoDeBlocos {
    /// Escreve a faixa inteira de uma fonte num WAV, bloco a bloco, com zeros nas lacunas.
    public func escreverWAV(_ blocos: [BlocoRegistrado], em url: URL) throws {
        let ordenados = blocos.filter { $0.estado != .descartado }.sorted { $0.inicio < $1.inicio }
        let total = ordenados.map(\.fim).max() ?? 0
        guard FileManager.default.createFile(atPath: url.path, contents: nil, attributes: [.posixPermissions: 0o600]) else {
            throw CocoaError(.fileWriteUnknown)
        }
        let saida = try FileHandle(forWritingTo: url)
        defer { try? saida.close() }
        var cabecalho = Data()
        func u32(_ v: UInt32) { withUnsafeBytes(of: v.littleEndian) { cabecalho.append(contentsOf: $0) } }
        func u16(_ v: UInt16) { withUnsafeBytes(of: v.littleEndian) { cabecalho.append(contentsOf: $0) } }
        let bytes = UInt32(total * 4)
        cabecalho.append(contentsOf: Array("RIFF".utf8)); u32(36 + bytes); cabecalho.append(contentsOf: Array("WAVE".utf8))
        cabecalho.append(contentsOf: Array("fmt ".utf8)); u32(16); u16(3); u16(1); u32(UInt32(taxaLancamento)); u32(UInt32(taxaLancamento * 4)); u16(4); u16(32)
        cabecalho.append(contentsOf: Array("data".utf8)); u32(bytes)
        try saida.write(contentsOf: cabecalho)
        var posicao = 0
        for b in ordenados where b.fim > posicao {
            if b.inicio > posicao { try saida.write(contentsOf: Data(count: (b.inicio - posicao) * 4)) }
            var amostras: [Float]
            if b.estado == .lacuna || b.arquivo == nil { amostras = [Float](repeating: 0, count: b.quantidade) }
            else { amostras = try ler(b) }
            let pular = max(0, posicao - b.inicio)
            if pular > 0 { amostras.removeFirst(min(pular, amostras.count)) }
            try saida.write(contentsOf: DiscoDeBlocos.dados(amostras))
            posicao = b.fim
        }
        if total > posicao { try saida.write(contentsOf: Data(count: (total - posicao) * 4)) }
    }
}
