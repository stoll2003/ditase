import Foundation
import Nucleo

/// Ajustes do usuário, guardados na tabela `config` do banco.
final class Preferencias {
    enum Tecla: String, CaseIterable {
        case fn, opcaoDireita, comandoDireito, controleDireito

        var nome: String {
            switch self {
            case .fn: return "Fn (🌐)"
            case .opcaoDireita: return "Opção direita (⌥)"
            case .comandoDireito: return "Comando direito (⌘)"
            case .controleDireito: return "Controle direito (⌃)"
            }
        }

        var simbolo: String {
            switch self {
            case .fn: return "fn"
            case .opcaoDireita: return "⌥ direita"
            case .comandoDireito: return "⌘ direito"
            case .controleDireito: return "⌃ direito"
            }
        }
    }

    static let padroes: [String: String] = [
        "atalho": Tecla.opcaoDireita.rawValue,
        "limpezaIA": "1",
        "modelo": "Qwen3.5-2B-Q4_K_M.gguf",
        "sons": "1",
        "limiteMinutos": "120",
        "pararEmSilencioMinutos": "5",
        "guardarAudioDias": "7",
        "microfone": "",
        "pilulaSempreVisivel": "1",
        "espacoNoFim": "1",
        // "0" depois que a pessoa dispensa o aviso de baixar o modelo no Início.
        "avisoModelo": "1",
    ]

    private let banco: Banco
    var aoMudar: ((String) -> Void)?

    init(banco: Banco) { self.banco = banco }

    func ler(_ chave: String) -> String { banco.config(chave) ?? Self.padroes[chave] ?? "" }

    func definir(_ chave: String, _ valor: String) {
        guard Self.padroes[chave] != nil else { return }
        try? banco.definir(config: chave, valor: valor)
        aoMudar?(chave)
    }

    var todas: [String: String] {
        var saida: [String: String] = [:]
        for chave in Self.padroes.keys { saida[chave] = ler(chave) }
        return saida
    }

    var atalho: Tecla { Tecla(rawValue: ler("atalho")) ?? .opcaoDireita }
    var limpezaIA: Bool { ler("limpezaIA") == "1" }
    var modelo: String { ler("modelo") }
    var sons: Bool { ler("sons") == "1" }
    var limiteMinutos: Double { max(1, Double(ler("limiteMinutos")) ?? 120) }
    var pararEmSilencioMinutos: Double { max(0, Double(ler("pararEmSilencioMinutos")) ?? 5) }
    var guardarAudioDias: Int { max(0, Int(ler("guardarAudioDias")) ?? 7) }
    var microfone: String { ler("microfone") }
    var pilulaSempreVisivel: Bool { ler("pilulaSempreVisivel") == "1" }
    var espacoNoFim: Bool { ler("espacoNoFim") == "1" }
}
