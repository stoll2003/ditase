import Foundation
import XCTest
@testable import Nucleo

/// Garantias 2, 3 e 5: costura com dono único por fonte, zero trechos e linha de tempo comum.
final class CosturaETempoTests: XCTestCase {
    private func aoVivo(_ roteiro: [PalavraReconhecida], segundos: Double, fonte: FonteDeAudio = .mac,
                        oraculo: Oraculo? = nil) async throws -> (Ambiente, String) {
        let amb = try Ambiente()
        let doc = try amb.novaReuniao()
        let coletor = ColetorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, inicio: 0)
        let processador = ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, reconhecedor: oraculo ?? Oraculo(roteiro))
        try await coletor.abrir(fonte, em: 0)
        try await amb.capturar(coletor, fonte, Sintetico.pcm(roteiro, segundos: segundos), aoSelar: processador)
        try await coletor.encerrar(em: segundos)
        try await processador.fechar()
        return (amb, doc)
    }

    // MARK: Costura

    func testPalavraQueCruzaOCorteEntreBlocosSaiUmaVezEInteira() async throws {
        // Palavras atravessam os cortes de 1 s dos blocos e as bordas das janelas de reconhecimento.
        let roteiro = Sintetico.roteiro((1...24).map { "p\($0)" }, inicio: 0.85, duracao: 0.45, pausa: 0.3)
        let (amb, doc) = try await aoVivo(roteiro, segundos: 20)
        XCTAssertEqual(try amb.palavras(doc), roteiro.map(\.texto))
        XCTAssertFalse(try amb.textos(doc).joined().contains("~"), "palavra truncada na fronteira foi confirmada")
    }

    func testDuasRepeticoesReaisProximasSaoMantidas() async throws {
        let roteiro = [PalavraReconhecida(texto: "sim", inicio: 1.0, fim: 1.3), PalavraReconhecida(texto: "sim", inicio: 1.42, fim: 1.72),
                       PalavraReconhecida(texto: "claro", inicio: 3.0, fim: 3.4)]
        let (amb, doc) = try await aoVivo(roteiro, segundos: 6)
        XCTAssertEqual(try amb.palavras(doc), ["sim", "sim", "claro"])
    }

    func testFalaContinuaSemPausaNaoPerdeNemDuplica() async throws {
        // 60 palavras sem folga para corte livre: a costura força o corte no fim de uma palavra.
        let roteiro = Sintetico.roteiro((1...60).map { "c\($0)" }, inicio: 0.3, duracao: 0.4, pausa: 0.05)
        let (amb, doc) = try await aoVivo(roteiro, segundos: 29)
        XCTAssertEqual(try amb.palavras(doc), roteiro.map(\.texto))
    }

    func testUltimaPalavraSemSilencioEntraNoFechamento() async throws {
        let roteiro = Sintetico.roteiro(["combinado", "até", "quinta"], inicio: 1.0, duracao: 0.5, pausa: 0.2)
        let fim = roteiro.last!.fim // o áudio termina no fim da última palavra
        let (amb, doc) = try await aoVivo(roteiro, segundos: fim)
        XCTAssertEqual(try amb.palavras(doc), ["combinado", "até", "quinta"])
    }

    func testDecisaoSoConfirmaComFronteiraResolvida() {
        let palavras = [PalavraReconhecida(texto: "a", inicio: 0.5, fim: 0.9), PalavraReconhecida(texto: "b", inicio: 2.6, fim: 3.1)]
        let r = Costura.decidir(palavras: palavras, confirmadoAte: 0, limite: 3 * taxaLancamento, fechamento: false) { _ in true }
        XCTAssertEqual(r.confirmadas.map(\.texto), ["a"], "palavra que encosta no limite fica na cauda")
        XCTAssertLessThanOrEqual(r.novoConfirmado, Int(2.5 * Double(taxaLancamento)), "o corte fica antes da folga da palavra seguinte")
        XCTAssertGreaterThan(r.novoConfirmado, Int(0.9 * Double(taxaLancamento)))
    }

    // MARK: Zero trechos

    func testVazioComVozFicaPendenteENovaTentativaReconhece() async throws {
        let roteiro = Sintetico.roteiro(["isto", "precisa", "aparecer"], inicio: 1.0)
        let teimoso = Oraculo(roteiro, vaziasPrimeiro: 1_000)
        let (amb, doc) = try await aoVivo(roteiro, segundos: 5, oraculo: teimoso)
        XCTAssertTrue(try amb.textos(doc).isEmpty)
        let pend = try amb.armazem.pendencias(documento: doc)
        XCTAssertEqual(pend.filter { $0.estado == "aberta" }.count, 1, "vazio com voz precisa ficar pendente")
        XCTAssertTrue(try amb.armazem.blocos(documento: doc).contains { $0.estado == .pendente })
        XCTAssertNotNil(try amb.armazem.documento(doc)?.pendentes)

        // Mesmo PCM, reconhecedor restabelecido: a nova tentativa transcreve.
        let processador = ProcessadorDeReuniao(documento: doc, armazem: try amb.reabrir(), disco: amb.disco, reconhecedor: Oraculo(roteiro))
        try await processador.recuperar()
        XCTAssertEqual(try amb.palavras(doc), ["isto", "precisa", "aparecer"])
        XCTAssertFalse(try amb.armazem.blocos(documento: doc).contains { $0.estado == .pendente || $0.estado == .capturado })
        // Repetir não muda nada.
        try await ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, reconhecedor: Oraculo(roteiro)).recuperar()
        XCTAssertEqual(try amb.palavras(doc), ["isto", "precisa", "aparecer"])
    }

    func testVazioPersistenteContinuaRepetivelComContador() async throws {
        let roteiro = Sintetico.roteiro(["fala"], inicio: 1.0)
        let (amb, doc) = try await aoVivo(roteiro, segundos: 3, oraculo: Oraculo(roteiro, vaziasPrimeiro: 1_000))
        try await ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco,
                                       reconhecedor: Oraculo(roteiro, vaziasPrimeiro: 1_000)).recuperar()
        let p = try XCTUnwrap(amb.armazem.pendencias(documento: doc).first)
        XCTAssertEqual(p.estado, "aberta")
        XCTAssertEqual(p.tentativas, 2)
        XCTAssertTrue(p.motivo.contains("2 tentativas"))
    }

    func testSilencioConfirmadoETerminal() async throws {
        let oraculo = Oraculo([])
        let (amb, doc) = try await aoVivo([], segundos: 4, oraculo: oraculo)
        let blocos = try amb.armazem.blocos(documento: doc)
        XCTAssertFalse(blocos.isEmpty)
        XCTAssertTrue(blocos.allSatisfy { $0.estado == .semFala })
        XCTAssertTrue(try amb.armazem.pendencias(documento: doc).isEmpty)
        try await ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, reconhecedor: oraculo).recuperar()
        XCTAssertEqual(oraculo.chamadas, 0, "silêncio confirmado não volta ao reconhecedor")
    }

    func testCriterioDeAtividadeComFalaCurtaEBaixa() {
        var curta = [Float](repeating: 0, count: 16_000)
        for i in 8_000..<(8_000 + 2_400) { curta[i] = 0.02 * Float(sin(Double(i) * 0.3)) } // 150 ms a cerca de -40 dBFS
        XCTAssertTrue(AtividadeDeVoz.tem(curta))
        let chiado = (0..<16_000).map { _ in Float.random(in: -0.003...0.003) }
        XCTAssertFalse(AtividadeDeVoz.tem(chiado))
        var clique = [Float](repeating: 0, count: 16_000)
        for i in 100..<400 { clique[i] = 0.5 } // 19 ms: abaixo de 60 ms seguidos
        XCTAssertFalse(AtividadeDeVoz.tem(clique))
    }

    // MARK: Linha de tempo

    func testMicrofoneAtrasadoEntraNoInstanteDaCaptura() {
        let linha = LinhaDoTempo(inicio: 100)
        var mac = CursorDeFonte(abertura: 0), mic = CursorDeFonte(abertura: 0)
        XCTAssertEqual(mac.receber(tempo: 100.0, quantidade: 1_600, linha: linha), [.amostras(origem: 0, quantidade: 1_600, indice: 0)])
        // O buffer do microfone chega depois, mas foi carimbado no mesmo instante.
        XCTAssertEqual(mic.receber(tempo: 100.0, quantidade: 1_600, linha: linha), [.amostras(origem: 0, quantidade: 1_600, indice: 0)])
        XCTAssertEqual(mic.receber(tempo: 100.1, quantidade: 1_600, linha: linha), [.amostras(origem: 0, quantidade: 1_600, indice: 1_600)])
    }

    func testDerivaPequenaAbsorvidaLacunaExplicitaERepeticaoDescartada() {
        let linha = LinhaDoTempo(inicio: 0)
        var c = CursorDeFonte(abertura: 0)
        _ = c.receber(tempo: 0, quantidade: 1_600, linha: linha)
        XCTAssertEqual(c.receber(tempo: 0.13, quantidade: 1_600, linha: linha), [.amostras(origem: 0, quantidade: 1_600, indice: 1_600)])
        XCTAssertEqual(c.receber(tempo: 0.5, quantidade: 1_600, linha: linha), [.lacuna(3_200..<8_000), .amostras(origem: 0, quantidade: 1_600, indice: 8_000)])
        XCTAssertTrue(c.receber(tempo: 0.4, quantidade: 3_200, linha: linha).isEmpty, "amostras repetidas no tempo saem")
        XCTAssertEqual(c.proximo, 9_600)
    }

    func testPausaFechaNoMarcoEIgnoraCallbacksAntigos() {
        var linha = LinhaDoTempo(inicio: 0)
        var c = CursorDeFonte(abertura: 15_200)
        linha.pausar(em: 1.0)
        // Buffer que começa antes da pausa e termina depois: só a parte anterior entra.
        XCTAssertEqual(c.receber(tempo: 0.95, quantidade: 1_600, linha: linha), [.amostras(origem: 0, quantidade: 800, indice: 15_200)])
        XCTAssertTrue(c.receber(tempo: 2.0, quantidade: 1_600, linha: linha).isEmpty, "callback durante a pausa é ignorado")
        linha.retomar(em: 5.0)
        XCTAssertEqual(linha.indice(5.0), 16_000)
        var depois = CursorDeFonte(abertura: 16_000)
        XCTAssertEqual(depois.receber(tempo: 5.0, quantidade: 1_600, linha: linha), [.amostras(origem: 0, quantidade: 1_600, indice: 16_000)])
    }

    func testTrocaDeDispositivoAbreNovaGeracaoComLacunaVisivel() async throws {
        let amb = try Ambiente()
        let doc = try amb.novaReuniao(fonte: "macEMicrofone")
        let coletor = ColetorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, inicio: 0)
        let antes = Sintetico.roteiro(["antes", "da", "troca"], inicio: 0.2, duracao: 0.3, pausa: 0.25)
        let depois = Sintetico.roteiro(["depois", "da", "troca"], inicio: 3.2, duracao: 0.3, pausa: 0.25)
        try await coletor.abrir(.mic, em: 0)
        try await amb.capturar(coletor, .mic, Array(Sintetico.pcm(antes, segundos: 2.5)), desde: 0)
        let novaGeracao = try await coletor.abrir(.mic, em: 3.0)
        XCTAssertEqual(novaGeracao, 1)
        let pcm = Sintetico.pcm(depois, segundos: 6)
        try await amb.capturar(coletor, .mic, Array(pcm[(3 * taxaLancamento)...]), desde: 3.0)
        try await coletor.encerrar(em: 6)
        let processador = ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, reconhecedor: Oraculo(antes + depois))
        try await processador.fechar()
        let blocos = try amb.armazem.blocos(documento: doc)
        XCTAssertEqual(Set(blocos.map(\.id.geracao)), [0, 1])
        XCTAssertEqual(try amb.armazem.costuras(documento: doc).count, 2)
        let lacuna = try XCTUnwrap(amb.armazem.documento(doc)?.lacunas?.first)
        XCTAssertEqual(lacuna.inicio, 2.5, accuracy: 0.01)
        XCTAssertEqual(lacuna.fim, 3.0, accuracy: 0.01)
        let doc2 = try XCTUnwrap(amb.armazem.documento(doc))
        XCTAssertEqual(doc2.trechos.flatMap { $0.texto.split(separator: " ") }.map(String.init), ["antes", "da", "troca", "depois", "da", "troca"])
        XCTAssertTrue(doc2.trechos.allSatisfy { $0.voz == "voce" && $0.origem == "microfone" })
        XCTAssertEqual(doc2.trechos.last?.inicio ?? 0, 3.2, accuracy: 0.01)
    }

    // MARK: Atalho

    func testAtalhoDuranteReuniaoUsaDerivacaoPropria() async throws {
        final class Caixa: @unchecked Sendable { var reuniao: [Float] = []; let trava = NSLock() }
        let caixa = Caixa()
        let distribuidor = DistribuidorDoMicrofone { lote in caixa.trava.lock(); caixa.reuniao += lote.amostras; caixa.trava.unlock() }
        distribuidor.entregar(tempo: 0, amostras: [1, 1])
        let ditado = distribuidor.abrirDerivacao()
        distribuidor.entregar(tempo: 0.1, amostras: [2, 2])
        distribuidor.fecharDerivacao()
        distribuidor.entregar(tempo: 0.2, amostras: [3, 3])
        var recebidos: [Float] = []
        for await lote in ditado { recebidos += lote }
        XCTAssertEqual(recebidos, [2, 2], "o atalho recebe só o trecho entre começar e soltar")
        XCTAssertEqual(caixa.reuniao, [1, 1, 2, 2, 3, 3], "a reunião recebe o microfone uma única vez")

        // Fila do ditado estourada encerra só a derivação.
        let lenta = distribuidor.abrirDerivacao(limite: 1)
        for i in 0..<5 { distribuidor.entregar(tempo: 1 + Double(i), amostras: [9]) }
        XCTAssertFalse(distribuidor.derivacaoAberta)
        XCTAssertEqual(distribuidor.estourosDaDerivacao, 1)
        XCTAssertEqual(caixa.reuniao.count, 11)
        _ = lenta
    }

    func testCaudaNoFechamentoExpandeContextoQuandoJanelaCurtaFalha() async throws {
        struct ReconhecedorExigente: ReconhecedorDeFala {
            let roteiro: [PalavraReconhecida]
            func palavras(_ amostras: [Float], inicio: Double, fimDaFala: Double?) async throws -> [PalavraReconhecida] {
                let fim = inicio + Double(amostras.count) / Double(taxaLancamento)
                return roteiro.filter { p in
                    if p.texto == "combinado" {
                        // Só reconhece a palavra final quando recebe contexto longo (> 4 s).
                        return amostras.count > 4 * taxaLancamento && p.centro >= inicio && p.centro <= fim
                    }
                    return p.centro >= inicio && p.centro <= fim
                }
            }
        }
        let roteiro = Sintetico.roteiro(["primeira", "frase", "da", "reunião"], inicio: 0.5, duracao: 0.4, pausa: 0.3)
            + [PalavraReconhecida(texto: "combinado", inicio: 6.2, fim: 6.8)]
        let amb = try Ambiente()
        let doc = try amb.novaReuniao()
        let coletor = ColetorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, inicio: 0)
        let processador = ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco,
                                               reconhecedor: ReconhecedorExigente(roteiro: roteiro))
        try await coletor.abrir(.mac, em: 0)
        // Primeiro fecha até 5 s (forçando confirmadoAte antes da última fala).
        let pcm = Sintetico.pcm(roteiro, segundos: 7)
        try await amb.capturar(coletor, .mac, Array(pcm[0..<(5 * taxaLancamento)]), desde: 0)
        try await processador.fechar()
        // Depois entrega os últimos 2 s (confirmadoAte já está em 5 s; a janela tem < 3 s) e fecha de novo.
        try await amb.capturar(coletor, .mac, Array(pcm[(5 * taxaLancamento)...]), desde: 5.0)
        try await coletor.encerrar(em: 7)
        // Reabre costura simulando novo passo de fechamento após janela curta.
        try amb.armazem.escrever { c in
            try c.rodar("UPDATE lt_costuras SET fechada = 0 WHERE documento = ?", [.texto(doc)])
        }
        try await processador.fechar()
        XCTAssertEqual(try amb.palavras(doc), ["primeira", "frase", "da", "reunião", "combinado"])
        XCTAssertTrue(try amb.armazem.pendencias(documento: doc).isEmpty)
    }
}
