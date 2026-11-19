import Foundation

/// Onde o app guarda as coisas: tudo em ~/Library/Application Support/Ditado.
enum Caminhos {
    private static let raizDiaria = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("Ditado", isDirectory: true)
    private static var raizTemporaria: URL?

    static var raiz: URL { raizTemporaria ?? raizDiaria }

    /// Configurado uma vez, antes de criar serviços ou iniciar tarefas do utilitário.
    static func isolarUtilitario() throws {
        let pasta = FileManager.default.temporaryDirectory
            .appendingPathComponent("ditase-teste-" + UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: pasta, withIntermediateDirectories: false,
                                                attributes: [.posixPermissions: 0o700])
        raizTemporaria = pasta
    }

    static func bancoDoUtilitario(_ caminho: String?) throws -> String {
        guard let caminho else { return banco.path }
        if caminho == ":memory:" { return caminho }
        let destino = URL(fileURLWithPath: caminho).standardizedFileURL.resolvingSymlinksInPath()
        let pastaDiaria = raizDiaria.resolvingSymlinksInPath()
        let bancoDiario = pastaDiaria.appendingPathComponent("ditado.sqlite").resolvingSymlinksInPath()
        let logDiario = pastaDiaria.appendingPathComponent("ditado.log").resolvingSymlinksInPath()
        guard !destino.path.hasPrefix(pastaDiaria.path + "/"),
              destino.path != bancoDiario.path, destino.path != logDiario.path
        else { throw ErroLancamento.parametro("use um banco de teste em --banco") }
        return destino.path
    }

    static var banco: URL { raiz.appendingPathComponent("ditado.sqlite") }
    // Modelos já instalados são insumos de leitura compartilhados.
    // DITASE_PASTA_MODELOS aponta a pasta para outro lugar (provas do download, sem tocar na do usuário).
    static var modelos: URL {
        if let outra = ProcessInfo.processInfo.environment["DITASE_PASTA_MODELOS"], !outra.isEmpty {
            let url = URL(fileURLWithPath: outra, isDirectory: true)
            try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
            return url
        }
        if raizTemporaria != nil { return raizDiaria.appendingPathComponent("Models", isDirectory: true) }
        return pasta("Models")
    }
    /// Diários do formato anterior: só lidos pela migração.
    static var reunioesPendentes: URL { raiz.appendingPathComponent("ReunioesPendentes", isDirectory: true) }
    /// PCM retido das reuniões até a passada final de falantes (pasta privada por reunião).
    static var audioDeReuniao: URL {
        let url = raiz.appendingPathComponent("ReunioesAudio", isDirectory: true)
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        return url
    }
    static var audios: URL { pasta("Audio") }
    static var registro: URL { raiz.appendingPathComponent("ditado.log") }
    static var pidDoServidor: URL { raiz.appendingPathComponent("llama-server.pid") }

    private static func pasta(_ nome: String) -> URL {
        let url = raiz.appendingPathComponent(nome, isDirectory: true)
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }
}

/// Registro simples em arquivo, para diagnosticar sem depender do Console.
enum Registro {
    private static let fila = DispatchQueue(label: "ditado.registro")
    private static let formato: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()

    static func nota(_ mensagem: String) {
        let linha = "\(formato.string(from: Date())) \(mensagem)\n"
        fila.async {
            let url = Caminhos.registro
            if let tamanho = (try? FileManager.default.attributesOfItem(atPath: url.path))?[.size] as? Int, tamanho > 2_000_000 {
                try? FileManager.default.removeItem(at: url)
            }
            if let h = try? FileHandle(forWritingTo: url) {
                h.seekToEndOfFile()
                h.write(Data(linha.utf8))
                try? h.close()
            } else {
                try? Data(linha.utf8).write(to: url)
            }
        }
    }
}
