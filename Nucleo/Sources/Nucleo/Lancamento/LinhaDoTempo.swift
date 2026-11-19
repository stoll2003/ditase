import Foundation

/// Linha de tempo comum da reunião, em amostras de 16 kHz contadas só no tempo ativo.
/// Os buffers chegam com o carimbo da captura (relógio monotônico do host, em segundos).
public struct LinhaDoTempo: Sendable, Equatable {
    public let inicio: Double
    private var pausas: [(de: Double, ate: Double?)] = []

    public init(inicio: Double) { self.inicio = inicio }

    public static func == (a: LinhaDoTempo, b: LinhaDoTempo) -> Bool {
        a.inicio == b.inicio && a.pausas.map(\.de) == b.pausas.map(\.de) && a.pausas.map(\.ate) == b.pausas.map(\.ate)
    }

    public var pausada: Bool { pausas.last.map { $0.ate == nil } ?? false }

    /// Pausa e fim fecham as fontes neste instante: amostras carimbadas depois são ignoradas.
    public mutating func pausar(em t: Double) {
        guard !pausada else { return }
        pausas.append((t, nil))
    }

    public mutating func retomar(em t: Double) {
        guard pausada, let de = pausas.last?.de else { return }
        pausas[pausas.count - 1] = (de, max(de, t))
    }

    /// Índice da amostra no tempo ativo. Nulo dentro de uma pausa ou antes do início.
    public func indice(_ t: Double) -> Int? {
        guard t >= inicio else { return nil }
        var parado = 0.0
        for p in pausas {
            if t < p.de { break }
            guard let ate = p.ate, t >= ate else { return nil }
            parado += ate - p.de
        }
        return Int(((t - inicio - parado) * Double(taxaLancamento)).rounded())
    }

    /// Partes ativas de um buffer: (primeira amostra no buffer, quantidade, índice na linha de tempo).
    public func partesAtivas(tempo t: Double, quantidade n: Int) -> [(origem: Int, quantidade: Int, indice: Int)] {
        let taxa = Double(taxaLancamento)
        var limites: [Int] = [0, n]
        let marcos = [inicio] + pausas.flatMap { [$0.de, $0.ate].compactMap { $0 } }
        for marco in marcos {
            let k = Int(((marco - t) * taxa).rounded())
            if k > 0 && k < n { limites.append(k) }
        }
        let cortes = Array(Set(limites)).sorted()
        var saida: [(origem: Int, quantidade: Int, indice: Int)] = []
        for (a, b) in zip(cortes, cortes.dropFirst()) where b > a {
            let meio = t + (Double(a) + Double(b - a) / 2) / taxa
            guard let m = indice(meio) else { continue }
            let i = m - Int((Double(b - a) / 2).rounded())
            if let ultima = saida.last, ultima.origem + ultima.quantidade == a, ultima.indice + ultima.quantidade == i {
                saida[saida.count - 1].quantidade += b - a
            } else {
                saida.append((a, b - a, i))
            }
        }
        return saida
    }
}

/// Cursor de uma fonte numa geração: coloca cada buffer na linha de tempo, absorve deriva pequena,
/// declara lacuna quando faltam amostras e descarta amostras repetidas no tempo.
public struct CursorDeFonte: Sendable, Equatable {
    /// 60 ms: abaixo disso a diferença é deriva do relógio e fica absorvida.
    public static let tolerancia = 960

    public enum Evento: Equatable, Sendable {
        /// Amostras do buffer `origem..<origem+quantidade` vão para a linha de tempo a partir de `indice`.
        case amostras(origem: Int, quantidade: Int, indice: Int)
        case lacuna(Range<Int>)
    }

    public private(set) var proximo: Int

    public init(abertura: Int) { proximo = abertura }

    public mutating func receber(tempo t: Double, quantidade n: Int, linha: LinhaDoTempo) -> [Evento] {
        var eventos: [Evento] = []
        for parte in linha.partesAtivas(tempo: t, quantidade: n) {
            var origem = parte.origem
            var quantidade = parte.quantidade
            let diferenca = parte.indice - proximo
            if diferenca > Self.tolerancia {
                eventos.append(.lacuna(proximo..<parte.indice))
                proximo = parte.indice
            } else if diferenca < -Self.tolerancia {
                let repetidas = min(quantidade, -diferenca)
                origem += repetidas
                quantidade -= repetidas
            }
            guard quantidade > 0 else { continue }
            eventos.append(.amostras(origem: origem, quantidade: quantidade, indice: proximo))
            proximo += quantidade
        }
        return eventos
    }
}
