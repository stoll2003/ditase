import Foundation
import XCTest
@testable import Nucleo

final class TranscricaoLongaTests: XCTestCase {
    func testAlinhaPelaMaiorSobreposicaoEAceitaIncerteza() {
        let vozes = [FaixaDeVoz(voz: "a", inicio: 0, fim: 2), FaixaDeVoz(voz: "b", inicio: 1, fim: 3)]
        XCTAssertEqual(AlinhamentoDeVoz.identificar(inicio: 0, fim: 0.5, faixas: vozes), "a")
        XCTAssertEqual(AlinhamentoDeVoz.identificar(inicio: 1.2, fim: 1.6, faixas: vozes), "desconhecida")
        XCTAssertEqual(AlinhamentoDeVoz.identificar(inicio: 2.4, fim: 2.9, faixas: vozes), "b")
        XCTAssertEqual(AlinhamentoDeVoz.identificar(inicio: 5, fim: 6, faixas: vozes), "desconhecida")
    }

    func testVizinhancaDeJanelasNaoDuplicaPalavraENaoPerdeSobreposicaoDeFontes() {
        XCTAssertFalse(AlinhamentoDeVoz.pertence(inicio: 2.8, fim: 3.2, a: 0..<3))
        XCTAssertTrue(AlinhamentoDeVoz.pertence(inicio: 2.8, fim: 3.2, a: 3..<6))
        let a = TrechoLongo(inicio: 4, fim: 5, voz: "mac", texto: "Um")
        let b = TrechoLongo(inicio: 3, fim: 5, voz: "voce", texto: "Dois")
        XCTAssertEqual(AlinhamentoDeVoz.juntar([a], [b]), [b, a])
    }

    func testPassadaFinalMantemEdicaoManualDoTrechoEDoNome() {
        var doc = TranscricaoLonga(nome: "Teste", fase: "gravando", fonte: "mac")
        var primeiro = TrechoLongo(inicio: 0, fim: 1, voz: "aoVivo-1", texto: "Primeiro", parcial: true)
        var segundo = TrechoLongo(inicio: 2, fim: 4, voz: "aoVivo-2", texto: "Segundo", parcial: true)
        segundo.vozManual = true
        doc.trechos = [primeiro, segundo]
        doc.atualizarVozes()
        XCTAssertTrue(doc.renomear(voz: "aoVivo-1", nome: "Joana"))
        primeiro.voz = "offline-0"
        segundo.voz = "offline-1"
        primeiro.parcial = false
        segundo.parcial = false
        doc.aplicarPassadaFinal([primeiro, segundo])
        XCTAssertEqual(doc.vozes.first?.nome, "Joana")
        XCTAssertEqual(doc.trechos.map(\.voz), ["offline-0", "aoVivo-2"])
        XCTAssertFalse(doc.trechos.contains { $0.parcial })
    }

    func testUneNomesIguaisEPreservaUniaoNaPassadaFinal() {
        let a = TrechoLongo(inicio: 0, fim: 1, voz: "a", texto: "um")
        let b = TrechoLongo(inicio: 2, fim: 3, voz: "b", texto: "dois")
        var doc = TranscricaoLonga(nome: "Teste", fase: "gravando", fonte: "mac")
        doc.trechos = [a, b]
        doc.atualizarVozes()
        XCTAssertTrue(doc.renomear(voz: "a", nome: "João"))
        XCTAssertTrue(doc.renomear(voz: "b", nome: "joao"))
        XCTAssertEqual(doc.vozes.count, 1)
        let novo = TrechoLongo(inicio: 4, fim: 5, voz: "b", texto: "três")
        doc.trechos.append(novo)
        doc.atualizarVozes()
        let refeito = [a, b, novo].map { anterior in
            var trecho = anterior
            trecho.voz = "offline-" + anterior.voz
            return trecho
        }
        doc.aplicarPassadaFinal(refeito)
        XCTAssertEqual(doc.trechos.map(\.voz), ["a", "a", "a"])
    }

    func testListaLeveEConsultaIndividualComFalhaDeResumo() throws {
        let banco = try Banco(caminho: ":memory:")
        var doc = TranscricaoLonga(nome: "Reunião", fase: "pronta", fonte: "mac")
        doc.bruto = "texto bruto"
        doc.trechos = [TrechoLongo(inicio: 0, fim: 1, voz: "a", texto: "fala sintética")]
        doc.marcarFalhaResumo()
        try banco.salvar(transcricao: doc)
        let resumoDaLista = try XCTUnwrap(banco.transcricoes(tipo: "reuniao", detalhes: false).first)
        XCTAssertTrue(resumoDaLista.trechos.isEmpty)
        XCTAssertNil(resumoDaLista.bruto)
        XCTAssertNotNil(resumoDaLista.erroResumo)
        XCTAssertEqual(try banco.transcricao(id: doc.id), doc)
    }

    func testRetencaoAlcancaApenasAsReservasExpiradas() {
        let agora = Date()
        let reserva = "ditase-42-" + UUID().uuidString
        XCTAssertEqual(RetencaoDeReuniao.processoDaReserva(nome: reserva), 42)
        XCTAssertTrue(RetencaoDeReuniao.reservaVencida(nome: reserva, alteracao: agora.addingTimeInterval(-86_400), agora: agora))
        XCTAssertFalse(RetencaoDeReuniao.reservaVencida(nome: reserva, alteracao: agora.addingTimeInterval(-10), agora: agora))
        XCTAssertFalse(RetencaoDeReuniao.reservaVencida(nome: "entrada.wav", alteracao: agora.addingTimeInterval(-90_000), agora: agora))
    }

    func testReabreDocumentoERenomeiaSemTocarNaTranscricaoOuNoDitado() throws {
        let banco = try Banco(caminho: ":memory:")
        let idDitado = try banco.inserir(Ditado(iniciadoEm: 1, texto: "Ditado"))
        var doc = TranscricaoLonga(nome: "Reunião de teste", fase: "pronta", fonte: "macEMicrofone")
        doc.trechos = [TrechoLongo(inicio: 1, fim: 2, voz: "voce", texto: "Transcrição")]
        doc.atualizarVozes()
        XCTAssertEqual(doc.vozes.first?.nome, "Você")
        XCTAssertTrue(doc.renomear(voz: "voce", nome: "Ana"))
        doc.resumo = ["Resumo"]
        doc.resumoGerado = true
        try banco.salvar(transcricao: doc)
        doc.nome = "Outro título"
        try banco.salvar(transcricao: doc)
        XCTAssertEqual(try banco.transcricoes(tipo: "reuniao"), [doc])
        XCTAssertEqual(try banco.transcricao(id: doc.id), doc)
        XCTAssertEqual(try banco.transcricoes(tipo: "arquivo"), [])
        XCTAssertEqual(try banco.ditado(id: idDitado)?.texto, "Ditado")
    }
}
