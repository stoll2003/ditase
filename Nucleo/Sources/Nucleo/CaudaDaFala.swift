import Foundation

/// Localiza o último grupo de quadros com energia, separado por uma pausa.
/// Índices no PCM mono de 16 kHz, sem contar silêncio acrescentado ao ASR.
public struct CaudaDaFala: Equatable, Sendable {
    public static let capacidade = 8 * 16_000
    public let inicio: Int
    public let fim: Int

    public static func localizar(_ audio: [Float]) -> CaudaDaFala? {
        let quadro = 320
        let pausaMinima = 13 // 260 ms, para atravessar pausas curtas dentro da mesma frase.
        var inicio: Int?
        var fim: Int?
        var quietos = 0
        var ativos = 0
        var posicao = audio.count
        while posicao > 0 {
            let anterior = max(0, posicao - quadro)
            let energia = audio[anterior..<posicao].reduce(Float(0)) { $0 + $1 * $1 }
            if energia / Float(posicao - anterior) >= 0.008 * 0.008 {
                if fim == nil { fim = posicao }
                inicio = anterior
                ativos += posicao - anterior
                quietos = 0
            } else if inicio != nil {
                quietos += 1
                if quietos >= pausaMinima { break }
            }
            posicao = anterior
        }
        guard ativos >= 1_920, let inicio, let fim else { return nil }
        return CaudaDaFala(inicio: inicio, fim: fim)
    }
}
