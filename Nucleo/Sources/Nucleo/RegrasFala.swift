import Foundation

/// Limpeza por regras, instantânea, aplicada a toda transcrição antes do modelo de linguagem.
/// É também o texto de reserva quando o modelo falha, demora ou inventa.
public enum RegrasFala {
    /// Hesitações que só saem quando seguidas de pausa (vírgula, ponto, reticências).
    static let hesitacoes: Set<String> = ["ha", "han", "ahn", "ah", "ahm", "hum", "hm", "hmm", "uhm", "uh", "um-hum", "eh", "ehh", "ee", "eee", "aa", "aaa"]
    /// Estas saem em qualquer posição. "ha" não está aqui: sem acento não dá para saber se é "hã" ou "há".
    static let hesitacoesCertas: Set<String> = ["han", "ahn", "ahm", "hum", "hm", "hmm", "uhm", "uh", "ehh", "ee", "eee", "aa", "aaa"]
    /// Palavras de ligação que, repetidas em seguida, são gaguejo e não ênfase.
    static let ligacoes: Set<String> = [
        "a", "o", "as", "os", "um", "uma", "de", "do", "da", "dos", "das", "em", "no", "na", "nos", "nas", "e", "que", "se",
        "com", "por", "para", "pra", "pro", "eu", "ele", "ela", "isso", "esse", "essa", "este", "esta", "ao", "aos", "ou", "mas", "porque", "como", "quando", "meu", "minha",
    ]

    public static func filtrar(_ bruto: String) -> String {
        var texto = bruto.replacingOccurrences(of: "\u{00A0}", with: " ")
        texto = aplicarComandos(texto)
        texto = tirarHesitacoes(texto)
        texto = tirarGaguejo(texto)
        return arrumarEspacos(texto)
    }

    /// "nova linha" e "novo parágrafo" ditos como comando, isto é, isolados por pausa ou pontuação.
    static func aplicarComandos(_ texto: String) -> String {
        var saida = texto
        let padroes: [(String, String)] = [
            (#"(?i)(^|[.,;:!?…]\s*|\n\s*)nov[oa]\s+par[aá]grafo\b[.,;:!?…]?\s*"#, "$1\n\n"),
            (#"(?i)(^|[.,;:!?…]\s*|\n\s*)nova\s+linha\b[.,;:!?…]?\s*"#, "$1\n"),
        ]
        for (padrao, troca) in padroes {
            saida = saida.replacingOccurrences(of: padrao, with: troca, options: .regularExpression)
        }
        // A vírgula que antecedia o comando vira ponto: a frase acabou ali.
        saida = saida.replacingOccurrences(of: #"[,;:]\s*\n"#, with: ".\n", options: .regularExpression)
        return saida
    }

    static func tirarHesitacoes(_ texto: String) -> String {
        var saida = ""
        var cursor = texto.startIndex
        let palavras = Palavras.ocorrencias(texto)
        for (i, p) in palavras.enumerated() {
            let chave = String(p.texto).lowercased().folding(options: .diacriticInsensitive, locale: nil)
            let original = String(p.texto).lowercased()
            let depois = p.intervalo.upperBound < texto.endIndex ? texto[p.intervalo.upperBound] : " "
            let seguidaDePausa = depois == "," || depois == "." || depois == "…"
            // "há" é verbo ("há três processos", "há dois anos") e nunca hesitação, embora sem acento
            // fique igual a "hã".
            if original == "há" { continue }
            let ehHesitacao = hesitacoesCertas.contains(chave)
                || original == "hã"
                || (hesitacoes.contains(chave) && seguidaDePausa && (i == 0 || chave != "ah"))
                || (original == "é" && texto[p.intervalo.upperBound...].hasPrefix("..."))
            guard ehHesitacao else { continue }
            saida += texto[cursor..<p.intervalo.lowerBound]
            // Leva junto a pontuação de pausa colada na hesitação.
            var fim = p.intervalo.upperBound
            while fim < texto.endIndex, ",.…".contains(texto[fim]) { fim = texto.index(after: fim) }
            let terminaFrase = texto[p.intervalo.upperBound..<fim].contains(".") && !saida.trimmingCharacters(in: .whitespaces).isEmpty
            let jaPontuado = saida.trimmingCharacters(in: .whitespaces).last.map { ".!?…,;:\n".contains($0) } ?? true
            if terminaFrase && !jaPontuado { saida = saida.trimmingCharacters(in: .whitespaces) + ". " }
            cursor = fim
        }
        saida += texto[cursor...]
        return saida
    }

    static func tirarGaguejo(_ texto: String) -> String {
        let palavras = Palavras.ocorrencias(texto)
        guard palavras.count > 1 else { return texto }
        var remover: [Range<String.Index>] = []
        for i in 1..<palavras.count {
            let a = palavras[i - 1], b = palavras[i]
            let entre = texto[a.intervalo.upperBound..<b.intervalo.lowerBound]
            guard entre.allSatisfy({ $0 == " " || $0 == "," }) else { continue }
            let ka = String(a.texto).lowercased(), kb = String(b.texto).lowercased()
            // "eu eu", "de de": repetição de palavra de ligação. "e-eu eu": corte no meio da palavra.
            let repetida = ka == kb && ligacoes.contains(ka)
            let cortada = ka.hasSuffix("-") || (ka.count <= 2 && kb.hasPrefix(ka) && kb.count > ka.count && entre.contains(",") && !ligacoes.contains(ka))
            if repetida || cortada { remover.append(a.intervalo.lowerBound..<b.intervalo.lowerBound) }
        }
        var saida = texto
        for r in remover.reversed() { saida.removeSubrange(r) }
        return saida
    }

    /// Espaços, pontuação solta e maiúscula no começo de cada frase.
    public static func arrumarEspacos(_ texto: String) -> String {
        var t = texto
        t = t.replacingOccurrences(of: #"[ \t]+"#, with: " ", options: .regularExpression)
        t = t.replacingOccurrences(of: #" +([,.;:!?…])"#, with: "$1", options: .regularExpression)
        t = t.replacingOccurrences(of: #"([,;:])\1+"#, with: "$1", options: .regularExpression)
        t = t.replacingOccurrences(of: #",\s*([.!?])"#, with: "$1", options: .regularExpression)
        t = t.replacingOccurrences(of: #"(^|\n)[ ,;:.]+"#, with: "$1", options: .regularExpression)
        t = t.replacingOccurrences(of: #" *\n *"#, with: "\n", options: .regularExpression)
        t = t.replacingOccurrences(of: #"\n{3,}"#, with: "\n\n", options: .regularExpression)
        t = t.trimmingCharacters(in: .whitespacesAndNewlines)
        return maiusculasDeFrase(t)
    }

    /// Maiúscula na primeira letra do texto, de cada linha e depois de ponto seguido de espaço.
    /// Ponto sem espaço ("arquivo.pdf", "3.5") não abre frase.
    static func maiusculasDeFrase(_ texto: String) -> String {
        var saida = ""
        saida.reserveCapacity(texto.count)
        var inicioDeFrase = true
        var aposTerminador = false
        for c in texto {
            if inicioDeFrase, c.isLetter {
                saida += c.uppercased()
                inicioDeFrase = false
                aposTerminador = false
                continue
            }
            saida.append(c)
            if c == "\n" {
                inicioDeFrase = true
            } else if c == "." || c == "!" || c == "?" || c == "…" {
                aposTerminador = true
            } else if c.isWhitespace {
                if aposTerminador { inicioDeFrase = true }
            } else if c == "\"" || c == "“" || c == "(" || c == "'" || c == "”" || c == ")" {
                // aspas e parênteses não mudam o estado
            } else {
                inicioDeFrase = false
                aposTerminador = false
            }
        }
        return saida
    }
}
