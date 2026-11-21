import CryptoKit
import Foundation
import XCTest
@testable import Nucleo

/// Servidor em memória: respeita (ou não) o Range, corta a conexão e registra os pedidos.
private final class TransporteFalso: TransporteDeDownload, @unchecked Sendable {
    let dados: Data
    var status = 200
    var respeitaIntervalo = true
    /// Corta a conexão depois de entregar este byte (posição absoluta no arquivo).
    var cortarEm: Int64?
    var pausaPorPedaco: UInt64 = 0
    var pedaco = 64 << 10
    private let trava = NSLock()
    private(set) var pedidos: [Int64] = []

    init(dados: Data) { self.dados = dados }

    func abrir(_ url: URL, aPartirDe inicio: Int64) async throws -> RespostaDeDownload {
        trava.withLock { pedidos.append(inicio) }
        let origem = respeitaIntervalo ? Int(inicio) : 0
        let codigo = status != 200 ? status : (respeitaIntervalo && inicio > 0 ? 206 : 200)
        let dados = dados, cortar = cortarEm.map(Int.init), tamanho = pedaco, pausa = pausaPorPedaco
        let fluxo = AsyncThrowingStream<Data, Error> { c in
            let tarefa = Task {
                guard codigo < 300 else { c.finish(); return }
                var i = origem
                while i < dados.count {
                    if Task.isCancelled { c.finish(throwing: URLError(.cancelled)); return }
                    var fim = min(i + tamanho, dados.count)
                    if let cortar, fim > cortar { fim = cortar }
                    if fim > i { c.yield(dados.subdata(in: i..<fim)) }
                    if let cortar, fim >= cortar { c.finish(throwing: URLError(.networkConnectionLost)); return }
                    i = fim
                    if pausa > 0 { try? await Task.sleep(nanoseconds: pausa) }
                }
                c.finish()
            }
            c.onTermination = { _ in tarefa.cancel() }
        }
        return RespostaDeDownload(status: codigo, pedacos: fluxo)
    }
}

final class BaixadorDeModeloTests: XCTestCase {
    private var pasta: URL!
    private var dados: Data!
    private var modelo: ModeloRemoto!

    override func setUpWithError() throws {
        pasta = FileManager.default.temporaryDirectory.appendingPathComponent("baixador-" + UUID().uuidString, isDirectory: true)
        var gerador = SystemRandomNumberGenerator()
        dados = Data((0..<(3 << 20) + 12_345).map { _ in UInt8.random(in: 0...255, using: &gerador) })
        modelo = ModeloRemoto(arquivo: "teste.gguf", url: URL(string: "https://exemplo.invalid/teste.gguf")!,
                              sha256: Self.hash(dados), tamanho: Int64(dados.count))
    }

    override func tearDownWithError() throws { try? FileManager.default.removeItem(at: pasta) }

    private static func hash(_ d: Data) -> String { SHA256.hash(data: d).map { String(format: "%02x", $0) }.joined() }

    private func baixador(_ t: TransporteFalso, livre: Int64? = nil, modelo: ModeloRemoto? = nil) -> BaixadorDeModelo {
        BaixadorDeModelo(modelo: modelo ?? self.modelo, pasta: pasta, transporte: t, espacoLivre: { _ in livre })
    }

    private func esperarErro(_ esperado: ErroDeDownload, _ bloco: () async throws -> Void,
                             file: StaticString = #filePath, line: UInt = #line) async {
        do {
            try await bloco()
            XCTFail("deveria falhar com \(esperado)", file: file, line: line)
        } catch {
            XCTAssertEqual(error as? ErroDeDownload, esperado, file: file, line: line)
        }
    }

    func testModeloPadraoFixadoNumaRevisaoComHash() {
        let m = ModeloRemoto.qwenPadrao
        XCTAssertEqual(m.arquivo, "Qwen3.5-4B-Q4_K_M.gguf")
        XCTAssertTrue(m.url.absoluteString.hasPrefix("https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/"))
        let revisao = m.url.pathComponents[m.url.pathComponents.count - 2]
        XCTAssertEqual(revisao.count, 40, "a URL precisa apontar para um commit, não para main")
        XCTAssertEqual(m.sha256, "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4")
        XCTAssertEqual(m.tamanho, 2_740_937_888)
        XCTAssertEqual(m.url.lastPathComponent, m.arquivo)
        XCTAssertEqual(ModeloRemoto.qwen(arquivo: "Qwen3.5-2B-Q4_K_M.gguf"), .qwen2B)
        XCTAssertNil(ModeloRemoto.qwen(arquivo: "avulso.gguf"))
    }

    func testBaixaConfereERenomeiaParaONomeFinal() async throws {
        let t = TransporteFalso(dados: dados)
        let b = baixador(t)
        let etapas = Registro()
        let final = try await b.baixar { etapas.anotar($0) }
        XCTAssertEqual(final.lastPathComponent, "teste.gguf")
        XCTAssertEqual(try Data(contentsOf: final), dados)
        XCTAssertFalse(FileManager.default.fileExists(atPath: b.parcial.path))
        XCTAssertTrue(b.instalado)
        XCTAssertEqual(t.pedidos, [0])
        XCTAssertEqual(etapas.lista.last, .conferindo)
        XCTAssertTrue(etapas.lista.contains(.baixando(feito: Int64(dados.count), total: Int64(dados.count))))
        let feitos = etapas.lista.compactMap { if case .baixando(let f, _) = $0 { return f } else { return nil } }
        XCTAssertEqual(feitos, feitos.sorted(), "progresso nunca decresce")
    }

    func testHashDiferenteDescartaENaoCriaArquivoFinal() async throws {
        var corrompido = dados!
        corrompido[1000] ^= 0xFF
        let t = TransporteFalso(dados: corrompido)
        let b = baixador(t)
        do {
            try await b.baixar()
            XCTFail("deveria recusar o hash")
        } catch ErroDeDownload.hashDiferente(let esperado, let obtido) {
            XCTAssertEqual(esperado, modelo.sha256)
            XCTAssertEqual(obtido, Self.hash(corrompido))
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: b.destino.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: b.parcial.path), "parte corrompida não pode ser retomada")
    }

    func testQuedaNoMeioGuardaParteERetomaComRange() async throws {
        let t = TransporteFalso(dados: dados)
        t.cortarEm = 1_500_000
        let b = baixador(t)
        await esperarErro(.conexaoInterrompida) { try await b.baixar() }
        XCTAssertEqual(b.jaBaixado, 1_500_000)
        XCTAssertFalse(FileManager.default.fileExists(atPath: b.destino.path))

        t.cortarEm = nil
        try await b.baixar()
        XCTAssertEqual(t.pedidos, [0, 1_500_000], "a segunda tentativa pede só o que falta")
        XCTAssertEqual(try Data(contentsOf: b.destino), dados)
    }

    func testRetomaParteDeixadaPorExecucaoAnterior() async throws {
        try FileManager.default.createDirectory(at: pasta, withIntermediateDirectories: true)
        let b0 = baixador(TransporteFalso(dados: dados))
        try dados.prefix(1 << 20).write(to: b0.parcial)
        let t = TransporteFalso(dados: dados)
        let b = baixador(t)
        try await b.baixar()
        XCTAssertEqual(t.pedidos, [1 << 20])
        XCTAssertEqual(try Data(contentsOf: b.destino), dados)
    }

    func testServidorQueIgnoraRangeRecomecaDoZero() async throws {
        try FileManager.default.createDirectory(at: pasta, withIntermediateDirectories: true)
        let b0 = baixador(TransporteFalso(dados: dados))
        try Data(repeating: 0xAB, count: 700_000).write(to: b0.parcial)
        let t = TransporteFalso(dados: dados)
        t.respeitaIntervalo = false
        try await baixador(t).baixar()
        XCTAssertEqual(try Data(contentsOf: b0.destino), dados, "bytes velhos não podem ser emendados")
    }

    func testParteMaiorQueOEsperadoEDescartada() async throws {
        try FileManager.default.createDirectory(at: pasta, withIntermediateDirectories: true)
        let b0 = baixador(TransporteFalso(dados: dados))
        try (dados + Data(count: 10)).write(to: b0.parcial)
        let t = TransporteFalso(dados: dados)
        try await baixador(t).baixar()
        XCTAssertEqual(t.pedidos, [0])
        XCTAssertEqual(try Data(contentsOf: b0.destino), dados)
    }

    func testCancelarPreservaParteSemArquivoFinal() async throws {
        let t = TransporteFalso(dados: dados)
        t.pedaco = 32 << 10
        t.pausaPorPedaco = 2_000_000
        let b = baixador(t)
        let tarefa = Task { try await b.baixar() }
        while b.jaBaixado < 1 << 20 { try await Task.sleep(nanoseconds: 5_000_000) }
        tarefa.cancel()
        do {
            _ = try await tarefa.value
            XCTFail("deveria cancelar")
        } catch {
            XCTAssertEqual(error as? ErroDeDownload, .cancelado)
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: b.destino.path))
        XCTAssertGreaterThanOrEqual(b.jaBaixado, 1 << 20)
        XCTAssertLessThan(b.jaBaixado, Int64(dados.count))
        t.pausaPorPedaco = 0
        try await b.baixar()
        XCTAssertEqual(try Data(contentsOf: b.destino), dados)
    }

    func testDiscoCheioFalhaAntesDeBaixar() async {
        let t = TransporteFalso(dados: dados)
        let b = baixador(t, livre: 1_000)
        await esperarErro(.discoCheio(necessario: Int64(dados.count) + BaixadorDeModelo.folga, livre: 1_000)) { try await b.baixar() }
        XCTAssertTrue(t.pedidos.isEmpty)
        XCTAssertEqual(ErroDeDownload.discoCheio(necessario: 2_000_000_000, livre: 1_000).errorDescription?.contains("Falta espaço"), true)
    }

    func testEspacoConsideraSoOQueFalta() async throws {
        try FileManager.default.createDirectory(at: pasta, withIntermediateDirectories: true)
        let b0 = baixador(TransporteFalso(dados: dados))
        try dados.prefix(3 << 20).write(to: b0.parcial)
        let falta = Int64(dados.count - (3 << 20))
        try await baixador(TransporteFalso(dados: dados), livre: falta + BaixadorDeModelo.folga).baixar()
        XCTAssertTrue(b0.instalado)
    }

    func testDiscoCheioDuranteAEscritaViraErroClaro() {
        let posix = NSError(domain: NSPOSIXErrorDomain, code: Int(ENOSPC))
        let cocoa = NSError(domain: NSCocoaErrorDomain, code: NSFileWriteOutOfSpaceError)
        for e in [posix, cocoa] {
            guard case .discoCheio = BaixadorDeModelo.traduzir(e) else { return XCTFail("\(e)") }
        }
        XCTAssertEqual(BaixadorDeModelo.traduzir(URLError(.notConnectedToInternet)), .semRede)
        XCTAssertEqual(BaixadorDeModelo.traduzir(URLError(.cannotFindHost)), .semRede)
        XCTAssertEqual(BaixadorDeModelo.traduzir(URLError(.cancelled)), .cancelado)
    }

    func testSemRedeNaoCriaNada() async {
        struct Desconectado: TransporteDeDownload {
            func abrir(_ url: URL, aPartirDe inicio: Int64) async throws -> RespostaDeDownload { throw URLError(.notConnectedToInternet) }
        }
        let b = BaixadorDeModelo(modelo: modelo, pasta: pasta, transporte: Desconectado(), espacoLivre: { _ in nil })
        await esperarErro(.semRede) { try await b.baixar() }
        XCTAssertFalse(FileManager.default.fileExists(atPath: b.destino.path))
        XCTAssertNotNil(ErroDeDownload.semRede.errorDescription)
    }

    func testErroDoServidor() async {
        let t = TransporteFalso(dados: dados)
        t.status = 404
        await esperarErro(.servidor(404)) { try await baixador(t).baixar() }
    }

    func testJaInstaladoNaoBaixaDeNovo() async throws {
        try FileManager.default.createDirectory(at: pasta, withIntermediateDirectories: true)
        let t = TransporteFalso(dados: dados)
        let b = baixador(t)
        try dados.write(to: b.destino)
        XCTAssertFalse(b.instalado, "tamanho certo sem conferência não conta como instalado")
        XCTAssertTrue(b.precisaConferir)
        try await b.baixar()
        XCTAssertTrue(t.pedidos.isEmpty)
        XCTAssertTrue(b.instalado)
        try await b.baixar()
        XCTAssertTrue(t.pedidos.isEmpty)
    }

    /// Revisão final, P2: arquivo com o nome e o tamanho do modelo, mas conteúdo errado, não é aceito.
    func testArquivoExistenteInvalidoNaoContaComoInstaladoEESaiDoDisco() async throws {
        try FileManager.default.createDirectory(at: pasta, withIntermediateDirectories: true)
        let t = TransporteFalso(dados: dados)
        let b = baixador(t)
        try Data(count: dados.count).write(to: b.destino)
        XCTAssertFalse(b.instalado)
        await esperarErro(.hashDiferente(esperado: modelo.sha256, obtido: Self.hash(Data(count: dados.count)))) { try await b.baixar() }
        XCTAssertTrue(t.pedidos.isEmpty, "conferência não baixa de novo sozinha")
        XCTAssertFalse(FileManager.default.fileExists(atPath: b.destino.path))
        XCTAssertFalse(b.instalado)
        try await b.baixar()
        XCTAssertTrue(b.instalado)
    }

    func testArquivoTrocadoDepoisDeConferidoPerdeAConferencia() async throws {
        let t = TransporteFalso(dados: dados)
        let b = baixador(t)
        try await b.baixar()
        XCTAssertTrue(b.instalado)
        try Data(count: dados.count).write(to: b.destino)
        XCTAssertFalse(b.instalado)
        XCTAssertTrue(b.precisaConferir)
    }

    /// Revisão final r2, P2: modelo válido em pasta sem escrita vira uma falha só, sem apagar o
    /// modelo e sem reconferir sozinho; com escrita de volta, conclui.
    func testPastaSemEscritaFalhaUmaVezSemApagarOModelo() async throws {
        try FileManager.default.createDirectory(at: pasta, withIntermediateDirectories: true)
        let t = TransporteFalso(dados: dados)
        let b = baixador(t)
        try dados.write(to: b.destino)
        try FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: pasta.path)
        defer { try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: pasta.path) }
        do { try await b.baixar(); XCTFail("deveria falhar ao gravar o registro") }
        catch { XCTAssertTrue((error as? ErroDeDownload).map { if case .falha = $0 { return true } else { return false } } ?? false, "\(error)") }
        XCTAssertTrue(FileManager.default.fileExists(atPath: b.destino.path))
        XCTAssertFalse(b.instalado)
        XCTAssertTrue(t.pedidos.isEmpty)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: pasta.path)
        try await b.baixar()
        XCTAssertTrue(b.instalado)
    }

    /// Prova com a rede real, ligada só quando DITASE_PROVA_DOWNLOAD aponta para uma pasta temporária.
    /// Baixa uma parte, cancela, retoma até o fim e confere o SHA-256 fixado no código.
    /// DITASE_PROVA_TRANSPORTE=curl troca só a origem dos bytes (útil onde o URLSession não alcança a rede).
    func testProvaDownloadRealComRetomada() async throws {
        let ambiente = ProcessInfo.processInfo.environment
        guard let caminho = ambiente["DITASE_PROVA_DOWNLOAD"] else {
            throw XCTSkip("defina DITASE_PROVA_DOWNLOAD=/tmp/pasta para baixar o modelo real")
        }
        let pastaReal = URL(fileURLWithPath: caminho, isDirectory: true)
        let suporte = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        XCTAssertFalse(pastaReal.standardizedFileURL.path.hasPrefix(suporte.path), "nunca na pasta do usuário")
        let transporte: TransporteDeDownload = ambiente["DITASE_PROVA_TRANSPORTE"] == "curl" ? TransporteCurl() : TransporteURLSession()
        print("PROVA transporte \(type(of: transporte)) pasta \(pastaReal.path)")
        let b = BaixadorDeModelo(modelo: .qwenPadrao, pasta: pastaReal, transporte: transporte)
        let inicio = Date()
        let fim = Sinal()
        let primeira = Task { defer { fim.ligar() }; try await b.baixar() }
        while b.jaBaixado < 80 << 20, !fim.ligado { try await Task.sleep(nanoseconds: 100_000_000) }
        primeira.cancel()
        do { _ = try await primeira.value } catch { print("PROVA primeira tentativa terminou com: \(error)") }
        let parcial = b.jaBaixado
        XCTAssertGreaterThanOrEqual(parcial, 80 << 20, "a primeira tentativa precisa ter baixado algo antes do cancelamento")
        guard parcial >= 80 << 20 else { return }
        print("PROVA cancelado com \(parcial) bytes em \(b.parcial.lastPathComponent) depois de \(Int(Date().timeIntervalSince(inicio))) s")
        XCTAssertFalse(FileManager.default.fileExists(atPath: b.destino.path))
        let ultimo = Registro()
        try await b.baixar { ultimo.anotar($0) }
        print("PROVA concluido em \(Int(Date().timeIntervalSince(inicio))) s; primeira etapa da retomada: \(ultimo.lista.first.map(String.init(describing:)) ?? "-")")
        XCTAssertEqual(ultimo.lista.first, .baixando(feito: parcial, total: ModeloRemoto.qwenPadrao.tamanho), "retomou do byte \(parcial)")
        XCTAssertTrue(b.instalado)
        XCTAssertFalse(FileManager.default.fileExists(atPath: b.parcial.path))
        XCTAssertEqual(try BaixadorDeModelo.sha256(de: b.destino), ModeloRemoto.qwenPadrao.sha256)
        print("PROVA sha256 \(ModeloRemoto.qwenPadrao.sha256) confere; arquivo \(b.destino.path)")
    }
}

private final class Sinal: @unchecked Sendable {
    private let trava = NSLock()
    private var _ligado = false
    func ligar() { trava.withLock { _ligado = true } }
    var ligado: Bool { trava.withLock { _ligado } }
}

/// Bytes pelo curl do sistema (TLS validado por ele, proxy do ambiente respeitado). Só para a prova.
private struct TransporteCurl: TransporteDeDownload {
    func abrir(_ url: URL, aPartirDe inicio: Int64) async throws -> RespostaDeDownload {
        let cabecalhos = FileManager.default.temporaryDirectory.appendingPathComponent("curl-\(UUID().uuidString).txt")
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/curl")
        p.arguments = ["-sS", "-L", "-D", cabecalhos.path, "-o", "-"] + (inicio > 0 ? ["-r", "\(inicio)-"] : []) + [url.absoluteString]
        let saida = Pipe()
        p.standardOutput = saida
        p.standardError = FileHandle.standardError
        let (fluxo, c) = AsyncThrowingStream.makeStream(of: Data.self)
        let primeiro = Sinal()
        saida.fileHandleForReading.readabilityHandler = { h in
            let d = h.availableData
            if d.isEmpty { h.readabilityHandler = nil; return }
            primeiro.ligar()
            c.yield(d)
        }
        p.terminationHandler = { proc in
            primeiro.ligar()
            saida.fileHandleForReading.readabilityHandler = nil
            if let resto = try? saida.fileHandleForReading.readToEnd(), !resto.isEmpty { c.yield(resto) }
            proc.terminationStatus == 0 || proc.terminationReason == .uncaughtSignal
                ? c.finish(throwing: proc.terminationReason == .uncaughtSignal ? URLError(.cancelled) : nil)
                : c.finish(throwing: URLError(.networkConnectionLost))
        }
        c.onTermination = { _ in if p.isRunning { p.terminate() } }
        try p.run()
        while !primeiro.ligado { try await Task.sleep(nanoseconds: 20_000_000) }
        let texto = (try? String(contentsOf: cabecalhos, encoding: .utf8)) ?? ""
        try? FileManager.default.removeItem(at: cabecalhos)
        let status = texto.split(whereSeparator: \.isNewline).last { $0.hasPrefix("HTTP/") }
            .flatMap { $0.split(separator: " ").dropFirst().first.flatMap { Int($0) } } ?? 0
        return RespostaDeDownload(status: status, pedacos: fluxo)
    }
}

private final class Registro: @unchecked Sendable {
    private let trava = NSLock()
    private var _lista: [BaixadorDeModelo.Etapa] = []
    func anotar(_ e: BaixadorDeModelo.Etapa) { trava.withLock { _lista.append(e) } }
    var lista: [BaixadorDeModelo.Etapa] { trava.withLock { _lista } }
}
