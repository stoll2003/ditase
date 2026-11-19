import Nucleo
import AVFoundation
import CoreAudio
import Foundation

struct Microfone: Equatable {
    let uid: String
    let nome: String
    let id: AudioDeviceID
}

/// Captura o microfone e entrega blocos a 16 kHz mono, na ordem, para o reconhecedor.
/// Também grava o áudio em WAV, aos poucos, para o ditado não se perder se algo falhar.
final class Gravador {
    enum Erro: Error, CustomStringConvertible {
        case semPermissao, semEntrada, formato
        var description: String {
            switch self {
            case .semPermissao: return "sem permissão de microfone"
            case .semEntrada: return "nenhum microfone disponível"
            case .formato: return "formato de áudio não suportado"
            }
        }
    }

    static let formatoAlvo = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false)!

    private var motor: AVAudioEngine?
    private var entradaInstalada: AVAudioInputNode?
    private var conversor: AVAudioConverter?
    private var arquivo: AVAudioFile?
    private var continuacao: AsyncStream<[Float]>.Continuation?
    private(set) var distribuidorDaReuniao: DistribuidorDoMicrofone?
    private var continuacaoComTempo: AsyncStream<AmostrasComTempo>.Continuation?
    private var observadorConfiguracao: NSObjectProtocol?
    var aoMudarConfiguracao: (() -> Void)?
    var aoEstourarFila: (() -> Void)?
    private let fila = DispatchQueue(label: "ditado.gravador")
    /// Nível do último bloco, de 0 a 1. Chamado fora da thread principal.
    var aoNivel: ((Float) -> Void)?

    var gravando: Bool { motor?.isRunning ?? false }

    /// Captura para a reunião: cada bloco leva o instante do host em que foi capturado.
    func iniciarComTempo(microfone uid: String, limiteDeBuffers: Int) throws -> AsyncStream<AmostrasComTempo> {
        let (fluxo, entrada) = AsyncStream<AmostrasComTempo>.makeStream(bufferingPolicy: .bufferingOldest(limiteDeBuffers))
        continuacaoComTempo = entrada
        let aoEstourar = aoEstourarFila
        distribuidorDaReuniao = DistribuidorDoMicrofone { dados in
            if case .dropped = entrada.yield(AmostrasComTempo(tempo: dados.tempo, amostras: dados.amostras)) {
                entrada.finish()
                aoEstourar?()
            }
        }
        do { _ = try iniciar(microfone: uid, gravarEm: nil, limiteDeBuffers: limiteDeBuffers) }
        catch {
            continuacaoComTempo = nil
            distribuidorDaReuniao = nil
            entrada.finish()
            throw error
        }
        return fluxo
    }

    /// Começa a captura. O fluxo devolvido termina quando `parar` é chamado.
    func iniciar(microfone uid: String, gravarEm url: URL?, limiteDeBuffers: Int? = nil) throws -> AsyncStream<[Float]> {
        guard AVCaptureDevice.authorizationStatus(for: .audio) == .authorized else { throw Erro.semPermissao }
        let motor = AVAudioEngine()
        let entrada = motor.inputNode
        if !uid.isEmpty, let dispositivo = Self.microfones().first(where: { $0.uid == uid }), let unidade = entrada.audioUnit {
            var id = dispositivo.id
            AudioUnitSetProperty(unidade, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0, &id, UInt32(MemoryLayout<AudioDeviceID>.size))
        }
        let formatoDeEntrada = entrada.outputFormat(forBus: 0)
        guard formatoDeEntrada.sampleRate > 0, formatoDeEntrada.channelCount > 0 else { throw Erro.semEntrada }
        guard let conversor = AVAudioConverter(from: formatoDeEntrada, to: Self.formatoAlvo) else { throw Erro.formato }
        self.conversor = conversor
        if let url {
            let ajustes: [String: Any] = [
                AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: 16_000, AVNumberOfChannelsKey: 1,
                AVLinearPCMBitDepthKey: 16, AVLinearPCMIsFloatKey: false, AVLinearPCMIsBigEndianKey: false,
            ]
            arquivo = try? AVAudioFile(forWriting: url, settings: ajustes, commonFormat: .pcmFormatFloat32, interleaved: false)
        }
        let politica = limiteDeBuffers.map { AsyncStream<[Float]>.Continuation.BufferingPolicy.bufferingOldest($0) } ?? .unbounded
        let (fluxo, continuacao) = AsyncStream<[Float]>.makeStream(bufferingPolicy: politica)
        self.continuacao = continuacao

        entrada.installTap(onBus: 0, bufferSize: 2048, format: formatoDeEntrada) { [weak self] buffer, quando in
            self?.converter(buffer, tempo: quando.isHostTimeValid ? AVAudioTime.seconds(forHostTime: quando.hostTime) : Relogio.agora())
        }
        motor.prepare()
        do { try motor.start() } catch {
            entrada.removeTap(onBus: 0)
            continuacao.finish()
            self.continuacao = nil
            throw error
        }
        self.motor = motor
        entradaInstalada = entrada
        observarConfiguracao(motor)
        return fluxo
    }

    func observarConfiguracao(_ motor: AVAudioEngine) {
        retirarObservadorConfiguracao()
        observadorConfiguracao = NotificationCenter.default.addObserver(
            forName: .AVAudioEngineConfigurationChange, object: motor, queue: .main
        ) { [weak self] _ in self?.aoMudarConfiguracao?() }
    }

    private func retirarObservadorConfiguracao() {
        if let observadorConfiguracao { NotificationCenter.default.removeObserver(observadorConfiguracao) }
        observadorConfiguracao = nil
    }

    deinit { retirarObservadorConfiguracao() }

    private func converter(_ buffer: AVAudioPCMBuffer, tempo: Double) {
        guard let conversor else { return }
        let razao = Self.formatoAlvo.sampleRate / buffer.format.sampleRate
        let capacidade = AVAudioFrameCount(Double(buffer.frameLength) * razao) + 32
        guard let saida = AVAudioPCMBuffer(pcmFormat: Self.formatoAlvo, frameCapacity: capacidade) else { return }
        var entregue = false
        var erro: NSError?
        conversor.convert(to: saida, error: &erro) { _, estado in
            if entregue {
                estado.pointee = .noDataNow
                return nil
            }
            entregue = true
            estado.pointee = .haveData
            return buffer
        }
        guard erro == nil, saida.frameLength > 0, let canal = saida.floatChannelData?[0] else { return }
        let amostras = Array(UnsafeBufferPointer(start: canal, count: Int(saida.frameLength)))
        var soma: Float = 0
        for a in amostras { soma += a * a }
        aoNivel?(min(1, (soma / Float(amostras.count)).squareRoot() * 14))
        // Um único produtor (a thread de áudio): a ordem dos blocos é a ordem de chegada.
        if let distribuidorDaReuniao {
            distribuidorDaReuniao.entregar(tempo: tempo, amostras: amostras)
        } else if case .dropped? = continuacao?.yield(amostras) {
            continuacao?.finish()
            aoEstourarFila?()
        }
        fila.async { [weak self] in try? self?.arquivo?.write(from: saida) }
    }

    func parar() {
        guard let motor else { return }
        retirarObservadorConfiguracao()
        entradaInstalada?.removeTap(onBus: 0)
        entradaInstalada = nil
        motor.stop()
        self.motor = nil
        conversor = nil
        continuacao?.finish()
        continuacao = nil
        continuacaoComTempo?.finish()
        continuacaoComTempo = nil
        distribuidorDaReuniao = nil
        fila.sync { arquivo = nil }
    }

    // MARK: - Dispositivos

    static func pedirPermissao(_ resposta: @escaping (Bool) -> Void) {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: resposta(true)
        case .notDetermined: AVCaptureDevice.requestAccess(for: .audio) { ok in DispatchQueue.main.async { resposta(ok) } }
        default: resposta(false)
        }
    }

    static var permissao: String {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: return "concedida"
        case .notDetermined: return "pendente"
        default: return "negada"
        }
    }

    static func microfones() -> [Microfone] {
        var endereco = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDevices, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        var tamanho: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(AudioObjectID(kAudioObjectSystemObject), &endereco, 0, nil, &tamanho) == noErr else { return [] }
        var ids = [AudioDeviceID](repeating: 0, count: Int(tamanho) / MemoryLayout<AudioDeviceID>.size)
        guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &endereco, 0, nil, &tamanho, &ids) == noErr else { return [] }
        return ids.compactMap { id in
            var canais = AudioObjectPropertyAddress(mSelector: kAudioDevicePropertyStreamConfiguration, mScope: kAudioDevicePropertyScopeInput, mElement: kAudioObjectPropertyElementMain)
            var t: UInt32 = 0
            guard AudioObjectGetPropertyDataSize(id, &canais, 0, nil, &t) == noErr, t > 0 else { return nil }
            let lista = UnsafeMutableRawPointer.allocate(byteCount: Int(t), alignment: MemoryLayout<AudioBufferList>.alignment)
            defer { lista.deallocate() }
            guard AudioObjectGetPropertyData(id, &canais, 0, nil, &t, lista) == noErr else { return nil }
            let buffers = UnsafeMutableAudioBufferListPointer(lista.assumingMemoryBound(to: AudioBufferList.self))
            guard buffers.reduce(0, { $0 + Int($1.mNumberChannels) }) > 0 else { return nil }
            guard let uid = texto(id, kAudioDevicePropertyDeviceUID), let nome = texto(id, kAudioObjectPropertyName) else { return nil }
            return Microfone(uid: uid, nome: nome, id: id)
        }
    }

    private static func texto(_ id: AudioDeviceID, _ seletor: AudioObjectPropertySelector) -> String? {
        var endereco = AudioObjectPropertyAddress(mSelector: seletor, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        var valor: Unmanaged<CFString>?
        var tamanho = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
        guard AudioObjectGetPropertyData(id, &endereco, 0, nil, &tamanho, &valor) == noErr, let valor else { return nil }
        return valor.takeRetainedValue() as String
    }
}
