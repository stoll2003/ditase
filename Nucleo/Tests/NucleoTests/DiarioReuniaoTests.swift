import Foundation
import XCTest
@testable import Nucleo

final class DiarioReuniaoTests: XCTestCase {
    private func comDiario(_ bloco: (DiarioReuniao, TranscricaoLonga, URL) throws -> Void) throws {
        let raiz = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: raiz) }
        let doc = TranscricaoLonga(nome: "Reunião sintética", fase: "gravando", fonte: "mac")
        try bloco(DiarioReuniao(raiz: raiz, documento: doc), doc, raiz)
    }

    func testReabreAmostrasETrechosEmTodasAsFases() throws {
        for fase in ["gravando", "pausada", "finalizando"] {
            try comDiario { escrita, original, raiz in
                var doc = original
                doc.fase = fase
                try escrita.salvar(doc)
                try escrita.salvarAudio(inicio: 0, sistema: [0.25], microfone: [])
                try escrita.salvarAudio(inicio: 0, sistema: [0.25, 0.5], microfone: [])
                try escrita.salvarAudio(inicio: 2, sistema: [0.75], microfone: [])
                let trecho = TrechoLongo(inicio: 1, fim: 2, voz: "a", texto: "fala sintética")
                try escrita.salvarTrechos(inicio: 0, trechos: [trecho])

                let outroProcesso = try XCTUnwrap(DiarioReuniao.listar(raiz: raiz).first)
                XCTAssertEqual(try outroProcesso.documento().fase, fase)
                XCTAssertEqual(try outroProcesso.audios().map(\.inicio), [0, 2])
                XCTAssertEqual(try outroProcesso.audios().flatMap { try DiarioReuniao.audio(em: $0.url).sistema }, [0.25, 0.5, 0.75])
                XCTAssertEqual(try outroProcesso.provisoria().trechos, [trecho])
            }
        }
    }

    func testReservaSoSaiAposCommit() throws {
        try comDiario { diario, original, raiz in
            var doc = original
            doc.fase = "pronta"
            try diario.salvarAudio(inicio: 0, sistema: [0.3], microfone: [0.4])
            XCTAssertThrowsError(try diario.concluir(doc, persistir: { _ in throw CocoaError(.fileWriteOutOfSpace) },
                                                     reler: { doc }))
            XCTAssertEqual(try DiarioReuniao.listar(raiz: raiz).count, 1)
            let banco = try Banco(caminho: raiz.appendingPathComponent("teste.sqlite").path)
            XCTAssertTrue(try diario.concluir(doc, persistir: { try banco.salvar(transcricao: $0) },
                                              reler: { try banco.transcricao(id: doc.id) }))
            XCTAssertEqual(try banco.transcricao(id: doc.id), doc)
            XCTAssertTrue(try DiarioReuniao.listar(raiz: raiz).isEmpty)
        }
    }
}
