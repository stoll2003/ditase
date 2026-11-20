import XCTest
@testable import Nucleo

final class FidelidadeTests: XCTestCase {
    func testAceitaPontuacaoMaiusculaEParagrafo() throws {
        let casos = try casosFidelidade("testAceitaPontuacaoMaiusculaEParagrafo")
        let modelo = "Eu montei o rascunho hoje. Depois eu reviso.\n\nAmanhã eu publico."
        XCTAssertEqual(Fidelidade.fundir(original: casos[0].original, modelo: casos[0].modelo), modelo)
    }

    func testNaoTrocaGiriaPorSinonimo() throws {
        let casos = try casosFidelidade("testNaoTrocaGiriaPorSinonimo")
        XCTAssertEqual(Fidelidade.fundir(original: casos[0].original, modelo: casos[0].modelo), "As tabelas já estão linkadas nele, e esse trem funciona.")
    }

    func testNaoCorrigeNomeProprio() throws {
        let casos = try casosFidelidade("testNaoCorrigeNomeProprio")
        XCTAssertEqual(Fidelidade.fundir(original: casos[0].original, modelo: casos[0].modelo), "A gente publica no Nebulo, então.")
    }

    func testRestauraFraseCortada() throws {
        let casos = try casosFidelidade("testRestauraFraseCortada")
        let r = Fidelidade.fundir(original: casos[0].original, modelo: casos[0].modelo)
        XCTAssertTrue(r.contains("eu quero montar um filtro aqui que organiza isso"), r)
        XCTAssertTrue(r.hasSuffix("Depois a gente vê."), r)
    }

    func testAceitaCorteDeMuleta() throws {
        let casos = try casosFidelidade("testAceitaCorteDeMuleta")
        XCTAssertEqual(Fidelidade.fundir(original: casos[0].original, modelo: casos[0].modelo), "Eu preciso disso aí pra amanhã.")
    }

    func testAceitaCorteDeRecomeco() throws {
        let casos = try casosFidelidade("testAceitaCorteDeRecomeco")
        let modelo = "O teste está falhando porque a porta está ocupada."
        XCTAssertEqual(Fidelidade.fundir(original: casos[0].original, modelo: casos[0].modelo), modelo)
    }

    func testAceitaAutocorrecao() throws {
        let casos = try casosFidelidade("testAceitaAutocorrecao")
        let modelo = "Reserva na quinta às duas, que é melhor."
        XCTAssertEqual(Fidelidade.fundir(original: casos[0].original, modelo: casos[0].modelo), modelo)
    }

    func testDescartaPalavraQueNinguemFalou() throws {
        let casos = try casosFidelidade("testDescartaPalavraQueNinguemFalou")
        XCTAssertEqual(Fidelidade.fundir(original: casos[0].original, modelo: casos[0].modelo), "Salva um álbum com essa capa.")
    }

    func testAspasDoModeloFicam() throws {
        let casos = try casosFidelidade("testAspasDoModeloFicam")
        let modelo = "Ele disse tipo \"bússola\" e eu quis dizer \"válvula\"."
        XCTAssertEqual(Fidelidade.fundir(original: casos[0].original, modelo: casos[0].modelo), modelo)
    }

    func testModeloVazioDevolveOriginal() throws {
        let casos = try casosFidelidade("testModeloVazioDevolveOriginal")
        XCTAssertEqual(Fidelidade.fundir(original: casos[0].original, modelo: casos[0].modelo), "Texto falado.")
    }

    func testTextoLongoNaoPerdeNenhumaPalavraDeConteudo() {
        let frases = (0..<120).map { "A frase número \($0) fala de assunto \($0 * 7) com detalhe." }
        let original = frases.joined(separator: " ")
        let modelo = frases.enumerated().map { $0.offset % 10 == 9 ? $0.element + "\n\n" : $0.element + " " }.joined()
        let r = Fidelidade.fundir(original: original, modelo: modelo)
        XCTAssertEqual(Palavras.normalizadas(r), Palavras.normalizadas(original))
        XCTAssertTrue(r.contains("\n\n"))
    }

    // Regressões da revisão independente (2026-11-15).

    func testNaoAceitaTrocaDeAcentoQueMudaAPalavra() throws {
        let casos = try casosFidelidade("testNaoAceitaTrocaDeAcentoQueMudaAPalavra")
        XCTAssertEqual(Fidelidade.fundir(original: casos[0].original, modelo: casos[0].modelo), "Ele pode pagar amanhã.")
        XCTAssertEqual(Fidelidade.fundir(original: casos[1].original, modelo: casos[1].modelo), "Meu avô comprou o carro ontem.")
    }

    func testMantemSinalDeNumero() throws {
        let casos = try casosFidelidade("testMantemSinalDeNumero")
        XCTAssertEqual(Fidelidade.fundir(original: casos[0].original, modelo: casos[0].modelo), "O saldo ficou em -100 reais ontem.")
    }

    func testNaoApagaPronome() throws {
        let casos = try casosFidelidade("testNaoApagaPronome")
        let original = "Eu quero que ele feche a porta amanhã."
        XCTAssertEqual(Fidelidade.fundir(original: casos[0].original, modelo: casos[0].modelo), original)
    }

    func testNaVerdadeNaoAutorizaApagarAFraseAnterior() throws {
        let casos = try casosFidelidade("testNaVerdadeNaoAutorizaApagarAFraseAnterior")
        let original = "A oficina não entregou a peça ontem. Na verdade precisamos ligar hoje e anotar tudo no caderno da bancada imediatamente."
        XCTAssertEqual(Fidelidade.fundir(original: casos[0].original, modelo: casos[0].modelo), original)
    }

    func testCorrecaoNaoAtravessaFraseEncerrada() throws {
        let casos = try casosFidelidade("testCorrecaoNaoAtravessaFraseEncerrada")
        let r = Fidelidade.fundir(original: casos[0].original, modelo: casos[0].modelo)
        XCTAssertTrue(r.hasPrefix("O ensaio começa na sexta."), r)
        XCTAssertTrue(r.hasSuffix("reserva a sala verde.") || r.hasSuffix("Reserva a sala verde."), r)
        XCTAssertFalse(r.contains("azul"), r)
    }

    func testMuletaComSentidoFica() throws {
        let casos = try casosFidelidade("testMuletaComSentidoFica")
        let original = "Foi assim que eu fiz e ficou bem melhor então."
        XCTAssertEqual(Fidelidade.fundir(original: casos[0].original, modelo: casos[0].modelo), original)
    }
}
