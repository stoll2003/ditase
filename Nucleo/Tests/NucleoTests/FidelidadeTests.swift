import XCTest
@testable import Nucleo

final class FidelidadeTests: XCTestCase {
    func testAceitaPontuacaoMaiusculaEParagrafo() {
        let original = "eu montei o rascunho hoje depois eu reviso amanhã eu publico"
        let modelo = "Eu montei o rascunho hoje. Depois eu reviso.\n\nAmanhã eu publico."
        XCTAssertEqual(Fidelidade.fundir(original: original, modelo: modelo), modelo)
    }

    func testNaoTrocaGiriaPorSinonimo() {
        let original = "As tabelas já estão linkadas nele e esse trem funciona."
        let modelo = "As tabelas já estão vinculadas nele, e essa coisa funciona."
        XCTAssertEqual(Fidelidade.fundir(original: original, modelo: modelo), "As tabelas já estão linkadas nele, e esse trem funciona.")
    }

    func testNaoCorrigeNomeProprio() {
        let original = "A gente publica no Nebulo então."
        let modelo = "A gente publica no Nebula Cloud, então."
        XCTAssertEqual(Fidelidade.fundir(original: original, modelo: modelo), "A gente publica no Nebulo, então.")
    }

    func testRestauraFraseCortada() {
        let original = "Na verdade eu quero montar um filtro aqui que organiza isso. Depois a gente vê."
        let modelo = "Na verdade, organiza isso. Depois a gente vê."
        let r = Fidelidade.fundir(original: original, modelo: modelo)
        XCTAssertTrue(r.contains("eu quero montar um filtro aqui que organiza isso"), r)
        XCTAssertTrue(r.hasSuffix("Depois a gente vê."), r)
    }

    func testAceitaCorteDeMuleta() {
        let original = "Então, né, eu preciso disso aí, tipo, pra amanhã, tá?"
        let modelo = "Eu preciso disso aí pra amanhã."
        XCTAssertEqual(Fidelidade.fundir(original: original, modelo: modelo), "Eu preciso disso aí pra amanhã.")
    }

    func testAceitaCorteDeRecomeco() {
        let original = "O teste está falhando porque o teste está falhando porque a porta está ocupada."
        let modelo = "O teste está falhando porque a porta está ocupada."
        XCTAssertEqual(Fidelidade.fundir(original: original, modelo: modelo), modelo)
    }

    func testAceitaAutocorrecao() {
        let original = "Reserva a sala na sexta, não, espera, reserva na quinta às duas que é melhor."
        let modelo = "Reserva na quinta às duas, que é melhor."
        XCTAssertEqual(Fidelidade.fundir(original: original, modelo: modelo), modelo)
    }

    func testDescartaPalavraQueNinguemFalou() {
        let original = "Salva um álbum com essa capa."
        let modelo = "Por favor, salva um álbum bonito com essa capa, obrigado."
        XCTAssertEqual(Fidelidade.fundir(original: original, modelo: modelo), "Salva um álbum com essa capa.")
    }

    func testAspasDoModeloFicam() {
        let original = "Ele disse tipo bússola e eu quis dizer válvula."
        let modelo = "Ele disse tipo \"bússola\" e eu quis dizer \"válvula\"."
        XCTAssertEqual(Fidelidade.fundir(original: original, modelo: modelo), modelo)
    }

    func testModeloVazioDevolveOriginal() {
        XCTAssertEqual(Fidelidade.fundir(original: "Texto falado.", modelo: ""), "Texto falado.")
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

    func testNaoAceitaTrocaDeAcentoQueMudaAPalavra() {
        XCTAssertEqual(Fidelidade.fundir(original: "Ele pode pagar amanhã.", modelo: "Ele pôde pagar amanhã."), "Ele pode pagar amanhã.")
        XCTAssertEqual(Fidelidade.fundir(original: "Meu avô comprou o carro ontem.", modelo: "Minha avó comprou o carro ontem."), "Meu avô comprou o carro ontem.")
    }

    func testMantemSinalDeNumero() {
        XCTAssertEqual(Fidelidade.fundir(original: "O saldo ficou em -100 reais ontem.", modelo: "O saldo ficou em 100 reais ontem."), "O saldo ficou em -100 reais ontem.")
    }

    func testNaoApagaPronome() {
        let original = "Eu quero que ele feche a porta amanhã."
        XCTAssertEqual(Fidelidade.fundir(original: original, modelo: "Eu quero que feche a porta amanhã."), original)
    }

    func testNaVerdadeNaoAutorizaApagarAFraseAnterior() {
        let original = "A oficina não entregou a peça ontem. Na verdade precisamos ligar hoje e anotar tudo no caderno da bancada imediatamente."
        let modelo = "Na verdade precisamos ligar hoje e anotar tudo no caderno da bancada imediatamente."
        XCTAssertEqual(Fidelidade.fundir(original: original, modelo: modelo), original)
    }

    func testCorrecaoNaoAtravessaFraseEncerrada() {
        let original = "O ensaio começa na sexta. Reserva a sala azul, quer dizer, reserva a sala verde."
        let modelo = "Reserva a sala verde."
        let r = Fidelidade.fundir(original: original, modelo: modelo)
        XCTAssertTrue(r.hasPrefix("O ensaio começa na sexta."), r)
        XCTAssertTrue(r.hasSuffix("reserva a sala verde.") || r.hasSuffix("Reserva a sala verde."), r)
        XCTAssertFalse(r.contains("azul"), r)
    }

    func testMuletaComSentidoFica() {
        let original = "Foi assim que eu fiz e ficou bem melhor então."
        XCTAssertEqual(Fidelidade.fundir(original: original, modelo: "Foi que eu fiz e ficou melhor."), original)
    }
}
