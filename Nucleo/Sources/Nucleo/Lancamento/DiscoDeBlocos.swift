import CryptoKit
import Foundation

/// PCM dos blocos em arquivos Float32 crus, um por bloco, numa pasta privada por reunião.
/// A gravação é durável antes do registro no banco: temporário, F_FULLFSYNC, renomeação e
/// sincronização da pasta. O nome do arquivo carrega a identidade selada do bloco.
public struct DiscoDeBlocos: Sendable {
    public enum Falha: LocalizedError {
        case incompleto(String), soma(String), nome(String), espaco
        public var errorDescription: String? {
            switch self {
            case .incompleto: return "Áudio do bloco ausente ou incompleto."
            case .soma: return "Áudio do bloco não confere com o registro."
            case .nome: return "Arquivo de bloco com nome inválido."
            case .espaco: return "Pouco espaço livre em disco. A captura parou para não perder o que já foi gravado."
            }
        }
    }

    public typealias Escrita = @Sendable (Data, URL) throws -> Void
    public let raiz: URL
    private let escrita: Escrita
    /// Abaixo deste espaço livre a captura para: o PCM fica retido até a passada final.
    public static let espacoMinimo: Int64 = 500 * 1024 * 1024

    public init(raiz: URL, escrita: @escaping Escrita = DiscoDeBlocos.escritaDuravel) {
        self.raiz = raiz
        self.escrita = escrita
    }

    /// Grava em temporário, força o disco, renomeia e sincroniza a pasta.
    public static let escritaDuravel: Escrita = { dados, destino in
        let pasta = destino.deletingLastPathComponent()
        let temporario = pasta.appendingPathComponent(destino.lastPathComponent + ".tmp")
        let fd = open(temporario.path, O_WRONLY | O_CREAT | O_TRUNC, 0o600)
        guard fd >= 0 else { throw CocoaError(.fileWriteUnknown) }
        var ok = dados.withUnsafeBytes { bytes -> Bool in
            var feito = 0
            while feito < bytes.count {
                let n = write(fd, bytes.baseAddress! + feito, bytes.count - feito)
                if n <= 0 { return false }
                feito += n
            }
            return true
        }
        ok = ok && fcntl(fd, F_FULLFSYNC) == 0
        let erro = errno
        close(fd)
        guard ok else {
            unlink(temporario.path)
            throw CocoaError(erro == ENOSPC ? .fileWriteOutOfSpace : .fileWriteUnknown)
        }
        guard rename(temporario.path, destino.path) == 0 else { throw CocoaError(.fileWriteUnknown) }
        let dir = open(pasta.path, O_RDONLY)
        guard dir >= 0 else { throw CocoaError(.fileWriteUnknown) }
        defer { close(dir) }
        guard fsync(dir) == 0 else { throw CocoaError(.fileWriteUnknown) }
    }

    public func pasta(documento: String) throws -> URL {
        let url = raiz.appendingPathComponent(documento, isDirectory: true)
        let existia = FileManager.default.fileExists(atPath: url.path)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        if !existia {
            marcarSemBackup()
            for pasta in [raiz, raiz.deletingLastPathComponent()] {
                let fd = open(pasta.path, O_RDONLY)
                guard fd >= 0 else { throw CocoaError(.fileWriteUnknown) }
                let resultado = fsync(fd)
                close(fd)
                guard resultado == 0 else { throw CocoaError(.fileWriteUnknown) }
            }
        }
        return url
    }

    static func nome(_ id: IdBloco, inicio: Int, quantidade: Int, soma: String) -> String {
        "\(id.fonte.rawValue)_\(id.geracao)_\(id.seq)_\(inicio)_\(quantidade)_\(soma).f32"
    }

    static func soma(_ dados: Data) -> String { SHA256.hash(data: dados).map { String(format: "%02x", $0) }.joined() }

    static func dados(_ amostras: [Float]) -> Data { amostras.withUnsafeBytes { Data($0) } }

    /// Grava e devolve o registro pronto para o banco, em estado capturado.
    public func gravar(_ id: IdBloco, inicio: Int, amostras: [Float]) throws -> BlocoRegistrado {
        let dados = Self.dados(amostras)
        let soma = Self.soma(dados)
        let nome = Self.nome(id, inicio: inicio, quantidade: amostras.count, soma: soma)
        try escrita(dados, try pasta(documento: id.documento).appendingPathComponent(nome))
        return BlocoRegistrado(id: id, inicio: inicio, quantidade: amostras.count, arquivo: id.documento + "/" + nome,
                               tamanho: dados.count, soma: soma, estado: .capturado)
    }

    /// Lê e confere tamanho e soma. Divergência vira falha: o chamador marca a lacuna.
    public func ler(_ bloco: BlocoRegistrado) throws -> [Float] {
        guard let arquivo = bloco.arquivo,
              let dados = try? Data(contentsOf: raiz.appendingPathComponent(arquivo)),
              dados.count == bloco.quantidade * MemoryLayout<Float>.size else { throw Falha.incompleto(bloco.id.texto) }
        if let soma = bloco.soma, Self.soma(dados) != soma { throw Falha.soma(bloco.id.texto) }
        var saida = [Float](repeating: 0, count: bloco.quantidade)
        _ = saida.withUnsafeMutableBytes { dados.copyBytes(to: $0) }
        return saida
    }

    public func existe(_ bloco: BlocoRegistrado) -> Bool {
        guard let arquivo = bloco.arquivo,
              let atributos = try? FileManager.default.attributesOfItem(atPath: raiz.appendingPathComponent(arquivo).path),
              let tamanho = atributos[.size] as? NSNumber else { return false }
        return tamanho.intValue == bloco.quantidade * MemoryLayout<Float>.size
    }

    public func remover(_ arquivo: String) {
        try? apagar(arquivo)
    }

    /// Nome do WAV temporário da identificação de falantes, dentro da pasta da reunião.
    public static let prefixoWAV = "falantes-"

    /// Só "<UUID>/<nome>.f32", sem barra, ponto-ponto ou nome vazio: o caminho vem do banco e
    /// nunca pode apontar para fora da pasta de uma reunião.
    public static func arquivoValido(_ arquivo: String) -> Bool {
        let partes = arquivo.split(separator: "/", omittingEmptySubsequences: false)
        guard partes.count == 2, UUID(uuidString: String(partes[0])) != nil else { return false }
        let nome = partes[1]
        return nome.hasSuffix(".f32") && nome.count > 4 && !nome.hasPrefix(".") && !nome.contains("\\")
    }

    /// Como `arquivoValido`, exigindo também que o arquivo seja da reunião do registro.
    public static func arquivoValido(_ arquivo: String, documento: String) -> Bool {
        arquivoValido(arquivo) && arquivo.hasPrefix(documento + "/")
    }

    /// Tipo da entrada sem seguir link (lstat).
    private static func tipo(_ url: URL) -> FileAttributeType? {
        (try? FileManager.default.attributesOfItem(atPath: url.path))?[.type] as? FileAttributeType
    }

    /// Remove só um arquivo comum dentro de uma pasta real de reunião: link no lugar da pasta
    /// ou do arquivo nunca é seguido, para a limpeza não alcançar nada fora da raiz de áudio.
    public func apagar(_ arquivo: String) throws {
        guard Self.arquivoValido(arquivo) else { throw Falha.nome(arquivo) }
        let url = raiz.appendingPathComponent(arquivo)
        guard let pasta = Self.tipo(url.deletingLastPathComponent()) else { return }
        guard pasta == .typeDirectory else { throw Falha.nome(arquivo) }
        guard let proprio = Self.tipo(url) else { return }
        guard proprio == .typeRegular else { throw Falha.nome(arquivo) }
        do { try FileManager.default.removeItem(at: url) }
        catch let erro as CocoaError where erro.code == .fileNoSuchFile {}
    }

    public func removerPasta(documento: String) {
        guard UUID(uuidString: documento) != nil, !documento.contains("/") else { return }
        let url = raiz.appendingPathComponent(documento, isDirectory: true)
        guard Self.tipo(url) == .typeDirectory else { return }
        try? FileManager.default.removeItem(at: url)
    }

    /// O áudio bruto é transitório e sensível: a raiz fica fora do backup (o banco continua no backup).
    /// Reaplicada a cada abertura, também para pastas criadas por versões anteriores.
    public func marcarSemBackup() {
        guard FileManager.default.fileExists(atPath: raiz.path) else { return }
        var semBackup = URLResourceValues()
        semBackup.isExcludedFromBackup = true
        var raizMutavel = raiz
        try? raizMutavel.setResourceValues(semBackup)
    }

    /// Arquivos completos sem registro no banco, prontos para adoção. Temporários nunca renomeados
    /// não são duráveis e saem. Arquivo com tamanho diferente do nome fica de fora (e é informado).
    public func orfaos(registrados: Set<String>) -> (adotaveis: [BlocoRegistrado], invalidos: [String]) {
        var adotaveis: [BlocoRegistrado] = []
        var invalidos: [String] = []
        let fm = FileManager.default
        let chaves: [URLResourceKey] = [.isDirectoryKey, .isSymbolicLinkKey, .isRegularFileKey, .fileSizeKey]
        guard let pastas = try? fm.contentsOfDirectory(at: raiz, includingPropertiesForKeys: chaves) else { return ([], []) }
        for pasta in pastas where UUID(uuidString: pasta.lastPathComponent) != nil {
            // Link no lugar da pasta de uma reunião não é seguido.
            guard let tipo = try? pasta.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey]),
                  tipo.isDirectory == true, tipo.isSymbolicLink != true else { continue }
            let documento = pasta.lastPathComponent
            for url in (try? fm.contentsOfDirectory(at: pasta, includingPropertiesForKeys: chaves)) ?? [] {
                let valores = try? url.resourceValues(forKeys: Set(chaves))
                if valores?.isSymbolicLink == true {
                    invalidos.append(documento + "/" + url.lastPathComponent)
                    continue
                }
                if url.pathExtension == "tmp" { try? fm.removeItem(at: url); continue }
                // WAV de identificação deixado por uma saída no meio da passada final.
                if url.pathExtension == "wav", url.lastPathComponent.hasPrefix(Self.prefixoWAV) { try? fm.removeItem(at: url); continue }
                let relativo = documento + "/" + url.lastPathComponent
                guard url.pathExtension == "f32", !registrados.contains(relativo) else { continue }
                let partes = url.deletingPathExtension().lastPathComponent.split(separator: "_")
                guard partes.count == 6, let fonte = FonteDeAudio(rawValue: String(partes[0])),
                      let geracao = Int(partes[1]), let seq = Int(partes[2]), let inicio = Int(partes[3]),
                      let quantidade = Int(partes[4]), quantidade > 0, quantidade <= Int.max / 4, valores?.fileSize == quantidade * MemoryLayout<Float>.size,
                      let dados = try? Data(contentsOf: url) else {
                    invalidos.append(relativo)
                    continue
                }
                guard dados.count == quantidade * MemoryLayout<Float>.size, Self.soma(dados) == partes[5] else {
                    invalidos.append(relativo)
                    continue
                }
                adotaveis.append(BlocoRegistrado(id: IdBloco(documento: documento, fonte: fonte, geracao: geracao, seq: seq),
                                                 inicio: inicio, quantidade: quantidade, arquivo: relativo, tamanho: dados.count,
                                                 soma: Self.soma(dados), estado: .capturado))
            }
        }
        // A listagem da pasta não tem ordem garantida; a adoção precisa ir do começo da fala para o fim.
        adotaveis.sort {
            ($0.id.documento, $0.id.fonte.rawValue, $0.id.geracao, $0.inicio, $0.id.seq)
                < ($1.id.documento, $1.id.fonte.rawValue, $1.id.geracao, $1.inicio, $1.id.seq)
        }
        return (adotaveis, invalidos.sorted())
    }

    /// Espaço livre do volume. Alguns ambientes devolvem zero na medida "importante"; aí vale a capacidade disponível.
    public func espacoLivre() -> Int64? {
        let valores = try? raiz.deletingLastPathComponent().resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey, .volumeAvailableCapacityKey])
        if let importante = valores?.volumeAvailableCapacityForImportantUsage, importante > 0 { return importante }
        return valores?.volumeAvailableCapacity.map(Int64.init)
    }

    /// Junta as amostras de uma fonte e geração numa faixa, com zeros onde não há áudio.
    public func faixa(_ blocos: [BlocoRegistrado], de: Int, ate: Int) throws -> (amostras: [Float], quebrados: [BlocoRegistrado]) {
        var saida = [Float](repeating: 0, count: max(0, ate - de))
        var quebrados: [BlocoRegistrado] = []
        for b in blocos where b.fim > de && b.inicio < ate && b.arquivo != nil && b.estado != .lacuna && b.estado != .descartado {
            let amostras: [Float]
            do { amostras = try ler(b) } catch { quebrados.append(b); continue }
            let a = max(de, b.inicio), z = min(ate, b.fim)
            for i in a..<z { saida[i - de] = amostras[i - b.inicio] }
        }
        return (saida, quebrados)
    }

    /// WAV Float32 mono de 16 kHz para a separação de falantes.
    public static func escreverWAV(_ amostras: [Float], em url: URL) throws {
        var d = Data()
        func u32(_ v: UInt32) { withUnsafeBytes(of: v.littleEndian) { d.append(contentsOf: $0) } }
        func u16(_ v: UInt16) { withUnsafeBytes(of: v.littleEndian) { d.append(contentsOf: $0) } }
        let bytes = UInt32(amostras.count * 4)
        d.append(contentsOf: Array("RIFF".utf8)); u32(36 + bytes); d.append(contentsOf: Array("WAVE".utf8))
        d.append(contentsOf: Array("fmt ".utf8)); u32(16); u16(3); u16(1); u32(UInt32(taxaLancamento)); u32(UInt32(taxaLancamento * 4)); u16(4); u16(32)
        d.append(contentsOf: Array("data".utf8)); u32(bytes)
        d.append(dados(amostras))
        try d.write(to: url, options: .atomic)
    }
}
