import Foundation

/// Decide se a saída do modelo de linguagem pode substituir o texto das regras.
/// O modelo só pode tirar, pontuar e corrigir de leve; se acrescentar conteúdo
/// ou cortar demais, o ditado fica com o texto das regras.
public enum Guarda {
    public enum Recusa: String, Equatable, Sendable {
        case vazio, acrescentou, cortouDemais, respondeu
    }

    public static func avaliar(original: String, limpo: String) -> Recusa? {
        let antes = Palavras.normalizadas(original)
        let depois = Palavras.normalizadas(limpo)
        if antes.isEmpty { return depois.isEmpty ? nil : .acrescentou }
        if depois.isEmpty { return antes.count > 3 ? .vazio : nil }

        var disponiveis: [String: Int] = [:]
        for p in antes { disponiveis[p, default: 0] += 1 }
        var novas = 0
        for p in depois {
            if let n = disponiveis[p], n > 0 { disponiveis[p] = n - 1 }
            else if !ehVariacao(p, de: antes) { novas += 1 }
        }
        // Correção de concordância e de número troca algumas palavras; conteúdo novo troca muitas.
        let toleradas = max(2, Int((Double(antes.count) * 0.12).rounded()))
        if novas > toleradas { return depois.count > antes.count + toleradas ? .respondeu : .acrescentou }
        if antes.count >= 8, Double(depois.count) < Double(antes.count) * 0.5 { return .cortouDemais }
        return nil
    }

    /// Mesma palavra com outra terminação ("ver" e "veja", "coisas" e "coisa") ou colada ("micro coisas" e "microcoisas").
    static func ehVariacao(_ palavra: String, de originais: [String]) -> Bool {
        guard palavra.count >= 3 else { return false }
        let raiz = palavra.prefix(max(3, palavra.count - 3))
        for o in originais where o.count >= 3 {
            if o.hasPrefix(raiz) || palavra.hasPrefix(o) || palavra.hasSuffix(o) { return true }
        }
        return false
    }
}
