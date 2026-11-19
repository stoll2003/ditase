import AVFoundation
import CoreMedia
import Foundation
import Nucleo
import ScreenCaptureKit

/// Relógio monotônico do host, em segundos: o mesmo das amostras do ScreenCaptureKit e do AVAudioEngine.
enum Relogio {
    static func agora() -> Double { CMClockGetTime(CMClockGetHostTimeClock()).seconds }
}

/// Amostras de 16 kHz mono com o instante de captura da primeira amostra.
struct AmostrasComTempo: Sendable {
    let tempo: Double
    let amostras: [Float]
}

/// Registra só a saída de áudio. O filtro precisa de uma tela, mas não há saída
/// de vídeo registrada e nenhum quadro é lido pela aplicação.
final class CapturaReuniao: NSObject, SCStreamOutput, SCStreamDelegate {
    private var stream: SCStream?
    private let fila = DispatchQueue(label: "ditado.reuniao.sistema")
    private let conversor = ConversorLancamento()
    let amostras: AsyncThrowingStream<AmostrasComTempo, Error>
    private let emissor: AsyncThrowingStream<AmostrasComTempo, Error>.Continuation

    override init() {
        (amostras, emissor) = AsyncThrowingStream.makeStream(bufferingPolicy: .bufferingOldest(512))
        super.init()
    }

    func iniciar() async throws {
        let conteudo = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
        guard let tela = conteudo.displays.first else {
            throw ErroLancamento.captura("Nenhuma tela disponível para iniciar a captura de áudio do Mac.")
        }
        let filtro = SCContentFilter(display: tela, excludingWindows: [])
        let config = SCStreamConfiguration()
        config.capturesAudio = true
        config.excludesCurrentProcessAudio = true
        config.sampleRate = 48_000
        config.channelCount = 1
        config.width = 2
        config.height = 2
        config.minimumFrameInterval = CMTime(seconds: 1, preferredTimescale: 60)
        let novo = SCStream(filter: filtro, configuration: config, delegate: self)
        try novo.addStreamOutput(self, type: .audio, sampleHandlerQueue: fila)
        try await novo.startCapture()
        stream = novo
    }

    func parar() async {
        guard let stream else {
            emissor.finish()
            return
        }
        self.stream = nil
        try? await stream.stopCapture()
        await withCheckedContinuation { (parou: CheckedContinuation<Void, Never>) in
            fila.async { [emissor] in
                emissor.finish()
                parou.resume()
            }
        }
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .audio, sampleBuffer.isValid, sampleBuffer.numSamples > 0 else { return }
        do {
            let pts = sampleBuffer.presentationTimeStamp
            let tempo = pts.isValid ? pts.seconds : Relogio.agora()
            if case .dropped = emissor.yield(AmostrasComTempo(tempo: tempo, amostras: try conversor.converter(sampleBuffer))) {
                emissor.finish(throwing: ErroLancamento.captura("A fila do áudio do Mac ficou cheia. A fala após o atraso pode estar incompleta."))
            }
        } catch { emissor.finish(throwing: error) }
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) { emissor.finish(throwing: error) }
}
