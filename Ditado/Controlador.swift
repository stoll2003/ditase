import AppKit
import Combine
import Foundation
import Nucleo

/// Amarra atalho, microfone, sessão de ditado, banco, colagem e pílula.
@MainActor
final class Controlador {
    enum Fase: String { case parado, gravando, processando }

    let banco: Banco
    let prefs: Preferencias
    let transcritor = TranscritorParakeet()
    let servidor: ServidorLLM
    let modeloIA = GerenciadorDeModelo()
    var modeloEmUso: String { modeloIA.modeloEmUso(escolhido: prefs.modelo) }
    lazy var lancamento = ServicoLancamento(controlador: self)
    let atalho = Atalho()
    let pilula = JanelaDaPilula()
    private let gravador = Gravador()
    private let ortografia = Ortografia()

    private(set) var fase: Fase = .parado
    private(set) var maosLivres = false
    private(set) var estadoDoReconhecedor: TranscritorParakeet.Estado = .parado
    private(set) var estadoDoLimpador: ServidorLLM.Estado = .desligado
    /// Avisa a interface: "estado", "historico" ou "dicionario".
    var aoMudar: ((String) -> Void)?

    private var sessao: SessaoDitado?
    private var ditado: Ditado?
    private var inicio = Date()
    private var bombeamento: Task<Void, Never>?
    private var relogio: Timer?
    private var ultimoNivel = Date.distantPast
    private var compartilhaMicrofone = false
    private var eventosLancamento = Set<AnyCancellable>()
    private var trabalhoNaPilula: String?

    init(banco: Banco) {
        self.banco = banco
        prefs = Preferencias(banco: banco)
        let instrucoes = Bundle.main.url(forResource: "prompt-limpeza", withExtension: "md", subdirectory: "Recursos").flatMap { try? String(contentsOf: $0, encoding: .utf8) }
            ?? Bundle.main.url(forResource: "prompt-limpeza", withExtension: "md").flatMap { try? String(contentsOf: $0, encoding: .utf8) } ?? ""
        servidor = ServidorLLM(instrucoes: instrucoes)
        if instrucoes.isEmpty { Registro.nota("ATENÇÃO: prompt de limpeza não encontrado no pacote") }
    }

    func ligar() {
        recuperarInterrompidos()
        limparAudiosAntigos()
        ReservaDeAudio.removerOrfas()

        atalho.tecla = prefs.atalho
        atalho.aoComecar = { [weak self] in self?.comecar(maosLivres: false) ?? false }
        atalho.aoTerminar = { [weak self] in self?.terminar() }
        atalho.aoCancelar = { [weak self] in self?.cancelar() }
        atalho.aoMaosLivres = { [weak self] in self?.entrarEmMaosLivres() }
        atalho.aoColarUltimo = { [weak self] in self?.colarUltimo() }
        atalho.aoInterromper = { [weak self] in self?.terminar(colar: false) }
        atalho.iniciar()

        pilula.modelo.sempreVisivel = prefs.pilulaSempreVisivel
        pilula.modelo.atalho = prefs.atalho.simbolo
        pilula.modelo.aoAlternar = { [weak self] in self?.alternarPelaInterface() }
        pilula.modelo.aoParar = { [weak self] in
            self?.atalho.encerradoPorFora()
            self?.terminar()
        }
        pilula.modelo.aoCancelar = { [weak self] in
            self?.atalho.encerradoPorFora()
            self?.cancelar()
        }
        pilula.mostrar(.ociosa)
        ligarLancamentoNaPilula()

        prefs.aoMudar = { [weak self] chave in
            guard let self else { return }
            switch chave {
            case "atalho":
                self.atalho.tecla = self.prefs.atalho
                self.pilula.modelo.atalho = self.prefs.atalho.simbolo
            case "pilulaSempreVisivel": self.pilula.modelo.sempreVisivel = self.prefs.pilulaSempreVisivel
            case "limpezaIA", "modelo":
                // Trocar de modelo ou desligar a limpeza libera o que estiver carregado; o novo sobe no próximo ditado.
                self.servidor.parar()
            default: break
            }
            self.aoMudar?("estado")
        }

        modeloIA.aoMudar = { [weak self] in self?.aoMudar?("estado") }
        modeloIA.aoConcluir = { [weak self] arquivo in self?.modeloInstalado(arquivo) }

        servidor.aoMudar = { [weak self] estado in
            Task { @MainActor in
                self?.estadoDoLimpador = estado
                self?.aoMudar?("estado")
            }
        }
        Task {
            await transcritor.observar { [weak self] estado in
                Task { @MainActor in
                    self?.estadoDoReconhecedor = estado
                    self?.aoMudar?("estado")
                }
            }
            // Carrega o reconhecedor já na abertura: o primeiro ditado não espera.
            do { try await transcritor.carregar() } catch { Registro.nota("reconhecedor falhou: \(error)") }
        }
        // O modelo de limpeza não é carregado na abertura: sobe no primeiro ditado e sai da memória
        // poucos minutos depois do último. Parado, o app não gasta processador nem bateria.
    }

    /// Download terminado e conferido: passa a usar o modelo e já o carrega, para o estado
    /// chegar a "Pronta" e o primeiro ditado não esperar.
    private func modeloInstalado(_ modelo: String) {
        guard prefs.limpezaIA, modeloEmUso == modelo else { return }
        Task.detached { [servidor] in await servidor.garantir(modelo: modelo) }
    }

    private func ligarLancamentoNaPilula() {
        let publicador = lancamento.processamento
        publicador.estados.compactMap { $0 }.sink { [weak self] estado in
            self?.projetarNaPilula(estado)
        }.store(in: &eventosLancamento)
        publicador.resultados.sink { [weak self] resultado in
            self?.encerrarNaPilula(resultado)
        }.store(in: &eventosLancamento)

        pilula.modelo.aoPausarReuniao = { [weak self] in
            self?.comandarReuniaoPelaPilula(encerrar: false)
        }
        pilula.modelo.aoEncerrarReuniao = { [weak self] in
            self?.comandarReuniaoPelaPilula(encerrar: true)
        }
    }

    private func projetarNaPilula(_ estado: EstadoProcessamento) {
        let captura = estado.reuniaoAtiva ? CapturaDaPilula(
            pausada: estado.fase == "pausada", segundos: Int(estado.decorrido ?? 0),
            nivel: Double(estado.nivel ?? 0), motivoSemAudio: estado.motivo
        ) : nil
        pilula.mostrarReuniao(captura)
        guard fase == .parado else { return }
        let evento = ProgressoDaPilula(id: estado.id, progresso: estado.progresso, etapa: estado.etapa.rotulo,
                                      estimativa: estado.estimativa, longo: estado.longo)
        if pilula.modelo.progresso != evento { pilula.modelo.receber(evento) }
        if !estado.reuniaoAtiva, fase == .parado, pilula.modelo.fase == .ociosa {
            trabalhoNaPilula = estado.id
            pilula.mostrar(.processando)
        }
    }

    private func encerrarNaPilula(_ resultado: ResultadoProcessamento) {
        let ocupavaPilula = trabalhoNaPilula == resultado.id
        trabalhoNaPilula = nil
        pilula.mostrarReuniao(nil)
        guard fase == .parado, var evento = pilula.modelo.progresso, evento.id == resultado.id else { return }
        switch resultado.resultado {
        case .concluido: evento.resultado = .concluido
        case .cancelado: evento.resultado = .cancelado
        case .erro: evento.resultado = .erro(resultado.motivo ?? "O processamento falhou.")
        }
        pilula.modelo.receber(evento)
        guard fase == .parado else { return }
        if resultado.resultado == .erro {
            pilula.avisar(resultado.motivo ?? "O processamento falhou.")
        } else if ocupavaPilula, pilula.modelo.fase == .processando {
            pilula.mostrar(.ociosa)
        }
    }

    func iniciarReuniaoPelaPilula() -> Bool {
        guard Gravador.permissao == "concedida", lancamento.audioDoMacDisponivel else { return false }
        do {
            try lancamento.iniciarReuniao(fonte: "macEMicrofone")
            return true
        } catch {
            if fase == .parado { pilula.avisar(error.localizedDescription) }
            return false
        }
    }

    private func comandarReuniaoPelaPilula(encerrar: Bool) {
        guard let id = lancamento.processamento.atual?.id, lancamento.processamento.atual?.reuniaoAtiva == true else { return }
        do {
            if encerrar { try lancamento.encerrar(id: id) }
            else { try lancamento.pausar(id: id) }
        } catch {
            if fase == .parado { pilula.avisar(error.localizedDescription) }
        }
    }

    func desligar() {
        if fase == .gravando { pararMicrofoneDoDitado() }
        eventosLancamento.removeAll()
        lancamento.desligar()
        servidor.parar()
    }

    private func aquecerLimpador() {
        let modelo = modeloEmUso
        Task.detached { [servidor] in await servidor.garantir(modelo: modelo) }
    }

    // MARK: - Ciclo do ditado

    @discardableResult
    func comecar(maosLivres: Bool) -> Bool {
        guard fase == .parado else { return false }
        guard Gravador.permissao == "concedida" else {
            Gravador.pedirPermissao { [weak self] _ in self?.aoMudar?("estado") }
            pilula.avisar("Permita o microfone nos Ajustes")
            return false
        }
        let app = NSWorkspace.shared.frontmostApplication
        var novo = Ditado(iniciadoEm: Date().timeIntervalSince1970, app: app?.bundleIdentifier ?? "", appNome: app?.localizedName ?? "", estado: "gravando", motor: transcritor.nome)
        do { novo.id = try banco.inserir(novo) } catch {
            Registro.nota("banco: \(error)")
            pilula.avisar("Erro ao abrir o histórico")
            return false
        }
        let id = novo.id
        let banco = banco
        let termos = (try? banco.termos()) ?? []
        let dicionario = AplicadorDicionario(termos: termos, ehPalavraComum: ortografia.verificador)
        let limpador = prefs.limpezaIA ? servidor.limpador(modelo: modeloEmUso) : nil
        if prefs.limpezaIA { aquecerLimpador() }
        let nova = SessaoDitado(transcritor: transcritor, limpador: limpador, dicionario: dicionario, aoTrecho: { ordem, bruto in
            do { try banco.gravarTrecho(ditado: id, ordem: ordem, bruto: bruto) } catch { Registro.nota("trecho \(ordem) do ditado \(id) não foi gravado: \(error)") }
        })

        let arquivo = prefs.guardarAudioDias > 0 ? Caminhos.audios.appendingPathComponent("\(id).wav") : nil
        let fluxo: AsyncStream<[Float]>
        do {
            if let compartilhado = lancamento.abrirDerivacao() {
                fluxo = compartilhado
                compartilhaMicrofone = true
            } else {
                fluxo = try gravador.iniciar(microfone: prefs.microfone, gravarEm: arquivo)
                compartilhaMicrofone = false
            }
        } catch {
            Registro.nota("microfone: \(error)")
            try? banco.apagarDitado(id: id)
            pilula.avisar("Microfone indisponível")
            return false
        }
        novo.audio = arquivo?.lastPathComponent ?? ""
        if !novo.audio.isEmpty { try? banco.atualizar(novo) }
        ditado = novo
        sessao = nova
        inicio = Date()
        self.maosLivres = maosLivres
        pilula.modelo.suspenderProgresso()
        trabalhoNaPilula = nil
        fase = .gravando
        let reservaCompartilhada = compartilhaMicrofone ? arquivo : nil
        bombeamento = Task.detached {
            let escritor = reservaCompartilhada.flatMap { try? EscritorLancamento($0) }
            for await bloco in fluxo {
                try? escritor?.gravar(bloco)
                await nova.alimentar(bloco)
            }
        }

        pilula.modelo.zerarNiveis()
        pilula.modelo.segundos = 0
        gravador.aoNivel = { [weak self] nivel in
            DispatchQueue.main.async {
                guard let self, self.fase == .gravando, Date().timeIntervalSince(self.ultimoNivel) > 0.05 else { return }
                self.ultimoNivel = Date()
                self.pilula.modelo.empurrar(nivel: nivel)
            }
        }
        // Pequena espera: se a tecla foi usada como modificador, a pílula nem aparece.
        DispatchQueue.main.asyncAfter(deadline: .now() + (maosLivres ? 0 : 0.12)) { [weak self] in
            guard let self, self.fase == .gravando, self.ditado?.id == id else { return }
            self.pilula.mostrar(self.maosLivres ? .maosLivres : .gravando)
        }
        relogio = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.acompanhar() }
        }
        tocar("Tink")
        aoMudar?("estado")
        return true
    }

    private func pararMicrofoneDoDitado() {
        if compartilhaMicrofone { lancamento.fecharDerivacao() }
        else { gravador.parar() }
        compartilhaMicrofone = false
    }

    func nivelDoMicrofoneCompartilhado(_ nivel: Float) {
        guard fase == .gravando, compartilhaMicrofone, Date().timeIntervalSince(ultimoNivel) > 0.05 else { return }
        ultimoNivel = Date()
        pilula.modelo.empurrar(nivel: nivel)
    }

    func entrarEmMaosLivres() {
        guard fase == .gravando else { return }
        maosLivres = true
        pilula.mostrar(.maosLivres)
        aoMudar?("estado")
    }

    /// Chamado pela interface: começa em mãos livres ou termina o que estiver gravando.
    func alternarPelaInterface() {
        switch fase {
        case .parado:
            if comecar(maosLivres: true) { atalho.iniciadoEmMaosLivres() }
        case .gravando:
            atalho.encerradoPorFora()
            terminar()
        case .processando:
            break
        }
    }

    private func acompanhar() {
        guard fase == .gravando, let sessao else { return }
        let decorrido = Date().timeIntervalSince(inicio)
        pilula.modelo.segundos = Int(decorrido)
        if decorrido >= prefs.limiteMinutos * 60 {
            Registro.nota("ditado encerrado pelo limite de \(prefs.limiteMinutos) min")
            atalho.encerradoPorFora()
            terminar()
            return
        }
        let limiteDeSilencio = prefs.pararEmSilencioMinutos * 60
        guard maosLivres, limiteDeSilencio > 0 else { return }
        Task {
            let silencio = await sessao.medidor.silencio
            if silencio >= limiteDeSilencio, self.fase == .gravando, self.sessao === sessao {
                Registro.nota("ditado encerrado por \(Int(silencio)) s de silêncio")
                self.atalho.encerradoPorFora()
                self.terminar()
            }
        }
    }

    func terminar(colar: Bool = true) {
        guard fase == .gravando, let sessao, var atual = ditado else { return }
        let soltou = Date()
        pararMicrofoneDoDitado()
        relogio?.invalidate()
        fase = .processando
        pilula.mostrar(.processando)
        aoMudar?("estado")
        let bomba = bombeamento
        Task {
            await bomba?.value
            let r = await sessao.finalizar()
            let latencia = Date().timeIntervalSince(soltou)
            atual.duracao = soltou.timeIntervalSince(self.inicio)
            atual.duracaoFala = r.duracaoFala
            atual.latencia = latencia
            atual.textoBruto = r.textoBruto
            atual.texto = r.texto
            atual.palavras = r.palavras
            atual.limpador = r.limpador
            atual.trocasDicionario = r.trocas.count
            self.concluir(atual, resultado: r, colar: colar)
        }
    }

    private func concluir(_ final: Ditado, resultado r: ResultadoDitado, colar: Bool) {
        var d = final
        defer {
            sessao = nil
            ditado = nil
            bombeamento = nil
            maosLivres = false
            fase = .parado
            aoMudar?("estado")
            aoMudar?("historico")
        }
        if r.texto.isEmpty {
            guard r.falhas > 0 else {
                // Silêncio de verdade: não há o que guardar.
                try? banco.apagarDitado(id: d.id)
                apagarAudio(d.audio)
                pilula.avisar("Não ouvi nada", por: 1.6)
                return
            }
            // O reconhecedor falhou. O áudio é a única reserva: fica guardado, e o histórico
            // mostra o ditado com a opção de transcrever de novo.
            d.estado = "falhou"
            do { try banco.atualizar(d) } catch { Registro.nota("ditado \(d.id): não gravou a falha: \(error)") }
            Registro.nota("ditado \(d.id): reconhecedor falhou em \(r.falhas) trecho(s); áudio \(d.audio.isEmpty ? "não guardado" : d.audio)")
            pilula.avisar(d.audio.isEmpty ? "Não consegui transcrever" : "Não transcrevi. O áudio ficou guardado", por: 3.5)
            return
        }
        d.estado = r.falhas > 0 ? "parcial" : "concluido"
        do {
            try banco.concluir(d)
        } catch {
            // Os trechos de reserva continuam no banco; o ditado é recuperado na próxima abertura.
            Registro.nota("ditado \(d.id): resultado não foi gravado, reserva mantida: \(error)")
        }
        try? banco.contarUso(termos: r.trocas.map(\.termoId))
        Registro.nota("ditado \(d.id): \(d.palavras) palavras, \(String(format: "%.1f", d.duracao)) s, espera \(String(format: "%.2f", d.latencia)) s, limpador \(d.limpador), recusas \(r.recusas), falhas \(r.falhas)")
        if !r.trocas.isEmpty { aoMudar?("dicionario") }
        guard colar else {
            pilula.avisar("Ditado guardado no histórico, sem colar", por: 3)
            return
        }
        let texto = prefs.espacoNoFim && !d.texto.hasSuffix("\n") ? d.texto + " " : d.texto
        switch Insersor.inserir(texto) {
        case .colado:
            if r.falhas > 0 { pilula.avisar("Parte do áudio não foi transcrita", por: 3.5) } else { pilula.mostrar(.ociosa) }
            tocar("Pop")
        case .soCopiado:
            pilula.avisar("Copiado. Cole com ⌘V", por: 3)
        }
    }

    /// Passa de novo pelo reconhecedor o áudio guardado de um ditado que falhou ou ficou parcial.
    func retranscrever(id: Int64) {
        guard fase == .parado, var d = try? banco.ditado(id: id), !d.audio.isEmpty else { return }
        let arquivo = Caminhos.audios.appendingPathComponent(d.audio)
        guard let amostras = try? ModoLinha.lerAudio(arquivo.path), !amostras.isEmpty else {
            pilula.avisar("O áudio desse ditado não existe mais", por: 3)
            return
        }
        let termos = (try? banco.termos()) ?? []
        let limpador = prefs.limpezaIA ? servidor.limpador(modelo: modeloEmUso) : nil
        let nova = SessaoDitado(transcritor: transcritor, limpador: limpador, dicionario: AplicadorDicionario(termos: termos, ehPalavraComum: ortografia.verificador))
        pilula.modelo.suspenderProgresso()
        trabalhoNaPilula = nil
        fase = .processando
        pilula.mostrar(.processando)
        aoMudar?("estado")
        Task {
            var i = 0
            while i < amostras.count {
                let fim = min(i + 16_000, amostras.count)
                await nova.alimentar(Array(amostras[i..<fim]))
                i = fim
            }
            let r = await nova.finalizar()
            if !r.texto.isEmpty {
                d.texto = r.texto
                d.textoBruto = r.textoBruto
                d.palavras = r.palavras
                d.duracaoFala = r.duracaoFala
                d.limpador = r.limpador
                d.estado = r.falhas > 0 ? "parcial" : "concluido"
                do { try self.banco.atualizar(d) } catch { Registro.nota("ditado \(d.id): nova transcrição não foi gravada: \(error)") }
                self.pilula.avisar(r.falhas > 0 ? "Parte do áudio não foi transcrita" : "Transcrito. Está no histórico", por: 3)
            } else {
                self.pilula.avisar("Ainda não consegui transcrever", por: 3)
            }
            self.fase = .parado
            self.aoMudar?("estado")
            self.aoMudar?("historico")
        }
    }

    func cancelar() {
        guard fase == .gravando, let sessao, let atual = ditado else { return }
        pararMicrofoneDoDitado()
        relogio?.invalidate()
        bombeamento?.cancel()
        Task { await sessao.cancelar() }
        try? banco.apagarDitado(id: atual.id)
        apagarAudio(atual.audio)
        self.sessao = nil
        ditado = nil
        bombeamento = nil
        maosLivres = false
        fase = .parado
        pilula.mostrar(.ociosa)
        aoMudar?("estado")
    }

    func colarUltimo() {
        guard fase == .parado, let ultimo = try? banco.ultimoConcluido(), !ultimo.texto.isEmpty else {
            pilula.avisar("Nenhum ditado para colar", por: 1.6)
            return
        }
        if Insersor.inserir(ultimo.texto) == .soCopiado { pilula.avisar("Copiado. Cole com ⌘V", por: 3) }
    }

    // MARK: - Manutenção

    /// Ditado que ficou "gravando" porque o app fechou no meio: o que já estava transcrito volta ao histórico.
    private func recuperarInterrompidos() {
        let interrompidos: [Ditado]
        do { interrompidos = try banco.interrompidos() } catch {
            Registro.nota("recuperação: não li os ditados interrompidos: \(error)")
            return
        }
        for var d in interrompidos {
            // Erro de leitura não é "sem trechos": nesse caso nada é apagado.
            let trechos: [String]
            do { trechos = try banco.trechos(ditado: d.id) } catch {
                Registro.nota("recuperação: não li os trechos do ditado \(d.id): \(error)")
                continue
            }
            if trechos.isEmpty {
                // Sem texto transcrito. Se há áudio guardado, o ditado fica como falho para nova tentativa.
                if !d.audio.isEmpty, FileManager.default.fileExists(atPath: Caminhos.audios.appendingPathComponent(d.audio).path) {
                    d.estado = "falhou"
                    try? banco.atualizar(d)
                } else {
                    try? banco.apagarDitado(id: d.id)
                }
                continue
            }
            d.textoBruto = trechos.joined(separator: " ")
            d.texto = RegrasFala.filtrar(d.textoBruto)
            d.palavras = Palavras.contar(d.texto)
            d.estado = "recuperado"
            d.limpador = "regras"
            do {
                try banco.concluir(d)
                Registro.nota("ditado \(d.id) recuperado com \(d.palavras) palavras")
            } catch {
                Registro.nota("recuperação: ditado \(d.id) não foi gravado, reserva mantida: \(error)")
            }
        }
    }

    private func limparAudiosAntigos() {
        let dias = prefs.guardarAudioDias
        let limite = Date().addingTimeInterval(-Double(dias) * 86_400)
        let arquivos = (try? FileManager.default.contentsOfDirectory(at: Caminhos.audios, includingPropertiesForKeys: [.contentModificationDateKey])) ?? []
        for arquivo in arquivos {
            let data = (try? arquivo.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate ?? .distantPast
            if dias == 0 || data < limite { try? FileManager.default.removeItem(at: arquivo) }
        }
    }

    private func apagarAudio(_ nome: String) {
        guard !nome.isEmpty else { return }
        try? FileManager.default.removeItem(at: Caminhos.audios.appendingPathComponent(nome))
    }

    private func tocar(_ nome: String) {
        guard prefs.sons, let som = NSSound(named: nome) else { return }
        som.volume = 0.25
        som.play()
    }
}

/// Diz se uma palavra existe em português, para o dicionário não trocar palavra comum por termo parecido.
/// O corretor do sistema só é consultado na thread principal; as respostas ficam em cache.
final class Ortografia: @unchecked Sendable {
    private let trava = NSLock()
    private var cache: [String: Bool] = [:]
    private let disponivel: Bool

    init() { disponivel = NSSpellChecker.shared.availableLanguages.contains("pt_BR") }

    var verificador: (@Sendable (String) -> Bool)? {
        guard disponivel else { return nil }
        return { [self] palavra in ehComum(palavra) }
    }

    private func ehComum(_ palavra: String) -> Bool {
        let chave = palavra.lowercased()
        if let sabida = trava.withLock({ cache[chave] }) { return sabida }
        let consulta = {
            NSSpellChecker.shared.checkSpelling(of: chave, startingAt: 0, language: "pt_BR", wrap: false, inSpellDocumentWithTag: 0, wordCount: nil).location == NSNotFound
        }
        let resposta = Thread.isMainThread ? consulta() : DispatchQueue.main.sync(execute: consulta)
        trava.withLock { cache[chave] = resposta }
        return resposta
    }
}
