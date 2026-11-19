import Foundation

/// Diário privado. O arquivo parcial é substituído no mesmo índice até virar um bloco inteiro.
/// A recuperação é aditiva: trechos já gravados nunca são substituídos, e o diário só sai
/// depois que o SQLite devolve um documento que cobre tudo o que ele guardava.
public struct DiarioReuniao: Sendable {
    public struct PCM: Codable, Sendable {
        private let dadosSistema: Data
        private let dadosMicrofone: Data

        public init(sistema: [Float], microfone: [Float]) {
            dadosSistema = sistema.withUnsafeBytes { Data($0) }
            dadosMicrofone = microfone.withUnsafeBytes { Data($0) }
        }

        public var sistema: [Float] { Self.amostras(dadosSistema) }
        public var microfone: [Float] { Self.amostras(dadosMicrofone) }

        private static func amostras(_ dados: Data) -> [Float] {
            let quantidade = dados.count / MemoryLayout<Float>.size
            var saida = [Float](repeating: 0, count: quantidade)
            _ = saida.withUnsafeMutableBytes { dados.copyBytes(to: $0) }
            return saida
        }
    }

    public enum Falha: LocalizedError {
        case naoConfirmado
        public var errorDescription: String? {
            "O banco não confirmou toda a transcrição da reunião. O áudio continua guardado para recuperar."
        }
    }

    public typealias Escrita = @Sendable (Data, URL) throws -> Void
    public static let escritaAtomica: Escrita = { dados, url in try dados.write(to: url, options: .atomic) }

    public let pasta: URL
    private let escrita: Escrita

    public init(raiz: URL, documento: TranscricaoLonga, escrita: @escaping Escrita = DiarioReuniao.escritaAtomica) throws {
        guard UUID(uuidString: documento.id) != nil else { throw CocoaError(.fileWriteInvalidFileName) }
        pasta = raiz.appendingPathComponent(documento.id, isDirectory: true)
        self.escrita = escrita
        try FileManager.default.createDirectory(at: pasta, withIntermediateDirectories: true,
                                               attributes: [.posixPermissions: 0o700])
        try salvar(documento)
    }

    private init(pasta: URL) {
        self.pasta = pasta
        escrita = Self.escritaAtomica
    }

    public static func listar(raiz: URL) throws -> [DiarioReuniao] {
        guard FileManager.default.fileExists(atPath: raiz.path) else { return [] }
        return try FileManager.default.contentsOfDirectory(at: raiz, includingPropertiesForKeys: [.isDirectoryKey, .isSymbolicLinkKey])
            .filter { url in
                guard UUID(uuidString: url.lastPathComponent) != nil,
                      let info = try? url.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey])
                else { return false }
                return info.isDirectory == true && info.isSymbolicLink == false
            }.map { DiarioReuniao(pasta: $0) }
    }

    public func salvar(_ documento: TranscricaoLonga) throws {
        try gravar(documento, em: pasta.appendingPathComponent("reuniao.plist"))
    }

    public func salvarAudio(inicio: Int, sistema: [Float], microfone: [Float]) throws {
        try gravar(PCM(sistema: sistema, microfone: microfone), em: pasta.appendingPathComponent("pcm-\(inicio).plist"))
    }

    /// Grava os trechos de um bloco uma única vez. Se já existem, ficam como estão.
    @discardableResult
    public func salvarTrechos(inicio: Int, trechos: [TrechoLongo]) throws -> [TrechoLongo] {
        if let existentes = try trechosSalvos(inicio: inicio) { return existentes }
        try gravar(trechos, em: urlTrechos(inicio))
        return trechos
    }

    public func trechosSalvos(inicio: Int) throws -> [TrechoLongo]? {
        let url = urlTrechos(inicio)
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        return try ler([TrechoLongo].self, de: url)
    }

    public var id: String { pasta.lastPathComponent }

    public func documento() throws -> TranscricaoLonga {
        try ler(TranscricaoLonga.self, de: pasta.appendingPathComponent("reuniao.plist"))
    }

    public func trechos() throws -> [TrechoLongo] {
        try entradas(prefixo: "trechos-").flatMap { try ler([TrechoLongo].self, de: $0.url) }
    }

    public func provisoria() throws -> TranscricaoLonga {
        var doc = try documento()
        let editados = Dictionary(uniqueKeysWithValues: doc.trechos.filter { $0.vozManual == true }.map { ($0.id, $0) })
        let gravados = try trechos()
        let ids = Set(gravados.map(\.id))
        // Edição de um trecho refeito na passada final ainda não tem bloco em disco: ela também fica.
        doc.trechos = AlinhamentoDeVoz.juntar(gravados.map { editados[$0.id] ?? $0 },
                                              doc.trechos.filter { $0.vozManual == true && !ids.contains($0.id) })
        doc.atualizarVozes()
        return doc
    }

    public func audios() throws -> [(inicio: Int, url: URL)] { try entradas(prefixo: "pcm-") }

    public static func audio(em url: URL) throws -> PCM {
        try PropertyListDecoder().decode(PCM.self, from: Data(contentsOf: url))
    }

    /// Segundo final do áudio guardado, lido só do último bloco.
    public func fimDoAudio() throws -> Double {
        guard let ultimo = try audios().last else { return 0 }
        return Double(ultimo.inicio + (try Self.audio(em: ultimo.url).sistema.count)) / 16_000
    }

    /// Verdadeiro só quando o documento salvo contém toda a fala e todo o áudio do diário.
    public func coberto(por salvo: TranscricaoLonga?) throws -> Bool {
        guard let salvo, salvo.fase == "pronta", salvo.id == id else { return false }
        guard CoberturaDeReuniao.cobre(salvo.trechos, try trechos()) else { return false }
        return (salvo.duracao ?? salvo.trechos.last?.fim ?? 0) + 1 >= (try fimDoAudio())
    }

    /// Persiste, relê do banco e só então retira o diário. Se a releitura não confirma, o diário fica.
    @discardableResult
    public func concluir(_ documento: TranscricaoLonga, persistir: (TranscricaoLonga) throws -> Void,
                         reler: () throws -> TranscricaoLonga?) throws -> Bool {
        // Nada incompleto chega ao banco, e nada sai do disco sem a releitura confirmar.
        guard try coberto(por: documento) else { throw Falha.naoConfirmado }
        try persistir(documento)
        let salvo = try reler()
        guard salvo == documento, try coberto(por: salvo) else { throw Falha.naoConfirmado }
        do { try remover(); return true }
        catch { return false }
    }

    public func remover() throws { try FileManager.default.removeItem(at: pasta) }

    private func urlTrechos(_ inicio: Int) -> URL { pasta.appendingPathComponent("trechos-\(inicio).plist") }

    private func entradas(prefixo: String) throws -> [(inicio: Int, url: URL)] {
        try FileManager.default.contentsOfDirectory(at: pasta, includingPropertiesForKeys: nil)
            .compactMap { url -> (Int, URL)? in
                let nome = url.deletingPathExtension().lastPathComponent
                guard nome.hasPrefix(prefixo), url.pathExtension == "plist",
                      let indice = Int(nome.dropFirst(prefixo.count)), indice >= 0 else { return nil }
                return (indice, url)
            }.sorted { $0.0 < $1.0 }
    }

    private func gravar<T: Encodable>(_ valor: T, em url: URL) throws {
        let encoder = PropertyListEncoder()
        encoder.outputFormat = .binary
        try escrita(try encoder.encode(valor), url)
    }

    private func ler<T: Decodable>(_ tipo: T.Type, de url: URL) throws -> T {
        try PropertyListDecoder().decode(tipo, from: Data(contentsOf: url))
    }
}

public enum CoberturaDeReuniao {
    /// Trechos do diário que o resultado não traz. A recuperação os acrescenta em vez de perdê-los.
    public static func faltantes(_ salvos: [TrechoLongo], _ doDiario: [TrechoLongo]) -> [TrechoLongo] {
        doDiario.filter { !cobre(salvos, [$0]) }
    }

    /// Um trecho do diário está coberto quando o salvo tem o mesmo ID, ou um trecho da mesma fonte
    /// que ocupa ao menos 30% do seu intervalo (a passada final pode refazer a cauda com outro ID).
    public static func cobre(_ salvos: [TrechoLongo], _ doDiario: [TrechoLongo]) -> Bool {
        let ids = Set(salvos.map(\.id))
        return doDiario.allSatisfy { trecho in
            if trecho.texto.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || ids.contains(trecho.id) { return true }
            return salvos.contains { candidato in
                candidato.doMicrofone == trecho.doMicrofone
                    && AlinhamentoDeVoz.sobreposicao(candidato, trecho) >= 0.3 * max(0.02, trecho.fim - trecho.inicio)
            }
        }
    }
}
