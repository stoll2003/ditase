import AppKit
import UniformTypeIdentifiers
import WebKit

/// Entrega os arquivos da interface (pasta `ui` do pacote) pelo esquema `ditado://app/`.
final class ServidorDeArquivos: NSObject, WKURLSchemeHandler {
    private let raiz: URL

    init(raiz: URL) { self.raiz = raiz.standardizedFileURL }

    func webView(_ webView: WKWebView, start tarefa: WKURLSchemeTask) {
        guard let url = tarefa.request.url else { return }
        var caminho = url.path
        if caminho.isEmpty || caminho == "/" { caminho = "/index.html" }
        let arquivo = raiz.appendingPathComponent(String(caminho.dropFirst())).standardizedFileURL
        // Nada fora da pasta da interface.
        guard arquivo.path.hasPrefix(raiz.path + "/"), let dados = try? Data(contentsOf: arquivo) else {
            tarefa.didFailWithError(URLError(.fileDoesNotExist))
            return
        }
        let tipo = UTType(filenameExtension: arquivo.pathExtension)?.preferredMIMEType ?? "application/octet-stream"
        let resposta = HTTPURLResponse(url: url, statusCode: 200, httpVersion: "HTTP/1.1", headerFields: [
            "Content-Type": tipo, "Content-Length": String(dados.count), "Cache-Control": "no-store",
        ])!
        tarefa.didReceive(resposta)
        tarefa.didReceive(dados)
        tarefa.didFinish()
    }

    func webView(_ webView: WKWebView, stop tarefa: WKURLSchemeTask) {}
}

/// Faixa invisível no topo da janela por onde se arrasta, já que a web view ocupa a janela toda.
private final class FaixaDeArrasto: NSView {
    override var mouseDownCanMoveWindow: Bool { true }
    override func mouseUp(with event: NSEvent) {
        if event.clickCount == 2 { window?.zoom(nil) }
    }
}

/// Lê URLs do pasteboard de arraste do macOS, sem receber caminhos do JavaScript.
final class VistaDeLancamento: WKWebView {
    var aoSoltarArquivos: (([URL]) -> Void)?

    static func arquivos(_ pasteboard: NSPasteboard) -> [URL] {
        let urls = pasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL] ?? []
        return urls.filter { url in
            url.isFileURL && (try? url.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true
        }
    }

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        if !Self.arquivos(sender.draggingPasteboard).isEmpty { return .copy }
        return super.draggingEntered(sender)
    }

    override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation {
        if !Self.arquivos(sender.draggingPasteboard).isEmpty { return .copy }
        return super.draggingUpdated(sender)
    }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        let arquivos = Self.arquivos(sender.draggingPasteboard)
        if arquivos.isEmpty { return super.performDragOperation(sender) }
        aoSoltarArquivos?(arquivos)
        return aoSoltarArquivos != nil
    }
}

@MainActor
final class JanelaPrincipal: NSObject, NSWindowDelegate, WKNavigationDelegate {
    let janela: NSWindow
    let vista: WKWebView
    private let ponte: Ponte
    var aoCarregar: (() -> Void)?

    init(controlador: Controlador) {
        ponte = Ponte(controlador: controlador)
        let config = WKWebViewConfiguration()
        let raiz = Bundle.main.resourceURL!.appendingPathComponent("ui")
        config.setURLSchemeHandler(ServidorDeArquivos(raiz: raiz), forURLScheme: "ditado")
        config.userContentController.addScriptMessageHandler(ponte, contentWorld: .page, name: "ditado")
        let vistaNativa = VistaDeLancamento(frame: .zero, configuration: config)
        vista = vistaNativa
        vista.setValue(false, forKey: "drawsBackground")
        vista.allowsMagnification = false
        ponte.vista = vista

        janela = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1040, height: 700), styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView], backing: .buffered, defer: false)
        janela.title = "Dita.se"
        janela.titleVisibility = .hidden
        janela.titlebarAppearsTransparent = true
        janela.isReleasedWhenClosed = false
        janela.minSize = NSSize(width: 860, height: 560)
        janela.backgroundColor = NSColor(name: nil) { $0.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? NSColor(white: 0.11, alpha: 1) : NSColor(white: 0.93, alpha: 1) }
        janela.setFrameAutosaveName("JanelaPrincipal")
        super.init()
        vistaNativa.registerForDraggedTypes([.fileURL])
        vistaNativa.aoSoltarArquivos = { [weak controlador, weak ponte] urls in
            if let id = controlador?.lancamento.adicionar(urls) {
                ponte?.emitir("ditado:arquivos-soltos", id)
            }
        }


        let conteudo = NSView(frame: janela.contentLayoutRect)
        vista.translatesAutoresizingMaskIntoConstraints = false
        let faixa = FaixaDeArrasto()
        faixa.translatesAutoresizingMaskIntoConstraints = false
        conteudo.addSubview(vista)
        conteudo.addSubview(faixa)
        NSLayoutConstraint.activate([
            vista.topAnchor.constraint(equalTo: conteudo.topAnchor), vista.bottomAnchor.constraint(equalTo: conteudo.bottomAnchor),
            vista.leadingAnchor.constraint(equalTo: conteudo.leadingAnchor), vista.trailingAnchor.constraint(equalTo: conteudo.trailingAnchor),
            faixa.topAnchor.constraint(equalTo: conteudo.topAnchor), faixa.leadingAnchor.constraint(equalTo: conteudo.leadingAnchor),
            faixa.trailingAnchor.constraint(equalTo: conteudo.trailingAnchor), faixa.heightAnchor.constraint(equalToConstant: 30),
        ])
        janela.contentView = conteudo
        janela.delegate = self
        vista.navigationDelegate = self
        if janela.frame.origin == .zero { janela.center() }

        if let dev = ProcessInfo.processInfo.environment["DITADO_UI_DEV"], let url = URL(string: dev) {
            vista.load(URLRequest(url: url))
        } else {
            vista.load(URLRequest(url: URL(string: "ditado://app/index.html")!))
        }
    }

    func mostrar() {
        NSApp.activate(ignoringOtherApps: true)
        janela.makeKeyAndOrderFront(nil)
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) { aoCarregar?() }

    /// Links externos abrem no navegador, nunca dentro do app.
    func webView(_ webView: WKWebView, decidePolicyFor acao: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        guard let url = acao.request.url else { return decisionHandler(.cancel) }
        if url.scheme == "ditado" || (url.host == "localhost" && ProcessInfo.processInfo.environment["DITADO_UI_DEV"] != nil) {
            decisionHandler(.allow)
        } else {
            if url.scheme == "https" { NSWorkspace.shared.open(url) }
            decisionHandler(.cancel)
        }
    }

    /// Grava em PNG o que a interface está mostrando. Usado na conferência visual.
    func capturar(em destino: URL, depois: @escaping (Bool) -> Void) {
        vista.takeSnapshot(with: nil) { imagem, _ in
            guard let tiff = imagem?.tiffRepresentation, let rep = NSBitmapImageRep(data: tiff), let png = rep.representation(using: .png, properties: [:]) else { return depois(false) }
            depois((try? png.write(to: destino)) != nil)
        }
    }
}
