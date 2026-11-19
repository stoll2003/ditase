import Foundation

/// Uma captura de microfone, dois destinos com filas próprias: a reunião recebe tudo uma vez;
/// o atalho de ditado recebe uma derivação com começo e fim próprios. Estourar a fila do ditado
/// encerra só a derivação; a reunião continua.
public final class DistribuidorDoMicrofone: @unchecked Sendable {
    public typealias Amostras = (tempo: Double, amostras: [Float])
    private let trava = NSLock()
    private var derivacao: AsyncStream<[Float]>.Continuation?
    private let paraReuniao: @Sendable (Amostras) -> Void
    public private(set) var estourosDaDerivacao = 0

    public init(paraReuniao: @escaping @Sendable (Amostras) -> Void) { self.paraReuniao = paraReuniao }

    public func abrirDerivacao(limite: Int = 512) -> AsyncStream<[Float]> {
        let (fluxo, entrada) = AsyncStream<[Float]>.makeStream(bufferingPolicy: .bufferingOldest(limite))
        trava.lock()
        derivacao?.finish()
        derivacao = entrada
        trava.unlock()
        return fluxo
    }

    public func fecharDerivacao() {
        trava.lock()
        derivacao?.finish()
        derivacao = nil
        trava.unlock()
    }

    public var derivacaoAberta: Bool {
        trava.lock(); defer { trava.unlock() }
        return derivacao != nil
    }

    public func entregar(tempo: Double, amostras: [Float]) {
        paraReuniao((tempo, amostras))
        trava.lock()
        let destino = derivacao
        trava.unlock()
        if case .dropped = destino?.yield(amostras) {
            trava.lock()
            estourosDaDerivacao += 1
            derivacao?.finish()
            derivacao = nil
            trava.unlock()
        }
    }
}
