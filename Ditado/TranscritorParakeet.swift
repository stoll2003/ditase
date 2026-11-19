import FluidAudio
import Foundation
import Nucleo

/// Reconhecedor de fala local: Parakeet Ultra pelo FluidAudio, rodando no Neural Engine.
///
/// Escolhido por medição na voz do usuário (bench/): mesma precisão do Whisper large-v3-turbo
/// e cerca de sete vezes mais rápido. O modelo (uns 600 MB) é baixado uma vez.
actor TranscritorParakeet: Transcritor {
    enum Estado: Equatable {
        case parado, carregando, pronto
        case falhou(String)
    }

    nonisolated let nome = "parakeet-ultra"
    private var gerenciador: AsrManager?
    private var carga: Task<AsrManager, Error>?
    private(set) var estado: Estado = .parado
    private var aoMudar: (@Sendable (Estado) -> Void)?

    func observar(_ bloco: @escaping @Sendable (Estado) -> Void) {
        aoMudar = bloco
        bloco(estado)
    }

    private func mudar(_ novo: Estado) {
        estado = novo
        aoMudar?(novo)
    }

    /// Baixa (na primeira vez) e carrega o modelo. Chamar de novo depois de uma falha tenta outra vez.
    @discardableResult
    func carregar() async throws -> AsrManager {
        if let gerenciador { return gerenciador }
        if let carga { return try await carga.value }
        mudar(.carregando)
        let tarefa = Task { () -> AsrManager in
            let modelos = try await AsrModels.downloadAndLoad(version: .ultra)
            let g = AsrManager(config: .default)
            try await g.loadModels(modelos)
            return g
        }
        carga = tarefa
        do {
            let g = try await tarefa.value
            gerenciador = g
            carga = nil
            mudar(.pronto)
            return g
        } catch {
            carga = nil
            mudar(.falhou(String(describing: error)))
            throw error
        }
    }

    func transcrever(_ amostras: [Float]) async throws -> String {
        let g = try await carregar()
        // O modelo exige pelo menos um segundo de áudio; trecho curto é completado com silêncio.
        var entrada = amostras
        if entrada.count < 17_600 { entrada += [Float](repeating: 0, count: 17_600 - entrada.count) }
        var estadoDoDecodificador = TdtDecoderState.make(decoderLayers: await g.decoderLayerCount)
        let resultado = try await g.transcribe(entrada, decoderState: &estadoDoDecodificador, language: .portuguese)
        return resultado.text
    }
}
