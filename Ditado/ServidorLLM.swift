import Darwin
import Foundation
import Nucleo

/// Mantém um llama-server local com o modelo de limpeza (Qwen) carregado.
///
/// O servidor sobe quando um ditado começa, fica quente entre ditados e é desligado
/// depois de um tempo parado, para devolver a memória. Só escuta em 127.0.0.1.
final class ServidorLLM: @unchecked Sendable {
    enum Estado: Equatable {
        case desligado
        case iniciando
        case pronto
        /// Não há llama-server instalado.
        case semBinario
        /// O arquivo do modelo não está na pasta de modelos.
        case semModelo(String)
        case falhou(String)
    }

    static let candidatos = ["/opt/homebrew/bin/llama-server", "/usr/local/bin/llama-server"]
    /// Minutos parado até desligar o servidor e liberar a memória do modelo.
    /// Curto de propósito: o modelo ocupa mais de 1 GB e recarrega em 2 a 3 s, enquanto a pessoa
    /// ainda está falando o começo do ditado seguinte.
    static let ociosoMinutos = 3.0

    private let trava = NSLock()
    private var processo: Process?
    private var porta = 0
    private var modeloAtual = ""
    private var _estado: Estado = .desligado
    private var subida: Task<Bool, Never>?
    private var desligamento: DispatchWorkItem?
    private let instrucoes: String
    var aoMudar: (@Sendable (Estado) -> Void)?

    /// Verdadeiro nos modos de linha de comando e de captura: eles não são o app em uso e por isso
    /// não podem derrubar o servidor dele nem registrar o próprio como "o" servidor.
    static var utilitario = false

    init(instrucoes: String) {
        self.instrucoes = instrucoes
        if !Self.utilitario { matarOrfao() }
    }

    var estado: Estado { trava.withLock { _estado } }

    private func mudar(_ novo: Estado) {
        trava.withLock { _estado = novo }
        aoMudar?(novo)
    }

    static var binario: String? {
        let embutido = Bundle.main.resourceURL?.appendingPathComponent("llama/llama-server").path
        return ([embutido].compactMap { $0 } + candidatos).first { FileManager.default.isExecutableFile(atPath: $0) }
    }

    /// Limpador que espera o servidor ficar pronto antes de cada pedido.
    func limpador(modelo: String) -> Limpador { LimpadorSobDemanda(servidor: self, modelo: modelo, instrucoes: instrucoes) }

    /// Garante o servidor no ar com o modelo pedido. Devolve falso se não for possível.
    @discardableResult
    func garantir(modelo: String) async -> Bool {
        adiarDesligamento()
        let (jaPronto, emCurso): (Bool, Task<Bool, Never>?) = trava.withLock {
            if _estado == .pronto, modeloAtual == modelo, processo?.isRunning == true { return (true, nil) }
            if modeloAtual == modelo, let subida { return (false, subida) }
            return (false, nil)
        }
        if jaPronto { return true }
        if let emCurso { return await emCurso.value }
        let tarefa = Task { await self.subir(modelo: modelo) }
        trava.withLock {
            subida = tarefa
            modeloAtual = modelo
        }
        let ok = await tarefa.value
        trava.withLock { if subida == tarefa { subida = nil } }
        return ok
    }

    var portaAtual: Int { trava.withLock { porta } }

    private func subir(modelo: String) async -> Bool {
        pararProcesso()
        guard let binario = Self.binario else {
            mudar(.semBinario)
            return false
        }
        let arquivo = Caminhos.modelos.appendingPathComponent(modelo)
        // Modelos do catálogo só sobem depois da conferência.
        let conferido = ModeloRemoto.qwen(arquivo: modelo).map {
            BaixadorDeModelo(modelo: $0, pasta: Caminhos.modelos).instalado
        } ?? true
        guard conferido, FileManager.default.fileExists(atPath: arquivo.path) else {
            mudar(.semModelo(modelo))
            return false
        }
        mudar(.iniciando)
        let novaPorta = Self.portaLivre()
        let p = Process()
        p.executableURL = URL(fileURLWithPath: binario)
        p.arguments = [
            "-m", arquivo.path, "--host", "127.0.0.1", "--port", String(novaPorta),
            "-c", "8192", "-ngl", "99", "-np", "1", "--jinja", "--reasoning-budget", "0", "--no-slots",
        ]
        p.standardOutput = FileHandle.nullDevice
        p.standardError = FileHandle.nullDevice
        do { try p.run() } catch {
            mudar(.falhou("não iniciou: \(error.localizedDescription)"))
            return false
        }
        trava.withLock {
            processo = p
            porta = novaPorta
        }
        if !Self.utilitario { try? String(p.processIdentifier).write(to: Caminhos.pidDoServidor, atomically: true, encoding: .utf8) }
        Registro.nota("llama-server pid \(p.processIdentifier) porta \(novaPorta) modelo \(modelo)")

        let saude = URL(string: "http://127.0.0.1:\(novaPorta)/health")!
        let prazo = Date().addingTimeInterval(90)
        while Date() < prazo {
            guard p.isRunning else {
                mudar(.falhou("o servidor encerrou ao carregar o modelo"))
                return false
            }
            if let (_, r) = try? await URLSession.shared.data(from: saude), (r as? HTTPURLResponse)?.statusCode == 200 {
                // Primeira chamada deixa as instruções em cache: os ditados só pagam o texto novo.
                _ = try? await LimpadorLlama(porta: novaPorta, modelo: modelo, instrucoes: instrucoes).limpar(PedidoLimpeza(transcricao: "teste de aquecimento do modelo"))
                mudar(.pronto)
                return true
            }
            try? await Task.sleep(nanoseconds: 250_000_000)
        }
        pararProcesso()
        mudar(.falhou("o modelo demorou demais para carregar"))
        return false
    }

    func parar() {
        trava.withLock { desligamento }?.cancel()
        pararProcesso()
        mudar(.desligado)
    }

    private func pararProcesso() {
        let p: Process? = trava.withLock {
            defer { processo = nil }
            return processo
        }
        if let p, p.isRunning {
            p.terminate()
            DispatchQueue.global().asyncAfter(deadline: .now() + 2) { if p.isRunning { kill(p.processIdentifier, SIGKILL) } }
        }
        if !Self.utilitario { try? FileManager.default.removeItem(at: Caminhos.pidDoServidor) }
    }

    private func adiarDesligamento() {
        let item = DispatchWorkItem { [weak self] in
            Registro.nota("llama-server parado por ociosidade")
            self?.parar()
        }
        // Chamado de várias threads (cada pedido de limpeza renova o prazo).
        let anterior: DispatchWorkItem? = trava.withLock {
            defer { desligamento = item }
            return desligamento
        }
        anterior?.cancel()
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.ociosoMinutos * 60, execute: item)
    }

    /// Servidor que sobrou de uma execução anterior que não encerrou direito.
    private func matarOrfao() {
        guard let texto = try? String(contentsOf: Caminhos.pidDoServidor, encoding: .utf8), let pid = Int32(texto.trimmingCharacters(in: .whitespacesAndNewlines)) else { return }
        var nome = [CChar](repeating: 0, count: 4096)
        if proc_pidpath(pid, &nome, UInt32(nome.count)) > 0, String(cString: nome).hasSuffix("llama-server") { kill(pid, SIGTERM) }
        try? FileManager.default.removeItem(at: Caminhos.pidDoServidor)
    }

    private static func portaLivre() -> Int {
        let s = socket(AF_INET, SOCK_STREAM, 0)
        defer { close(s) }
        var endereco = sockaddr_in()
        endereco.sin_family = sa_family_t(AF_INET)
        endereco.sin_addr.s_addr = inet_addr("127.0.0.1")
        endereco.sin_port = 0
        var tamanho = socklen_t(MemoryLayout<sockaddr_in>.size)
        let ok = withUnsafeMutablePointer(to: &endereco) { p in
            p.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(s, $0, tamanho) == 0 && getsockname(s, $0, &tamanho) == 0 }
        }
        return ok ? Int(UInt16(bigEndian: endereco.sin_port)) : 18_434
    }
}

/// Espera o servidor subir (ele começa a carregar quando a gravação começa) e repassa o pedido.
private struct LimpadorSobDemanda: Limpador {
    let servidor: ServidorLLM
    let modelo: String
    let instrucoes: String
    var nome: String { modelo.replacingOccurrences(of: ".gguf", with: "") }

    func limpar(_ pedido: PedidoLimpeza) async throws -> String {
        guard await servidor.garantir(modelo: modelo) else { throw ErroDeLimpeza("modelo de limpeza indisponível") }
        return try await LimpadorLlama(porta: servidor.portaAtual, modelo: nome, instrucoes: instrucoes).limpar(pedido)
    }
}
