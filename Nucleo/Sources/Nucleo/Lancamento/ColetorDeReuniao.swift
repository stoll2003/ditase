import Foundation

/// Recebe as amostras das fontes com o carimbo da captura e sela blocos duráveis.
/// Cada fonte tem fila e estado próprios: uma falha numa não derruba a outra.
/// Fora do ator principal: a escrita em disco e no banco acontece aqui.
public actor ColetorDeReuniao {
    public typealias AoSelar = @Sendable (_ fonte: FonteDeAudio, _ geracao: Int, _ fim: Int) async -> Void

    private struct Fonte {
        var geracao: Int
        var seq: Int
        var cursor: CursorDeFonte
        var buffer: [Float] = []
        var inicioBuffer: Int
    }

    public let documento: String
    private let armazem: ArmazemLancamento
    private let disco: DiscoDeBlocos
    private let tamanhoBloco: Int
    private var linha: LinhaDoTempo
    private var fontes: [FonteDeAudio: Fonte] = [:]
    private var aoSelar: AoSelar?
    private var encerrado = false

    public init(documento: String, armazem: ArmazemLancamento, disco: DiscoDeBlocos, inicio: Double,
                tamanhoBloco: Int = taxaLancamento) {
        self.documento = documento
        self.armazem = armazem
        self.disco = disco
        self.tamanhoBloco = tamanhoBloco
        linha = LinhaDoTempo(inicio: inicio)
    }

    public func definirAoSelar(_ acao: @escaping AoSelar) { aoSelar = acao }

    public var linhaDoTempo: LinhaDoTempo { linha }

    /// Abre a fonte (ou uma geração nova dela, depois de troca de dispositivo ou reinício).
    /// A geração anterior é selada no ponto em que parou.
    @discardableResult
    public func abrir(_ fonte: FonteDeAudio, em t: Double) throws -> Int {
        let geracao: Int
        let abertura = linha.indice(t) ?? fontes[fonte].map { $0.cursor.proximo } ?? 0
        if var anterior = fontes[fonte] {
            try selar(fonte)
            anterior = fontes[fonte]!
            // O intervalo entre a geração que parou e a nova fica registrado como lacuna visível.
            if abertura - anterior.cursor.proximo > CursorDeFonte.tolerancia {
                let id = IdBloco(documento: documento, fonte: fonte, geracao: anterior.geracao, seq: anterior.seq)
                try armazem.registrar(BlocoRegistrado(id: id, inicio: anterior.cursor.proximo, quantidade: abertura - anterior.cursor.proximo,
                                                      arquivo: nil, tamanho: nil, soma: nil, estado: .lacuna,
                                                      erro: "A fonte foi reiniciada; sem áudio neste intervalo."))
            }
            geracao = anterior.geracao + 1
        } else { geracao = 0 }
        try armazem.abrirCostura(documento: documento, fonte: fonte, geracao: geracao, inicio: abertura)
        fontes[fonte] = Fonte(geracao: geracao, seq: 0, cursor: CursorDeFonte(abertura: abertura), inicioBuffer: abertura)
        return geracao
    }

    public func geracao(_ fonte: FonteDeAudio) -> Int? { fontes[fonte]?.geracao }

    /// Coloca um buffer na linha de tempo. Lacunas viram blocos sem áudio, visíveis na transcrição.
    /// Sem pontos de suspensão: dois buffers da mesma fonte nunca se intercalam.
    public func receber(_ fonte: FonteDeAudio, tempo t: Double, amostras: [Float]) throws {
        guard !encerrado, var f = fontes[fonte], !amostras.isEmpty else { return }
        let eventos = f.cursor.receber(tempo: t, quantidade: amostras.count, linha: linha)
        fontes[fonte]?.cursor = f.cursor
        for evento in eventos {
            switch evento {
            case .lacuna(let faixa):
                try selar(fonte)
                guard var g = fontes[fonte] else { return }
                let id = IdBloco(documento: documento, fonte: fonte, geracao: g.geracao, seq: g.seq)
                try armazem.registrar(BlocoRegistrado(id: id, inicio: faixa.lowerBound, quantidade: faixa.count, arquivo: nil, tamanho: nil,
                                                      soma: nil, estado: .lacuna, erro: "A fonte não entregou áudio neste intervalo."))
                g.seq += 1
                g.inicioBuffer = faixa.upperBound
                fontes[fonte] = g
            case .amostras(let origem, let quantidade, let indice):
                guard var g = fontes[fonte] else { return }
                if g.buffer.isEmpty { g.inicioBuffer = indice }
                g.buffer += amostras[origem..<(origem + quantidade)]
                fontes[fonte] = g
                while let atual = fontes[fonte], atual.buffer.count >= tamanhoBloco {
                    try selar(fonte, quantidade: tamanhoBloco)
                }
            }
        }
    }

    /// Sela o que estiver em memória de uma fonte (ou só o primeiro bloco cheio): grava de forma
    /// durável, registra e só então avisa o processador.
    private func selar(_ fonte: FonteDeAudio, quantidade: Int? = nil) throws {
        guard var f = fontes[fonte], !f.buffer.isEmpty else { return }
        let n = min(quantidade ?? f.buffer.count, f.buffer.count)
        if let livre = disco.espacoLivre(), livre < DiscoDeBlocos.espacoMinimo { throw DiscoDeBlocos.Falha.espaco }
        let id = IdBloco(documento: documento, fonte: fonte, geracao: f.geracao, seq: f.seq)
        let bloco = try disco.gravar(id, inicio: f.inicioBuffer, amostras: Array(f.buffer.prefix(n)))
        try armazem.registrar(bloco)
        f.buffer.removeFirst(n)
        f.seq += 1
        f.inicioBuffer += n
        fontes[fonte] = f
        if let aoSelar {
            let geracao = f.geracao, fim = bloco.fim
            Task { await aoSelar(fonte, geracao, fim) }
        }
    }

    /// Pausa: as fontes fecham no marco e o que estava em memória é selado.
    public func pausar(em t: Double) throws {
        linha.pausar(em: t)
        try selarTudo()
    }

    public func retomar(em t: Double) { linha.retomar(em: t) }

    /// Marca o fim antes de drenar os buffers entregues pela captura.
    public func marcarFim(em t: Double) { linha.pausar(em: t) }

    /// Fim da captura: fecha no marco, sela e ignora callbacks posteriores.
    public func encerrar(em t: Double) throws {
        linha.pausar(em: t)
        try selarTudo()
        encerrado = true
    }

    public func selarTudo() throws {
        var erro: Error?
        for fonte in FonteDeAudio.allCases where fontes[fonte] != nil {
            do { try selar(fonte) } catch { erro = erro ?? error }
        }
        if let erro { throw erro }
    }

    /// Amostras ainda só em memória: não garantidas se o processo terminar agora.
    public var amostrasEmMemoria: Int { fontes.values.reduce(0) { $0 + $1.buffer.count } }

    public var fimAtual: Int { fontes.values.map { $0.cursor.proximo }.max() ?? 0 }
}
