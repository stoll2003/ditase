import XCTest
@testable import Nucleo

final class BancoTests: XCTestCase {
    func novo() throws -> Banco { try Banco(caminho: ":memory:") }

    func testCicloDoDitado() throws {
        let b = try novo()
        var d = Ditado(iniciadoEm: 1000, app: "com.exemplo", appNome: "Exemplo")
        d.id = try b.inserir(d)
        XCTAssertEqual(try b.interrompidos().map(\.id), [d.id])
        XCTAssertTrue(try b.historico().isEmpty, "ditado em gravação não aparece no histórico")
        d.estado = "concluido"
        d.texto = "Olá, mundo 100%."
        d.palavras = 3
        d.duracao = 2
        try b.atualizar(d)
        XCTAssertEqual(try b.historico(), [d])
        XCTAssertEqual(try b.historico(busca: "mundo").count, 1)
        XCTAssertEqual(try b.historico(busca: "100%").count, 1)
        XCTAssertEqual(try b.historico(busca: "%").count, 1)
        XCTAssertEqual(try b.historico(busca: "_").count, 0, "curinga do LIKE é tratado como texto")
        XCTAssertEqual(try b.ultimoConcluido()?.id, d.id)
        try b.apagarDitado(id: d.id)
        XCTAssertTrue(try b.historico().isEmpty)
    }

    func testTrechosSomemComODitado() throws {
        let b = try novo()
        let id = try b.inserir(Ditado(iniciadoEm: 1))
        try b.gravarTrecho(ditado: id, ordem: 1, bruto: "dois")
        try b.gravarTrecho(ditado: id, ordem: 0, bruto: "um")
        XCTAssertEqual(try b.trechos(ditado: id), ["um", "dois"])
        try b.apagarDitado(id: id)
        XCTAssertTrue(try b.trechos(ditado: id).isEmpty)
    }

    func testConcluirSoApagaTrechosSeOResultadoFoiGravado() throws {
        let b = try novo()
        var d = Ditado(iniciadoEm: 1)
        d.id = try b.inserir(d)
        try b.gravarTrecho(ditado: d.id, ordem: 0, bruto: "texto de reserva")
        // Ditado que não existe mais: a gravação falha e a reserva do outro não é tocada.
        var fantasma = d
        fantasma.id = 9999
        XCTAssertThrowsError(try b.concluir(fantasma))
        XCTAssertEqual(try b.trechos(ditado: d.id), ["texto de reserva"])
        d.estado = "concluido"
        d.texto = "Texto de reserva."
        d.palavras = 3
        try b.concluir(d)
        XCTAssertTrue(try b.trechos(ditado: d.id).isEmpty)
        XCTAssertEqual(try b.historico().first?.texto, "Texto de reserva.")
    }

    func testChaveExternaEvitaImportarDuasVezes() throws {
        let b = try novo()
        try b.inserir(Ditado(iniciadoEm: 1, estado: "concluido", origem: "wispr"), chaveExterna: "wispr:abc")
        XCTAssertTrue(try b.existe(chaveExterna: "wispr:abc"))
        XCTAssertFalse(try b.existe(chaveExterna: "wispr:xyz"))
        XCTAssertThrowsError(try b.inserir(Ditado(iniciadoEm: 2), chaveExterna: "wispr:abc"))
    }

    func testDicionario() throws {
        let b = try novo()
        let claude = try b.salvar(termo: Termo(termo: " Claude ", ouvidoComo: ["Cloud", "cloud", "", "Claude"]))
        XCTAssertEqual(claude.termo, "Claude")
        XCTAssertEqual(claude.ouvidoComo, ["Cloud"])
        // Mesmo termo de novo junta as formas em vez de duplicar.
        let de_novo = try b.salvar(termo: Termo(termo: "claude", ouvidoComo: ["Clode"]))
        XCTAssertEqual(de_novo.id, claude.id)
        XCTAssertEqual(try b.termos().count, 1)
        XCTAssertEqual(try b.termos()[0].ouvidoComo, ["Cloud", "Clode"])
        try b.contarUso(termos: [claude.id, claude.id])
        XCTAssertEqual(try b.termos()[0].usos, 2)
        var editado = try b.termos()[0]
        editado.ouvidoComo = []
        try b.salvar(termo: editado)
        XCTAssertEqual(try b.termos()[0].ouvidoComo, [])
        try b.apagarTermo(id: claude.id)
        XCTAssertTrue(try b.termos().isEmpty)
        XCTAssertThrowsError(try b.salvar(termo: Termo(termo: "   ")))
    }

    func testConfig() throws {
        let b = try novo()
        XCTAssertNil(b.config("x"))
        try b.definir(config: "x", valor: "1")
        try b.definir(config: "x", valor: "2")
        XCTAssertEqual(b.config("x"), "2")
    }

    func testPersisteEmArquivo() throws {
        let caminho = NSTemporaryDirectory() + "ditado-teste-\(UUID().uuidString)/banco.sqlite"
        defer { try? FileManager.default.removeItem(atPath: (caminho as NSString).deletingLastPathComponent) }
        do {
            let b = try Banco(caminho: caminho)
            try b.inserir(Ditado(iniciadoEm: 5, palavras: 2, texto: "oi oi", estado: "concluido"))
        }
        XCTAssertEqual(try Banco(caminho: caminho).historico().first?.texto, "oi oi")
    }
}

final class EstatisticasTests: XCTestCase {
    var cal: Calendar = {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "America/Sao_Paulo")!
        return c
    }()

    func data(_ dia: Int, _ hora: Int) -> Double {
        cal.date(from: DateComponents(year: 2026, month: 11, day: dia, hour: hora))!.timeIntervalSince1970
    }

    func testCalculo() {
        let agora = Date(timeIntervalSince1970: data(15, 12))
        let ditados = [
            Ditado(iniciadoEm: data(15, 9), duracao: 70, duracaoFala: 60, palavras: 160, app: "a", appNome: "App A", estado: "concluido", latencia: 1.0, trocasDicionario: 2),
            Ditado(iniciadoEm: data(14, 22), duracao: 30, duracaoFala: 30, palavras: 60, app: "b", appNome: "App B", estado: "concluido", latencia: 3.0),
            Ditado(iniciadoEm: data(13, 9), duracao: 60, duracaoFala: 60, palavras: 180, app: "a", appNome: "App A", estado: "concluido", latencia: 2.0),
            Ditado(iniciadoEm: data(10, 9), duracao: 120, duracaoFala: 0, palavras: 300, app: "a", estado: "concluido", origem: "wispr"),
            Ditado(iniciadoEm: data(1, 9), duracao: 1, duracaoFala: 1, palavras: 0, app: "a", estado: "concluido"),
        ]
        let e = Estatisticas.calcular(ditados, agora: agora, calendario: cal, diasNoGrafico: 7)
        XCTAssertEqual(e.totalPalavras, 700)
        XCTAssertEqual(e.totalDitados, 4)
        XCTAssertEqual(e.minutosFalados, 4.5, accuracy: 0.001)
        XCTAssertEqual(e.ppmMedio, 700 / 4.5, accuracy: 0.01)
        XCTAssertEqual(e.minutosEconomizados, 700 / 40 - 4.5, accuracy: 0.001)
        XCTAssertEqual(e.diasSeguidos, 3)
        XCTAssertEqual(e.maiorSequencia, 3)
        XCTAssertEqual(e.diasAtivos, 4)
        XCTAssertEqual(e.palavrasHoje, 160)
        XCTAssertEqual(e.palavrasNaSemana, 700)
        XCTAssertEqual(e.latenciaMediana, 2.0)
        XCTAssertEqual(e.trocasDicionario, 2)
        XCTAssertEqual(e.porDia.count, 7)
        XCTAssertEqual(e.porDia.last?.palavras, 160)
        XCTAssertEqual(e.porDia.map(\.dia).last, "2026-11-15")
        XCTAssertEqual(e.porHora[9], 640)
        XCTAssertEqual(e.porHora[22], 60)
        XCTAssertEqual(e.porDiaDaSemana[6], 160, "15/11/2026 é domingo")
        XCTAssertEqual(e.porApp.first?.nome, "App A")
        XCTAssertEqual(e.porApp.first?.palavras, 640)
        XCTAssertEqual(e.maiorDitadoPalavras, 300)
    }

    func testSequenciaContaAPartirDeOntemSeHojeVazio() {
        let agora = Date(timeIntervalSince1970: data(15, 8))
        let ditados = [14, 13].map { Ditado(iniciadoEm: data($0, 10), duracao: 10, duracaoFala: 10, palavras: 20, estado: "concluido") }
        XCTAssertEqual(Estatisticas.calcular(ditados, agora: agora, calendario: cal).diasSeguidos, 2)
    }

    func testVazio() {
        let e = Estatisticas.calcular([], calendario: cal)
        XCTAssertEqual(e.totalPalavras, 0)
        XCTAssertEqual(e.ppmMedio, 0)
        XCTAssertEqual(e.porDia.count, 30)
    }
}
