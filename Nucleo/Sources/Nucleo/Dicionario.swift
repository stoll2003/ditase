import Foundation

/// Um termo que o reconhecedor costuma errar: nome próprio, sigla, jargão.
public struct Termo: Codable, Equatable, Identifiable, Sendable {
    public var id: Int64
    /// Como deve sair escrito.
    public var termo: String
    /// Formas erradas em que ele costuma sair ("Cloud" para "Claude").
    public var ouvidoComo: [String]
    public var usos: Int
    /// "manual" ou "wispr" (importado).
    public var origem: String
    public var criadoEm: Double

    public init(id: Int64 = 0, termo: String, ouvidoComo: [String] = [], usos: Int = 0, origem: String = "manual", criadoEm: Double = Date().timeIntervalSince1970) {
        self.id = id
        self.termo = termo
        self.ouvidoComo = ouvidoComo
        self.usos = usos
        self.origem = origem
        self.criadoEm = criadoEm
    }
}

public struct Troca: Equatable, Sendable {
    public let de: String
    public let para: String
    public let termoId: Int64
}

/// Aplica o dicionário a um texto: grafia exata, formas "ouvido como" e, para palavras
/// que não existem em português, semelhança de som.
public struct AplicadorDicionario: Sendable {
    struct Entrada: Sendable {
        let chave: String
        let termo: Termo
        let exata: Bool
    }

    let entradas: [Entrada]
    let fonetica: [(chave: String, termo: Termo)]
    let ehPalavraComum: (@Sendable (String) -> Bool)?
    /// Semelhança de som mínima para trocar sem o usuário ter cadastrado a forma errada.
    static let limiarSom = 0.8

    public let vocabulario: [String]

    public init(termos: [Termo], ehPalavraComum: (@Sendable (String) -> Bool)? = nil) {
        var entradas: [Entrada] = []
        var fonetica: [(String, Termo)] = []
        for t in termos {
            let chave = Self.chaveColada(t.termo)
            guard !chave.isEmpty else { continue }
            entradas.append(Entrada(chave: chave, termo: t, exata: true))
            for forma in t.ouvidoComo {
                let c = Self.chaveColada(forma)
                if !c.isEmpty, c != chave { entradas.append(Entrada(chave: c, termo: t, exata: false)) }
            }
            let som = Self.chaveDeSom(t.termo)
            if som.count >= 5 { fonetica.append((som, t)) }
        }
        self.entradas = entradas
        self.fonetica = fonetica
        self.ehPalavraComum = ehPalavraComum
        self.vocabulario = termos.map(\.termo)
    }

    public func aplicar(_ texto: String) -> (texto: String, trocas: [Troca]) {
        guard !entradas.isEmpty else { return (texto, []) }
        let palavras = Palavras.ocorrencias(texto)
        let normais = palavras.map { Palavras.normalizar($0.texto) }
        var trocas: [(Range<String.Index>, Termo, String)] = []
        var i = 0
        while i < palavras.count {
            var achou: (janela: Int, termo: Termo)?
            for janela in stride(from: min(4, palavras.count - i), through: 1, by: -1) {
                // Só junta palavras separadas por espaço ou hífen; pontuação quebra o termo.
                if janela > 1 {
                    let entre = texto[palavras[i].intervalo.upperBound..<palavras[i + janela - 1].intervalo.lowerBound]
                    if entre.contains(where: { ".,;:!?…\n()\"".contains($0) }) { continue }
                }
                let colada = normais[i..<(i + janela)].joined()
                if let e = entradas.first(where: { $0.chave == colada }) {
                    achou = (janela, e.termo)
                    break
                }
                if janela <= 2, let t = porSom(palavras: palavras[i..<(i + janela)].map { String($0.texto) }) {
                    achou = (janela, t)
                    break
                }
            }
            if let achou {
                let intervalo = palavras[i].intervalo.lowerBound..<palavras[i + achou.janela - 1].intervalo.upperBound
                let atual = String(texto[intervalo])
                if atual != achou.termo.termo { trocas.append((intervalo, achou.termo, atual)) }
                i += achou.janela
            } else {
                i += 1
            }
        }
        var saida = texto
        for (intervalo, termo, _) in trocas.reversed() { saida.replaceSubrange(intervalo, with: termo.termo) }
        return (saida, trocas.map { Troca(de: $0.2, para: $0.1.termo, termoId: $0.1.id) })
    }

    func porSom(palavras: [String]) -> Termo? {
        guard !fonetica.isEmpty else { return nil }
        // Palavra que existe em português nunca é trocada por semelhança: para isso serve o "ouvido como".
        // Vale para cada palavra da janela, senão "no vitepress" viraria "VitePress" e comeria o "no".
        if let comum = ehPalavraComum {
            if palavras.contains(where: { comum($0) }) { return nil }
        } else if palavras.count == 1 ? palavras[0].count < 6 : palavras.contains(where: { $0.count < 3 }) {
            return nil
        }
        let som = Self.chaveDeSom(palavras.joined())
        guard som.count >= 4 else { return nil }
        var melhor: (Double, Termo)?
        for (chave, termo) in fonetica {
            guard abs(chave.count - som.count) <= 2 else { continue }
            let s = Self.semelhanca(som, chave)
            if s >= Self.limiarSom, s > (melhor?.0 ?? 0) { melhor = (s, termo) }
        }
        return melhor?.1
    }

    // MARK: - Chaves

    static func chaveColada(_ texto: String) -> String {
        Palavras.ocorrencias(texto).map { Palavras.normalizar($0.texto) }.joined()
    }

    /// Aproximação de como a palavra soa em português do Brasil, para comparar o que
    /// o reconhecedor escreveu com o termo cadastrado.
    static func chaveDeSom(_ texto: String) -> String {
        var s = texto.lowercased().replacingOccurrences(of: "ç", with: "s")
        s = Palavras.normalizar(s)
        let regras: [(String, String)] = [
            ("ph", "f"), ("ch", "x"), ("sh", "x"), ("lh", "li"), ("nh", "ni"), ("ck", "k"), ("th", "t"),
            ("que", "ke"), ("qui", "ki"), ("qu", "ku"), ("gue", "ge"), ("gui", "gi"),
            ("sce", "se"), ("sci", "si"), ("xce", "se"), ("xci", "si"), ("ce", "se"), ("ci", "si"),
            ("c", "k"), ("q", "k"), ("y", "i"), ("w", "u"), ("z", "s"), ("h", ""), ("ou", "o"), ("ei", "e"),
        ]
        for (de, para) in regras { s = s.replacingOccurrences(of: de, with: para) }
        var colapsada = ""
        for c in s where colapsada.last != c { colapsada.append(c) }
        if colapsada.hasSuffix("e") { colapsada = String(colapsada.dropLast()) + "i" }
        if colapsada.hasSuffix("o") { colapsada = String(colapsada.dropLast()) + "u" }
        if colapsada.hasSuffix("m") { colapsada = String(colapsada.dropLast()) + "n" }
        return colapsada
    }

    static func semelhanca(_ a: String, _ b: String) -> Double {
        let x = Array(a), y = Array(b)
        if x.isEmpty || y.isEmpty { return 0 }
        var anterior = Array(0...y.count)
        for i in 1...x.count {
            var atual = [i] + Array(repeating: 0, count: y.count)
            for j in 1...y.count {
                atual[j] = min(anterior[j] + 1, atual[j - 1] + 1, anterior[j - 1] + (x[i - 1] == y[j - 1] ? 0 : 1))
            }
            anterior = atual
        }
        return 1 - Double(anterior[y.count]) / Double(max(x.count, y.count))
    }
}
