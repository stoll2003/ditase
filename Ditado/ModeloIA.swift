import Foundation
import Nucleo

/// Baixa sob demanda o modelo padrão da limpeza e do resumo, sem bloquear a interface.
///
/// O estado é derivado do disco a cada leitura (arquivo final, `.part`, llama-server), mais o
/// que estiver acontecendo agora (download em curso, última falha). Assim nunca fica preso a um
/// estado velho se a pessoa colocar o arquivo à mão na pasta de modelos.
@MainActor
final class GerenciadorDeModelo {
    enum Situacao: Equatable {
        /// Não há llama-server embutido nem pelo Homebrew: a IA não roda neste Mac.
        case indisponivel
        case ausente(parcial: Int64)
        case baixando(feito: Int64, total: Int64)
        case conferindo
        case instalado
        case falhou(String, parcial: Int64)
    }

    private var modeloEmCurso: ModeloRemoto?
    var aoMudar: (() -> Void)?
    var aoConcluir: ((String) -> Void)?

    private var tarefa: Task<Void, Never>?
    private var etapa: BaixadorDeModelo.Etapa?
    private var falha: (modelo: ModeloRemoto, texto: String)?
    private var ultimoAviso = Date.distantPast
    private var avisoAgendado = false

    var baixando: Bool { tarefa != nil }

    func modeloParaDownload(escolhido: String) -> ModeloRemoto {
        modeloEmCurso ?? ModeloRemoto.qwen(arquivo: escolhido) ?? .qwenPadrao
    }

    func situacao(escolhido: String) -> Situacao {
        if ServidorLLM.binario == nil { return .indisponivel }
        let modelo = modeloParaDownload(escolhido: escolhido)
        let baixador = BaixadorDeModelo(modelo: modelo, pasta: Caminhos.modelos)
        if tarefa != nil {
            if case .baixando(let feito, let total) = etapa { return .baixando(feito: feito, total: total) }
            if etapa == .conferindo { return .conferindo }
            return .baixando(feito: baixador.jaBaixado, total: modelo.tamanho)
        }
        if ModeloRemoto.qwen(arquivo: escolhido) == nil,
           FileManager.default.fileExists(atPath: Caminhos.modelos.appendingPathComponent(escolhido).path) {
            return .instalado
        }
        if baixador.instalado { return .instalado }
        if let falha, falha.modelo == modelo { return .falhou(falha.texto, parcial: baixador.jaBaixado) }
        // Confere uma cópia já presente, sem iniciar tráfego de rede.
        if baixador.precisaConferir {
            iniciar(modelo, soConferir: true)
            return .conferindo
        }
        return .ausente(parcial: baixador.jaBaixado)
    }

    func baixar(escolhido: String) {
        iniciar(modeloParaDownload(escolhido: escolhido))
    }

    private func iniciar(_ modelo: ModeloRemoto, soConferir: Bool = false) {
        let baixador = BaixadorDeModelo(modelo: modelo, pasta: Caminhos.modelos)
        guard tarefa == nil, ServidorLLM.binario != nil, !baixador.instalado else { return }
        modeloEmCurso = modelo
        falha = nil
        etapa = soConferir ? .conferindo : nil
        Registro.nota("modelo: \(soConferir ? "conferência" : "download") de \(modelo.arquivo) a partir de \(baixador.jaBaixado) bytes")
        tarefa = Task { [weak self] in
            do {
                if soConferir {
                    try await Task.detached { try baixador.conferirExistente() }.value
                } else {
                    try await baixador.baixar { [weak self] etapa in
                        Task { @MainActor in self?.avancar(etapa) }
                    }
                }
                Registro.nota("modelo: \(baixador.modelo.arquivo) instalado e conferido")
                self?.terminar(falha: nil)
                self?.aoConcluir?(modelo.arquivo)
            } catch let erro as ErroDeDownload where erro == .cancelado {
                Registro.nota("modelo: download cancelado em \(baixador.jaBaixado) bytes")
                self?.terminar(falha: nil)
            } catch {
                let texto = (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
                Registro.nota("modelo: falha no download: \(error)")
                self?.terminar(falha: (modelo, texto))
            }
        }
        aoMudar?()
    }

    func cancelar() { tarefa?.cancel() }

    private func terminar(falha: (ModeloRemoto, String)?) {
        tarefa = nil
        modeloEmCurso = nil
        etapa = nil
        self.falha = falha
        aoMudar?()
    }

    /// O progresso chega a cada MiB; a interface é avisada no máximo 5 vezes por segundo.
    private func avancar(_ nova: BaixadorDeModelo.Etapa) {
        guard tarefa != nil else { return }
        if case .baixando(let novo, _) = nova, case .baixando(let atual, _) = etapa, novo < atual { return }
        etapa = nova
        let agora = Date()
        if nova == .conferindo || agora.timeIntervalSince(ultimoAviso) >= 0.2 {
            ultimoAviso = agora
            aoMudar?()
        } else if !avisoAgendado {
            avisoAgendado = true
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { [weak self] in
                guard let self else { return }
                self.avisoAgendado = false
                self.ultimoAviso = Date()
                self.aoMudar?()
            }
        }
    }
}

extension GerenciadorDeModelo {
    /// `Dita.se --baixar-modelo [--cancelar-em MB]`, com DITASE_PASTA_MODELOS obrigatório: prova o
    /// mesmo caminho dos Ajustes (estado, progresso, cancelamento e retomada) sem tocar na pasta do usuário.
    static func linhaDeComando() async -> Int32 {
        guard let pasta = ProcessInfo.processInfo.environment["DITASE_PASTA_MODELOS"], !pasta.isEmpty else {
            FileHandle.standardError.write(Data("erro: defina DITASE_PASTA_MODELOS com uma pasta de teste\n".utf8))
            return 2
        }
        let cancelarEm = ModoLinha.argumento("--cancelar-em").flatMap(Int64.init).map { $0 << 20 }
        let g = GerenciadorDeModelo()
        let escolhido = ModoLinha.argumento("--modelo") ?? ModeloRemoto.qwenPadrao.arquivo
        var ultimo = ""
        var cancelou = false
        func mostrar() {
            let texto: String
            switch g.situacao(escolhido: escolhido) {
            case .indisponivel: texto = "indisponivel"
            case .ausente(let p): texto = "ausente parcial=\(p)"
            case .baixando(let f, let t): texto = "baixando \(f * 100 / max(t, 1))%"
            case .conferindo: texto = "conferindo"
            case .instalado: texto = "instalado"
            case .falhou(let m, let p): texto = "falhou parcial=\(p): \(m)"
            }
            if texto != ultimo { print("estado: \(texto)"); ultimo = texto }
        }
        g.aoMudar = {
            mostrar()
            if let limite = cancelarEm, !cancelou, case .baixando(let f, _) = g.situacao(escolhido: escolhido), f >= limite {
                cancelou = true
                print("cancelando em \(f) bytes")
                g.cancelar()
            }
        }
        print("pasta: \(pasta)")
        mostrar()
        g.baixar(escolhido: escolhido)
        while g.baixando { try? await Task.sleep(nanoseconds: 50_000_000) }
        mostrar()
        if case .instalado = g.situacao(escolhido: escolhido) { return 0 }
        return cancelou ? 3 : 1
    }
}

/// Resumo que não pode rodar por falta do modelo ou do llama-server; vai para a aba e a pílula
/// com o texto exato do que falta.
struct ResumoSemModelo: LocalizedError {
    let errorDescription: String?

    static func verificar(modelo: String) throws {
        if ServidorLLM.binario == nil {
            throw ResumoSemModelo(errorDescription: "O resumo usa a IA local, que não está disponível neste Mac. A transcrição foi salva.")
        }
        if !FileManager.default.fileExists(atPath: Caminhos.modelos.appendingPathComponent(modelo).path) {
            throw ResumoSemModelo(errorDescription: "O resumo precisa do modelo de IA. Baixe-o em Ajustes, no grupo Limpeza por IA, e peça o resumo de novo. A transcrição foi salva.")
        }
    }
}
