import Foundation
import XCTest
@testable import Nucleo

/// Regressões de costura, adoção de órfãos e privacidade do áudio em disco.
final class RodadaSeisTests: XCTestCase {
    private final class Caixa: @unchecked Sendable {
        private let trava = NSLock()
        private var ids: [String] = []
        func somar(_ novos: [String]) { trava.withLock { ids += novos } }
        var todos: [String] { trava.withLock { ids } }
    }

    private func palavras(_ armazem: ArmazemLancamento, _ doc: String) throws -> [String] {
        try armazem.documento(doc)?.trechos.sorted { $0.inicio < $1.inicio }
            .flatMap { $0.texto.split(separator: " ").map(String.init) } ?? []
    }

    /// Costura que já existia com ponto à frente de um órfão sem texto não descarta a fala dele.
    func testOrfaoAnteriorAoPontoDeCosturaExistenteViraPendenciaETranscreve() async throws {
        let amb = try Ambiente()
        let doc = try amb.novaReuniao()
        let roteiro = [PalavraReconhecida(texto: "primeira", inicio: 0.2, fim: 0.6),
                       PalavraReconhecida(texto: "terceira", inicio: 2.2, fim: 2.6)]
        let pcm = Sintetico.pcm(roteiro, segundos: 3)
        let c = try ConexaoSQLite(caminho: amb.banco, escrita: true)
        try c.rodar("INSERT INTO lt_costuras (documento, fonte, geracao, confirmado_ate) VALUES (?, 'mac', 0, 32000)", [.texto(doc)])
        for (seq, inicio) in [(0, 0), (2, 32_000)] {
            _ = try amb.disco.gravar(IdBloco(documento: doc, fonte: .mac, geracao: 0, seq: seq), inicio: inicio,
                                     amostras: Array(pcm[inicio..<(inicio + taxaLancamento)]))
        }
        let reaberto = try amb.reabrir()
        XCTAssertEqual(reaberto.prepararAbertura(diarios: nil, disco: amb.disco).adotados, 2)
        XCTAssertEqual(try reaberto.costuras(documento: doc).first?.confirmadoAte, 32_000, "a costura antiga não recua")
        XCTAssertEqual(try reaberto.pendencias(documento: doc).filter { $0.estado == "aberta" }.map(\.inicio), [0])
        let p = ProcessadorDeReuniao(documento: doc, armazem: reaberto, disco: amb.disco, reconhecedor: Oraculo(roteiro))
        try await p.recuperar()
        try await p.encerrarCosturas()
        let identificados = try await p.passadaFinal(Identificador([FaixaDeVoz(voz: "pessoa-1", inicio: 0, fim: 3)]))
        XCTAssertTrue(identificados)
        XCTAssertEqual(try palavras(reaberto, doc), ["primeira", "terceira"])
    }

    /// Adoção sobre intervalo que já tem texto: nada se resolve sozinho. O texto fica como está, o PCM
    /// continua no disco e o motivo fica visível; a passada final recusa liberar.
    private func adotadoSobreTexto(texto gravado: (inicio: Double, fim: Double, texto: String),
                                   roteiro: [PalavraReconhecida], inicioBloco: Int = 0) async throws {
        let amb = try Ambiente()
        let doc = try amb.novaReuniao()
        let c = try ConexaoSQLite(caminho: amb.banco, escrita: true)
        try c.rodar("INSERT INTO lt_costuras (documento, fonte, geracao, confirmado_ate, fechada) VALUES (?, 'mac', 0, 16000, 1)", [.texto(doc)])
        let amostra = Int((gravado.inicio * Double(taxaLancamento)).rounded())
        try ArmazemLancamento.inserir(TrechoNovo(id: "\(doc)|mac|0|\(amostra)", fonte: .mac, inicio: gravado.inicio, fim: gravado.fim,
                                                 texto: gravado.texto, voz: "chamada"), documento: doc, versao: "", em: c)
        let pcm = Sintetico.pcm(roteiro, segundos: 1.6)
        let bloco = try amb.disco.gravar(IdBloco(documento: doc, fonte: .mac, geracao: 0, seq: 0), inicio: inicioBloco,
                                         amostras: Array(pcm[inicioBloco..<(inicioBloco + taxaLancamento)]))
        let reaberto = try amb.reabrir()
        XCTAssertEqual(reaberto.prepararAbertura(diarios: nil, disco: amb.disco).adotados, 1)
        let p = ProcessadorDeReuniao(documento: doc, armazem: reaberto, disco: amb.disco, reconhecedor: Oraculo(roteiro))
        try await p.recuperar()
        try await p.encerrarCosturas()
        do {
            _ = try await p.passadaFinal(Identificador([FaixaDeVoz(voz: "pessoa-1", inicio: 0, fim: 1)]))
            XCTFail("a passada final não pode liberar o áudio com a pendência aberta")
        } catch {}
        XCTAssertEqual(try reaberto.documento(doc)?.trechos.map(\.texto), [gravado.texto])
        XCTAssertTrue(amb.disco.existe(bloco))
        let abertas = try reaberto.pendencias(documento: doc).filter { $0.estado == "aberta" }
        XCTAssertEqual(abertas.count, 1)
        XCTAssertTrue(abertas.first?.motivo.contains("já tem texto") == true, abertas.first?.motivo ?? "")
    }

    func testAdotadoSobreTextoIgualFicaRetidoSemDuplicar() async throws {
        try await adotadoSobreTexto(texto: (0.2, 0.6, "primeira"), roteiro: [PalavraReconhecida(texto: "primeira", inicio: 0.2, fim: 0.6)])
    }

    /// Palavra que faltava no meio do texto gravado não se perde nem entra fora de ordem.
    func testAdotadoSobreTextoComPalavraFaltandoFicaRetido() async throws {
        try await adotadoSobreTexto(texto: (0.10, 0.80, "ligue amanhã"),
                                    roteiro: [PalavraReconhecida(texto: "ligue", inicio: 0.10, fim: 0.25),
                                              PalavraReconhecida(texto: "nunca", inicio: 0.35, fim: 0.50),
                                              PalavraReconhecida(texto: "amanhã", inicio: 0.60, fim: 0.80)])
    }

    /// Repetição legítima e acento lexical não são confundidos (nada é deduzido).
    /// Órfão que começa dentro de um trecho gravado (centro do trecho antes dele) fica retido.
    func testAdotadoQueComecaDentroDeTrechoGravadoFicaRetido() async throws {
        try await adotadoSobreTexto(texto: (0.10, 0.88, "ligue amanhã"),
                                    roteiro: [PalavraReconhecida(texto: "amanhã", inicio: 0.55, fim: 0.88),
                                              PalavraReconhecida(texto: "cedo", inicio: 0.90, fim: 0.98)],
                                    inicioBloco: 9600)
    }

    func testAdotadoSobreTextoComRepeticaoOuAcentoFicaRetido() async throws {
        try await adotadoSobreTexto(texto: (0.05, 0.90, "não"),
                                    roteiro: [PalavraReconhecida(texto: "não", inicio: 0.10, fim: 0.20),
                                              PalavraReconhecida(texto: "não", inicio: 0.60, fim: 0.85)])
        try await adotadoSobreTexto(texto: (0.05, 0.90, "avô"),
                                    roteiro: [PalavraReconhecida(texto: "avó", inicio: 0.10, fim: 0.30),
                                              PalavraReconhecida(texto: "avô", inicio: 0.50, fim: 0.80)])
    }

    /// Pendência normal (sem adoção): todas as palavras da janela entram.
    func testPendenciaNormalRecuperaSegundaOcorrencia() async throws {
        let amb = try Ambiente()
        let doc = try amb.novaReuniao()
        let roteiro = [PalavraReconhecida(texto: "não", inicio: 0.10, fim: 0.20),
                       PalavraReconhecida(texto: "não", inicio: 0.60, fim: 0.85)]
        let coletor = ColetorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, inicio: 0)
        try await coletor.abrir(.mac, em: 0)
        try await amb.capturar(coletor, .mac, Sintetico.pcm(roteiro, segundos: 1))
        try await coletor.encerrar(em: 1)
        let c = try ConexaoSQLite(caminho: amb.banco, escrita: true)
        try c.rodar("INSERT INTO lt_costuras (documento, fonte, geracao, confirmado_ate, fechada) VALUES (?, 'mac', 0, 16000, 1) ON CONFLICT DO UPDATE SET confirmado_ate = 16000, fechada = 1", [.texto(doc)])
        try ArmazemLancamento.inserir(TrechoNovo(id: "\(doc)|mac|0|1600", fonte: .mac, inicio: 0.10, fim: 0.20, texto: "não", voz: "chamada"),
                                      documento: doc, versao: "", em: c)
        try c.rodar("INSERT INTO lt_pendencias (id, documento, fonte, geracao, inicio, fim, motivo, estado) VALUES (?, ?, 'mac', 0, 6400, 16000, 'voz sem texto', 'aberta')",
                    [.texto("\(doc)|mac|0|6400"), .texto(doc)])
        let p = ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, reconhecedor: Oraculo(roteiro))
        try await p.repetirPendencias()
        XCTAssertEqual(try palavras(amb.armazem, doc), ["não", "não"])
    }

    /// Pendência normal sobre texto também fica retida, decidida antes de reconhecer;
    /// a pendência independente seguinte continua sendo tentada.
    func testPendenciaSobreTextoRetidaSemReconhecerEASeguinteResolve() async throws {
        let amb = try Ambiente()
        let doc = try amb.novaReuniao()
        let roteiro = [PalavraReconhecida(texto: "ligue", inicio: 0.10, fim: 0.25),
                       PalavraReconhecida(texto: "nunca", inicio: 0.35, fim: 0.50),
                       PalavraReconhecida(texto: "amanhã", inicio: 0.60, fim: 0.80),
                       PalavraReconhecida(texto: "depois", inicio: 2.20, fim: 2.60)]
        let coletor = ColetorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, inicio: 0)
        try await coletor.abrir(.mac, em: 0)
        try await amb.capturar(coletor, .mac, Sintetico.pcm(roteiro, segundos: 3))
        try await coletor.encerrar(em: 3)
        let c = try ConexaoSQLite(caminho: amb.banco, escrita: true)
        try ArmazemLancamento.inserir(TrechoNovo(id: "\(doc)|mac|0|1600", fonte: .mac, inicio: 0.10, fim: 0.80, texto: "ligue amanhã", voz: "chamada"),
                                      documento: doc, versao: "", em: c)
        for (inicio, fim) in [(5600, 8000), (32000, 44800)] {
            try c.rodar("INSERT INTO lt_pendencias (id, documento, fonte, geracao, inicio, fim, motivo, estado) VALUES (?, ?, 'mac', 0, ?, ?, 'voz sem texto', 'aberta')",
                        [.texto("\(doc)|mac|0|\(inicio)"), .texto(doc), .inteiro(Int64(inicio)), .inteiro(Int64(fim))])
        }
        let oraculo = Oraculo(roteiro)
        let p = ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, reconhecedor: oraculo)
        try await p.repetirPendencias()
        XCTAssertEqual(oraculo.chamadas, 1, "só a pendência sem texto sobreposto chama o reconhecedor")
        let pend = try amb.armazem.pendencias(documento: doc)
        XCTAssertEqual(pend.first { $0.inicio == 5600 }?.estado, "aberta")
        XCTAssertTrue(pend.first { $0.inicio == 5600 }?.motivo.contains("já tem texto") == true)
        XCTAssertEqual(pend.first { $0.inicio == 32000 }?.estado, "resolvida")
        XCTAssertEqual(try amb.armazem.documento(doc)?.trechos.map(\.texto), ["ligue amanhã", "depois"])
    }

    /// Palavra confirmada pelo centro que começa antes do fim da pendência não retém o áudio.
    func testPalavraNaBordaDaPendenciaNaoRetem() async throws {
        let amb = try Ambiente()
        let doc = try amb.novaReuniao()
        let roteiro = [PalavraReconhecida(texto: "antes", inicio: 0.60, fim: 1.00),
                       PalavraReconhecida(texto: "depois", inicio: 1.13, fim: 1.55)]
        let coletor = ColetorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, inicio: 0)
        try await coletor.abrir(.mac, em: 0)
        try await amb.capturar(coletor, .mac, Sintetico.pcm(roteiro, segundos: 2))
        try await coletor.encerrar(em: 2)
        let c = try ConexaoSQLite(caminho: amb.banco, escrita: true)
        try ArmazemLancamento.inserir(TrechoNovo(id: "\(doc)|mac|0|18080", fonte: .mac, inicio: 1.13, fim: 1.55, texto: "depois", voz: "chamada"),
                                      documento: doc, versao: "", em: c)
        try c.rodar("INSERT INTO lt_pendencias (id, documento, fonte, geracao, inicio, fim, motivo, estado) VALUES (?, ?, 'mac', 0, 8000, 19200, 'voz sem texto', 'aberta')",
                    [.texto("\(doc)|mac|0|8000"), .texto(doc)])
        let p = ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, reconhecedor: Oraculo(roteiro))
        try await p.repetirPendencias()
        XCTAssertEqual(try amb.armazem.pendencias(documento: doc).first?.estado, "resolvida")
        XCTAssertEqual(try palavras(amb.armazem, doc), ["antes", "depois"])
    }

    /// O evento ao vivo leva o ID com que o trecho ficou no banco, inclusive com sufixo.
    func testEventoAoVivoUsaIDPersistidoNaColisaoEntrePassos() async throws {
        let amb = try Ambiente()
        let doc = try amb.novaReuniao()
        let roteiro = [PalavraReconhecida(texto: "continuando", inicio: 1.0, fim: 1.6)]
        let c = try ConexaoSQLite(caminho: amb.banco, escrita: true)
        let ocupado = "\(doc)|mac|0|16000"
        try ArmazemLancamento.inserir(TrechoNovo(id: ocupado, fonte: .mac, inicio: 1.0, fim: 1.2, texto: "ok.", voz: "chamada"),
                                      documento: doc, versao: "", em: c)
        let coletor = ColetorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, inicio: 0)
        try await coletor.abrir(.mac, em: 0)
        try await amb.capturar(coletor, .mac, Sintetico.pcm(roteiro, segundos: 3))
        try await coletor.encerrar(em: 3)
        let caixa = Caixa()
        let p = ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, reconhecedor: Oraculo(roteiro))
        await p.definirAoConfirmar { trechos, _ in caixa.somar(trechos.map(\.id)) }
        try await p.fechar()
        let noBanco = try c.consultar("SELECT id FROM lt_trechos WHERE documento = ? AND texto = 'continuando'", [.texto(doc)]) { $0.texto(0) }
        XCTAssertEqual(noBanco, [ocupado + "#1"])
        XCTAssertEqual(caixa.todos, noBanco)
    }

    /// Pendência antiga mais curta (aberta ou já fechada) ou fala parcial no texto
    /// não autorizam liberar o bloco adotado: toda a fala dele termina no texto antes de o PCM sair.
    func testOrfaoAtrasadoComPendenciaCurtaFechadaOuFalaParcialNaoPerdeFala() async throws {
        for cenario in ["pendenciaCurtaAberta", "pendenciaCurtaResolvida"] {
            let amb = try Ambiente()
            let doc = try amb.novaReuniao()
            let roteiro = [PalavraReconhecida(texto: "primeira", inicio: 0.10, fim: 0.20),
                           PalavraReconhecida(texto: "segunda", inicio: 0.48, fim: 0.80)]
            let c = try ConexaoSQLite(caminho: amb.banco, escrita: true)
            try c.rodar("INSERT INTO lt_costuras (documento, fonte, geracao, confirmado_ate, fechada) VALUES (?, 'mac', 0, 16000, 1)", [.texto(doc)])
            switch cenario {
            case "pendenciaCurtaAberta", "pendenciaCurtaResolvida":
                try c.rodar("INSERT INTO lt_pendencias (id, documento, fonte, geracao, inicio, fim, motivo, estado) VALUES (?, ?, 'mac', 0, 0, 4800, 'antiga', ?)",
                            [.texto("\(doc)|mac|0|0"), .texto(doc), .texto(cenario == "pendenciaCurtaAberta" ? "aberta" : "resolvida")])
            default:
                try ArmazemLancamento.inserir(TrechoNovo(id: "\(doc)|mac|0|1600", fonte: .mac, inicio: 0.10, fim: 0.20, texto: "primeira", voz: "chamada"),
                                              documento: doc, versao: "", em: c)
            }
            _ = try amb.disco.gravar(IdBloco(documento: doc, fonte: .mac, geracao: 0, seq: 0), inicio: 0,
                                     amostras: Sintetico.pcm(roteiro, segundos: 1))
            let reaberto = try amb.reabrir()
            XCTAssertEqual(reaberto.prepararAbertura(diarios: nil, disco: amb.disco).adotados, 1, cenario)
            let p = ProcessadorDeReuniao(documento: doc, armazem: reaberto, disco: amb.disco, reconhecedor: Oraculo(roteiro))
            try await p.recuperar()
            try await p.encerrarCosturas()
            let identificados = try await p.passadaFinal(Identificador([FaixaDeVoz(voz: "pessoa-1", inicio: 0, fim: 1)]))
            XCTAssertTrue(identificados, cenario)
            XCTAssertEqual(try palavras(reaberto, doc), ["primeira", "segunda"], cenario)
        }
    }

    /// Pasta de reunião que é link simbólico nunca é seguida pela limpeza.
    func testLimpezaNaoSegueLinkNoLugarDaPastaDaReuniao() throws {
        let amb = try Ambiente()
        let doc = UUID().uuidString
        let externa = amb.raiz.appendingPathComponent("externa", isDirectory: true)
        try FileManager.default.createDirectory(at: externa, withIntermediateDirectories: true)
        let alvo = externa.appendingPathComponent("mac_0_0_0_1_x.f32")
        try Data(count: 4).write(to: alvo)
        try FileManager.default.createDirectory(at: amb.disco.raiz, withIntermediateDirectories: true)
        try FileManager.default.createSymbolicLink(at: amb.disco.raiz.appendingPathComponent(doc), withDestinationURL: externa)
        XCTAssertThrowsError(try amb.disco.apagar("\(doc)/mac_0_0_0_1_x.f32"))
        amb.disco.removerPasta(documento: doc)
        XCTAssertTrue(FileManager.default.fileExists(atPath: alvo.path))
        XCTAssertFalse(DiscoDeBlocos.arquivoValido("\(doc)/a.f32", documento: UUID().uuidString))
    }

    /// A limpeza da passada final não segue referência para arquivo de outra reunião.
    func testLimpezaFinalNaoApagaArquivoDeOutraReuniao() async throws {
        let amb = try Ambiente()
        let doc = try amb.novaReuniao()
        let outra = try amb.disco.gravar(IdBloco(documento: UUID().uuidString, fonte: .mac, geracao: 0, seq: 0), inicio: 0, amostras: [0.4])
        let c = try ConexaoSQLite(caminho: amb.banco, escrita: true)
        try c.rodar("""
            INSERT INTO lt_blocos (id, documento, fonte, geracao, seq, inicio, quantidade, arquivo, tamanho, soma, estado, tentativas, audio_liberado)
            VALUES (?, ?, 'mac', 0, 0, 0, 1, ?, 4, NULL, 'transcrito', 0, 1)
            """, [.texto("\(doc)|mac|0|0"), .texto(doc), .texto(try XCTUnwrap(outra.arquivo))])
        let p = ProcessadorDeReuniao(documento: doc, armazem: amb.armazem, disco: amb.disco, reconhecedor: Oraculo([]))
        await p.limparArquivos()
        XCTAssertTrue(amb.disco.existe(outra))
    }

    // MARK: Privacidade do áudio em disco

    func testApagarSoAceitaArquivoDeUmaReuniao() throws {
        let amb = try Ambiente()
        let doc = UUID().uuidString
        let b = try amb.disco.gravar(IdBloco(documento: doc, fonte: .mac, geracao: 0, seq: 0), inicio: 0, amostras: [0.1, 0.2])
        let fora = amb.raiz.appendingPathComponent("fora.f32")
        try Data([1]).write(to: fora)
        for invalido in ["", "../fora.f32", "\(doc)/../../fora.f32", "\(doc)", "nao-uuid/x.f32", "\(doc)/x.txt"] {
            XCTAssertThrowsError(try amb.disco.apagar(invalido), invalido)
        }
        amb.disco.removerPasta(documento: "")
        amb.disco.removerPasta(documento: "..")
        XCTAssertTrue(FileManager.default.fileExists(atPath: fora.path))
        XCTAssertTrue(amb.disco.existe(b), "a pasta de áudio não pode sumir por id vazio")
        try amb.disco.apagar(try XCTUnwrap(b.arquivo))
        XCTAssertFalse(amb.disco.existe(b))
    }

    func testLimpezaDeAberturaIgnoraCaminhoInvalidoNoBanco() throws {
        let amb = try Ambiente()
        let doc = try amb.novaReuniao()
        let vitima = try amb.disco.gravar(IdBloco(documento: UUID().uuidString, fonte: .mac, geracao: 0, seq: 0), inicio: 0, amostras: [0.3])
        let c = try ConexaoSQLite(caminho: amb.banco, escrita: true)
        try c.rodar("""
            INSERT INTO lt_blocos (id, documento, fonte, geracao, seq, inicio, quantidade, arquivo, tamanho, soma, estado, tentativas, audio_liberado)
            VALUES (?, ?, 'mac', 0, 0, 0, 1, ?, 4, NULL, 'transcrito', 0, 1)
            """, [.texto("\(doc)|mac|0|0"), .texto(doc), .texto("..")])
        let reaberto = try amb.reabrir()
        let r = reaberto.prepararAbertura(diarios: nil, disco: amb.disco)
        XCTAssertTrue(r.falhas.isEmpty, "\(r.falhas)")
        XCTAssertTrue(amb.disco.existe(vitima))
        XCTAssertTrue(FileManager.default.fileExists(atPath: amb.disco.raiz.path))
    }

    func testAdocaoIgnoraLinkSimbolicoERemoveWAVDeIdentificacaoEsquecido() throws {
        let amb = try Ambiente()
        let doc = UUID().uuidString
        let pasta = try amb.disco.pasta(documento: doc)
        let externo = amb.raiz.appendingPathComponent("externo.bin")
        try Data(count: 8).write(to: externo)
        let link = pasta.appendingPathComponent("mac_0_0_0_2_\(DiscoDeBlocos.soma(Data(count: 8))).f32")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: externo)
        let wav = pasta.appendingPathComponent(DiscoDeBlocos.prefixoWAV + UUID().uuidString + ".wav")
        try Data(count: 44).write(to: wav)
        let r = try amb.reabrir().prepararAbertura(diarios: nil, disco: amb.disco)
        XCTAssertEqual(r.adotados, 0)
        XCTAssertTrue(r.invalidos.contains(doc + "/" + link.lastPathComponent))
        XCTAssertFalse(FileManager.default.fileExists(atPath: wav.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: externo.path))
    }

    func testBancoEPastaDeAudioSoDoUsuarioEAudioForaDoBackup() throws {
        let amb = try Ambiente()
        _ = try amb.disco.pasta(documento: UUID().uuidString)
        let modo = { (caminho: String) -> Int in
            ((try? FileManager.default.attributesOfItem(atPath: caminho)[.posixPermissions] as? NSNumber)??.intValue ?? -1) & 0o777
        }
        XCTAssertEqual(modo(amb.banco), 0o600)
        XCTAssertEqual(modo(amb.disco.raiz.path), 0o700)
        let valores = try amb.disco.raiz.resourceValues(forKeys: [.isExcludedFromBackupKey])
        XCTAssertEqual(valores.isExcludedFromBackup, true)
    }
}
