import AppKit
import Combine
import UniformTypeIdentifiers
import Foundation
import Nucleo
import WebKit

/// Ponte entre a interface web e o app. A interface chama
/// `window.webkit.messageHandlers.ditado.postMessage({metodo, params})` e recebe JSON em texto.
@MainActor
final class Ponte: NSObject, WKScriptMessageHandlerWithReply {
    private let controlador: Controlador
    weak var vista: WKWebView?
    private var eventosLancamento = Set<AnyCancellable>()

    init(controlador: Controlador) {
        self.controlador = controlador
        super.init()
        controlador.aoMudar = { [weak self] assunto in self?.emitir(assunto) }
        controlador.lancamento.aoMudar = { [weak self] in self?.emitir("lancamento") }
        let publicador = controlador.lancamento.processamento
        publicador.estados.compactMap { $0 }.sink { [weak self] valor in
            self?.emitir("ditado:progresso", valor)
        }.store(in: &eventosLancamento)
        publicador.resultados.sink { [weak self] valor in
            self?.emitir("ditado:resultado", valor)
        }.store(in: &eventosLancamento)
        controlador.lancamento.intervalos.sink { [weak self] valor in
            self?.emitir("ditado:intervalos", valor)
        }.store(in: &eventosLancamento)
        controlador.lancamento.segmentos.sink { [weak self] valor in
            self?.emitir("ditado:trechos", valor)
        }.store(in: &eventosLancamento)
    }

    /// Avisa a interface de que algo mudou; ela busca de novo o que lhe interessa.
    func emitir(_ assunto: String) {
        guard let assunto = Self.json(assunto) else { return }
        vista?.evaluateJavaScript("window.dispatchEvent(new CustomEvent('ditado', { detail: \(assunto) }))", completionHandler: nil)
    }

    func emitir<T: Encodable>(_ evento: String, _ valor: T) {
        guard let conteudo = try? codificar(valor) else { return }
        vista?.evaluateJavaScript("window.dispatchEvent(new CustomEvent('\(evento)', { detail: \(conteudo) }))", completionHandler: nil)
    }

    func userContentController(_ controller: WKUserContentController, didReceive mensagem: WKScriptMessage, replyHandler: @escaping (Any?, String?) -> Void) {
        guard let corpo = mensagem.body as? [String: Any], let metodo = corpo["metodo"] as? String else {
            replyHandler(nil, "pedido inválido")
            return
        }
        let params = corpo["params"] as? [String: Any] ?? [:]
        let lancamento = controlador.lancamento
        if ["lancamento.escolher", "lancamento.adicionar", "arquivo.transcrever"].contains(metodo) {
            lancamento.escolher { [weak lancamento] urls in
                guard let id = lancamento?.adicionar(urls) else { replyHandler("null", nil); return }
                replyHandler(Self.json(id), nil)
            }
            return
        }
        if metodo == "lancamento.repetir" {
            guard let id = params["id"] as? String else { replyHandler(nil, "falta id"); return }
            let repetir: (URL) -> Void = { [weak lancamento] url in
                do {
                    guard let lancamento else { replyHandler(nil, "motor indisponível"); return }
                    try lancamento.repetir(id: id, url: url)
                    replyHandler("true", nil)
                } catch { replyHandler(nil, error.localizedDescription) }
            }
            do {
                if let url = try lancamento.fonteParaRepetir(id: id) { repetir(url) }
                else {
                    lancamento.escolher(varios: false) { urls in
                        if let url = urls.first { repetir(url) }
                        else { replyHandler("false", nil) }
                    }
                }
            } catch { replyHandler(nil, error.localizedDescription) }
            return
        }
        if metodo == "lancamento.exportar" {
            guard let texto = params["texto"] as? String, let nome = params["nome"] as? String,
                  let formato = params["formato"] as? String, ["txt", "md", "srt", "vtt"].contains(formato)
            else { replyHandler(nil, "formato de exportação inválido"); return }
            let painel = NSSavePanel()
            painel.nameFieldStringValue = URL(fileURLWithPath: nome).lastPathComponent
            if let tipo = UTType(filenameExtension: formato) { painel.allowedContentTypes = [tipo] }
            painel.begin { resposta in
                guard resposta == .OK, let url = painel.url else { replyHandler("false", nil); return }
                do { try texto.write(to: url, atomically: true, encoding: .utf8); replyHandler("true", nil) }
                catch { replyHandler(nil, error.localizedDescription) }
            }
            return
        }
        Task {
            do { replyHandler(try await atender(metodo, params), nil) }
            catch { replyHandler(nil, error.localizedDescription) }
        }
    }

    struct ErroDaPonte: Error, CustomStringConvertible {
        let description: String
    }

    static func json(_ valor: Any) -> String? {
        if let s = valor as? String, let d = try? JSONEncoder().encode(s) { return String(data: d, encoding: .utf8) }
        guard JSONSerialization.isValidJSONObject(valor), let d = try? JSONSerialization.data(withJSONObject: valor) else { return nil }
        return String(data: d, encoding: .utf8)
    }

    private func codificar<T: Encodable>(_ valor: T) throws -> String {
        String(data: try JSONEncoder().encode(valor), encoding: .utf8) ?? "null"
    }

    private func inteiro(_ params: [String: Any], _ chave: String) throws -> Int64 {
        if let n = params[chave] as? NSNumber { return n.int64Value }
        throw ErroDaPonte(description: "falta \(chave)")
    }

    func atender(_ metodo: String, _ params: [String: Any]) async throws -> String {
        let banco = controlador.banco
        let lance = controlador.lancamento
        func texto(_ chave: String) throws -> String {
            guard let valor = params[chave] as? String,
                  !valor.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            else { throw ErroDaPonte(description: "falta " + chave) }
            return valor
        }
        if metodo.hasPrefix("lancamento.") || metodo.hasPrefix("arquivo.") || metodo.hasPrefix("reuniao.") || metodo.hasPrefix("falante.") {
            _ = try await lance.preparar()
        }
        switch metodo {
        case "lancamento.estado":
            return try codificar(lance.estado())
        case "lancamento.documento":
            return try codificar(lance.documento(id: texto("id")))
        case "lancamento.buscarArquivos":
            let termo = (params["termo"] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            return try codificar(termo.isEmpty ? [] : lance.procurarArquivos(termo))
        case "lancamento.recuperar":
            try lance.recuperarReuniao(id: texto("id"))
        case "lancamento.descartar":
            try await lance.descartarReuniao(id: texto("id"))
        case "arquivo.listar":
            return try codificar(lance.estado().arquivos)
        case "reuniao.listar":
            return try codificar(lance.estado().anteriores)
        case "arquivo.cancelar":
            lance.cancelar(id: try texto("id"))
        case "lancamento.remover":
            try await lance.remover(id: texto("id"))
        case "lancamento.desfazer":
            try await lance.desfazer(id: texto("id"))
        case "lancamento.cancelarFila":
            lance.cancelarFila()
        case "reuniao.iniciar", "lancamento.iniciar":
            try lance.iniciarReuniao(fonte: texto("fonte"))
        case "reuniao.pausar", "lancamento.pausar":
            try lance.pausar(id: texto("id"))
        case "reuniao.encerrar", "lancamento.encerrar":
            try lance.encerrar(id: texto("id"))
        case "lancamento.resumir":
            try lance.resumir(id: texto("id"))
        case "lancamento.cancelarResumo":
            lance.cancelarResumo(id: try texto("id"))
        case "lancamento.permissao":
            try lance.pedirPermissao(texto("qual"))
        case "lancamento.titulo":
            let nome = try texto("nome")
            try await lance.alterar(id: texto("id"), campo: .nome(String(nome.prefix(160))))
        case "falante.renomear", "lancamento.voz":
            try await lance.renomearVoz(id: texto("id"), voz: texto("voz"), nome: texto("nome"))
        case "lancamento.trecho":
            let voz = params["voz"] as? String
            let nome = voz == nil ? try String(texto("nome").prefix(120)) : nil
            try await lance.reatribuir(id: texto("id"), trecho: texto("trecho"), voz: voz, nome: nome)

        case "estado":
            return Self.json(estado()) ?? "{}"

        case "historico":
            let busca = params["busca"] as? String ?? ""
            let limite = (params["limite"] as? NSNumber)?.intValue ?? 200
            return try codificar(try banco.historico(limite: min(max(limite, 1), 1000), busca: busca))

        case "estatisticas":
            let dias = (params["dias"] as? NSNumber)?.intValue ?? 30
            return try codificar(Estatisticas.calcular(try banco.paraEstatisticas(), diasNoGrafico: min(max(dias, 7), 365)))

        case "dicionario.listar":
            return try codificar(try banco.termos())

        case "dicionario.salvar":
            guard let nome = params["termo"] as? String else { throw ErroDaPonte(description: "falta termo") }
            let formas = params["ouvidoComo"] as? [String] ?? []
            var termo = Termo(id: (params["id"] as? NSNumber)?.int64Value ?? 0, termo: nome, ouvidoComo: formas)
            if termo.id != 0, let atual = try banco.termos().first(where: { $0.id == termo.id }) {
                termo.usos = atual.usos
                termo.origem = atual.origem
                termo.criadoEm = atual.criadoEm
            }
            return try codificar(try banco.salvar(termo: termo))

        case "dicionario.apagar":
            try banco.apagarTermo(id: try inteiro(params, "id"))
            return "true"

        case "ajustes.ler":
            return Self.json(ajustes()) ?? "{}"

        case "ajustes.definir":
            guard let chave = params["chave"] as? String, let valor = params["valor"] as? String else { throw ErroDaPonte(description: "falta chave ou valor") }
            controlador.prefs.definir(chave, valor)
            return "true"

        case "permissao.pedir":
            switch params["qual"] as? String {
            case "microfone":
                if Gravador.permissao == "negada" {
                    NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")!)
                } else {
                    Gravador.pedirPermissao { [weak self] _ in self?.emitir("estado") }
                }
            case "acessibilidade":
                Insersor.pedirAcessibilidade()
                NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")!)
            default:
                throw ErroDaPonte(description: "permissão desconhecida")
            }
            return "true"

        case "ditado.apagar":
            let id = try inteiro(params, "id")
            if let d = try banco.ditado(id: id), !d.audio.isEmpty {
                try? FileManager.default.removeItem(at: Caminhos.audios.appendingPathComponent(d.audio))
            }
            try banco.apagarDitado(id: id)
            emitir("historico")
            return "true"

        case "ditado.retranscrever":
            controlador.retranscrever(id: try inteiro(params, "id"))
            return "true"

        case "copiar":
            guard let texto = params["texto"] as? String else { throw ErroDaPonte(description: "falta texto") }
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(texto, forType: .string)
            return "true"

        case "gravacao.alternar":
            controlador.alternarPelaInterface()
            return "true"

        case "wispr.importar":
            let resumo = ImportadorWispr.importar(para: banco)
            emitir("historico")
            emitir("dicionario")
            return try codificar(resumo)

        case "modelo.baixar":
            controlador.modeloIA.baixar()
            return "true"

        case "modelo.cancelar":
            controlador.modeloIA.cancelar()
            return "true"

        case "abrir.pastaDeModelos":
            NSWorkspace.shared.open(Caminhos.modelos)
            return "true"

        default:
            throw ErroDaPonte(description: "método desconhecido: \(metodo)")
        }
        return "true"
    }

    private func estado() -> [String: Any] {
        let reconhecedor: (String, String)
        switch controlador.estadoDoReconhecedor {
        case .parado: reconhecedor = ("parado", "")
        case .carregando: reconhecedor = ("carregando", "")
        case .pronto: reconhecedor = ("pronto", "")
        case .falhou(let m): reconhecedor = ("falhou", m)
        }
        let limpador: (String, String)
        switch controlador.estadoDoLimpador {
        case .desligado: limpador = ("desligado", "")
        case .iniciando: limpador = ("iniciando", "")
        case .pronto: limpador = ("pronto", "")
        case .semBinario: limpador = ("semBinario", "")
        case .semModelo(let m): limpador = ("semModelo", m)
        case .falhou(let m): limpador = ("falhou", m)
        }
        let tecla = controlador.prefs.atalho
        let gerenciador = controlador.modeloIA
        var modeloIA: [String: Any] = ["arquivo": gerenciador.modelo.arquivo, "tamanho": gerenciador.modelo.tamanho,
                                       "aviso": controlador.prefs.ler("avisoModelo") == "1"]
        switch gerenciador.situacao(escolhido: controlador.prefs.modelo) {
        case .indisponivel: modeloIA["estado"] = "indisponivel"
        case .ausente(let parcial): modeloIA.merge(["estado": "ausente", "feito": parcial]) { $1 }
        case .baixando(let feito, let total): modeloIA.merge(["estado": "baixando", "feito": feito, "tamanho": total]) { $1 }
        case .conferindo: modeloIA.merge(["estado": "conferindo", "feito": gerenciador.modelo.tamanho]) { $1 }
        case .instalado: modeloIA["estado"] = "instalado"
        case .falhou(let motivo, let parcial): modeloIA.merge(["estado": "falhou", "detalhe": motivo, "feito": parcial]) { $1 }
        }
        return [
            "fase": controlador.fase.rawValue,
            "maosLivres": controlador.maosLivres,
            "reconhecedor": ["estado": reconhecedor.0, "detalhe": reconhecedor.1, "nome": "Parakeet Ultra"],
            "limpador": ["estado": limpador.0, "detalhe": limpador.1, "ligado": controlador.prefs.limpezaIA, "modelo": controlador.prefs.modelo.replacingOccurrences(of: ".gguf", with: "")],
            "modeloIA": modeloIA,
            "permissoes": ["microfone": Gravador.permissao, "acessibilidade": Insersor.temAcessibilidade ? "concedida" : "pendente"],
            "atalho": ["tecla": tecla.rawValue, "nome": tecla.nome, "simbolo": tecla.simbolo, "instalado": controlador.atalho.instalado],
            "wisprAberto": !NSRunningApplication.runningApplications(withBundleIdentifier: "com.electron.wispr-flow").isEmpty,
            "wisprDisponivel": ImportadorWispr.disponivel,
            "versao": Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "",
        ]
    }

    private func ajustes() -> [String: Any] {
        let arquivos = ((try? FileManager.default.contentsOfDirectory(atPath: Caminhos.modelos.path)) ?? []).filter { $0.hasSuffix(".gguf") }.sorted()
        return [
            "valores": controlador.prefs.todas,
            "teclas": Preferencias.Tecla.allCases.map { ["id": $0.rawValue, "nome": $0.nome] },
            "microfones": Gravador.microfones().map { ["uid": $0.uid, "nome": $0.nome] },
            "modelos": arquivos,
            "pastaDeModelos": Caminhos.modelos.path,
            "llamaInstalado": ServidorLLM.binario != nil,
        ]
    }
}
