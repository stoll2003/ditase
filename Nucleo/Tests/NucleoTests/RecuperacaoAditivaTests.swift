import Foundation
import XCTest
@testable import Nucleo

/// Rodada 3: a recuperação nunca destrói dado, e a saída tem teto rígido.
final class RecuperacaoAditivaTests: XCTestCase {
    private var raiz: URL!

    override func setUpWithError() throws {
        raiz = FileManager.default.temporaryDirectory.appendingPathComponent("r3-" + UUID().uuidString)
    }

    override func tearDownWithError() throws { try? FileManager.default.removeItem(at: raiz) }

    private func trecho(_ inicio: Double, _ fim: Double, voz: String = "a", texto: String = "fala sintética",
                        microfone: Bool = false) -> TrechoLongo {
        var t = TrechoLongo(inicio: inicio, fim: fim, voz: voz, texto: texto)
        if microfone { t.origem = "microfone" }
        return t
    }

    func testTrechosGravadosNaoSaoSubstituidosPorListaVazia() throws {
        let diario = try DiarioReuniao(raiz: raiz, documento: TranscricaoLonga(nome: "R", fase: "gravando", fonte: "mac"))
        let publicado = trecho(0, 1)
        try diario.salvarTrechos(inicio: 0, trechos: [publicado])
        XCTAssertEqual(try diario.salvarTrechos(inicio: 0, trechos: []), [publicado])
        XCTAssertEqual(try diario.trechosSalvos(inicio: 0), [publicado])
        XCTAssertNil(try diario.trechosSalvos(inicio: 48_000))
        XCTAssertEqual(try diario.provisoria().trechos, [publicado])
    }

    func testConcluirSoRetiraODiarioQuandoOBancoConfirmaTudo() throws {
        var doc = TranscricaoLonga(nome: "R", fase: "gravando", fonte: "mac")
        let diario = try DiarioReuniao(raiz: raiz, documento: doc)
        let publicado = trecho(0, 1)
        try diario.salvarTrechos(inicio: 0, trechos: [publicado])
        try diario.salvarAudio(inicio: 0, sistema: [Float](repeating: 0.2, count: 48_000), microfone: [])
        let banco = try Banco(caminho: raiz.appendingPathComponent("teste.sqlite").path)

        doc.fase = "pronta"
        doc.duracao = 3
        // Resultado vazio: o banco grava, mas não cobre a fala do diário.
        XCTAssertThrowsError(try diario.concluir(doc, persistir: { try banco.salvar(transcricao: $0) },
                                                 reler: { try banco.transcricao(id: doc.id) }))
        XCTAssertTrue(FileManager.default.fileExists(atPath: diario.pasta.path))
        XCTAssertNil(try banco.transcricao(id: doc.id), "documento incompleto não pode ser arquivado")
        XCTAssertEqual(CoberturaDeReuniao.faltantes(doc.trechos, [publicado]), [publicado])
        // Releitura diferente do que foi persistido também não retira.
        doc.trechos = [publicado]
        XCTAssertThrowsError(try diario.concluir(doc, persistir: { _ in }, reler: { nil }))
        XCTAssertEqual(try diario.trechosSalvos(inicio: 0), [publicado])
        // Áudio além da duração salva: há fala não processada.
        var curto = doc
        curto.duracao = 1
        XCTAssertFalse(try diario.coberto(por: curto))

        XCTAssertTrue(try diario.concluir(doc, persistir: { try banco.salvar(transcricao: $0) },
                                          reler: { try banco.transcricao(id: doc.id) }))
        XCTAssertFalse(FileManager.default.fileExists(atPath: diario.pasta.path))
    }

    func testCoberturaPorIDOuIntervaloDaMesmaFonte() {
        let original = trecho(10, 12)
        var refeito = trecho(10.1, 11.9)
        refeito.id = "outro"
        XCTAssertTrue(CoberturaDeReuniao.cobre([original], [original]))
        XCTAssertTrue(CoberturaDeReuniao.cobre([refeito], [original]))
        XCTAssertFalse(CoberturaDeReuniao.cobre([trecho(10, 12, voz: "voce", microfone: true)], [original]))
        XCTAssertFalse(CoberturaDeReuniao.cobre([], [original]))
        XCTAssertTrue(CoberturaDeReuniao.cobre([], [trecho(0, 1, texto: "  ")]))
    }

    func testPassadaFinalLevaEdicaoManualParaTrechoRefeitoComOutroID() {
        var doc = TranscricaoLonga(nome: "R", fase: "finalizando", fonte: "macEMicrofone")
        var editado = trecho(0, 1, voz: "V")
        editado.vozManual = true
        doc.trechos = [editado]
        doc.atualizarVozes()
        XCTAssertTrue(doc.renomear(voz: "V", nome: "Pessoa escolhida"))

        let novo = trecho(0.05, 1, voz: "fluxo-0")
        let doMic = trecho(0, 1, voz: "voce", texto: "outra fala", microfone: true)
        doc.aplicarPassadaFinal([doMic, novo])

        let herdado = doc.trechos.first { !$0.doMicrofone }
        XCTAssertEqual(herdado?.id, editado.id)
        XCTAssertEqual(herdado?.voz, "V")
        XCTAssertEqual(herdado?.vozManual, true)
        XCTAssertEqual(doc.vozes.first { $0.id == "V" }?.nome, "Pessoa escolhida")
        XCTAssertEqual(doc.trechos.first { $0.doMicrofone }?.voz, "voce")
    }

    func testRecuperarPreservaEdicaoManualFeitaDepoisDeArquivar() {
        var salvo = TranscricaoLonga(nome: "Planejamento", fase: "pronta", fonte: "mac")
        var bruto = trecho(0, 1, voz: "fluxo-1")
        var editado = bruto
        editado.voz = "X"
        editado.vozManual = true
        salvo.trechos = [editado]
        salvo.atualizarVozes()
        XCTAssertTrue(salvo.renomear(voz: "X", nome: "Ana"))

        var doDiario = TranscricaoLonga(nome: "Reunião", fase: "finalizando", fonte: "mac")
        doDiario.id = salvo.id
        bruto.parcial = true
        doDiario.trechos = [bruto, trecho(3, 4, voz: "fluxo-0")]
        doDiario.atualizarVozes()
        doDiario.herdarEdicoes(de: salvo)

        XCTAssertEqual(doDiario.nome, "Planejamento")
        XCTAssertEqual(doDiario.trechos.first?.voz, "X")
        XCTAssertEqual(doDiario.vozes.first { $0.id == "X" }?.nome, "Ana")
        XCTAssertEqual(doDiario.trechos.count, 2)
    }

    func testListaLeveTrazContagemDePalavrasEPrevia() throws {
        try FileManager.default.createDirectory(at: raiz, withIntermediateDirectories: true)
        let banco = try Banco(caminho: raiz.appendingPathComponent("teste.sqlite").path)
        var arquivo = TranscricaoLonga(nome: "SINTETICO.WAV", fase: "pronto")
        let textos = ["Vamos revisar o plano.", "Combinado, até quinta-feira."]
        arquivo.trechos = textos.enumerated().map { trecho(Double($0.offset), Double($0.offset) + 1, texto: $0.element) }
        try banco.salvar(transcricao: arquivo)
        var vazia = TranscricaoLonga(nome: "Silêncio", fase: "pronta", fonte: "mac")
        vazia.criadoEm -= 10
        try banco.salvar(transcricao: vazia)

        let leve = try XCTUnwrap(banco.transcricoes(tipo: "arquivo", detalhes: false).first)
        XCTAssertTrue(leve.trechos.isEmpty)
        XCTAssertEqual(leve.palavras, textos.map(Palavras.contar).reduce(0, +))
        XCTAssertEqual(leve.previa, textos[0])
        let semFala = try XCTUnwrap(banco.transcricoes(tipo: "reuniao", detalhes: false).first)
        XCTAssertEqual(semFala.palavras, 0)
        XCTAssertNil(semFala.previa)
        // O documento completo não ganha campos de lista.
        XCTAssertNil(try banco.transcricao(id: arquivo.id)?.palavras)
    }

    @MainActor
    func testSaidaRespondeNoPrazoMesmoComEscritaLentaECapturaPresa() async throws {
        let diario = try DiarioReuniao(raiz: raiz, documento: TranscricaoLonga(nome: "R", fase: "gravando", fonte: "mac"),
                                       escrita: { dados, url in
                                           Thread.sleep(forTimeInterval: 1.2)
                                           try DiarioReuniao.escritaAtomica(dados, url)
                                       })
        let saida = SaidaComPrazo()
        let inicio = ContinuousClock.now
        var resposta: Duration?
        var respostas = 0
        var maiorIntervalo = Duration.zero
        let iniciou = saida.iniciar(prazo: .seconds(2), trabalho: {
            try? await Task.detached { try diario.salvarAudio(inicio: 0, sistema: [0.5], microfone: []) }.value
            try? await Task.sleep(for: .seconds(30))
        }, responder: {
            respostas += 1
            resposta = inicio.duration(to: .now)
        })
        XCTAssertTrue(iniciou)
        XCTAssertFalse(saida.iniciar(trabalho: {}, responder: { respostas += 1 }))
        var anterior = ContinuousClock.now
        while resposta == nil {
            try await Task.sleep(for: .milliseconds(10))
            maiorIntervalo = max(maiorIntervalo, anterior.duration(to: .now))
            anterior = .now
        }
        let tempo = try XCTUnwrap(resposta)
        XCTAssertGreaterThanOrEqual(tempo, .milliseconds(1_990))
        XCTAssertLessThan(tempo, .milliseconds(2_150))
        XCTAssertLessThan(maiorIntervalo, .milliseconds(200), "o ator principal ficou preso")
        XCTAssertEqual(try DiarioReuniao.audio(em: diario.pasta.appendingPathComponent("pcm-0.plist")).sistema, [0.5])
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertEqual(respostas, 1)
    }

    @MainActor
    func testSaidaRapidaRespondeAoTerminarOTrabalho() async throws {
        let saida = SaidaComPrazo()
        let inicio = ContinuousClock.now
        var resposta: Duration?
        saida.iniciar(trabalho: { try? await Task.sleep(for: .milliseconds(20)) },
                      responder: { resposta = inicio.duration(to: .now) })
        while resposta == nil { try await Task.sleep(for: .milliseconds(5)) }
        XCTAssertLessThan(try XCTUnwrap(resposta), .milliseconds(500))
        XCTAssertFalse(saida.emCurso)
    }
}
