import Foundation

/// Responde ao sistema uma única vez: quando o trabalho de saída termina ou quando o prazo vence,
/// o que vier primeiro. Um só relógio, iniciado junto com o trabalho, sem esperas encadeadas.
/// O trabalho precisa liberar o ator principal (escritas em disco ficam fora dele) para o prazo valer.
@MainActor
public final class SaidaComPrazo {
    public private(set) var emCurso = false
    private var responder: (@MainActor () -> Void)?

    public init() {}

    /// Devolve falso se uma saída já está em curso; nesse caso nada novo é iniciado.
    @discardableResult
    public func iniciar(prazo: Duration = .seconds(2), trabalho: @escaping @MainActor () async -> Void,
                        responder: @escaping @MainActor () -> Void) -> Bool {
        guard !emCurso else { return false }
        emCurso = true
        self.responder = responder
        let relogio = Task { @MainActor [weak self] in
            do { try await Task.sleep(for: prazo) } catch { return }
            self?.responderUmaVez()
        }
        Task { @MainActor [weak self] in
            await trabalho()
            relogio.cancel()
            self?.responderUmaVez()
        }
        return true
    }

    private func responderUmaVez() {
        guard emCurso, let responder else { return }
        emCurso = false
        self.responder = nil
        responder()
    }
}
