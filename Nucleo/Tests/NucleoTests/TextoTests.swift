import XCTest
@testable import Nucleo

final class RegrasFalaTests: XCTestCase {
    func testTiraHesitacoes() {
        XCTAssertEqual(RegrasFala.filtrar("Então, hã, eu preciso ahn daquele caderno."), "Então, eu preciso daquele caderno.")
        XCTAssertEqual(RegrasFala.filtrar("Hum, acho que sim."), "Acho que sim.")
    }

    func testVerboHaNaoEhHesitacao() {
        XCTAssertEqual(RegrasFala.filtrar("Há três pacotes pendentes."), "Há três pacotes pendentes.")
        XCTAssertEqual(RegrasFala.filtrar("O plano está ativo há dois anos."), "O plano está ativo há dois anos.")
        XCTAssertEqual(RegrasFala.filtrar("Hã, preciso do resumo."), "Preciso do resumo.")
        XCTAssertEqual(RegrasFala.filtrar("Isso ha muito tempo não acontece."), "Isso ha muito tempo não acontece.")
    }

    func testPreservaPalavrasComSentido() {
        // "ah" no meio de frase e gíria ficam como foram ditas.
        let frase = "Aí eu falei ah não, esse trem aí tá muito simples, tipo, deixa assim."
        XCTAssertEqual(RegrasFala.filtrar(frase), frase)
    }

    func testTiraGaguejoDeLigacao() {
        XCTAssertEqual(RegrasFala.filtrar("Eu eu quero que que você olhe o o roteiro."), "Eu quero que você olhe o roteiro.")
    }

    func testNaoTiraRepeticaoDeEnfase() {
        XCTAssertEqual(RegrasFala.filtrar("Isso é muito muito bom, não não faz isso."), "Isso é muito muito bom, não não faz isso.")
    }

    func testComandosDeFormatacao() {
        XCTAssertEqual(RegrasFala.filtrar("Primeira parte. Novo parágrafo. Segunda parte."), "Primeira parte.\n\nSegunda parte.")
        XCTAssertEqual(RegrasFala.filtrar("Item um, nova linha, item dois."), "Item um.\nItem dois.")
    }

    func testComandoNoMeioDaFraseNaoEhComando() {
        let frase = "Eles lançaram uma nova linha de produtos."
        XCTAssertEqual(RegrasFala.filtrar(frase), frase)
    }

    func testMaiusculaEEspacos() {
        XCTAssertEqual(RegrasFala.arrumarEspacos("  olá ,  mundo . tudo bem ? sim"), "Olá, mundo. Tudo bem? Sim")
        XCTAssertEqual(RegrasFala.arrumarEspacos("abre o arquivo.pdf e o Qwen3.5 agora"), "Abre o arquivo.pdf e o Qwen3.5 agora")
        XCTAssertEqual(RegrasFala.arrumarEspacos("frase um. frase dois"), "Frase um. Frase dois")
    }
}

final class GuardaTests: XCTestCase {
    let original = "então é o seguinte eu preciso que você olhe aquele trem lá da tabela e me fala se tá certo"

    func testAceitaPontuacaoERemocao() {
        XCTAssertNil(Guarda.avaliar(original: original, limpo: "É o seguinte: eu preciso que você olhe aquele trem lá da tabela e me fala se tá certo."))
    }

    func testAceitaPequenaCorrecaoGramatical() {
        XCTAssertNil(Guarda.avaliar(original: "talvez ver como que a gente pode fazer esse tipo de micro ajustes", limpo: "Talvez veja como a gente pode fazer esse tipo de microajustes."))
    }

    func testRecusaResposta() {
        let recusa = Guarda.avaliar(original: "qual que é a distância da Terra até a Lua você sabe me dizer", limpo: "A distância média entre a Terra e a Lua é de cerca de trezentos e oitenta e quatro mil quilômetros ao longo da órbita.")
        XCTAssertNotNil(recusa)
    }

    func testRecusaAcrescimo() {
        XCTAssertEqual(Guarda.avaliar(original: original, limpo: original + " Além disso, confira também os totais de cada coluna e o rodapé."), .respondeu)
    }

    func testRecusaCorteExcessivo() {
        XCTAssertEqual(Guarda.avaliar(original: original, limpo: "Olhe a tabela."), .cortouDemais)
    }

    func testRecusaVazio() {
        XCTAssertEqual(Guarda.avaliar(original: original, limpo: "  "), .vazio)
    }

    func testAutocorrecaoCurtaPassa() {
        XCTAssertNil(Guarda.avaliar(original: "reserva a sala na sexta não espera reserva na quinta às duas que é melhor", limpo: "Reserva na quinta às duas, que é melhor."))
    }
}

final class DicionarioTests: XCTestCase {
    let termos = [
        Termo(id: 1, termo: "VitePress"),
        Termo(id: 2, termo: "Svelte", ouvidoComo: ["Suelti", "Svelt"]),
        Termo(id: 3, termo: "Vortex"),
        Termo(id: 4, termo: "Estúdio Aurora", ouvidoComo: ["Estúdio Alrora"]),
    ]

    func testGrafiaEEspacamento() {
        let a = AplicadorDicionario(termos: termos)
        let r = a.aplicar("Criar um guia no Vite Press e no vitepress.")
        XCTAssertEqual(r.texto, "Criar um guia no VitePress e no VitePress.")
        XCTAssertEqual(r.trocas.count, 2)
    }

    func testOuvidoComo() {
        let a = AplicadorDicionario(termos: termos)
        XCTAssertEqual(a.aplicar("Migramos a tela para Suelti e React.").texto, "Migramos a tela para Svelte e React.")
        XCTAssertEqual(a.aplicar("Gravamos no Estúdio Alrora.").texto, "Gravamos no Estúdio Aurora.")
    }

    func testSemelhancaDeSomSoParaPalavraDesconhecida() {
        let comuns: Set<String> = ["visto", "vista", "rodei", "o", "eu"]
        let a = AplicadorDicionario(termos: termos, ehPalavraComum: { comuns.contains($0.lowercased()) })
        XCTAssertEqual(a.aplicar("Eu rodei o Vortec.").texto, "Eu rodei o Vortex.")
        XCTAssertEqual(a.aplicar("Eu rodei o visto.").texto, "Eu rodei o visto.")
    }

    func testNaoAtravessaPontuacao() {
        let a = AplicadorDicionario(termos: termos)
        XCTAssertEqual(a.aplicar("Deixa vite. Press nenhum.").texto, "Deixa vite. Press nenhum.")
    }

    func testTextoJaCertoNaoContaTroca() {
        let a = AplicadorDicionario(termos: termos)
        XCTAssertTrue(a.aplicar("O VitePress e o Svelte.").trocas.isEmpty)
    }

    func testChaveDeSom() {
        XCTAssertEqual(AplicadorDicionario.chaveDeSom("Matriz"), AplicadorDicionario.chaveDeSom("Matris"))
        XCTAssertGreaterThan(AplicadorDicionario.semelhanca(AplicadorDicionario.chaveDeSom("Fabrisio"), AplicadorDicionario.chaveDeSom("Fabricio")), 0.9)
    }
}

final class PalavrasTests: XCTestCase {
    func testContagem() {
        XCTAssertEqual(Palavras.contar("Olá, mundo! Guarda-chuva e d'água: 3 itens."), 7)
        XCTAssertEqual(Palavras.contar(""), 0)
    }

    func testCauda() {
        XCTAssertEqual(Palavras.cauda("um dois, três. Quatro cinco!", palavras: 2), "Quatro cinco!")
        XCTAssertEqual(Palavras.cauda("um dois", palavras: 5), "um dois")
    }
}
