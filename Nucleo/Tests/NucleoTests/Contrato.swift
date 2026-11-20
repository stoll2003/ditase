import XCTest
@testable import Nucleo

private struct CasoTexto: Decodable { let operacao, entrada, esperado: String }
private struct ParteAudio: Decodable { let fala: Bool; let segundos: Double }

func casosCompartilhados<T: Decodable>(_ arquivo: String, como: T.Type) throws -> T {
    var raiz = URL(fileURLWithPath: #filePath)
    for _ in 0..<4 { raiz.deleteLastPathComponent() }
    let url = raiz.appendingPathComponent("contrato/casos").appendingPathComponent(arquivo + ".json")
    return try JSONDecoder().decode(T.self, from: Data(contentsOf: url))
}

func conferirTexto(_ nome: String) throws {
    let casos = try casosCompartilhados("regras_fala", como: [String: [CasoTexto]].self)
    for caso in try XCTUnwrap(casos[nome]) {
        let saida = caso.operacao == "filtrar" ? RegrasFala.filtrar(caso.entrada) : RegrasFala.arrumarEspacos(caso.entrada)
        XCTAssertEqual(saida, caso.esperado, nome)
    }
}

func audioDoContrato(_ nome: String) throws -> [Float] {
    let casos = try casosCompartilhados("segmentador", como: [String: [ParteAudio]].self)
    return audio(try XCTUnwrap(casos[nome]).map { ($0.fala, $0.segundos) })
}

struct CasoFidelidade: Decodable { let original, modelo: String }
func casosFidelidade(_ nome: String) throws -> [CasoFidelidade] {
    let casos = try casosCompartilhados("fidelidade", como: [String: [CasoFidelidade]].self)
    return try XCTUnwrap(casos[nome])
}
