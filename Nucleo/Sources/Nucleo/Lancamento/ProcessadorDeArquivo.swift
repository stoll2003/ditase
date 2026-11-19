import CryptoKit
import Foundation

public protocol FonteDePCM: Sendable {
    var quantidade: Int { get }
    func ler(inicio: Int, quantidade: Int) throws -> [Float]
}

/// Transcreve um arquivo numa versão própria. Os trechos da versão só ficam visíveis quando todas as
/// janelas, a cauda e a separação de falantes terminam, numa transação que exige esta tentativa
/// ainda em curso e mais nova que a ativa. Cancelamento, falha ou vazio preservam a versão anterior.
public struct ProcessadorDeArquivo: Sendable {
    public typealias Progresso = @Sendable (_ etapa: String, _ fracao: Double?) async -> Void

    public enum Desfecho: Equatable, Sendable {
        case publicada, semFala, superada
    }

    public let armazem: ArmazemLancamento
    public let reconhecedor: ReconhecedorDeFala
    public let identificador: IdentificadorDeFalantes?
    public let preparar: PreparadorDeTexto

    public init(armazem: ArmazemLancamento, reconhecedor: ReconhecedorDeFala, identificador: IdentificadorDeFalantes?,
                preparar: @escaping PreparadorDeTexto = { $0 }) {
        self.armazem = armazem
        self.reconhecedor = reconhecedor
        self.identificador = identificador
        self.preparar = preparar
    }

    /// - Parameters:
    ///   - conferirEntrada: lança se o arquivo de origem mudou desde o início da tentativa.
    public func transcrever(versao: ArmazemLancamento.Versao, pcm: FonteDePCM, wav: URL?, inicio: Date = Date(),
                            conferirEntrada: @Sendable () throws -> Void = {},
                            progresso: @escaping Progresso = { _, _ in }) async throws -> Desfecho {
        let taxa = Double(taxaLancamento)
        let total = pcm.quantidade
        var palavras: [PalavraReconhecida] = []
        var hash = SHA256()
        let passo = taxaLancamento * 15
        let margem = taxaLancamento
        await progresso("transcrevendo", 0)
        for (seq, inicioJanela) in stride(from: 0, to: total, by: passo).enumerated() {
            try Task.checkCancellation()
            let fim = min(inicioJanela + passo, total)
            let esquerda = max(0, inicioJanela - margem)
            let direita = min(total, fim + margem)
            let dados = try pcm.ler(inicio: esquerda, quantidade: direita - esquerda)
            let faixa = (inicioJanela - esquerda)..<(fim - esquerda)
            guard dados.count == direita - esquerda else { throw ErroDeBanco(description: "A janela de áudio está incompleta.") }
            let pcmJanela = DiscoDeBlocos.dados(Array(dados[faixa]))
            hash.update(data: pcmJanela)
            let janela = Double(inicioJanela) / taxa..<Double(fim) / taxa
            var palavrasDaJanela = try await reconhecedor.palavras(dados, inicio: Double(esquerda) / taxa, fimDaFala: nil)
                .filter { AlinhamentoDeVoz.pertence(inicio: $0.inicio, fim: $0.fim, a: janela) }
            if fim == total {
                let cauda = Array(dados.suffix(CaudaDaFala.capacidade))
                let base = Double(direita - cauda.count) / taxa
                if let recuperada = try await Self.recuperarCauda(cauda, base: base, ultimoFim: palavrasDaJanela.last?.fim ?? palavras.last?.fim,
                                                                 reconhecedor: reconhecedor) {
                    palavrasDaJanela.removeAll { $0.inicio >= recuperada.limite }
                    palavrasDaJanela += recuperada.palavras.filter { janela.contains($0.centro) }
                }
            }
            let temAtividade = AtividadeDeVoz.tem(dados[faixa])
            let estado: EstadoBloco = palavrasDaJanela.isEmpty ? (temAtividade ? .pendente : .semFala) : .transcrito
            guard try armazem.confirmarJanela(versao: versao.id, seq: seq, inicio: inicioJanela, fim: fim,
                                            soma: DiscoDeBlocos.soma(pcmJanela), estado: estado) else { return .superada }
            if estado == .pendente {
                throw ErroDeBanco(description: "Uma janela contém fala, mas o reconhecedor não devolveu texto. Tente transcrever novamente.")
            }
            palavras += palavrasDaJanela
            await progresso("transcrevendo", Double(fim) / Double(max(1, total)))
        }
        let identidade = hash.finalize().map { String(format: "%02x", $0) }.joined()
        var faixas: [FaixaDeVoz] = []
        var baixa = false
        if let identificador, let wav, !palavras.isEmpty {
            await progresso("diarizando", nil)
            faixas = try await identificador.faixas(wav: wav)
            await progresso("diarizando", 1)
        }
        try Task.checkCancellation()
        await progresso("limpando", nil)
        var trechos: [TrechoNovo] = []
        for (n, grupo) in Self.agrupar(palavras, faixas: faixas, comFalantes: identificador != nil).enumerated() {
            try Task.checkCancellation()
            let texto = await preparar(grupo.texto)
            guard !texto.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { continue }
            if grupo.voz == "desconhecida" && identificador != nil { baixa = true }
            trechos.append(TrechoNovo(id: "\(versao.documento)|\(versao.id)|\(n)", fonte: .arquivo, inicio: grupo.inicio, fim: grupo.fim,
                                      texto: texto, voz: grupo.voz))
        }
        try conferirEntrada()
        try Task.checkCancellation()
        let publicada = try armazem.publicar(.init(versao: versao.id, identidade: identidade, trechos: trechos, duracao: Double(total) / taxa,
                                                   bruto: palavras.map(\.texto).joined(separator: " "),
                                                   processamento: Date().timeIntervalSince(inicio), baixaConfianca: baixa, quantidadePCM: total))
        if publicada { await progresso("limpando", 1); return .publicada }
        return trechos.isEmpty ? .semFala : .superada
    }

    /// Recupera a última fala quando o reconhecimento da janela a deixou de fora.
    static func recuperarCauda(_ audio: [Float], base: Double, ultimoFim: Double?,
                               reconhecedor: ReconhecedorDeFala) async throws -> (limite: Double, palavras: [PalavraReconhecida])? {
        guard let cauda = CaudaDaFala.localizar(audio) else { return nil }
        let taxa = Double(taxaLancamento)
        let inicio = base + Double(cauda.inicio) / taxa
        let fim = base + Double(cauda.fim) / taxa
        let corteAnterior = ultimoFim ?? base
        guard fim - corteAnterior >= 0.28 else { return nil }
        let ate = min(audio.count, cauda.fim + 3_200)
        for margemAntes in [1_600, 16_000, audio.count] {
            let de = max(0, cauda.inicio - margemAntes)
            do {
                let palavras = try await reconhecedor.palavras(Array(audio[de..<ate]), inicio: base + Double(de) / taxa,
                                                              fimDaFala: Double(min(ate, cauda.fim + 1_280) - de) / taxa)
                let uteis = palavras.filter { $0.centro >= inicio - 0.10 }
                if let ultima = uteis.last, ultima.fim > corteAnterior {
                    return (inicio - 0.10, uteis)
                }
            } catch {
                try Task.checkCancellation()
                return nil
            }
            if de == 0 { break }
        }
        return nil
    }

    /// Agrupa palavras por voz, pausa, duração e pontuação final.
    static func agrupar(_ palavras: [PalavraReconhecida], faixas: [FaixaDeVoz], comFalantes: Bool)
        -> [(inicio: Double, fim: Double, voz: String, texto: String)] {
        var saida: [(inicio: Double, fim: Double, voz: String, texto: String)] = []
        let ordenadas = faixas.sorted { $0.inicio < $1.inicio }
        var primeiro = 0
        for palavra in palavras {
            while primeiro < ordenadas.count, ordenadas[primeiro].fim <= palavra.inicio { primeiro += 1 }
            let candidatas = Array(ordenadas[primeiro...].prefix { $0.inicio < palavra.fim })
            let voz = comFalantes ? AlinhamentoDeVoz.identificar(inicio: palavra.inicio, fim: palavra.fim, faixas: candidatas) : "desconhecida"
            if var ultimo = saida.last, ultimo.voz == voz, palavra.inicio - ultimo.fim < 1.2, palavra.fim - ultimo.inicio <= 15,
               !ultimo.texto.hasSuffix("."), !ultimo.texto.hasSuffix("?"), !ultimo.texto.hasSuffix("!") {
                ultimo.texto += " " + palavra.texto
                ultimo.fim = max(ultimo.fim, palavra.fim)
                saida[saida.count - 1] = ultimo
            } else {
                saida.append((palavra.inicio, palavra.fim, voz, palavra.texto))
            }
        }
        return saida
    }
}
