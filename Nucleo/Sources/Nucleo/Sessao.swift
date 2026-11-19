import Foundation

public struct ResultadoDitado: Sendable {
    /// Texto final, pronto para colar.
    public var texto = ""
    /// O que o reconhecedor ouviu, sem nenhuma arrumação.
    public var textoBruto = ""
    public var palavras = 0
    public var trocas: [Troca] = []
    /// Nome do modelo que arrumou o texto, ou "regras" quando nenhum trecho passou por modelo.
    public var limpador = "regras"
    /// Trechos em que a saída do modelo foi recusada e valeu o texto das regras.
    public var recusas = 0
    /// Trechos que o reconhecedor não conseguiu transcrever.
    public var falhas = 0
    public var duracaoFala = 0.0
}

/// Um ditado em andamento, do primeiro som ao texto final.
///
/// O áudio entra aos poucos por `alimentar`. O segmentador corta nas pausas, cada trecho é
/// transcrito na hora e os trechos já transcritos vão sendo arrumados pelo modelo de linguagem
/// enquanto a pessoa continua falando. `finalizar` só espera o que ainda não ficou pronto.
public actor SessaoDitado {
    public struct Config: Sendable {
        /// Pausa, em segundos, que abre parágrafo novo.
        public var pausaDeParagrafo = 2.2
        /// Um parágrafo só é aberto pela pausa se o atual já tiver este tanto de palavras.
        public var palavrasMinimasNoParagrafo = 35
        /// Palavras do texto já pronto mandadas ao modelo como contexto.
        public var palavrasDeContexto = 40
        /// Abaixo disso o trecho não vai ao modelo.
        public var minimoParaModelo = 4
        /// Tempo máximo de uma chamada ao modelo: base mais um tanto por palavra.
        public var esperaBase = 8.0
        public var esperaPorPalavra = 0.35
        /// Trechos à espera do reconhecedor mantidos em memória; os demais vão para o disco.
        public var trechosEmMemoria = 12
        public var segmentador = Segmentador.Config()
        public init() {}
    }

    struct Trecho: Sendable {
        let ordem: Int
        var bruto: String
        let novoParagrafo: Bool
        /// Começa no meio de uma frase (o trecho anterior foi cortado por tamanho).
        let continua: Bool
    }

    struct Parte: Sendable {
        let texto: String
        let novoParagrafo: Bool
        /// Continua a frase da parte anterior.
        var continua = false
    }

    private let transcritor: Transcritor
    private let limpador: Limpador?
    private let dicionario: AplicadorDicionario
    private let config: Config
    private let aoTrecho: (@Sendable (Int, String) -> Void)?

    private var segmentador: Segmentador
    private var entrada: AsyncStream<FilaDeTrechos.Item>.Continuation?
    private let fila: FilaDeTrechos
    private var transcrevendo: Task<Void, Never>?
    private var limpando: Task<Void, Never>?
    private var pendentes: [Trecho] = []
    private var partes: [Parte] = []
    private var brutos: [String] = []
    private var proximaOrdem = 0
    private var palavrasNoParagrafo = 0
    private var anteriorCortado = false
    private var anteriorAberto = false
    private var resultado = ResultadoDitado()
    private var cancelada = false

    public init(transcritor: Transcritor, limpador: Limpador?, dicionario: AplicadorDicionario, config: Config = Config(),
                aoTrecho: (@Sendable (Int, String) -> Void)? = nil) {
        self.transcritor = transcritor
        self.limpador = limpador
        self.dicionario = dicionario
        self.config = config
        self.aoTrecho = aoTrecho
        segmentador = Segmentador(config: config.segmentador)
        fila = FilaDeTrechos(limiteEmMemoria: config.trechosEmMemoria)
    }

    /// Nível do microfone (0 a 1) e segundos seguidos de silêncio.
    public var medidor: (nivel: Float, silencio: Double, fala: Double) {
        (segmentador.nivel, segmentador.silencioCorrido, segmentador.falaTotal)
    }

    /// Palavras já transcritas, para mostrar andamento em ditado longo.
    public var palavrasAteAgora: Int { brutos.reduce(0) { $0 + Palavras.contar($1) } }

    public func alimentar(_ amostras: [Float]) {
        guard !cancelada else { return }
        iniciarSePreciso()
        for s in segmentador.alimentar(amostras) { entrada?.yield(fila.guardar(s)) }
    }

    public func cancelar() {
        cancelada = true
        entrada?.finish()
        transcrevendo?.cancel()
        limpando?.cancel()
        fila.limpar()
    }

    /// Fecha o ditado e devolve o texto. Espera só o que ainda não terminou.
    public func finalizar() async -> ResultadoDitado {
        iniciarSePreciso()
        if let ultimo = segmentador.finalizar() { entrada?.yield(fila.guardar(ultimo)) }
        entrada?.finish()
        await transcrevendo?.value
        // A limpeza pode ter recebido trechos enquanto a última rodada corria.
        while !cancelada, limpando != nil || !pendentes.isEmpty {
            if let t = limpando { await t.value } else { dispararLimpeza() }
        }
        fila.limpar()
        return montarResultado()
    }

    /// Trechos em memória à espera do reconhecedor.
    var trechosEsperandoEmMemoria: Int { fila.esperandoEmMemoria }

    // MARK: - Transcrição

    private func iniciarSePreciso() {
        guard transcrevendo == nil else { return }
        let (fluxo, continuacao) = AsyncStream<FilaDeTrechos.Item>.makeStream(bufferingPolicy: .unbounded)
        entrada = continuacao
        transcrevendo = Task { [weak self] in
            for await item in fluxo {
                guard let self, !Task.isCancelled else { return }
                await self.transcrever(item)
            }
        }
    }

    private func transcrever(_ item: FilaDeTrechos.Item) async {
        let segmento = item.segmento
        let amostras = fila.retirar(item)
        let texto: String
        do {
            guard !amostras.isEmpty else { throw ErroDeLimpeza("trecho ilegível na fila em disco") }
            texto = try await transcritor.transcrever(amostras).trimmingCharacters(in: .whitespacesAndNewlines)
        } catch {
            resultado.falhas += 1
            return
        }
        guard !texto.isEmpty, !cancelada else { return }
        // A frase continua neste trecho quando o anterior foi cortado por tamanho ou quando o
        // reconhecedor o deixou em aberto (reticências, vírgula ou nenhuma pontuação no fim).
        let continua = anteriorCortado || anteriorAberto
        var bruto = continua ? minusculaInicial(texto) : texto
        if segmento.cortadoNoMeio, bruto.hasSuffix("."), !bruto.hasSuffix("..") { bruto.removeLast() }
        anteriorCortado = segmento.cortadoNoMeio
        anteriorAberto = Self.terminaEmAberto(bruto)

        let palavras = Palavras.contar(bruto)
        let novoParagrafo = !brutos.isEmpty && !continua && segmento.pausaAntes >= config.pausaDeParagrafo && palavrasNoParagrafo >= config.palavrasMinimasNoParagrafo
        palavrasNoParagrafo = novoParagrafo ? palavras : palavrasNoParagrafo + palavras
        let trecho = Trecho(ordem: proximaOrdem, bruto: bruto, novoParagrafo: novoParagrafo, continua: continua)
        proximaOrdem += 1
        brutos.append(bruto)
        pendentes.append(trecho)
        aoTrecho?(trecho.ordem, bruto)
        dispararLimpeza()
    }

    /// Tira a maiúscula que o reconhecedor e as regras põem no começo de todo trecho,
    /// quando o trecho continua a frase anterior. Termo do dicionário mantém a grafia.
    private func minusculaInicial(_ texto: String) -> String {
        guard let primeira = texto.first, primeira.isUppercase,
              let palavra = Palavras.ocorrencias(texto).first.map({ String($0.texto) }),
              !dicionario.vocabulario.contains(where: { $0.hasPrefix(palavra) }),
              // Sigla ("PDF", "IA") fica como está.
              !(palavra.count > 1 && palavra == palavra.uppercased())
        else { return texto }
        return primeira.lowercased() + texto.dropFirst()
    }

    // MARK: - Limpeza

    /// Arruma tudo que estiver pendente, um grupo por vez. O grupo é o que se acumulou
    /// enquanto a rodada anterior corria, até a próxima quebra de parágrafo.
    private func dispararLimpeza() {
        guard limpando == nil, !pendentes.isEmpty, !cancelada else { return }
        limpando = Task { [weak self] in
            while let self, !Task.isCancelled, let grupo = await self.proximoGrupo() {
                let parte = await self.limpar(grupo)
                await self.guardar(parte)
            }
            await self?.terminouLimpeza()
        }
    }

    private func terminouLimpeza() {
        limpando = nil
        // Um trecho pode ter chegado entre o fim da última rodada e este ponto.
        dispararLimpeza()
    }

    private func proximoGrupo() -> [Trecho]? {
        guard !pendentes.isEmpty, !cancelada else { return nil }
        var fim = 1
        while fim < pendentes.count, !pendentes[fim].novoParagrafo { fim += 1 }
        let grupo = Array(pendentes[..<fim])
        pendentes.removeFirst(fim)
        return grupo
    }

    private func guardar(_ parte: Parte) { partes.append(parte) }

    private func limpar(_ grupo: [Trecho]) async -> Parte {
        var junto = ""
        for t in grupo { junto = Self.emendar(junto, t.bruto, continua: t.continua) }
        let (comDicionario, trocas) = dicionario.aplicar(RegrasFala.filtrar(junto))
        resultado.trocas.append(contentsOf: trocas)
        let base = RegrasFala.arrumarEspacos(comDicionario)
        let novoParagrafo = grupo.first?.novoParagrafo ?? false
        let continua = grupo.first?.continua ?? false
        let palavras = Palavras.contar(base)
        func parte(_ texto: String) -> Parte { Parte(texto: continua ? minusculaInicial(texto) : texto, novoParagrafo: novoParagrafo, continua: continua) }
        guard let limpador, palavras >= config.minimoParaModelo else { return parte(base) }

        let anterior = Palavras.cauda(textoAteAgora(), palavras: config.palavrasDeContexto)
        let pedido = PedidoLimpeza(transcricao: base, anterior: anterior, vocabulario: dicionario.vocabulario)
        let espera = config.esperaBase + Double(palavras) * config.esperaPorPalavra
        guard let saida = await comPrazo(espera, { try await limpador.limpar(pedido) }) else {
            resultado.recusas += 1
            return parte(base)
        }
        if Guarda.avaliar(original: base, limpo: saida) != nil {
            resultado.recusas += 1
            return parte(base)
        }
        // Do modelo só entram pontuação, parágrafos e cortes seguros; as palavras são as do falante.
        let fundido = Fidelidade.fundir(original: base, modelo: saida)
        // O modelo pode desfazer uma grafia do dicionário; reaplica.
        let (final, _) = dicionario.aplicar(fundido)
        resultado.limpador = limpador.nome
        return parte(RegrasFala.arrumarEspacos(final))
    }

    /// Devolve o resultado da operação ou `nil` quando o prazo vence. Não espera a operação
    /// terminar depois do prazo: ela é cancelada e, se não cooperar, termina sozinha em segundo plano.
    /// (Um TaskGroup esperaria todos os filhos, e um modelo travado travaria o ditado junto.)
    private func comPrazo(_ segundos: Double, _ operacao: @escaping @Sendable () async throws -> String) async -> String? {
        let corrida = Corrida()
        return await withCheckedContinuation { (continuacao: CheckedContinuation<String?, Never>) in
            let trabalho = Task {
                let saida = try? await operacao()
                if corrida.vencer() { continuacao.resume(returning: saida) }
            }
            Task {
                try? await Task.sleep(nanoseconds: UInt64(segundos * 1_000_000_000))
                if corrida.vencer() {
                    trabalho.cancel()
                    continuacao.resume(returning: nil)
                }
            }
        }
    }

    // MARK: - Montagem

    private func textoAteAgora() -> String { Self.juntar(partes) }

    static func terminaEmAberto(_ texto: String) -> Bool {
        guard let ultimo = texto.last else { return false }
        if texto.hasSuffix("...") || ultimo == "…" || ultimo == "," || ultimo == ";" || ultimo == ":" || ultimo == "-" { return true }
        return ultimo.isLetter || ultimo.isNumber
    }

    /// Junta dois pedaços. Quando o segundo continua a frase, sai o ponto ou as reticências
    /// que o reconhecedor (ou o modelo) pôs no fim do primeiro só por causa da pausa.
    static func emendar(_ a: String, _ b: String, continua: Bool) -> String {
        if a.isEmpty { return b }
        if b.isEmpty { return a }
        if a.hasSuffix("\n") { return a + b }
        guard continua else { return a + " " + b }
        var inicio = a
        while let ultimo = inicio.last, ultimo == "." || ultimo == "…" { inicio.removeLast() }
        return inicio + " " + b
    }

    static func juntar(_ partes: [Parte]) -> String {
        var texto = ""
        for p in partes where !p.texto.isEmpty {
            if texto.isEmpty { texto = p.texto }
            else if p.novoParagrafo { texto += "\n\n" + p.texto }
            else { texto = emendar(texto, p.texto, continua: p.continua) }
        }
        return texto
    }

    private func montarResultado() -> ResultadoDitado {
        var r = resultado
        r.texto = Self.juntar(partes)
        r.textoBruto = brutos.joined(separator: " ")
        r.palavras = Palavras.contar(r.texto)
        r.duracaoFala = segmentador.falaTotal
        return r
    }
}

/// Garante que só o primeiro a terminar responde.
final class Corrida: @unchecked Sendable {
    private let trava = NSLock()
    private var decidida = false
    func vencer() -> Bool {
        trava.withLock {
            if decidida { return false }
            decidida = true
            return true
        }
    }
}

/// Fila dos trechos que esperam o reconhecedor. Enquanto ele acompanha a fala, tudo fica em
/// memória; se ele atrasa ou trava, os trechos além de um limite vão para arquivos temporários.
/// Assim a memória não cresce com a duração do ditado e nenhum áudio é descartado.
final class FilaDeTrechos: @unchecked Sendable {
    enum Onde { case memoria([Float]), disco(URL, Int) }
    struct Item {
        let segmento: Segmento
        let onde: Onde
    }

    private let trava = NSLock()
    private var emMemoria = 0
    private let limiteEmMemoria: Int
    private let pasta: URL
    private var criouPasta = false

    init(limiteEmMemoria: Int) {
        self.limiteEmMemoria = limiteEmMemoria
        pasta = FileManager.default.temporaryDirectory.appendingPathComponent("ditado-fila-\(UUID().uuidString)", isDirectory: true)
    }

    /// Quantos trechos estão em memória à espera. Usado nos testes.
    var esperandoEmMemoria: Int { trava.withLock { emMemoria } }

    func guardar(_ s: Segmento) -> Item {
        let cabe = trava.withLock { () -> Bool in
            if emMemoria < limiteEmMemoria {
                emMemoria += 1
                return true
            }
            return false
        }
        if cabe { return Item(segmento: s, onde: .memoria(s.amostras)) }
        do {
            if !criouPasta {
                try FileManager.default.createDirectory(at: pasta, withIntermediateDirectories: true)
                criouPasta = true
            }
            let arquivo = pasta.appendingPathComponent("\(s.inicio).f32")
            try s.amostras.withUnsafeBufferPointer { Data(buffer: $0) }.write(to: arquivo)
            let leve = Segmento(inicio: s.inicio, amostras: [], pausaAntes: s.pausaAntes, cortadoNoMeio: s.cortadoNoMeio)
            return Item(segmento: leve, onde: .disco(arquivo, s.amostras.count))
        } catch {
            // Sem disco, fica em memória: perder áudio é pior que gastar memória.
            trava.withLock { emMemoria += 1 }
            return Item(segmento: s, onde: .memoria(s.amostras))
        }
    }

    func retirar(_ item: Item) -> [Float] {
        switch item.onde {
        case .memoria(let amostras):
            trava.withLock { emMemoria -= 1 }
            return amostras
        case .disco(let arquivo, let quantas):
            defer { try? FileManager.default.removeItem(at: arquivo) }
            guard let dados = try? Data(contentsOf: arquivo) else { return [] }
            return dados.withUnsafeBytes { Array($0.bindMemory(to: Float.self).prefix(quantas)) }
        }
    }

    func limpar() { if criouPasta { try? FileManager.default.removeItem(at: pasta) } }
    deinit { limpar() }
}
