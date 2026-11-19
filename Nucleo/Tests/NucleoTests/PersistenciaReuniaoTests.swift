import Foundation
import SQLite3
import XCTest
@testable import Nucleo

/// Garantias 1, 4, 6 e 7, descarte, edições e teto de saída sobre o modelo novo.
final class PersistenciaReuniaoTests: XCTestCase {
    private let roteiro = Sintetico.roteiro(["primeira", "fala", "da", "reunião", "sintética"], inicio: 0.4, duracao: 0.4, pausa: 0.35)

    /// Reunião capturada e interrompida antes de qualquer transcrição.
    private func capturada(_ amb: Ambiente, segundos: Double = 5, fonte: FonteDeAudio = .mac,
                           roteiro outro: [PalavraReconhecida]? = nil) async throws -> String {
        let doc = try amb.novaReuniao()
        let coletor = ColetorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, inicio: 0)
        try await coletor.abrir(fonte, em: 0)
        try await amb.capturar(coletor, fonte, Sintetico.pcm(outro ?? roteiro, segundos: segundos))
        try await coletor.encerrar(em: segundos)
        return doc
    }

    // MARK: Garantia 1: bloco durável antes da fila; queda entre gravação, registro, commit e limpeza

    func testQuedaDepoisDeGravarEAntesDeRegistrarAdotaOArquivo() throws {
        let amb = try Ambiente()
        let doc = UUID().uuidString
        _ = try amb.disco.gravar(IdBloco(documento: doc, fonte: .mac, geracao: 0, seq: 0), inicio: 0, amostras: Sintetico.pcm(roteiro, segundos: 1))
        // Temporário de uma gravação interrompida: nunca foi durável, não é adotado.
        let pasta = try amb.disco.pasta(documento: doc)
        try Data(count: 100).write(to: pasta.appendingPathComponent("mac_0_1_16000_16000.f32.tmp"))
        let reaberto = try amb.reabrir()
        let r = reaberto.prepararAbertura(diarios: nil, disco: amb.disco)
        XCTAssertEqual(r.adotados, 1)
        XCTAssertEqual(try reaberto.blocos(documento: doc).map(\.estado), [.capturado])
        XCTAssertTrue(try reaberto.recuperaveis(excluindo: nil).contains { $0.id == doc })
        XCTAssertFalse(FileManager.default.fileExists(atPath: pasta.appendingPathComponent("mac_0_1_16000_16000.f32.tmp").path))
        // Repetir a abertura não adota de novo.
        XCTAssertEqual(reaberto.prepararAbertura(diarios: nil, disco: amb.disco).adotados, 0)
    }

    /// Vários órfãos da mesma costura, em qualquer ordem na pasta, não pulam o início.
    func testAdocaoDeVariosOrfaosComecaNoPrimeiroBloco() async throws {
        let amb = try Ambiente()
        let doc = UUID().uuidString
        let falas = ["primeira", "segunda", "terceira", "quarta", "quinta"]
        let roteiro = falas.enumerated().map { i, t in PalavraReconhecida(texto: t, inicio: Double(i) + 0.2, fim: Double(i) + 0.6) }
        let pcm = Sintetico.pcm(roteiro, segundos: 5)
        // Gravados do fim para o começo, para a listagem da pasta não coincidir com a ordem da fala.
        for seq in (0..<5).reversed() {
            let a = seq * taxaLancamento
            _ = try amb.disco.gravar(IdBloco(documento: doc, fonte: .mac, geracao: 0, seq: seq), inicio: a,
                                     amostras: Array(pcm[a..<(a + taxaLancamento)]))
        }
        let reaberto = try amb.reabrir()
        XCTAssertEqual(reaberto.prepararAbertura(diarios: nil, disco: amb.disco).adotados, 5)
        XCTAssertEqual(try reaberto.costuras(documento: doc).first?.confirmadoAte, 0)
        let p = ProcessadorDeReuniao(documento: doc, armazem: reaberto, disco: amb.disco, reconhecedor: Oraculo(roteiro))
        try await p.recuperar()
        try await p.encerrarCosturas()
        let identificados = try await p.passadaFinal(Identificador([FaixaDeVoz(voz: "pessoa-1", inicio: 0, fim: 5)]))
        XCTAssertTrue(identificados)
        let palavras = try reaberto.documento(doc)?.trechos.flatMap { $0.texto.split(separator: " ").map(String.init) } ?? []
        XCTAssertEqual(palavras, falas)
    }

    /// Dois trechos que começam na mesma amostra não travam a confirmação.
    func testTrechosComMesmoInicioNaoColidem() async throws {
        let amb = try Ambiente()
        let doc = try amb.novaReuniao()
        let coletor = ColetorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, inicio: 0)
        try await coletor.abrir(.mac, em: 0)
        let roteiro = [PalavraReconhecida(texto: "ok.", inicio: 1.0, fim: 1.15),
                       PalavraReconhecida(texto: "continuando", inicio: 1.0, fim: 1.5)]
        try await amb.capturar(coletor, .mac, Sintetico.pcm(roteiro, segundos: 3))
        try await coletor.encerrar(em: 3)
        let p = ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, reconhecedor: Oraculo(roteiro))
        try await p.fechar()
        XCTAssertEqual(Set(try amb.textos(doc)), ["ok.", "continuando"])
        let ids = Costura.trechos(roteiro, documento: doc, fonte: .mac, geracao: 0).map(\.id)
        XCTAssertEqual(Set(ids).count, 2)
        XCTAssertEqual(ids, Costura.trechos(roteiro, documento: doc, fonte: .mac, geracao: 0).map(\.id))
    }

    /// Trecho de outro passo que começa na amostra de um já gravado ganha ID próprio;
    /// o mesmo trecho repetido não duplica.
    func testInserirTrechoComIDOcupadoGanhaSufixoERepeticaoNaoDuplica() throws {
        let amb = try Ambiente()
        let doc = try amb.novaReuniao()
        let a = TrechoNovo(id: "\(doc)|mac|0|16000", fonte: .mac, inicio: 1.0, fim: 1.2, texto: "ok.", voz: "Chamada")
        let b = TrechoNovo(id: a.id, fonte: .mac, inicio: 1.0, fim: 1.6, texto: "continuando", voz: "Chamada")
        let c = try ConexaoSQLite(caminho: amb.banco, escrita: true)
        try ArmazemLancamento.inserir(a, documento: doc, versao: "", em: c)
        try ArmazemLancamento.inserir(b, documento: doc, versao: "", em: c)
        try ArmazemLancamento.inserir(a, documento: doc, versao: "", em: c)
        let ids = try c.consultar("SELECT id FROM lt_trechos WHERE documento = ? ORDER BY id", [.texto(doc)]) { $0.texto(0) }
        XCTAssertEqual(ids, [a.id, a.id + "#1"])
    }

    func testRegistroComArquivoAusenteViraLacunaVisivel() async throws {
        let amb = try Ambiente()
        let doc = try await capturada(amb, segundos: 3)
        let bloco = try XCTUnwrap(amb.armazem.blocos(documento: doc).first)
        amb.disco.remover(bloco.arquivo!)
        let reaberto = try amb.reabrir()
        XCTAssertEqual(reaberto.prepararAbertura(diarios: nil, disco: amb.disco).lacunas, 1)
        XCTAssertEqual(try reaberto.blocos(documento: doc).first?.estado, .lacuna)
        XCTAssertNotNil(try reaberto.documento(doc)?.lacunas)
    }

    func testSomaDivergenteNaLeituraViraLacuna() async throws {
        let amb = try Ambiente()
        let doc = try await capturada(amb, segundos: 2)
        let bloco = try XCTUnwrap(amb.armazem.blocos(documento: doc).first)
        let url = amb.disco.raiz.appendingPathComponent(bloco.arquivo!)
        var dados = try Data(contentsOf: url)
        dados[0] ^= 0xFF
        try dados.write(to: url)
        try await ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, reconhecedor: Oraculo(roteiro)).recuperar()
        XCTAssertEqual(try amb.armazem.blocos(documento: doc).first?.estado, .lacuna)
    }

    func testQuedaEntreRegistroECommitRecuperaPeloMesmoCaminho() async throws {
        let amb = try Ambiente()
        let interrompida = try await capturada(amb)
        XCTAssertTrue(try amb.armazem.blocos(documento: interrompida).allSatisfy { $0.estado == .capturado })
        try await ProcessadorDeReuniao(documento: interrompida, armazem: try amb.reabrir(), disco: amb.disco, reconhecedor: Oraculo(roteiro)).recuperar()
        // A mesma captura processada ao vivo produz o mesmo texto.
        let aoVivo = try await capturada(amb)
        try await ProcessadorDeReuniao(documento: aoVivo, armazem: amb.armazem, disco: amb.disco, reconhecedor: Oraculo(roteiro)).fechar()
        XCTAssertEqual(try amb.palavras(interrompida), roteiro.map(\.texto))
        XCTAssertEqual(try amb.textos(interrompida), try amb.textos(aoVivo))
    }

    func testQuedaDepoisDoCommitEAntesDaLimpezaRepeteALimpeza() async throws {
        let amb = try Ambiente()
        let doc = try await capturada(amb)
        let p = ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, reconhecedor: Oraculo(roteiro))
        try await p.fechar()
        // Commit da passada final com liberação, sem apagar os arquivos (queda logo depois).
        XCTAssertTrue(try amb.armazem.gravarFalantes(documento: doc, atribuicao: [:], identificados: true, liberarAudio: true))
        let blocos = try amb.armazem.blocos(documento: doc)
        XCTAssertTrue(blocos.allSatisfy { amb.disco.existe($0) })
        let reaberto = try amb.reabrir()
        reaberto.prepararAbertura(diarios: nil, disco: amb.disco)
        XCTAssertTrue(blocos.allSatisfy { !amb.disco.existe($0) })
        XCTAssertTrue(try reaberto.blocos(documento: doc).allSatisfy { $0.arquivo == nil })
        XCTAssertEqual(try amb.palavras(doc), roteiro.map(\.texto), "a limpeza não toca no texto")
    }

    func testTransacaoExigeEstadoETentativaEsperados() async throws {
        let amb = try Ambiente()
        let doc = try await capturada(amb)
        let a = ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, reconhecedor: Oraculo(roteiro))
        let b = ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, reconhecedor: Oraculo(roteiro))
        try await a.fechar()
        // B tenta confirmar a partir de um ponto antigo com outro dono: nada muda e nada duplica.
        let ok = try amb.armazem.confirmar(.init(documento: doc, fonte: .mac, geracao: 0, dono: "outro", esperado: 0, novo: 16_000,
                                                 trechos: [TrechoNovo(id: "x", fonte: .mac, inicio: 0, fim: 1, texto: "duplicado", voz: "chamada")],
                                                 comAtividade: [], pendencia: nil, fechar: false))
        XCTAssertFalse(ok)
        try await b.recuperar()
        XCTAssertEqual(try amb.palavras(doc), roteiro.map(\.texto))
    }

    // MARK: Descarte

    func testDescartarGravaEstadoTerminalAntesDaLimpeza() async throws {
        let amb = try Ambiente()
        let doc = try await capturada(amb)
        let arquivos = try amb.armazem.blocos(documento: doc).compactMap(\.arquivo)
        XCTAssertTrue(try amb.armazem.descartar(documento: doc))
        XCTAssertTrue(try amb.armazem.blocos(documento: doc).allSatisfy { $0.estado == .descartado })
        XCTAssertTrue(arquivos.allSatisfy { FileManager.default.fileExists(atPath: amb.disco.raiz.appendingPathComponent($0).path) },
                      "o estado terminal vem antes de qualquer arquivo sair")
        // Recuperação concorrente depois do descarte não confirma nada.
        do {
            try await ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, reconhecedor: Oraculo(roteiro)).recuperar()
        } catch {}
        XCTAssertTrue(try amb.textos(doc).isEmpty)
        let reaberto = try amb.reabrir()
        reaberto.prepararAbertura(diarios: nil, disco: amb.disco)
        XCTAssertTrue(arquivos.allSatisfy { !FileManager.default.fileExists(atPath: amb.disco.raiz.appendingPathComponent($0).path) })
        XCTAssertFalse(try reaberto.recuperaveis(excluindo: nil).contains { $0.id == doc }, "descartado não reaparece")
        XCTAssertFalse(try reaberto.lista(tipo: "reuniao").contains { $0.id == doc })
    }

    func testDescartarPreservaFalaConfirmadaEOcultaSoOPendente() async throws {
        let amb = try Ambiente()
        let doc = try await capturada(amb, segundos: 5)
        try await ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, reconhecedor: Oraculo(roteiro)).fechar()
        XCTAssertTrue(try amb.armazem.descartar(documento: doc))
        XCTAssertEqual(try amb.palavras(doc), roteiro.map(\.texto))
        XCTAssertTrue(try amb.armazem.lista(tipo: "reuniao").contains { $0.id == doc })
    }

    // MARK: Garantia 4 e edições

    func testPCMRetidoAtePassadaFinalEEdicoesPrevalecem() async throws {
        let amb = try Ambiente()
        let tres = [PalavraReconhecida(texto: "um.", inicio: 0.3, fim: 0.7), PalavraReconhecida(texto: "dois.", inicio: 2.0, fim: 2.4),
                    PalavraReconhecida(texto: "três.", inicio: 3.7, fim: 4.1)]
        let doc = try await capturada(amb, segundos: 5, roteiro: tres)
        let p = ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, reconhecedor: Oraculo(tres))
        try await p.fechar()
        XCTAssertTrue(try amb.armazem.blocos(documento: doc).allSatisfy { amb.disco.existe($0) }, "texto confirmado não libera o PCM")
        let antes = try XCTUnwrap(amb.armazem.documento(doc))
        XCTAssertEqual(Set(antes.vozes.map(\.nome)), ["Chamada"])
        let editado = try XCTUnwrap(antes.trechos.first?.id)
        try amb.armazem.reatribuir(documento: doc, trecho: editado, voz: "voce")

        // Passada final: duas pessoas na chamada.
        let faixas = [FaixaDeVoz(voz: "pessoa-a", inicio: 0, fim: 1.5), FaixaDeVoz(voz: "pessoa-b", inicio: 1.5, fim: 3.0),
                      FaixaDeVoz(voz: "pessoa-a", inicio: 3.0, fim: 5)]
        let r1 = try await p.passadaFinal(Identificador(faixas))

        XCTAssertTrue(r1)
        let depois = try XCTUnwrap(amb.armazem.documento(doc))
        XCTAssertEqual(depois.trechos.map(\.texto), antes.trechos.map(\.texto), "a passada final não reescreve texto")
        XCTAssertEqual(depois.trechos.first { $0.id == editado }?.voz, "voce", "edição manual prevalece sobre a atribuição final")
        XCTAssertTrue(depois.vozes.contains { $0.nome == "Pessoa 1" })
        XCTAssertTrue(try amb.armazem.blocos(documento: doc).allSatisfy { $0.arquivo == nil }, "o PCM sai depois do commit da passada final")

        // Edições depois da passada final: renomear e unir.
        let pessoas = depois.vozes.filter { $0.id.hasPrefix("pessoa") }
        XCTAssertEqual(pessoas.count, 2)
        guard pessoas.count == 2 else { return }
        try amb.armazem.renomear(documento: doc, voz: pessoas[0].id, nome: "Ana")
        try amb.armazem.renomear(documento: doc, voz: pessoas[1].id, nome: "ana")
        let final = try XCTUnwrap(amb.armazem.documento(doc))
        XCTAssertEqual(final.vozes.filter { $0.id != "voce" }.map(\.nome), ["Ana"])
        XCTAssertEqual(final.trechos.map(\.texto), antes.trechos.map(\.texto))
    }

    func testPassadaFinalIndisponivelMantemAudioParaNovaTentativa() async throws {
        let amb = try Ambiente()
        let doc = try await capturada(amb, segundos: 5)
        let p = ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, reconhecedor: Oraculo(roteiro))
        try await p.fechar()
        let r2 = try await p.passadaFinal(Identificador([], falhar: true))

        XCTAssertFalse(r2)
        XCTAssertTrue(try amb.armazem.blocos(documento: doc).allSatisfy { amb.disco.existe($0) })
        XCTAssertTrue(try amb.armazem.recuperaveis(excluindo: nil).contains { $0.id == doc })
        let r3 = try await p.passadaFinal(Identificador([FaixaDeVoz(voz: "x", inicio: 0, fim: 5)]))

        XCTAssertTrue(r3)
        XCTAssertFalse(try amb.armazem.recuperaveis(excluindo: nil).contains { $0.id == doc })
    }

    func testEdicoesFeitasAntesDaRecuperacaoContinuamDepois() async throws {
        let amb = try Ambiente()
        let doc = try await capturada(amb, segundos: 5)
        // Parte transcrita, depois queda.
        let p = ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, reconhecedor: Oraculo(roteiro))
        await p.blocoSelado(fonte: .mac, geracao: 0, fim: 0)
        let parcial = try XCTUnwrap(amb.armazem.documento(doc))
        let alvo = try XCTUnwrap(parcial.trechos.first)
        try amb.armazem.atribuirNovaPessoa(documento: doc, trecho: alvo.id, nome: "Bruno")
        try amb.armazem.definir(documento: doc, .nome("Planejamento"))
        try await ProcessadorDeReuniao(documento: doc, armazem: try amb.reabrir(), disco: amb.disco, reconhecedor: Oraculo(roteiro)).recuperar()
        let recuperada = try XCTUnwrap(amb.armazem.documento(doc))
        XCTAssertEqual(recuperada.nome, "Planejamento")
        XCTAssertEqual(recuperada.vozes.first { $0.id == recuperada.trechos.first { $0.id == alvo.id }?.voz }?.nome, "Bruno")
        XCTAssertEqual(try amb.palavras(doc), roteiro.map(\.texto))
    }

    // MARK: Teto de saída com escrita lenta e captura ativa

    @MainActor
    func testSaidaComEscritaLentaRespondeEmDoisSegundos() async throws {
        for custo in [1.2, 3.0] {
            final class Lenta: @unchecked Sendable { var ligada = false }
            let lenta = Lenta()
            let amb = try Ambiente(escrita: { dados, url in
                if lenta.ligada { Thread.sleep(forTimeInterval: custo) }
                try DiscoDeBlocos.escritaDuravel(dados, url)
            })
            let doc = try amb.novaReuniao()
            let coletor = ColetorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, inicio: 0)
            try await coletor.abrir(.mac, em: 0)
            let pcm = Sintetico.pcm(roteiro, segundos: 4)
            try await amb.capturar(coletor, .mac, Array(pcm[0..<(2 * taxaLancamento + 8_000)]))
            let duraveis = try amb.armazem.blocos(documento: doc).count
            lenta.ligada = true
            // Captura continua chegando enquanto o app sai.
            let captura = Task.detached {
                for i in stride(from: 2 * taxaLancamento + 8_000, to: pcm.count, by: 1_600) {
                    try? await coletor.receber(.mac, tempo: Double(i) / Double(taxaLancamento), amostras: Array(pcm[i..<min(pcm.count, i + 1_600)]))
                }
            }
            let saida = SaidaComPrazo()
            let inicio = ContinuousClock.now
            var resposta: Duration?
            var maior = Duration.zero
            saida.iniciar(prazo: .seconds(2), trabalho: { try? await coletor.encerrar(em: 4) },
                          responder: { resposta = inicio.duration(to: .now) })
            var anterior = ContinuousClock.now
            while resposta == nil {
                try await Task.sleep(for: .milliseconds(10))
                maior = max(maior, anterior.duration(to: .now)); anterior = .now
            }
            let tempo = try XCTUnwrap(resposta)
            XCTAssertLessThan(tempo, .milliseconds(2_150), "custo \(custo)")
            XCTAssertLessThan(maior, .milliseconds(200), "ator principal preso com custo \(custo)")
            XCTAssertTrue(try amb.armazem.blocos(documento: doc).prefix(duraveis).allSatisfy { amb.disco.existe($0) })
            // No instante da resposta: com 1,2 s a primeira gravação lenta já está registrada; com 3,0 s
            // nada novo consta como durável e o que ainda estava em memória não é garantido.
            let registrados = try amb.armazem.blocos(documento: doc).count
            if custo < 2 { XCTAssertGreaterThan(registrados, duraveis) } else { XCTAssertEqual(registrados, duraveis) }
            captura.cancel()
            try await Task.sleep(for: .seconds(custo))
            lenta.ligada = false
        }
    }

    func testSaidaDuranteGravacaoPausaEPassadaFinalReabreRecuperavel() async throws {
        let amb = try Ambiente()
        // Gravando: queda com captura aberta (nada selado depois do último bloco).
        let gravando = try await capturada(amb, segundos: 3)
        // Pausada: o coletor selou tudo no marco da pausa.
        let pausada = try amb.novaReuniao()
        let c2 = ColetorDeReuniao(documento: pausada, armazem: amb.armazem, disco: amb.disco, inicio: 0)
        try await c2.abrir(.mac, em: 0)
        try await amb.capturar(c2, .mac, Sintetico.pcm(roteiro, segundos: 2.5))
        try await c2.pausar(em: 2.5)
        try amb.armazem.mudarEstado(documento: pausada, para: "pausada", se: ["gravando"])
        // Passada final: texto completo, falantes ainda não identificados.
        let final = try await capturada(amb, segundos: 5)
        try await ProcessadorDeReuniao(documento: final, armazem: amb.armazem, disco: amb.disco, reconhecedor: Oraculo(roteiro)).fechar()
        try amb.armazem.mudarEstado(documento: final, para: "processando", se: ["gravando"])

        let reaberto = try amb.reabrir()
        reaberto.prepararAbertura(diarios: nil, disco: amb.disco)
        let lista = try reaberto.recuperaveis(excluindo: nil).map(\.id)
        XCTAssertEqual(Set(lista), [gravando, pausada, final])
        XCTAssertEqual(try reaberto.blocos(documento: pausada).reduce(0) { $0 + $1.quantidade }, Int(2.5 * Double(taxaLancamento)),
                       "a pausa selou a fração do último bloco")
        for doc in lista {
            let p = ProcessadorDeReuniao(documento: doc, armazem: reaberto, disco: amb.disco, reconhecedor: Oraculo(roteiro))
            try await p.recuperar()
            let r4 = try await p.passadaFinal(Identificador([FaixaDeVoz(voz: "a", inicio: 0, fim: 5)]))

            XCTAssertTrue(r4)
        }
        XCTAssertTrue(try reaberto.recuperaveis(excluindo: nil).isEmpty)
    }

    // MARK: Garantia 7: versões de arquivo

    private func publicar(_ amb: Ambiente, _ v: ArmazemLancamento.Versao, _ texto: String) throws -> Bool {
        try amb.armazem.publicar(.init(versao: v.id, identidade: "id-" + texto,
                                       trechos: texto.isEmpty ? [] : [TrechoNovo(id: v.id + "|0", fonte: .arquivo, inicio: 0, fim: 1, texto: texto, voz: "a")],
                                       duracao: 1, bruto: texto, processamento: 0.1, baixaConfianca: false))
    }

    func testVersaoCanceladaAntesDaPublicacaoPreservaAAnterior() throws {
        let amb = try Ambiente()
        let doc = UUID().uuidString
        let v1 = try amb.armazem.criarVersao(documento: doc, nome: "a.wav", origem: "/tmp/a.wav")
        XCTAssertTrue(try publicar(amb, v1, "primeira"))
        let v2 = try amb.armazem.criarVersao(documento: doc, nome: "a.wav", origem: "/tmp/a.wav")
        XCTAssertTrue(try amb.armazem.encerrarVersao(v2.id, estado: "cancelada"))
        XCTAssertFalse(try publicar(amb, v2, "cancelada"), "cancelamento imediatamente antes da publicação vence")
        XCTAssertEqual(try amb.textos(doc), ["primeira"])
        let v3 = try amb.armazem.criarVersao(documento: doc, nome: "a.wav", origem: "/tmp/a.wav")
        XCTAssertFalse(try publicar(amb, v3, ""), "resultado vazio inesperado não substitui")
        XCTAssertEqual(try amb.textos(doc), ["primeira"])
    }

    func testConclusaoForaDeOrdemNaoTornaVisivelATentativaAntiga() throws {
        let amb = try Ambiente()
        let doc = UUID().uuidString
        let antiga = try amb.armazem.criarVersao(documento: doc, nome: "a.wav", origem: "/tmp/a.wav")
        let nova = try amb.armazem.criarVersao(documento: doc, nome: "a.wav", origem: "/tmp/a.wav")
        XCTAssertTrue(try publicar(amb, nova, "nova"))
        XCTAssertFalse(try publicar(amb, antiga, "antiga"))
        XCTAssertEqual(try amb.textos(doc), ["nova"])
        XCTAssertEqual(try amb.armazem.versoes(documento: doc).first { $0.id == antiga.id }?.estado, "superada")
    }

    func testArquivoQueMudaDuranteATentativaNaoPublica() async throws {
        let amb = try Ambiente()
        let doc = UUID().uuidString
        let roteiro = Sintetico.roteiro(["arquivo", "sintético"], inicio: 0.5)
        let pcm = PCMEmMemoria(amostras: Sintetico.pcm(roteiro, segundos: 3))
        let processador = ProcessadorDeArquivo(armazem: amb.armazem, reconhecedor: Oraculo(roteiro), identificador: nil)
        let v1 = try amb.armazem.criarVersao(documento: doc, nome: "a.wav", origem: "/tmp/a.wav")
        let desfecho = try await processador.transcrever(versao: v1, pcm: pcm, wav: nil)
        XCTAssertEqual(desfecho, .publicada)
        XCTAssertEqual(try amb.armazem.documento(doc)?.fase, "pronto")
        let v2 = try amb.armazem.criarVersao(documento: doc, nome: "a.wav", origem: "/tmp/a.wav")
        struct Mudou: Error {}
        do {
            _ = try await processador.transcrever(versao: v2, pcm: pcm, wav: nil, conferirEntrada: { throw Mudou() })
            XCTFail("deveria falhar")
        } catch is Mudou {}
        XCTAssertEqual(try amb.armazem.versaoAtiva(documento: doc), v1.id)
        XCTAssertEqual(try amb.palavras(doc), ["arquivo", "sintético"])
    }

    // MARK: Garantia 6: migração

    private func legado(_ caminho: String, documentos: [TranscricaoLonga]) throws {
        let banco = try Banco(caminho: caminho)
        try banco.inserir(Ditado(iniciadoEm: 1, palavras: 3, textoBruto: "ditado comum", texto: "Ditado comum.", estado: "concluido"))
        try banco.salvar(termo: Termo(id: 0, termo: "Dita.se", ouvidoComo: ["dita ce"]))
        for d in documentos { try banco.salvar(transcricao: d) }
    }

    private func dump(_ caminho: String, _ tabela: String) throws -> [String] {
        var db: OpaquePointer?
        sqlite3_open(caminho, &db)
        defer { sqlite3_close(db) }
        var stmt: OpaquePointer?
        sqlite3_prepare_v2(db, "SELECT * FROM \(tabela) ORDER BY 1", -1, &stmt, nil)
        defer { sqlite3_finalize(stmt) }
        var linhas: [String] = []
        while sqlite3_step(stmt) == SQLITE_ROW {
            linhas.append((0..<sqlite3_column_count(stmt)).map { sqlite3_column_text(stmt, $0).map { String(cString: $0) } ?? "∅" }.joined(separator: "|"))
        }
        return linhas
    }

    private func reuniaoLegada(_ trechos: [TrechoLongo]) -> TranscricaoLonga {
        var doc = TranscricaoLonga(nome: "Reunião antiga", fase: "pronta", fonte: "mac")
        doc.trechos = trechos
        doc.atualizarVozes()
        return doc
    }

    func testMigracaoPreservaBancoLegadoEDitadoComumEImportaUmaVez() throws {
        var t1 = TrechoLongo(inicio: 0, fim: 1, voz: "offline-0", texto: "fala salva no histórico")
        t1.vozManual = true
        var reuniao = reuniaoLegada([t1, TrechoLongo(inicio: 2, fim: 3, voz: "voce", texto: "minha fala")])
        _ = reuniao.renomear(voz: "offline-0", nome: "Carla")
        reuniao.resumo = ["Tópico sintético"]
        reuniao.resumoGerado = true
        var arquivo = TranscricaoLonga(nome: "a.wav", fase: "pronto")
        arquivo.trechos = [TrechoLongo(inicio: 0, fim: 1, voz: "offline-1", texto: "arquivo antigo")]
        arquivo.atualizarVozes()
        arquivo.bruto = "arquivo antigo"
        arquivo.oculto = true
        var caminho = ""
        let amb = try Ambiente(legado: { c in caminho = c; try self.legado(c, documentos: [reuniao, arquivo]) })
        _ = amb
        let antes = try ["ditados", "trechos", "dicionario", "config", "transcricoes_longas"].map { try dump(caminho, $0) }
        let r = amb.armazem.prepararAbertura(diarios: nil, disco: amb.disco)
        XCTAssertEqual(r.documentos, 2)
        XCTAssertTrue(r.falhas.isEmpty, "\(r.falhas)")
        let depois = try ["ditados", "trechos", "dicionario", "config", "transcricoes_longas"].map { try dump(caminho, $0) }
        XCTAssertEqual(antes, depois, "esquema e dados do ditado comum e o JSON original ficam intactos")

        let migrada = try XCTUnwrap(amb.armazem.documento(reuniao.id))
        XCTAssertEqual(migrada.trechos.map(\.id), reuniao.trechos.map(\.id))
        XCTAssertEqual(migrada.trechos.map(\.texto), reuniao.trechos.map(\.texto))
        XCTAssertEqual(migrada.vozes.first { $0.id == "offline-0" }?.nome, "Carla")
        XCTAssertEqual(migrada.resumo, ["Tópico sintético"])
        XCTAssertEqual(migrada.fase, "pronta")
        let arq = try XCTUnwrap(amb.armazem.documento(arquivo.id))
        XCTAssertEqual(arq.fase, "pronto")
        XCTAssertEqual(arq.oculto, true)
        XCTAssertEqual(arq.bruto, "arquivo antigo")

        // Repetir não gera cópia nova.
        let contar = { try self.dump(caminho, "lt_trechos").count }
        let n = try contar()
        XCTAssertEqual(amb.armazem.prepararAbertura(diarios: nil, disco: amb.disco).documentos, 0)
        XCTAssertEqual(try contar(), n)
    }

    /// Diário antigo com trechos por bloco, metadado e PCM.
    private func diarioLegado(_ amb: Ambiente, id: String, trechosPorBloco: [Int: [TrechoLongo]], pcm: [Int: [Float]],
                              meta: (TranscricaoLonga) -> TranscricaoLonga = { $0 }) throws {
        var doc = TranscricaoLonga(nome: "Reunião interrompida", fase: "gravando", fonte: "mac")
        doc.id = id
        let diario = try DiarioReuniao(raiz: amb.diarios, documento: meta(doc))
        for (inicio, amostras) in pcm { try diario.salvarAudio(inicio: inicio, sistema: amostras, microfone: []) }
        for (inicio, trechos) in trechosPorBloco { try diario.salvarTrechos(inicio: inicio, trechos: trechos) }
    }

    func testMesmoIntervaloComFalaDiferenteNadaEApagado() throws {
        let doBanco = TrechoLongo(inicio: 0, fim: 1, voz: "a", texto: "fala só no banco")
        var reuniao = reuniaoLegada([doBanco])
        let id = reuniao.id
        reuniao.duracao = 3
        let amb = try Ambiente(legado: { c in try self.legado(c, documentos: [reuniao]) })
        let doDiario = TrechoLongo(inicio: 0, fim: 3, voz: "fluxo-0", texto: "fala diferente só no diário")
        var mesmoID = TrechoLongo(inicio: 0, fim: 1, voz: "a", texto: "mesmo ID com outro texto")
        mesmoID.id = doBanco.id
        try diarioLegado(amb, id: id, trechosPorBloco: [0: [doDiario, mesmoID]], pcm: [0: Sintetico.pcm(roteiro, segundos: 3)])
        let r = amb.armazem.prepararAbertura(diarios: amb.diarios, disco: amb.disco)
        XCTAssertTrue(r.falhas.isEmpty, "\(r.falhas)")
        XCTAssertEqual(r.divergencias, 1)
        let doc = try XCTUnwrap(amb.armazem.documento(id))
        XCTAssertEqual(Set(doc.trechos.map(\.texto)), ["fala só no banco", "fala diferente só no diário", "mesmo ID com outro texto"])
        XCTAssertEqual(doc.trechos.filter { $0.divergente == true }.map(\.texto), ["mesmo ID com outro texto"])
        XCTAssertNotNil(doc.aviso)
        XCTAssertEqual(try amb.armazem.blocos(documento: id).count, 1, "o PCM do diário veio junto")
        XCTAssertTrue(try amb.armazem.blocos(documento: id).allSatisfy { amb.disco.existe($0) })
    }

    func testPartesExclusivasNoBancoENoAudioPendenteViramUniaoERepetirNaoMuda() async throws {
        let rot = Sintetico.roteiro(["fala", "pendente"], inicio: 9.2, duracao: 0.4, pausa: 0.3)
        var reuniao = reuniaoLegada([TrechoLongo(inicio: 7, fim: 8, voz: "a", texto: "só no banco")])
        reuniao.duracao = 8
        let id = reuniao.id
        let amb = try Ambiente(legado: { c in try self.legado(c, documentos: [reuniao]) })
        let silencio = [Float](repeating: 0, count: 3 * taxaLancamento)
        let pendente = Array(Sintetico.pcm(rot, segundos: 12)[(9 * taxaLancamento)...])
        try diarioLegado(amb, id: id, trechosPorBloco: [80_000: [TrechoLongo(inicio: 5, fim: 6, voz: "fluxo-0", texto: "só no diário")]],
                         pcm: [80_000: silencio, 144_000: pendente]) { doc in
            var d = doc
            var editado = TrechoLongo(inicio: 5, fim: 6, voz: "manual", texto: "")
            editado.vozManual = true
            d.trechos = [editado]
            return d
        }
        amb.armazem.prepararAbertura(diarios: amb.diarios, disco: amb.disco)
        XCTAssertFalse(FileManager.default.fileExists(atPath: amb.diarios.appendingPathComponent(id).path), "diário conferido sai")
        XCTAssertEqual(Set(try amb.textos(id)), ["só no banco", "só no diário"])
        XCTAssertTrue(try amb.armazem.recuperaveis(excluindo: nil).contains { $0.id == id })
        let p = ProcessadorDeReuniao(documento: id, armazem: amb.armazem, disco: amb.disco, reconhecedor: Oraculo(rot))
        try await p.recuperar()
        XCTAssertEqual(Set(try amb.textos(id)), ["só no banco", "só no diário", "fala pendente"])
        let n = try amb.textos(id).count
        try await p.recuperar()
        amb.armazem.prepararAbertura(diarios: amb.diarios, disco: amb.disco)
        XCTAssertEqual(try amb.textos(id).count, n, "repetir a recuperação e a migração não muda nada")
    }

    func testDitadoComumIntactoComArmazemAberto() throws {
        var caminho = ""
        let amb = try Ambiente(legado: { c in caminho = c; try self.legado(c, documentos: []) })
        let banco = try Banco(caminho: caminho)
        let id = try banco.inserir(Ditado(iniciadoEm: 2, estado: "gravando"))
        try banco.gravarTrecho(ditado: id, ordem: 0, bruto: "trecho em andamento")
        _ = try amb.novaReuniao()
        XCTAssertEqual(try banco.trechos(ditado: id), ["trecho em andamento"])
        XCTAssertEqual(try banco.historico().count, 1)
    }
}
