import CryptoKit
import Foundation

/// Modelo que o app sabe baixar: endereço fixado numa revisão e hash conferido antes de usar.
public struct ModeloRemoto: Sendable, Equatable {
    public let arquivo: String
    public let url: URL
    public let sha256: String
    public let tamanho: Int64

    public init(arquivo: String, url: URL, sha256: String, tamanho: Int64) {
        self.arquivo = arquivo
        self.url = url
        self.sha256 = sha256.lowercased()
        self.tamanho = tamanho
    }

    /// Qwen3.5-4B quantizado em Q4_K_M. A Qwen publica só os pesos originais (Qwen/Qwen3.5-4B);
    /// o GGUF vem do repositório da Unsloth, fixado na revisão abaixo. Hash e tamanho são os do
    /// arquivo LFS dessa revisão.
    public static let qwenPadrao = ModeloRemoto(
        arquivo: "Qwen3.5-4B-Q4_K_M.gguf",
        url: URL(string: "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/b0bd07786f94b700012140fd9263b2de83d4f97b/Qwen3.5-4B-Q4_K_M.gguf")!,
        sha256: "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4",
        tamanho: 2_740_937_888
    )

    public static let qwen2B = ModeloRemoto(
        arquivo: "Qwen3.5-2B-Q4_K_M.gguf",
        url: URL(string: "https://huggingface.co/unsloth/Qwen3.5-2B-GGUF/resolve/f6d5376be1edb4d416d56da11e5397a961aca8ae/Qwen3.5-2B-Q4_K_M.gguf")!,
        sha256: "aaf42c8b7c3cab2bf3d69c355048d4a0ee9973d48f16c731c0520ee914699223",
        tamanho: 1_280_835_840
    )

    public static func qwen(arquivo: String) -> ModeloRemoto? {
        [qwenPadrao, qwen2B].first { $0.arquivo == arquivo }
    }
}

public enum ErroDeDownload: Error, Equatable, LocalizedError {
    case semRede
    case conexaoInterrompida
    case discoCheio(necessario: Int64, livre: Int64)
    case hashDiferente(esperado: String, obtido: String)
    case servidor(Int)
    case cancelado
    case falha(String)

    public var errorDescription: String? {
        switch self {
        case .semRede: return "Sem conexão com a internet. Confira a rede e tente de novo; o download continua de onde parou."
        case .conexaoInterrompida: return "A conexão caiu antes do fim. Tente de novo; o download continua de onde parou."
        case .discoCheio(let necessario, let livre):
            return "Falta espaço no disco: são necessários \(Self.tamanho(necessario)) e há \(Self.tamanho(livre)) livres."
        case .hashDiferente: return "O arquivo baixado não confere com o original e foi descartado. Tente de novo."
        case .servidor(let status): return "O servidor do modelo respondeu com erro \(status). Tente mais tarde."
        case .cancelado: return "Download cancelado."
        case .falha(let motivo): return "Não foi possível baixar o modelo: \(motivo)"
        }
    }

    static func tamanho(_ bytes: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
    }
}

/// Resposta do servidor a um pedido a partir de um byte.
public struct RespostaDeDownload: Sendable {
    public let status: Int
    public let pedacos: AsyncThrowingStream<Data, Error>

    public init(status: Int, pedacos: AsyncThrowingStream<Data, Error>) {
        self.status = status
        self.pedacos = pedacos
    }
}

/// De onde vêm os bytes. Em produção, HTTP com cabeçalho Range; nos testes, memória.
public protocol TransporteDeDownload: Sendable {
    func abrir(_ url: URL, aPartirDe inicio: Int64) async throws -> RespostaDeDownload
}

/// Baixa um modelo para `pasta` em `<arquivo>.part`, retoma de onde parou, confere o SHA-256
/// e só então renomeia para o nome final. Nunca deixa um arquivo final que não confira.
public struct BaixadorDeModelo: Sendable {
    public let modelo: ModeloRemoto
    public let pasta: URL
    public let transporte: TransporteDeDownload
    public let espacoLivre: @Sendable (URL) -> Int64?
    /// Folga além do que falta baixar, para não encher o disco até o último byte.
    public static let folga: Int64 = 64 << 20

    public init(modelo: ModeloRemoto, pasta: URL, transporte: TransporteDeDownload = TransporteURLSession(),
                espacoLivre: @escaping @Sendable (URL) -> Int64? = BaixadorDeModelo.espacoLivreNoVolume) {
        self.modelo = modelo
        self.pasta = pasta
        self.transporte = transporte
        self.espacoLivre = espacoLivre
    }

    public var destino: URL { pasta.appendingPathComponent(modelo.arquivo) }
    public var parcial: URL { pasta.appendingPathComponent(modelo.arquivo + ".part") }

    /// Registro de conferência: hash, tamanho e data do arquivo final já conferido.
    public var marcador: URL { pasta.appendingPathComponent(modelo.arquivo + ".conferido") }

    private var assinaturaAtual: String? {
        guard let atributos = try? FileManager.default.attributesOfItem(atPath: destino.path),
              let tamanho = atributos[.size] as? Int64, tamanho == modelo.tamanho,
              let data = atributos[.modificationDate] as? Date else { return nil }
        return "\(modelo.sha256) \(tamanho) \(Int64(data.timeIntervalSince1970 * 1000))"
    }

    /// Instalado só quando o arquivo final tem o tamanho esperado e foi conferido pelo hash
    /// (marcador batendo com tamanho e data do arquivo). Tamanho certo não basta.
    public var instalado: Bool {
        guard let atual = assinaturaAtual,
              let gravada = try? String(contentsOf: marcador, encoding: .utf8) else { return false }
        return gravada.trimmingCharacters(in: .whitespacesAndNewlines) == atual
    }

    /// Arquivo final com o tamanho certo, mas ainda sem conferência (cópia antiga, colocada à mão
    /// ou de uma versão anterior do app): precisa do hash antes do primeiro uso.
    public var precisaConferir: Bool { assinaturaAtual != nil && !instalado }

    /// Grava o registro de conferência. Sem ele o modelo não conta como instalado, então a falha
    /// sobe como erro claro em vez de deixar o app conferindo o arquivo de novo a cada consulta.
    private func marcarConferido() throws {
        guard let atual = assinaturaAtual else { throw ErroDeDownload.falha("o modelo sumiu durante a conferência") }
        do { try Data(atual.utf8).write(to: marcador, options: .atomic) }
        catch { throw ErroDeDownload.falha("o modelo confere, mas não consegui gravar o registro da conferência em \(pasta.path). Confira a permissão de escrita da pasta.") }
    }

    /// Confere o arquivo final já existente. Válido: grava o marcador. Inválido: apaga o arquivo e
    /// lança `hashDiferente`, sem baixar de novo sozinho (a pessoa decide pelo botão de download).
    public func conferirExistente() throws {
        let obtido = try Self.sha256(de: destino)
        guard obtido == modelo.sha256 else {
            try? FileManager.default.removeItem(at: destino)
            try? FileManager.default.removeItem(at: marcador)
            throw ErroDeDownload.hashDiferente(esperado: modelo.sha256, obtido: obtido)
        }
        try marcarConferido()
    }
    /// Bytes já baixados num download interrompido.
    public var jaBaixado: Int64 { min(Self.tamanhoDe(parcial) ?? 0, modelo.tamanho) }

    public enum Etapa: Sendable, Equatable {
        case baixando(feito: Int64, total: Int64)
        case conferindo
    }

    @discardableResult
    public func baixar(aoAvancar: @escaping @Sendable (Etapa) -> Void = { _ in }) async throws -> URL {
        let gerenciador = FileManager.default
        do { try gerenciador.createDirectory(at: pasta, withIntermediateDirectories: true) }
        catch { throw Self.traduzir(error) }
        if instalado { return destino }
        if precisaConferir {
            aoAvancar(.conferindo)
            try conferirExistente()
            return destino
        }

        var feito = Self.tamanhoDe(parcial) ?? 0
        if feito > modelo.tamanho {
            try? gerenciador.removeItem(at: parcial)
            feito = 0
        }
        if feito < modelo.tamanho {
            let falta = modelo.tamanho - feito
            if let livre = espacoLivre(pasta), livre < falta + Self.folga {
                throw ErroDeDownload.discoCheio(necessario: falta + Self.folga, livre: livre)
            }
            try await receber(aPartirDe: feito, aoAvancar: aoAvancar)
        }

        try Task.checkCancellation(como: .cancelado)
        aoAvancar(.conferindo)
        let obtido = try Self.sha256(de: parcial)
        guard obtido == modelo.sha256 else {
            try? gerenciador.removeItem(at: parcial)
            throw ErroDeDownload.hashDiferente(esperado: modelo.sha256, obtido: obtido)
        }
        do {
            if gerenciador.fileExists(atPath: destino.path) { try gerenciador.removeItem(at: destino) }
            try gerenciador.moveItem(at: parcial, to: destino)
        } catch { throw Self.traduzir(error) }
        try marcarConferido()
        return destino
    }

    private func receber(aPartirDe inicio: Int64, aoAvancar: @escaping @Sendable (Etapa) -> Void) async throws {
        let gerenciador = FileManager.default
        let resposta: RespostaDeDownload
        do { resposta = try await transporte.abrir(modelo.url, aPartirDe: inicio) }
        catch { throw Self.traduzir(error) }

        var feito = inicio
        switch resposta.status {
        case 206 where inicio > 0: break
        case 200:
            // Servidor ignorou o intervalo: recomeça do zero para não emendar bytes errados.
            feito = 0
        default:
            throw ErroDeDownload.servidor(resposta.status)
        }
        if !gerenciador.fileExists(atPath: parcial.path) || feito == 0 {
            guard gerenciador.createFile(atPath: parcial.path, contents: nil) else {
                throw ErroDeDownload.falha("não consegui criar o arquivo temporário em \(pasta.path)")
            }
        }
        let alca: FileHandle
        do {
            alca = try FileHandle(forWritingTo: parcial)
            try alca.truncate(atOffset: UInt64(feito))
            try alca.seekToEnd()
        } catch { throw Self.traduzir(error) }
        defer { try? alca.close() }

        aoAvancar(.baixando(feito: feito, total: modelo.tamanho))
        var acumulado = Data()
        var avisado = feito
        let gravar = {
            guard !acumulado.isEmpty else { return }
            guard feito + Int64(acumulado.count) <= modelo.tamanho else {
                throw ErroDeDownload.falha("o servidor enviou mais dados que o esperado")
            }
            do { try alca.write(contentsOf: acumulado) } catch { throw Self.traduzir(error) }
            feito += Int64(acumulado.count)
            acumulado.removeAll(keepingCapacity: true)
            if feito - avisado >= 1 << 20 || feito == modelo.tamanho {
                avisado = feito
                aoAvancar(.baixando(feito: feito, total: modelo.tamanho))
            }
        }
        do {
            for try await pedaco in resposta.pedacos {
                acumulado.append(pedaco)
                if acumulado.count >= 1 << 20 { try gravar() }
                if Task.isCancelled { break }
            }
            try gravar()
        } catch {
            try? gravar()
            throw Self.traduzir(error)
        }
        try Task.checkCancellation(como: .cancelado)
        guard feito == modelo.tamanho else { throw ErroDeDownload.conexaoInterrompida }
    }

    // MARK: - Apoio

    public static func sha256(de url: URL) throws -> String {
        let alca: FileHandle
        do { alca = try FileHandle(forReadingFrom: url) } catch { throw traduzir(error) }
        defer { try? alca.close() }
        var hash = SHA256()
        while true {
            try Task.checkCancellation(como: .cancelado)
            let bloco = try autoreleasepool { try alca.read(upToCount: 4 << 20) }
            guard let bloco, !bloco.isEmpty else { break }
            hash.update(data: bloco)
        }
        return hash.finalize().map { String(format: "%02x", $0) }.joined()
    }

    static func tamanhoDe(_ url: URL) -> Int64? {
        (try? FileManager.default.attributesOfItem(atPath: url.path))?[.size] as? Int64
    }

    public static let espacoLivreNoVolume: @Sendable (URL) -> Int64? = { url in
        let valores = try? url.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey, .volumeAvailableCapacityKey])
        if let importante = valores?.volumeAvailableCapacityForImportantUsage, importante > 0 { return importante }
        return valores?.volumeAvailableCapacity.map(Int64.init)
    }

    static func traduzir(_ erro: Error) -> ErroDeDownload {
        if let proprio = erro as? ErroDeDownload { return proprio }
        if erro is CancellationError { return .cancelado }
        if let url = erro as? URLError {
            switch url.code {
            case .cancelled: return .cancelado
            case .notConnectedToInternet, .cannotFindHost, .cannotConnectToHost, .dnsLookupFailed,
                 .internationalRoamingOff, .dataNotAllowed, .secureConnectionFailed:
                return .semRede
            case .networkConnectionLost, .timedOut: return .conexaoInterrompida
            default: return .falha(url.localizedDescription)
            }
        }
        let ns = erro as NSError
        if (ns.domain == NSCocoaErrorDomain && ns.code == NSFileWriteOutOfSpaceError)
            || (ns.domain == NSPOSIXErrorDomain && ns.code == Int(ENOSPC)) {
            return .discoCheio(necessario: 0, livre: 0)
        }
        if let interno = ns.userInfo[NSUnderlyingErrorKey] as? NSError,
           interno.domain == NSPOSIXErrorDomain, interno.code == Int(ENOSPC) {
            return .discoCheio(necessario: 0, livre: 0)
        }
        return .falha(ns.localizedDescription)
    }
}

private extension Task where Success == Never, Failure == Never {
    static func checkCancellation(como erro: ErroDeDownload) throws {
        if Task.isCancelled { throw erro }
    }
}

// MARK: - Transporte HTTP

/// HTTP pelo URLSession, entregando os dados em pedaços conforme chegam (sem passar byte a byte).
public struct TransporteURLSession: TransporteDeDownload {
    public init() {}

    public func abrir(_ url: URL, aPartirDe inicio: Int64) async throws -> RespostaDeDownload {
        var pedido = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 60)
        if inicio > 0 { pedido.setValue("bytes=\(inicio)-", forHTTPHeaderField: "Range") }
        let delegado = DelegadoDeFluxo()
        let configuracao = URLSessionConfiguration.ephemeral
        configuracao.waitsForConnectivity = false
        configuracao.timeoutIntervalForResource = 6 * 60 * 60
        let sessao = URLSession(configuration: configuracao, delegate: delegado, delegateQueue: nil)
        let tarefa = sessao.dataTask(with: pedido)
        delegado.aoTerminarFluxo = { tarefa.cancel(); sessao.finishTasksAndInvalidate() }
        let status = try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (c: CheckedContinuation<Int, Error>) in
                delegado.esperarResposta(c)
                tarefa.resume()
            }
        } onCancel: { tarefa.cancel() }
        return RespostaDeDownload(status: status, pedacos: delegado.fluxo)
    }
}

private final class DelegadoDeFluxo: NSObject, URLSessionDataDelegate, @unchecked Sendable {
    private let trava = NSLock()
    private var resposta: CheckedContinuation<Int, Error>?
    let fluxo: AsyncThrowingStream<Data, Error>
    private let continuacao: AsyncThrowingStream<Data, Error>.Continuation
    var aoTerminarFluxo: (@Sendable () -> Void)? {
        didSet {
            let acao = aoTerminarFluxo
            continuacao.onTermination = { _ in acao?() }
        }
    }

    override init() {
        (fluxo, continuacao) = AsyncThrowingStream.makeStream(of: Data.self)
    }

    func esperarResposta(_ c: CheckedContinuation<Int, Error>) { trava.withLock { resposta = c } }

    private func entregar(_ resultado: Result<Int, Error>) {
        let c: CheckedContinuation<Int, Error>? = trava.withLock {
            defer { resposta = nil }
            return resposta
        }
        c?.resume(with: resultado)
    }

    // O URLSession mantém o cabeçalho Range ao seguir o redirecionamento do Hugging Face para o CDN.
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse,
                    completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        entregar(.success(status))
        completionHandler((200...299).contains(status) ? .allow : .cancel)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        continuacao.yield(data)
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        if let error {
            entregar(.failure(error))
            continuacao.finish(throwing: error)
        } else {
            continuacao.finish()
        }
        session.finishTasksAndInvalidate()
    }
}
