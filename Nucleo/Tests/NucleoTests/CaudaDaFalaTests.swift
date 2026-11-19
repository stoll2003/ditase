import XCTest
@testable import Nucleo

final class CaudaDaFalaTests: XCTestCase {
    func testIsolaUltimaFalaAposPausaComESemSilencioFinal() throws {
        let anterior = [Float](repeating: 0.1, count: 16_000)
        let pausa = [Float](repeating: 0, count: 8_000)
        let fala = [Float](repeating: 0.1, count: 32_000)
        for fim in [fala, fala + pausa] {
            let cauda = try XCTUnwrap(CaudaDaFala.localizar(anterior + pausa + fim))
            XCTAssertEqual(cauda.inicio, 24_000)
            XCTAssertEqual(cauda.fim, 56_000)
        }
    }

    func testSilencioEEstaloNaoEnviamFalsoEnunciado() {
        var audio = [Float](repeating: 0, count: 48_000)
        XCTAssertNil(CaudaDaFala.localizar(audio))
        audio[47_999] = 1
        XCTAssertNil(CaudaDaFala.localizar(audio))
    }

    func testPausaEntrePalavrasNaoRecortaFrase() throws {
        let palavra = [Float](repeating: 0.1, count: 8_000)
        let intervalo = [Float](repeating: 0, count: 3_200)
        let cauda = try XCTUnwrap(CaudaDaFala.localizar(palavra + intervalo + palavra))
        XCTAssertEqual(cauda.inicio, 0)
        XCTAssertEqual(cauda.fim, 19_200)
    }
}
