import Foundation

/// Critério explícito de atividade de voz: ao menos 60 ms seguidos (3 quadros de 20 ms)
/// com RMS acima de 0,006 (cerca de -44 dBFS). Fala curta e baixa passa; silêncio e chiado, não.
public enum AtividadeDeVoz {
    public static let quadro = 320
    public static let limiarRMS: Float = 0.006
    public static let quadrosSeguidos = 3

    public static func tem(_ amostras: ArraySlice<Float>) -> Bool {
        var seguidos = 0
        var i = amostras.startIndex
        while i < amostras.endIndex {
            let fim = min(i + quadro, amostras.endIndex)
            var soma: Float = 0
            for j in i..<fim { soma += amostras[j] * amostras[j] }
            if (soma / Float(fim - i)).squareRoot() >= limiarRMS {
                seguidos += 1
                if seguidos >= quadrosSeguidos { return true }
            } else { seguidos = 0 }
            i = fim
        }
        return false
    }

    public static func tem(_ amostras: [Float]) -> Bool { tem(amostras[...]) }
}

/// Decisão de uma costura por fonte. O prefixo até `confirmadoAte` já está no banco; a janela
/// reconhecida traz contexto antes dele. Só confirma palavras com fronteira resolvida: o corte cai
/// num ponto livre de palavras antes do limite, e o que vem depois fica como cauda provisória.
public enum Costura {
    /// Margem livre em torno de cada palavra para o corte não encostar nela.
    public static let folga = 0.1
    /// Cauda máxima sem corte livre: depois disso corta no fim de uma palavra (fala contínua).
    /// Precisa ser menor que a janela de reconhecimento menos a margem (15 s - 0,8 s).
    public static let caudaMaxima = 10.0
    /// Distância mínima do limite para um corte forçado.
    public static let recuoForcado = 1.0

    public struct Resultado: Equatable, Sendable {
        public let confirmadas: [PalavraReconhecida]
        /// Novo ponto confirmado, em amostras.
        public let novoConfirmado: Int
        /// Faixa com voz e sem palavras que excedeu a cauda máxima ou chegou ao fechamento.
        public let pendente: Range<Int>?
    }

    /// - Parameters:
    ///   - palavras: palavras da janela, em segundos absolutos.
    ///   - confirmadoAte: ponto já confirmado, em amostras.
    ///   - limite: até onde a janela permite decidir, em amostras (o fim do áudio no fechamento).
    ///   - fechamento: pausa, fim ou recuperação: confirma tudo até o limite.
    ///   - atividade: atividade de voz numa faixa de amostras.
    public static func decidir(palavras: [PalavraReconhecida], confirmadoAte: Int, limite: Int, fechamento: Bool,
                               atividade: (Range<Int>) -> Bool) -> Resultado {
        let taxa = Double(taxaLancamento)
        let c0 = Double(confirmadoAte) / taxa
        let l = Double(limite) / taxa
        let candidatas = palavras.filter { $0.centro > c0 }.sorted { $0.inicio < $1.inicio }
        guard limite > confirmadoAte else { return Resultado(confirmadas: [], novoConfirmado: confirmadoAte, pendente: nil) }

        if fechamento {
            let confirmadas = candidatas.filter { $0.centro <= l }
            let semPalavras = confirmadas.isEmpty && atividade(confirmadoAte..<limite)
            return Resultado(confirmadas: confirmadas, novoConfirmado: limite,
                             pendente: semPalavras ? confirmadoAte..<limite : nil)
        }

        if candidatas.isEmpty {
            if !atividade(confirmadoAte..<limite) {
                return Resultado(confirmadas: [], novoConfirmado: limite, pendente: nil)
            }
            if l - c0 > caudaMaxima {
                return Resultado(confirmadas: [], novoConfirmado: limite, pendente: confirmadoAte..<limite)
            }
            return Resultado(confirmadas: [], novoConfirmado: confirmadoAte, pendente: nil)
        }

        // Maior ponto livre de palavras (com folga) depois do confirmado e até o limite.
        let ocupadas = candidatas.map { ($0.inicio - folga, $0.fim + folga) }
        var corte: Double?
        var p = l
        while p > c0 {
            if let palavra = ocupadas.first(where: { $0.0 < p && p < $0.1 }) {
                p = palavra.0
                continue
            }
            corte = p
            break
        }
        if corte == nil, l - c0 > caudaMaxima,
           let ultima = candidatas.last(where: { $0.fim <= l - recuoForcado && $0.fim > c0 }) {
            corte = ultima.fim
        }
        guard let corte, corte > c0 else {
            return Resultado(confirmadas: [], novoConfirmado: confirmadoAte, pendente: nil)
        }
        let novo = min(limite, max(confirmadoAte + 1, Int((corte * taxa).rounded())))
        let confirmadas = candidatas.filter { $0.centro < Double(novo) / taxa }
        return Resultado(confirmadas: confirmadas, novoConfirmado: novo, pendente: nil)
    }

    /// Agrupa palavras confirmadas em trechos por pausa, duração e pontuação final.
    /// O ID nasce da fonte, da geração e da primeira amostra: repetir não duplica. Dois trechos do
    /// mesmo lote que começam na mesma amostra recebem sufixo pela ordem.
    public static func trechos(_ palavras: [PalavraReconhecida], documento: String, fonte: FonteDeAudio, geracao: Int,
                               prefixo: String? = nil) -> [(id: String, inicio: Double, fim: Double, texto: String)] {
        var saida: [(id: String, inicio: Double, fim: Double, texto: String)] = []
        var usados: [String: Int] = [:]
        for palavra in palavras.sorted(by: { $0.inicio < $1.inicio }) {
            if var ultimo = saida.last, palavra.inicio - ultimo.fim < 1.2, palavra.fim - ultimo.inicio <= 15,
               !ultimo.texto.hasSuffix("."), !ultimo.texto.hasSuffix("?"), !ultimo.texto.hasSuffix("!") {
                ultimo.texto += " " + palavra.texto
                ultimo.fim = max(ultimo.fim, palavra.fim)
                saida[saida.count - 1] = ultimo
            } else {
                let amostra = Int((palavra.inicio * Double(taxaLancamento)).rounded())
                let base = (prefixo ?? "\(documento)|\(fonte.rawValue)|\(geracao)") + "|\(amostra)"
                let repeticao = usados[base, default: 0]
                usados[base] = repeticao + 1
                let id = repeticao == 0 ? base : base + "+\(repeticao)"
                saida.append((id, palavra.inicio, palavra.fim, palavra.texto))
            }
        }
        return saida
    }
}
