import Foundation

/// Reconhecedor de fala: recebe amostras a 16 kHz mono e devolve o texto.
public protocol Transcritor: Sendable {
    var nome: String { get }
    func transcrever(_ amostras: [Float]) async throws -> String
}

public struct PedidoLimpeza: Sendable {
    public let transcricao: String
    /// Fim do texto já arrumado, só como contexto.
    public let anterior: String
    public let vocabulario: [String]

    public init(transcricao: String, anterior: String = "", vocabulario: [String] = []) {
        self.transcricao = transcricao
        self.anterior = anterior
        self.vocabulario = vocabulario
    }
}

/// Modelo de linguagem que arruma a transcrição (pontuação, hesitações, autocorreções).
public protocol Limpador: Sendable {
    var nome: String { get }
    func limpar(_ pedido: PedidoLimpeza) async throws -> String
}

public struct ErroDeLimpeza: Error, CustomStringConvertible, Sendable {
    public let description: String
    public init(_ d: String) { description = d }
}

/// Limpador que fala com um llama-server local pela API compatível com a da OpenAI.
public struct LimpadorLlama: Limpador {
    public let nome: String
    let url: URL
    let instrucoes: String
    let sessao: URLSession

    public init(porta: Int, modelo: String, instrucoes: String) {
        nome = modelo
        url = URL(string: "http://127.0.0.1:\(porta)/v1/chat/completions")!
        self.instrucoes = instrucoes
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 120
        config.waitsForConnectivity = false
        sessao = URLSession(configuration: config)
    }

    public static func montarMensagem(_ pedido: PedidoLimpeza) -> String {
        var m = ""
        if !pedido.vocabulario.isEmpty {
            m += "<vocabulario>\(pedido.vocabulario.prefix(60).joined(separator: ", "))</vocabulario>\n"
        }
        if !pedido.anterior.isEmpty { m += "<anterior>\(pedido.anterior)</anterior>\n" }
        m += "<transcricao>\(pedido.transcricao)</transcricao>"
        return m
    }

    /// Tira o que modelos pequenos às vezes põem em volta da resposta.
    public static func extrairResposta(_ bruta: String) -> String {
        var t = bruta
        while let ini = t.range(of: "<think>"), let fim = t.range(of: "</think>", range: ini.upperBound..<t.endIndex) {
            t.removeSubrange(ini.lowerBound..<fim.upperBound)
        }
        t = t.trimmingCharacters(in: .whitespacesAndNewlines)
        for (abre, fecha) in [("<saida>", "</saida>"), ("<transcricao>", "</transcricao>")] where t.hasPrefix(abre) {
            t = String(t.dropFirst(abre.count))
            if let f = t.range(of: fecha, options: .backwards) { t = String(t[..<f.lowerBound]) }
        }
        return t.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    public func limpar(_ pedido: PedidoLimpeza) async throws -> String {
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let palavras = Palavras.contar(pedido.transcricao)
        let corpo: [String: Any] = [
            "messages": [
                ["role": "system", "content": instrucoes],
                ["role": "user", "content": Self.montarMensagem(pedido)],
            ],
            "temperature": 0,
            "max_tokens": min(4096, palavras * 4 + 96),
            "cache_prompt": true,
            "stream": false,
        ]
        req.httpBody = try JSONSerialization.data(withJSONObject: corpo)
        let (dados, resposta) = try await sessao.data(for: req)
        guard let http = resposta as? HTTPURLResponse, http.statusCode == 200 else {
            throw ErroDeLimpeza("llama-server respondeu \((resposta as? HTTPURLResponse)?.statusCode ?? -1)")
        }
        guard let json = try JSONSerialization.jsonObject(with: dados) as? [String: Any],
              let escolhas = json["choices"] as? [[String: Any]],
              let mensagem = escolhas.first?["message"] as? [String: Any],
              let conteudo = mensagem["content"] as? String
        else { throw ErroDeLimpeza("resposta sem conteúdo") }
        if (escolhas.first?["finish_reason"] as? String) == "length" { throw ErroDeLimpeza("resposta cortada pelo limite") }
        return Self.extrairResposta(conteudo)
    }
}
