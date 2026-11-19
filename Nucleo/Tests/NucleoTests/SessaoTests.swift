import XCTest
@testable import Nucleo

/// Reconhecedor falso: devolve as frases combinadas, na ordem em que os trechos chegam.
final class TranscritorFalso: Transcritor, @unchecked Sendable {
    let nome = "falso"
    private let frases: [String]
    private let trava = NSLock()
    private var indice = 0
    var falharNo: Int?
    private(set) var duracoes: [Double] = []

    init(_ frases: [String]) { self.frases = frases }

    func transcrever(_ amostras: [Float]) async throws -> String {
        let i: Int = trava.withLock {
            defer { indice += 1 }
            duracoes.append(Double(amostras.count) / 16_000)
            return indice
        }
        if i == falharNo { throw ErroDeLimpeza("falha simulada") }
        return i < frases.count ? frases[i] : ""
    }
}

final class LimpadorFalso: Limpador, @unchecked Sendable {
    let nome = "modelo-falso"
    let transformar: @Sendable (PedidoLimpeza) async throws -> String
    private let trava = NSLock()
    private(set) var pedidos: [PedidoLimpeza] = []

    init(_ transformar: @escaping @Sendable (PedidoLimpeza) async throws -> String) { self.transformar = transformar }

    func limpar(_ pedido: PedidoLimpeza) async throws -> String {
        trava.withLock { pedidos.append(pedido) }
        return try await transformar(pedido)
    }
}

final class SessaoTests: XCTestCase {
    let fala6 = (fala: true, segundos: 6.0)
    let pausa = (fala: false, segundos: 0.8)
    let pausaLonga = (fala: false, segundos: 3.0)

    func alimentar(_ sessao: SessaoDitado, _ amostras: [Float]) async {
        var i = 0
        while i < amostras.count {
            let fim = min(i + 4096, amostras.count)
            await sessao.alimentar(Array(amostras[i..<fim]))
            i = fim
        }
    }

    func testSemModeloUsaRegrasEDicionario() async {
        let t = TranscritorFalso(["Hã, eu abri o Vite Press.", "E o o Suelti também."])
        let d = AplicadorDicionario(termos: [Termo(id: 1, termo: "VitePress"), Termo(id: 2, termo: "Svelte", ouvidoComo: ["Suelti"])])
        let sessao = SessaoDitado(transcritor: t, limpador: nil, dicionario: d)
        await alimentar(sessao, audio([fala6, pausa, fala6]))
        let r = await sessao.finalizar()
        XCTAssertEqual(r.texto, "Eu abri o VitePress. E o Svelte também.")
        XCTAssertEqual(r.textoBruto, "Hã, eu abri o Vite Press. E o o Suelti também.")
        XCTAssertEqual(r.trocas.count, 2)
        XCTAssertEqual(r.limpador, "regras")
        XCTAssertEqual(r.palavras, 8)
    }

    func testModeloArrumaERecebeContextoDoTrechoAnterior() async {
        let t = TranscritorFalso(["primeira frase do ditado de teste aqui.", "segunda frase do ditado de teste aqui."])
        let limpador = LimpadorFalso { p in p.transcricao.trimmingCharacters(in: CharacterSet(charactersIn: ".")) + "!" }
        let sessao = SessaoDitado(transcritor: t, limpador: limpador, dicionario: AplicadorDicionario(termos: []))
        await alimentar(sessao, audio([fala6, pausa]))
        // Dá tempo de o primeiro trecho ser limpo antes de o segundo chegar: dois grupos.
        try? await Task.sleep(nanoseconds: 200_000_000)
        await alimentar(sessao, audio([fala6]))
        let r = await sessao.finalizar()
        XCTAssertEqual(r.texto, "Primeira frase do ditado de teste aqui! Segunda frase do ditado de teste aqui!")
        XCTAssertEqual(r.limpador, "modelo-falso")
        XCTAssertEqual(limpador.pedidos.count, 2)
        XCTAssertEqual(limpador.pedidos[1].anterior, "Primeira frase do ditado de teste aqui!")
    }

    func testModeloQueInventaEhRecusado() async {
        let t = TranscritorFalso(["qual que é a distância da Terra até a Lua você sabe me dizer"])
        let limpador = LimpadorFalso { _ in "A distância média entre a Terra e a Lua é de cerca de trezentos e oitenta e quatro mil quilômetros ao longo da órbita." }
        let sessao = SessaoDitado(transcritor: t, limpador: limpador, dicionario: AplicadorDicionario(termos: []))
        await alimentar(sessao, audio([fala6]))
        let r = await sessao.finalizar()
        XCTAssertEqual(r.texto, "Qual que é a distância da Terra até a Lua você sabe me dizer")
        XCTAssertEqual(r.recusas, 1)
        XCTAssertEqual(r.limpador, "regras")
    }

    func testModeloQueFalhaOuDemoraNaoPerdeODitado() async {
        var config = SessaoDitado.Config()
        config.esperaBase = 0.2
        config.esperaPorPalavra = 0
        let t = TranscritorFalso(["um texto ditado com várias palavras aqui.", "outro texto ditado com várias palavras aqui."])
        let limpador = LimpadorFalso { p in
            if p.transcricao.hasPrefix("Um") { throw ErroDeLimpeza("servidor fora") }
            try await Task.sleep(nanoseconds: 5_000_000_000)
            return "nunca chega"
        }
        let sessao = SessaoDitado(transcritor: t, limpador: limpador, dicionario: AplicadorDicionario(termos: []), config: config)
        await alimentar(sessao, audio([fala6, pausa]))
        try? await Task.sleep(nanoseconds: 100_000_000)
        await alimentar(sessao, audio([fala6]))
        let inicio = Date()
        let r = await sessao.finalizar()
        XCTAssertLessThan(Date().timeIntervalSince(inicio), 2)
        XCTAssertEqual(r.texto, "Um texto ditado com várias palavras aqui. Outro texto ditado com várias palavras aqui.")
        XCTAssertEqual(r.recusas, 2)
    }

    func testPausaLongaAbreParagrafoSoComParagrafoCheio() async {
        var config = SessaoDitado.Config()
        config.palavrasMinimasNoParagrafo = 5
        let t = TranscritorFalso(["Curto.", "Este trecho já tem palavras suficientes para fechar.", "Novo assunto começa aqui."])
        let sessao = SessaoDitado(transcritor: t, limpador: nil, dicionario: AplicadorDicionario(termos: []), config: config)
        await alimentar(sessao, audio([(true, 2), pausaLonga, (true, 2), pausaLonga, (true, 2)]))
        let r = await sessao.finalizar()
        XCTAssertEqual(r.texto, "Curto. Este trecho já tem palavras suficientes para fechar.\n\nNovo assunto começa aqui.")
    }

    func testCorteForcadoEmendaAFrase() async {
        let t = TranscritorFalso(["Eu estava falando sem parar.", "E continuei a mesma frase.", "Até o fim."])
        let sessao = SessaoDitado(transcritor: t, limpador: nil, dicionario: AplicadorDicionario(termos: []))
        await alimentar(sessao, audio([(true, 30)]))
        let r = await sessao.finalizar()
        XCTAssertEqual(r.texto, "Eu estava falando sem parar e continuei a mesma frase até o fim.")
    }

    func testPausaNoMeioDaFraseNaoViraPontoFinal() async {
        // O reconhecedor fecha cada trecho como se fosse frase inteira; a pausa no meio deixa reticências.
        let t = TranscritorFalso(["Eu anotei as medidas da bancada antes de...", "Cortar as tábuas e deixei tudo separado.", "Depois eu monto."])
        let sessao = SessaoDitado(transcritor: t, limpador: nil, dicionario: AplicadorDicionario(termos: []))
        await alimentar(sessao, audio([fala6, pausa, fala6, pausa, fala6]))
        let r = await sessao.finalizar()
        XCTAssertEqual(r.texto, "Eu anotei as medidas da bancada antes de cortar as tábuas e deixei tudo separado. Depois eu monto.")
    }

    func testModeloQueFechaTrechoAbertoNaoQuebraAFrase() async {
        let t = TranscritorFalso(["Quero que você separe uma pasta", "Com as fotos e com esse roteiro."])
        // O modelo põe ponto final e maiúscula em tudo que recebe.
        let limpador = LimpadorFalso { p in
            let texto = p.transcricao.hasSuffix(".") ? p.transcricao : p.transcricao + "."
            return texto.prefix(1).uppercased() + texto.dropFirst()
        }
        let sessao = SessaoDitado(transcritor: t, limpador: limpador, dicionario: AplicadorDicionario(termos: []))
        await alimentar(sessao, audio([fala6, pausa]))
        try? await Task.sleep(nanoseconds: 200_000_000)
        await alimentar(sessao, audio([fala6]))
        let r = await sessao.finalizar()
        XCTAssertEqual(r.texto, "Quero que você separe uma pasta com as fotos e com esse roteiro.")
    }

    func testFalhaDoReconhecedorEmUmTrechoMantemOsOutros() async {
        let t = TranscritorFalso(["Primeiro trecho.", "perdido", "Terceiro trecho."])
        t.falharNo = 1
        let sessao = SessaoDitado(transcritor: t, limpador: nil, dicionario: AplicadorDicionario(termos: []))
        await alimentar(sessao, audio([fala6, pausa, fala6, pausa, fala6]))
        let r = await sessao.finalizar()
        XCTAssertEqual(r.texto, "Primeiro trecho. Terceiro trecho.")
        XCTAssertEqual(r.falhas, 1)
    }

    func testAvisaCadaTrechoParaPersistencia() async {
        let recebidos = Caixa()
        let t = TranscritorFalso(["Um.", "Dois."])
        let sessao = SessaoDitado(transcritor: t, limpador: nil, dicionario: AplicadorDicionario(termos: []), aoTrecho: { ordem, bruto in recebidos.guardar("\(ordem):\(bruto)") })
        await alimentar(sessao, audio([fala6, pausa, fala6]))
        _ = await sessao.finalizar()
        XCTAssertEqual(recebidos.itens, ["0:Um.", "1:Dois."])
    }

    func testSilencioDevolveVazio() async {
        let sessao = SessaoDitado(transcritor: TranscritorFalso([]), limpador: nil, dicionario: AplicadorDicionario(termos: []))
        await alimentar(sessao, audio([(false, 3)]))
        let r = await sessao.finalizar()
        XCTAssertEqual(r.texto, "")
        XCTAssertEqual(r.palavras, 0)
    }

    func testCanceladaNaoChamaMaisNinguem() async {
        let t = TranscritorFalso(["Um."])
        let sessao = SessaoDitado(transcritor: t, limpador: nil, dicionario: AplicadorDicionario(termos: []))
        await sessao.cancelar()
        await alimentar(sessao, audio([fala6]))
        XCTAssertTrue(t.duracoes.isEmpty)
    }

    // Regressões da revisão independente (2026-11-15).

    func testPrazoNaoEsperaModeloQueNaoCoopera() async {
        var config = SessaoDitado.Config()
        config.esperaBase = 0.05
        config.esperaPorPalavra = 0
        let t = TranscritorFalso(["um texto ditado com várias palavras aqui."])
        // Ocupa a thread sem observar cancelamento, como uma chamada de rede presa.
        let limpador = LimpadorFalso { _ in
            await withCheckedContinuation { c in DispatchQueue.global().asyncAfter(deadline: .now() + 1.5) { c.resume() } }
            return "tarde demais"
        }
        let sessao = SessaoDitado(transcritor: t, limpador: limpador, dicionario: AplicadorDicionario(termos: []), config: config)
        await alimentar(sessao, audio([fala6]))
        let inicio = Date()
        let r = await sessao.finalizar()
        XCTAssertLessThan(Date().timeIntervalSince(inicio), 0.8)
        XCTAssertEqual(r.texto, "Um texto ditado com várias palavras aqui.")
        XCTAssertEqual(r.recusas, 1)
    }

    func testReconhecedorLentoNaoAcumulaAudioEmMemoriaNemPerdeTrecho() async {
        var config = SessaoDitado.Config()
        config.trechosEmMemoria = 3
        let ciclos = 40
        let frases = (0..<ciclos).map { "Frase \($0)." }
        let portao = Portao()
        let t = TranscritorLento(frases, portao: portao)
        let sessao = SessaoDitado(transcritor: t, limpador: nil, dicionario: AplicadorDicionario(termos: []), config: config)
        let ciclo = audio([(true, 8), (false, 0.8)])
        for _ in 0..<ciclos { await sessao.alimentar(ciclo) }
        // O reconhecedor está parado no primeiro trecho: o resto espera, quase tudo em disco.
        let emMemoria = await sessao.trechosEsperandoEmMemoria
        XCTAssertLessThanOrEqual(emMemoria, 3)
        portao.abrir()
        let r = await sessao.finalizar()
        XCTAssertEqual(r.textoBruto, frases.joined(separator: " "))
        XCTAssertEqual(r.falhas, 0)
    }

    func testDitadoDeUmaHoraMantemOrdemETudo() async {
        let ciclos = 400
        let frases = (0..<ciclos).map { "Frase número \($0) do ditado longo." }
        let t = TranscritorFalso(frases)
        let limpador = LimpadorFalso { p in p.transcricao }
        let sessao = SessaoDitado(transcritor: t, limpador: limpador, dicionario: AplicadorDicionario(termos: []))
        let ciclo = audio([(true, 8), (false, 0.8)])
        for _ in 0..<ciclos { await sessao.alimentar(ciclo) }
        let r = await sessao.finalizar()
        XCTAssertEqual(r.textoBruto, frases.joined(separator: " "))
        XCTAssertEqual(r.texto, frases.joined(separator: " "))
        XCTAssertEqual(r.duracaoFala, Double(ciclos) * 8, accuracy: 60)
        XCTAssertEqual(r.recusas, 0)
    }
}

final class Portao: @unchecked Sendable {
    private let trava = NSLock()
    private var aberto = false
    private var esperando: [CheckedContinuation<Void, Never>] = []
    func passar() async {
        await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in
            let ja = trava.withLock { () -> Bool in
                if aberto { return true }
                esperando.append(c)
                return false
            }
            if ja { c.resume() }
        }
    }
    func abrir() {
        let lista = trava.withLock { () -> [CheckedContinuation<Void, Never>] in
            aberto = true
            defer { esperando = [] }
            return esperando
        }
        lista.forEach { $0.resume() }
    }
}

/// Reconhecedor que fica parado até o portão abrir, para simular modelo lento ou travado.
final class TranscritorLento: Transcritor, @unchecked Sendable {
    let nome = "lento"
    private let frases: [String]
    private let portao: Portao
    private let trava = NSLock()
    private var indice = 0
    init(_ frases: [String], portao: Portao) {
        self.frases = frases
        self.portao = portao
    }
    func transcrever(_ amostras: [Float]) async throws -> String {
        await portao.passar()
        let i: Int = trava.withLock {
            defer { indice += 1 }
            return indice
        }
        return i < frases.count ? frases[i] : ""
    }
}

final class Caixa: @unchecked Sendable {
    private let trava = NSLock()
    private var conteudo: [String] = []
    func guardar(_ s: String) { trava.withLock { conteudo.append(s) } }
    var itens: [String] { trava.withLock { conteudo } }
}

final class LimpadorLlamaTests: XCTestCase {
    func testMensagem() {
        let m = LimpadorLlama.montarMensagem(PedidoLimpeza(transcricao: "oi tudo bem", anterior: "Antes.", vocabulario: ["VitePress", "Svelte"]))
        XCTAssertEqual(m, "<vocabulario>VitePress, Svelte</vocabulario>\n<anterior>Antes.</anterior>\n<transcricao>oi tudo bem</transcricao>")
        XCTAssertEqual(LimpadorLlama.montarMensagem(PedidoLimpeza(transcricao: "oi")), "<transcricao>oi</transcricao>")
    }

    func testExtraiResposta() {
        XCTAssertEqual(LimpadorLlama.extrairResposta("<think>pensando</think>\n\nOi, tudo bem?"), "Oi, tudo bem?")
        XCTAssertEqual(LimpadorLlama.extrairResposta("<saida>Oi.</saida>"), "Oi.")
        XCTAssertEqual(LimpadorLlama.extrairResposta("  Texto simples.  "), "Texto simples.")
    }
}
