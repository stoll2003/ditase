import AppKit
import Nucleo

@MainActor
final class Aplicativo: NSObject, NSApplicationDelegate {
    private var controlador: Controlador?
    private var janela: JanelaPrincipal?
    private var itemDaBarra: NSStatusItem?
    private let saida = SaidaComPrazo()

    func applicationDidFinishLaunching(_ notification: Notification) {
        ServidorLLM.utilitario = ModoLinha.argumento("--capturar") != nil || ModoLinha.tem("--importar-wispr")
        let captura = ModoLinha.argumento("--capturar")
        let utilitario = captura != nil || ModoLinha.tem("--importar-wispr")
        // Uma instância só: a segunda traz a primeira para a frente e sai.
        if !utilitario, let outra = NSRunningApplication.runningApplications(withBundleIdentifier: Bundle.main.bundleIdentifier ?? "").first(where: { $0 != .current }) {
            outra.activate()
            NSApp.terminate(nil)
            return
        }
        let banco: Banco
        do {
            // A captura de tela usa um banco à parte quando pedido, para mostrar dados de exemplo.
            let argumento = ModoLinha.argumento("--banco")
            let caminho = captura == nil ? argumento ?? Caminhos.banco.path : try Caminhos.bancoDoUtilitario(argumento)
            banco = try Banco(caminho: caminho)
        } catch {
            let alerta = NSAlert()
            alerta.messageText = "O Dita.se não conseguiu abrir o histórico"
            alerta.informativeText = String(describing: error)
            alerta.runModal()
            NSApp.terminate(nil)
            return
        }
        if ModoLinha.tem("--importar-wispr") {
            let r = ImportadorWispr.importar(para: banco)
            print("importados \(r.ditados) ditados e \(r.termos) termos (encontrado: \(r.encontrado))")
            exit(0)
        }
        let controlador = Controlador(banco: banco)
        self.controlador = controlador
        montarMenus()
        let janela = JanelaPrincipal(controlador: controlador)
        self.janela = janela

        if let captura {
            // Conferência visual: abre a página pedida, grava o PNG e sai. Não liga atalho nem modelos.
            let pagina = ModoLinha.argumento("--pagina") ?? "inicio"
            let espera = Double(ModoLinha.argumento("--espera") ?? "") ?? 1.5
            if let tema = ModoLinha.argumento("--tema") { NSApp.appearance = NSAppearance(named: tema == "claro" ? .aqua : .darkAqua) }
            if let largura = Double(ModoLinha.argumento("--largura") ?? ""), let altura = Double(ModoLinha.argumento("--altura") ?? "") {
                janela.janela.setContentSize(NSSize(width: largura, height: altura))
            }
            janela.aoCarregar = {
                janela.vista.evaluateJavaScript("location.hash = '#/\(pagina)'", completionHandler: nil)
                DispatchQueue.main.asyncAfter(deadline: .now() + espera) {
                    janela.capturar(em: URL(fileURLWithPath: captura)) { ok in exit(ok ? 0 : 1) }
                }
            }
            janela.janela.orderFrontRegardless()
            return
        }

        controlador.ligar()
        let modelo = controlador.pilula.modelo
        let abrir: (String) -> Void = { [weak janela] pagina in
            janela?.mostrar()
            janela?.vista.evaluateJavaScript("location.hash = '#/\(pagina)'", completionHandler: nil)
        }
        modelo.aoAbrirDitase = { abrir("inicio") }
        modelo.aoAbrirReuniao = { abrir("reuniao") }
        modelo.aoComecarReuniao = { [weak controlador] in
            guard let controlador else { return }
            if !controlador.iniciarReuniaoPelaPilula() { abrir("reuniao") }
        }
        modelo.aoEscolherArquivo = { [weak controlador] in
            abrir("arquivos")
            controlador?.lancamento.escolher { [weak controlador] urls in _ = controlador?.lancamento.adicionar(urls) }
        }
        janela.mostrar()
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows: Bool) -> Bool {
        janela?.mostrar()
        return true
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let controlador, controlador.lancamento.aguardaPersistencia else { return .terminateNow }
        // Teto rígido de 2 s contado desde o pedido. O trabalho não escreve no ator principal.
        saida.iniciar(prazo: .seconds(2), trabalho: { await controlador.lancamento.prepararSaida() },
                      responder: { sender.reply(toApplicationShouldTerminate: true) })
        return .terminateLater
    }

    func applicationWillTerminate(_ notification: Notification) { controlador?.desligar() }

    @objc private func abrir() { janela?.mostrar() }
    @objc private func colarUltimo() { controlador?.colarUltimo() }
    @objc private func alternar() { controlador?.alternarPelaInterface() }

    private func montarMenus() {
        let principal = NSMenu()
        let app = NSMenuItem()
        app.submenu = NSMenu()
        app.submenu?.addItem(withTitle: "Sobre o Dita.se", action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
        app.submenu?.addItem(.separator())
        app.submenu?.addItem(withTitle: "Ocultar o Dita.se", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        app.submenu?.addItem(withTitle: "Encerrar o Dita.se", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        principal.addItem(app)
        // Sem o menu Editar, ⌘C, ⌘V e ⌘A não funcionam nos campos da interface.
        let editar = NSMenuItem()
        editar.submenu = NSMenu(title: "Editar")
        editar.submenu?.addItem(withTitle: "Desfazer", action: Selector(("undo:")), keyEquivalent: "z")
        editar.submenu?.addItem(withTitle: "Refazer", action: Selector(("redo:")), keyEquivalent: "Z")
        editar.submenu?.addItem(.separator())
        editar.submenu?.addItem(withTitle: "Recortar", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        editar.submenu?.addItem(withTitle: "Copiar", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        editar.submenu?.addItem(withTitle: "Colar", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        editar.submenu?.addItem(withTitle: "Selecionar Tudo", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        principal.addItem(editar)
        let janelaMenu = NSMenuItem()
        janelaMenu.submenu = NSMenu(title: "Janela")
        janelaMenu.submenu?.addItem(withTitle: "Minimizar", action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m")
        janelaMenu.submenu?.addItem(withTitle: "Fechar", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        principal.addItem(janelaMenu)
        NSApp.mainMenu = principal
        NSApp.windowsMenu = janelaMenu.submenu

        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        item.button?.image = NSImage(systemSymbolName: "waveform", accessibilityDescription: "Dita.se")
        let menu = NSMenu()
        menu.addItem(withTitle: "Abrir o Dita.se", action: #selector(abrir), keyEquivalent: "").target = self
        menu.addItem(withTitle: "Ditar em mãos livres / Terminar", action: #selector(alternar), keyEquivalent: "").target = self
        menu.addItem(withTitle: "Colar o último ditado", action: #selector(colarUltimo), keyEquivalent: "").target = self
        menu.addItem(.separator())
        menu.addItem(withTitle: "Encerrar", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "")
        item.menu = menu
        itemDaBarra = item
    }
}

MainActor.assumeIsolated {
    let lancamento = LinhaLancamento.solicitado
    let ditado = ModoLinha.argumento("--transcrever")
    let baixarModelo = ModoLinha.tem("--baixar-modelo")
    if lancamento || ditado != nil || baixarModelo || ModoLinha.tem("--capturar") {
        do { try Caminhos.isolarUtilitario() }
        catch {
            FileHandle.standardError.write(Data(("erro: " + error.localizedDescription + "\n").utf8))
            exit(1)
        }
    }
    if lancamento || ditado != nil || baixarModelo {
        Task { @MainActor in
            ServidorLLM.utilitario = true
            if baixarModelo { exit(await GerenciadorDeModelo.linhaDeComando()) }
            if lancamento { exit(await LinhaLancamento.executar()) }
            if let ditado { exit(await ModoLinha.transcrever(ditado)) }
        }
        dispatchMain()
    }
}

MainActor.assumeIsolated {
    let aplicativo = NSApplication.shared
    let delegado = Aplicativo()
    aplicativo.delegate = delegado
    // No modo de linha de comando o app não aparece no Dock nem toma o foco.
    let utilitario = ModoLinha.argumento("--capturar") != nil || ModoLinha.tem("--importar-wispr")
    aplicativo.setActivationPolicy(utilitario ? .accessory : .regular)
    aplicativo.run()
}
