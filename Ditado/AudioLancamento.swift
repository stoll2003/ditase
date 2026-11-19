import AVFoundation
import CoreMedia
import Foundation
import Nucleo

enum ErroLancamento: LocalizedError {
    case formato, semFala, ocupado, parametro(String), captura(String), processamento(String)

    var errorDescription: String? {
        switch self {
        case .formato: return "O macOS não conseguiu decodificar a faixa de áudio deste arquivo."
        case .semFala: return "Nenhuma fala foi reconhecida."
        case .ocupado: return "Encerre o trabalho atual antes de iniciar outro."
        case .parametro(let campo): return "Parâmetro inválido: \(campo)."
        case .captura(let detalhe), .processamento(let detalhe): return detalhe
        }
    }
}

struct ReservaDeAudio {
    let pasta: URL

    init() throws {
        pasta = FileManager.default.temporaryDirectory.appendingPathComponent("ditase-\(getpid())-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: pasta, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
    }

    func remover() { try? FileManager.default.removeItem(at: pasta) }

    static func removerOrfas() {
        let fm = FileManager.default
        let chaves: [URLResourceKey] = [.isDirectoryKey, .isSymbolicLinkKey, .contentModificationDateKey]
        guard let itens = try? fm.contentsOfDirectory(at: fm.temporaryDirectory, includingPropertiesForKeys: chaves)
        else { return }
        let agora = Date()
        for item in itens {
            let nome = item.lastPathComponent
            guard let pid = RetencaoDeReuniao.processoDaReserva(nome: nome),
                  let valores = try? item.resourceValues(forKeys: Set(chaves)),
                  valores.isDirectory == true, valores.isSymbolicLink == false,
                  let alteracao = valores.contentModificationDate,
                  RetencaoDeReuniao.reservaVencida(nome: nome, alteracao: alteracao, agora: agora),
                  kill(pid, 0) != 0, errno == ESRCH else { continue }
            try? fm.removeItem(at: item)
        }
    }

    func arquivo(_ nome: String) -> URL { pasta.appendingPathComponent(nome + ".wav") }
}

final class EscritorLancamento {
    private let arquivo: AVAudioFile
    private(set) var amostras: Int = 0

    init(_ url: URL) throws {
        let formato = AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: 16_000, channels: 1, interleaved: true)!
        arquivo = try AVAudioFile(forWriting: url, settings: formato.settings, commonFormat: .pcmFormatFloat32, interleaved: false)
    }

    func gravar(_ dados: [Float]) throws {
        guard !dados.isEmpty else { return }
        guard let buffer = AVAudioPCMBuffer(pcmFormat: Gravador.formatoAlvo, frameCapacity: AVAudioFrameCount(dados.count)),
              let canal = buffer.floatChannelData?[0] else { throw ErroLancamento.formato }
        buffer.frameLength = AVAudioFrameCount(dados.count)
        dados.withUnsafeBufferPointer { canal.update(from: $0.baseAddress!, count: dados.count) }
        try arquivo.write(from: buffer)
        amostras += dados.count
    }
}

/// Uma instância por faixa, para preservar o estado do reamostrador entre buffers.
final class ConversorLancamento {
    private var conversor: AVAudioConverter?

    func converter(_ amostra: CMSampleBuffer) throws -> [Float] {
        guard let descricao = amostra.formatDescription else { throw ErroLancamento.formato }
        let formato = AVAudioFormat(cmAudioFormatDescription: descricao)
        guard let pcm = AVAudioPCMBuffer(pcmFormat: formato, frameCapacity: AVAudioFrameCount(amostra.numSamples))
        else { throw ErroLancamento.formato }
        pcm.frameLength = pcm.frameCapacity
        let status = CMSampleBufferCopyPCMDataIntoAudioBufferList(amostra, at: 0, frameCount: Int32(amostra.numSamples),
                                                                  into: pcm.mutableAudioBufferList)
        guard status == noErr else { throw ErroLancamento.formato }
        return try converter(pcm)
    }

    func converter(_ pcm: AVAudioPCMBuffer) throws -> [Float] {
        if pcm.format == Gravador.formatoAlvo { return Self.copiar(pcm) }
        if conversor?.inputFormat != pcm.format {
            conversor = AVAudioConverter(from: pcm.format, to: Gravador.formatoAlvo)
            conversor?.primeMethod = .none
        }
        guard let conversor, let saida = AVAudioPCMBuffer(pcmFormat: Gravador.formatoAlvo,
            frameCapacity: AVAudioFrameCount(ceil(Double(pcm.frameLength) * 16_000 / pcm.format.sampleRate)) + 64)
        else { throw ErroLancamento.formato }
        var entregue = false
        var erro: NSError?
        let status = conversor.convert(to: saida, error: &erro) { _, estado in
            estado.pointee = entregue ? .noDataNow : .haveData
            defer { entregue = true }
            return entregue ? nil : pcm
        }
        guard status != .error else { throw erro ?? ErroLancamento.formato as NSError }
        return Self.copiar(saida)
    }

    static func copiar(_ pcm: AVAudioPCMBuffer) -> [Float] {
        guard let canal = pcm.floatChannelData?[0] else { return [] }
        return Array(UnsafeBufferPointer(start: canal, count: Int(pcm.frameLength)))
    }
}

enum AudioLancamento {
    static let taxa = 16_000.0

    nonisolated static func decodificar(_ origem: URL, para destino: URL,
                                       progresso: @Sendable (Double) async -> Void) async throws {
        guard origem.isFileURL else { throw ErroLancamento.formato }
        let recipientesDeVideo: Set<String> = ["mp4", "mov", "m4v", "webm"]
        if !recipientesDeVideo.contains(origem.pathExtension.lowercased()),
           let audio = try? AVAudioFile(forReading: origem), audio.length > 0 {
            try await decodificarAudio(audio, destino: destino, progresso: progresso)
            return
        }
        let asset = AVURLAsset(url: origem)
        let faixa: AVAssetTrack
        do {
            guard let primeira = try await asset.loadTracks(withMediaType: .audio).first else { throw ErroLancamento.formato }
            faixa = primeira
        } catch { throw ErroLancamento.formato }
        let duracao = try? await faixa.load(.timeRange).duration.seconds
        let leitor = try AVAssetReader(asset: asset)
        let saida = AVAssetReaderTrackOutput(track: faixa, outputSettings: [
            AVFormatIDKey: kAudioFormatLinearPCM, AVLinearPCMBitDepthKey: 32,
            AVLinearPCMIsFloatKey: true, AVLinearPCMIsBigEndianKey: false, AVLinearPCMIsNonInterleaved: false
        ])
        guard leitor.canAdd(saida) else { throw ErroLancamento.formato }
        leitor.add(saida)
        guard leitor.startReading() else { throw ErroLancamento.formato }
        defer { if leitor.status == .reading { leitor.cancelReading() } }
        let escritor = try EscritorLancamento(destino)
        let conversor = ConversorLancamento()
        var ultimo = 0.0
        while let buffer = saida.copyNextSampleBuffer() {
            try Task.checkCancellation()
            try escritor.gravar(conversor.converter(buffer))
            let segundos = Double(escritor.amostras) / taxa
            if let duracao, duracao > 0, segundos - ultimo >= 0.25 {
                ultimo = segundos
                await progresso(min(1, segundos / duracao))
            }
        }
        guard leitor.status == .completed, escritor.amostras > 0 else { throw ErroLancamento.formato }
        await progresso(1)
    }

    private static func decodificarAudio(_ origem: AVAudioFile, destino: URL,
                                         progresso: @Sendable (Double) async -> Void) async throws {
        let escritor = try EscritorLancamento(destino)
        let conversor = ConversorLancamento()
        let total = origem.length
        let passo = AVAudioFrameCount(max(1, origem.processingFormat.sampleRate))
        guard let pcm = AVAudioPCMBuffer(pcmFormat: origem.processingFormat, frameCapacity: passo)
        else { throw ErroLancamento.formato }
        var lidos: AVAudioFramePosition = 0
        while lidos < total {
            try Task.checkCancellation()
            do { try origem.read(into: pcm) } catch { throw ErroLancamento.formato }
            if pcm.frameLength == 0 { break }
            lidos += AVAudioFramePosition(pcm.frameLength)
            try escritor.gravar(conversor.converter(pcm))
            await progresso(min(1, Double(lidos) / Double(total)))
        }
        guard escritor.amostras > 0 else { throw ErroLancamento.formato }
        await progresso(1)
    }

}

/// Cada janela abre um leitor próprio, com acesso independente dos demais consumidores.
struct FonteDeWAVLancamento: FonteDePCM {
    let url: URL
    let quantidade: Int

    init(_ url: URL) throws {
        self.url = url
        quantidade = try LeitorLancamento(url).quantidade
    }

    func ler(inicio: Int, quantidade: Int) throws -> [Float] {
        try LeitorLancamento(url).ler(inicio: inicio, quantidade: quantidade)
    }
}

final class LeitorLancamento {
    private let arquivo: AVAudioFile
    var quantidade: Int { Int(arquivo.length) }

    init(_ url: URL) throws { arquivo = try AVAudioFile(forReading: url) }

    func ler(inicio: Int? = nil, quantidade: Int) throws -> [Float] {
        if let inicio { arquivo.framePosition = AVAudioFramePosition(inicio) }
        let n = min(quantidade, Int(arquivo.length - arquivo.framePosition))
        guard n > 0 else { return [] }
        guard let pcm = AVAudioPCMBuffer(pcmFormat: arquivo.processingFormat,
                                         frameCapacity: AVAudioFrameCount(n)) else { throw ErroLancamento.formato }
        try arquivo.read(into: pcm)
        return ConversorLancamento.copiar(pcm)
    }
}
