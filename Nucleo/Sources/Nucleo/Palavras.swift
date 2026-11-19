import Foundation

/// Operações sobre palavras usadas pela contagem, pelo dicionário e pela guarda.
public enum Palavras {
    /// Uma palavra do texto e onde ela está.
    public struct Ocorrencia {
        public let texto: Substring
        public let intervalo: Range<String.Index>
    }

    /// Minúsculas, sem acento e sem pontuação: a forma usada para comparar o que foi dito.
    public static func normalizar(_ palavra: some StringProtocol) -> String {
        let semAcento = palavra.folding(options: [.diacriticInsensitive, .caseInsensitive], locale: Locale(identifier: "pt_BR"))
        return String(semAcento.unicodeScalars.filter { CharacterSet.alphanumerics.contains($0) })
    }

    /// Palavras do texto, na ordem, com a posição de cada uma. Hífen e apóstrofo no meio ficam na palavra.
    public static func ocorrencias(_ texto: String) -> [Ocorrencia] {
        var saida: [Ocorrencia] = []
        var inicio: String.Index?
        var i = texto.startIndex
        func ehLetra(_ c: Character) -> Bool { c.isLetter || c.isNumber }
        while i < texto.endIndex {
            let c = texto[i]
            let proximo = texto.index(after: i)
            let dentro = ehLetra(c)
                || ((c == "-" || c == "'" || c == "’") && inicio != nil && proximo < texto.endIndex && ehLetra(texto[proximo]))
            if dentro {
                if inicio == nil { inicio = i }
            } else if let ini = inicio {
                saida.append(Ocorrencia(texto: texto[ini..<i], intervalo: ini..<i))
                inicio = nil
            }
            i = proximo
        }
        if let ini = inicio { saida.append(Ocorrencia(texto: texto[ini...], intervalo: ini..<texto.endIndex)) }
        return saida
    }

    public static func contar(_ texto: String) -> Int { ocorrencias(texto).count }

    /// Palavras normalizadas, sem as vazias.
    public static func normalizadas(_ texto: String) -> [String] {
        ocorrencias(texto).map { normalizar($0.texto) }.filter { !$0.isEmpty }
    }

    /// As últimas `n` palavras do texto, com a pontuação original entre elas.
    public static func cauda(_ texto: String, palavras n: Int) -> String {
        let todas = ocorrencias(texto)
        guard todas.count > n, n > 0 else { return texto }
        return String(texto[todas[todas.count - n].intervalo.lowerBound...])
    }
}
