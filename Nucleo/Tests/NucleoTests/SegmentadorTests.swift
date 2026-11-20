import XCTest
@testable import Nucleo

/// Áudio sintético: tom para "fala", zeros para silêncio.
func audio(_ partes: [(fala: Bool, segundos: Double)]) -> [Float] {
    var saida: [Float] = []
    var fase = 0.0
    for p in partes {
        let n = Int(p.segundos * 16_000)
        for _ in 0..<n {
            fase += 2 * Double.pi * 220 / 16_000
            saida.append(p.fala ? Float(sin(fase)) * 0.2 : 0)
        }
    }
    return saida
}

func alimentarEmBlocos(_ s: inout Segmentador, _ amostras: [Float], bloco: Int = 1024) -> [Segmento] {
    var saida: [Segmento] = []
    var i = 0
    while i < amostras.count {
        let fim = min(i + bloco, amostras.count)
        saida += s.alimentar(Array(amostras[i..<fim]))
        i = fim
    }
    if let ultimo = s.finalizar() { saida.append(ultimo) }
    return saida
}

final class SegmentadorTests: XCTestCase {
    func testSilencioNaoGeraTrecho() throws {
        var s = Segmentador()
        XCTAssertTrue(alimentarEmBlocos(&s, try audioDoContrato("testSilencioNaoGeraTrecho")).isEmpty)
    }

    func testCliqueCurtoEhDescartado() throws {
        var s = Segmentador()
        XCTAssertTrue(alimentarEmBlocos(&s, try audioDoContrato("testCliqueCurtoEhDescartado")).isEmpty)
    }

    func testFalaCurtaViraUmTrechoComMargem() throws {
        var s = Segmentador()
        let trechos = alimentarEmBlocos(&s, try audioDoContrato("testFalaCurtaViraUmTrechoComMargem"))
        XCTAssertEqual(trechos.count, 1)
        XCTAssertEqual(trechos[0].duracao, 2.4, accuracy: 0.25)
        XCTAssertEqual(trechos[0].pausaAntes, 1.0, accuracy: 0.1)
        XCTAssertFalse(trechos[0].cortadoNoMeio)
    }

    func testPausaCurtaNaoCortaTrechoPequeno() throws {
        var s = Segmentador()
        let trechos = alimentarEmBlocos(&s, try audioDoContrato("testPausaCurtaNaoCortaTrechoPequeno"))
        XCTAssertEqual(trechos.count, 1)
    }

    func testPausaCortaDepoisDoMinimo() throws {
        var s = Segmentador()
        let trechos = alimentarEmBlocos(&s, try audioDoContrato("testPausaCortaDepoisDoMinimo"))
        XCTAssertEqual(trechos.count, 2)
        XCTAssertEqual(trechos[0].duracao, 6.2, accuracy: 0.3)
    }

    func testPausaLongaCortaSempreEInformaAPausa() throws {
        var s = Segmentador()
        let trechos = alimentarEmBlocos(&s, try audioDoContrato("testPausaLongaCortaSempreEInformaAPausa"))
        XCTAssertEqual(trechos.count, 2)
        XCTAssertEqual(trechos[1].pausaAntes, 3.0, accuracy: 0.15)
    }

    func testFalaSemPausaCortaNoLimiteSemPerderAmostra() throws {
        var s = Segmentador()
        let entrada = try audioDoContrato("testFalaSemPausaCortaNoLimiteSemPerderAmostra")
        let trechos = alimentarEmBlocos(&s, entrada)
        XCTAssertGreaterThanOrEqual(trechos.count, 3)
        XCTAssertTrue(trechos.allSatisfy { $0.duracao <= 13.6 })
        XCTAssertTrue(trechos.dropLast().allSatisfy(\.cortadoNoMeio))
        // Fala contínua: nenhum pedaço some entre um trecho e o próximo.
        XCTAssertEqual(trechos.reduce(0) { $0 + $1.amostras.count }, entrada.count)
        for (a, b) in zip(trechos, trechos.dropFirst()) { XCTAssertEqual(a.inicio + a.amostras.count, b.inicio) }
    }

    func testCorteForcadoProcuraOPontoQuieto() throws {
        var s = Segmentador()
        // Pausa de 0,3 s aos 11 s: curta demais para fechar, mas é onde o corte forçado deve cair.
        let trechos = alimentarEmBlocos(&s, try audioDoContrato("testCorteForcadoProcuraOPontoQuieto"))
        XCTAssertEqual(trechos.count, 2)
        XCTAssertEqual(trechos[0].duracao, 11.15, accuracy: 0.3)
    }

    func testUmaHoraDeFalaNaoAcumulaMemoriaNemPerdeTrecho() throws {
        var s = Segmentador()
        var total = 0
        let ciclo = try audioDoContrato("testUmaHoraDeFalaNaoAcumulaMemoriaNemPerdeTrecho")
        for _ in 0..<410 { // cerca de uma hora
            total += s.alimentar(ciclo).count
        }
        if s.finalizar() != nil { total += 1 }
        XCTAssertEqual(total, 410)
        XCTAssertEqual(s.falaTotal, 410 * 8, accuracy: 60)
    }

    func testVozBaixaAindaEhFala() throws {
        var s = Segmentador()
        let baixo = try audioDoContrato("testVozBaixaAindaEhFala").map { $0 * 0.05 }
        XCTAssertEqual(alimentarEmBlocos(&s, baixo).count, 1)
    }
}
