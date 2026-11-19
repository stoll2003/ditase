import Foundation

/// Funde a saída do modelo de linguagem com o que a pessoa falou.
///
/// Modelos pequenos, mesmo instruídos, trocam palavra por sinônimo ("linkadas" vira
/// "vinculadas"), "corrigem" nome próprio e encurtam frases. Aqui o modelo só propõe:
/// do texto dele ficam a pontuação, as maiúsculas, os parágrafos e os cortes de muleta,
/// repetição e autocorreção. As palavras continuam sendo as do falante.
public enum Fidelidade {
    /// Hesitações puras: o modelo pode tirar em qualquer posição.
    static let hesitacoes: Set<String> = ["ne", "ahn", "hum", "hm", "uhm", "eh", "ah"]
    /// Muletas que também são palavras com sentido ("foi assim", "está bem"). Só saem quando
    /// o próprio texto as isola com pontuação, que é como o reconhecedor marca a muleta falada.
    static let muletas: Set<String> = ["ta", "entao", "assim", "tipo", "bom", "bem", "olha", "ai", "sabe", "entendeu", "cara", "ok", "enfim"]
    /// Expressões que só servem para anunciar autocorreção. "Na verdade" e "desculpa" ficam de
    /// fora: aparecem em frase comum e não provam que o que veio antes foi desdito.
    static let correcoes: [[String]] = [
        ["nao", "espera"], ["quer", "dizer"], ["alias"], ["ou", "melhor"], ["minto"], ["corrigindo"], ["perdao"], ["digo"], ["melhor", "dizendo"],
    ]
    /// Até quantas palavras antes da expressão de correção, na mesma frase, contam como o trecho desdito.
    static let alcanceDaCorrecao = 8

    enum Passo { case igual(Int, Int), troca(Int, Int), some(Int), entra(Int) }

    public static func fundir(original: String, modelo: String) -> String {
        let a = Palavras.ocorrencias(original)
        let b = Palavras.ocorrencias(modelo)
        guard !a.isEmpty else { return "" }
        guard !b.isEmpty else { return original }
        let na = a.map { Palavras.normalizar($0.texto) }
        let nb = b.map { Palavras.normalizar($0.texto) }
        let passos = alinhar(na, nb)
        // O que há entre cada palavra do original e a seguinte (e antes da primeira, depois da última).
        func vaoOriginal(_ k: Int) -> Substring { // vão antes da palavra k; k == a.count é o fim
            let inicio = k == 0 ? original.startIndex : a[k - 1].intervalo.upperBound
            let fim = k == a.count ? original.endIndex : a[k].intervalo.lowerBound
            return original[inicio..<fim]
        }
        let pontuado = (0...a.count).map { k in k == 0 || k == a.count || vaoOriginal(k).contains(where: { ",.;:!?…\n".contains($0) }) }
        let fimDeFrase = (0...a.count).map { k in vaoOriginal(k).contains(where: { ".!?…\n".contains($0) }) }
        let podeCortar = na.indices.map { k in
            hesitacoes.contains(na[k]) || (String(a[k].texto).lowercased() == "hã") || (muletas.contains(na[k]) && pontuado[k] && pontuado[k + 1])
        }
        let perto = pertoDeCorrecao(na, fimDeFrase: fimDeFrase)
        /// Sinal colado a um número ("-100", "+3") faz parte do valor e não é pontuação do modelo.
        func sinal(_ k: Int) -> String {
            guard let primeiro = a[k].texto.first, primeiro.isNumber, a[k].intervalo.lowerBound > original.startIndex else { return "" }
            let antes = original[original.index(before: a[k].intervalo.lowerBound)]
            guard antes == "-" || antes == "+" || antes == "−" else { return "" }
            let i = original.index(before: a[k].intervalo.lowerBound)
            return i == original.startIndex || original[original.index(before: i)].isWhitespace ? String(antes) : ""
        }

        // O que vem antes de cada palavra do modelo (espaço, pontuação, quebra de linha) e o que sobra no fim.
        func antes(_ j: Int) -> Substring {
            modelo[(j == 0 ? modelo.startIndex : b[j - 1].intervalo.upperBound)..<b[j].intervalo.lowerBound]
        }
        let fim = modelo[b[b.count - 1].intervalo.upperBound...]

        var saida = ""
        var pendente: Substring? // separador de uma palavra do modelo que foi descartada
        // Depois de um trecho restaurado, a pontuação que vale é a do original: a do modelo foi
        // escrita para um texto sem aquele trecho e não se sabe mais onde ela cairia.
        var aposRestauro = false
        var ultimoEmitido = -1
        func separar(_ doModelo: Substring, original k: Int) {
            // Entre a última palavra do original que saiu e esta pode haver palavras cortadas; vale o
            // separador mais forte desse intervalo (o ponto final antes de uma autocorreção apagada).
            var doOriginal = vaoOriginal(k)
            if ultimoEmitido + 1 < k {
                for q in (ultimoEmitido + 1)..<k where forca(vaoOriginal(q)) > forca(doOriginal) { doOriginal = vaoOriginal(q) }
            }
            let sep: Substring = aposRestauro ? (doOriginal.isEmpty ? " " : doOriginal) : doModelo
            aposRestauro = false
            // Se o modelo inseriu palavra entre duas mantidas, vale o separador mais forte dos dois.
            let escolhido = (pendente.map { forca($0) > forca(sep) ? $0 : sep }) ?? sep
            pendente = nil
            if saida.isEmpty { saida += escolhido.filter { "\"“(¿¡".contains($0) } } else { saida += escolhido.isEmpty ? " " : escolhido }
        }

        var i = 0
        while i < passos.count {
            switch passos[i] {
            case .igual(let k, let j):
                // Mesma palavra a menos de acento e caixa: valem as letras do falante ("pode" não vira
                // "pôde"); do modelo só a maiúscula de início de frase.
                separar(antes(j), original: k)
                let s = sinal(k)
                if !s.isEmpty, !saida.hasSuffix(s) { saida += s }
                saida += comMaiusculaDe(String(b[j].texto), String(a[k].texto))
                ultimoEmitido = k
            case .troca(let k, let j):
                // Palavra trocada pelo modelo: fica a do falante, com a pontuação do modelo.
                separar(antes(j), original: k)
                let s = sinal(k)
                if !s.isEmpty, !saida.hasSuffix(s) { saida += s }
                saida += comMaiusculaDe(String(b[j].texto), String(a[k].texto))
                ultimoEmitido = k
            case .entra(let j):
                // Palavra que ninguém falou: sai. A pontuação em volta dela é aproveitada.
                let sep = antes(j)
                if pendente.map({ forca($0) < forca(sep) }) ?? true { pendente = sep }
            case .some:
                // Trecho que o modelo apagou: só é aceito se for muleta, repetição ou autocorreção.
                var ks: [Int] = []
                while i < passos.count, case .some(let k) = passos[i] {
                    ks.append(k)
                    i += 1
                }
                i -= 1
                // Cada frase do trecho apagado é julgada à parte: o modelo pode ter cortado, de uma vez,
                // uma frase inteira (que volta) e uma autocorreção (que sai).
                var grupos: [[Int]] = []
                for k in ks {
                    if grupos.isEmpty || fimDeFrase[k] { grupos.append([k]) } else { grupos[grupos.count - 1].append(k) }
                }
                for grupo in grupos where !podeSumir(grupo, na, podeCortar, perto) {
                    // Volta o trecho como foi dito, com a pontuação original entre as palavras.
                    pendente = nil
                    for k in grupo {
                        if !saida.isEmpty {
                            let vao = vaoOriginal(k)
                            saida += vao.isEmpty ? " " : String(vao.filter { !"-+−".contains($0) || sinal(k).isEmpty })
                        }
                        saida += sinal(k) + a[k].texto
                        ultimoEmitido = k
                    }
                    aposRestauro = true
                }
            }
            i += 1
        }
        saida += fim.filter { !$0.isLetter && !$0.isNumber }
        return RegrasFala.arrumarEspacos(saida)
    }

    // MARK: - Regras de corte

    static func podeSumir(_ ks: [Int], _ na: [String], _ podeCortar: [Bool], _ perto: [Bool]) -> Bool {
        guard let primeiro = ks.first, let ultimo = ks.last else { return true }
        // Também vale a palavra igual à mantida logo antes do corte, quando esta faz parte da
        // autocorreção: em "manda pro João, quer dizer, manda pra Maria" tanto faz qual "manda" ficou.
        let anterior = primeiro > 0 && perto[primeiro - 1] ? na[primeiro - 1] : nil
        if ks.allSatisfy({ podeCortar[$0] || perto[$0] || na[$0] == anterior }) { return true }
        // Recomeço: o que foi apagado é dito de novo logo em seguida ("a gente, a gente vai").
        // Uma palavra só não basta ("eu quero que ele" não é recomeço de "ele").
        let trecho = ks.map { na[$0] }
        guard trecho.count >= 2, trecho.count <= 8 else { return false }
        let depois = Array(na[(ultimo + 1)...].prefix(trecho.count + 2))
        if contem(depois, trecho) { return true }
        let antes = Array(na[..<primeiro].suffix(trecho.count + 2))
        return contem(antes, trecho)
    }

    static func contem(_ lista: [String], _ trecho: [String]) -> Bool {
        guard !trecho.isEmpty, lista.count >= trecho.count else { return false }
        for i in 0...(lista.count - trecho.count) where Array(lista[i..<(i + trecho.count)]) == trecho { return true }
        return false
    }

    /// Marca a expressão de correção e as palavras que a antecedem na mesma frase.
    /// Frase já encerrada antes dela não é o trecho desdito e não pode ser apagada por causa dela.
    static func pertoDeCorrecao(_ na: [String], fimDeFrase: [Bool]) -> [Bool] {
        var marcas = [Bool](repeating: false, count: na.count)
        guard !na.isEmpty else { return marcas }
        for i in na.indices {
            for expressao in correcoes where i + expressao.count <= na.count && Array(na[i..<(i + expressao.count)]) == expressao {
                for k in i..<(i + expressao.count) { marcas[k] = true }
                var k = i - 1
                var contados = 0
                // fimDeFrase[k + 1] é o vão entre a palavra k e a seguinte.
                while k >= 0, contados < alcanceDaCorrecao, !fimDeFrase[k + 1] {
                    marcas[k] = true
                    k -= 1
                    contados += 1
                }
            }
        }
        return marcas
    }

    // MARK: - Detalhes de texto

    static func forca(_ sep: Substring) -> Int {
        if sep.contains("\n") { return 4 }
        if sep.contains(where: { ".!?…".contains($0) }) { return 3 }
        if sep.contains(where: { ":;".contains($0) }) { return 2 }
        if sep.contains(",") { return 1 }
        return 0
    }

    /// A palavra do falante, com a inicial maiúscula se o modelo abriu frase ali.
    static func comMaiusculaDe(_ doModelo: String, _ doFalante: String) -> String {
        guard let m = doModelo.first, let f = doFalante.first else { return doFalante }
        if m.isUppercase, f.isLowercase { return f.uppercased() + doFalante.dropFirst() }
        return doFalante
    }

    // MARK: - Alinhamento

    /// Alinhamento de menor custo entre as duas sequências de palavras.
    static func alinhar(_ a: [String], _ b: [String]) -> [Passo] {
        let n = a.count, m = b.count
        var d = [[Int32]](repeating: [Int32](repeating: 0, count: m + 1), count: n + 1)
        for i in 0...n { d[i][0] = Int32(i) }
        for j in 0...m { d[0][j] = Int32(j) }
        if n > 0, m > 0 {
            for i in 1...n {
                for j in 1...m {
                    let custo: Int32 = a[i - 1] == b[j - 1] ? 0 : 1
                    d[i][j] = min(d[i - 1][j] + 1, d[i][j - 1] + 1, d[i - 1][j - 1] + custo)
                }
            }
        }
        var passos: [Passo] = []
        var i = n, j = m
        while i > 0 || j > 0 {
            if i > 0, j > 0, a[i - 1] == b[j - 1], d[i][j] == d[i - 1][j - 1] {
                passos.append(.igual(i - 1, j - 1))
                i -= 1
                j -= 1
            } else if i > 0, j > 0, d[i][j] == d[i - 1][j - 1] + 1 {
                passos.append(.troca(i - 1, j - 1))
                i -= 1
                j -= 1
            } else if i > 0, d[i][j] == d[i - 1][j] + 1 {
                passos.append(.some(i - 1))
                i -= 1
            } else {
                passos.append(.entra(j - 1))
                j -= 1
            }
        }
        return passos.reversed()
    }
}
