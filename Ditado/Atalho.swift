import AppKit
import ApplicationServices

/// O atalho global do ditado.
///
/// - Segurar a tecla: grava enquanto segura, transcreve ao soltar.
/// - Dois toques rápidos, ou Espaço com a tecla segurada: mãos livres. Um novo toque encerra.
/// - Esc cancela a gravação. ⌘⌃V cola de novo o último ditado.
///
/// Os eventos vêm de um event tap numa thread própria: na principal ele esperaria a vez
/// atrás do resto do app, e o macOS desliga taps lentos, perdendo o soltar da tecla.
final class Atalho {
    var aoComecar: (() -> Bool)?
    var aoTerminar: (() -> Void)?
    var aoCancelar: (() -> Void)?
    var aoMaosLivres: (() -> Void)?
    var aoColarUltimo: (() -> Void)?
    /// A tecla foi usada como modificador depois de já haver fala gravada: encerrar sem colar.
    var aoInterromper: (() -> Void)?

    private enum Modo { case parado, segurando, aguardandoSegundoToque, maosLivres }

    /// Soltar antes disso é toque, não ditado.
    private let toqueMaximo: TimeInterval = 0.28
    private let janelaDoSegundoToque: TimeInterval = 0.4
    /// Outra tecla nesta janela descarta a gravação; depois dela, guarda no histórico sem colar.
    private let janelaDeModificador: TimeInterval = 0.7

    private var modo: Modo = .parado { didSet { atualizarVisao() } }
    private var inicioDoAperto: TimeInterval = 0
    private var ignorarAteSoltar = false
    private var temporizador: Timer?
    private var verificador: Timer?
    private var porta: CFMachPort?

    // Lido na thread do tap.
    private struct Visao {
        var codigo: Int64 = 61
        var mascara: CGEventFlags = .maskAlternate
        var ativo = false
        var segurando = false
    }

    private let trava = NSLock()
    private var visao = Visao()
    private var apertadaNoTap = false
    private var consumidas = Set<Int64>()

    var tecla: Preferencias.Tecla = .opcaoDireita { didSet { atualizarVisao() } }
    var instalado: Bool { porta != nil }

    private static func codigo(_ t: Preferencias.Tecla) -> (Int64, CGEventFlags) {
        switch t {
        case .fn: return (63, .maskSecondaryFn)
        case .opcaoDireita: return (61, .maskAlternate)
        case .comandoDireito: return (54, .maskCommand)
        case .controleDireito: return (62, .maskControl)
        }
    }

    private func atualizarVisao() {
        let (codigo, mascara) = Self.codigo(tecla)
        let v = Visao(codigo: codigo, mascara: mascara, ativo: modo != .parado, segurando: modo == .segurando)
        trava.withLock { visao = v }
    }

    /// Instala o tap. Sem permissão de Acessibilidade ele não instala; tenta de novo até conseguir.
    func iniciar() {
        atualizarVisao()
        if instalar() { return }
        verificador = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] t in
            if self?.instalar() == true { t.invalidate() }
        }
    }

    private func instalar() -> Bool {
        guard porta == nil else { return true }
        guard AXIsProcessTrusted() else { return false }
        let mascara: CGEventMask = (1 << CGEventType.keyDown.rawValue) | (1 << CGEventType.keyUp.rawValue) | (1 << CGEventType.flagsChanged.rawValue)
        let retorno: CGEventTapCallBack = { _, tipo, evento, contexto in
            guard let contexto else { return Unmanaged.passUnretained(evento) }
            return Unmanaged<Atalho>.fromOpaque(contexto).takeUnretainedValue().recebeu(tipo, evento)
        }
        guard let porta = CGEvent.tapCreate(tap: .cgSessionEventTap, place: .headInsertEventTap, options: .defaultTap, eventsOfInterest: mascara,
                                            callback: retorno, userInfo: Unmanaged.passUnretained(self).toOpaque())
        else { return false }
        self.porta = porta
        let fonte = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, porta, 0)
        let thread = Thread {
            CFRunLoopAddSource(CFRunLoopGetCurrent(), fonte, .commonModes)
            CGEvent.tapEnable(tap: porta, enable: true)
            CFRunLoopRun()
        }
        thread.name = "ditado.atalho"
        thread.qualityOfService = .userInteractive
        thread.start()
        Registro.nota("atalho instalado")
        return true
    }

    /// Thread do tap: decide na hora se o evento é nosso e devolve o controle.
    private func recebeu(_ tipo: CGEventType, _ evento: CGEvent) -> Unmanaged<CGEvent>? {
        if tipo == .tapDisabledByTimeout || tipo == .tapDisabledByUserInput {
            if let porta { CGEvent.tapEnable(tap: porta, enable: true) }
            return Unmanaged.passUnretained(evento)
        }
        let v = trava.withLock { visao }
        let codigo = evento.getIntegerValueField(.keyboardEventKeycode)
        let quando = TimeInterval(evento.timestamp) / 1_000_000_000

        switch tipo {
        case .flagsChanged:
            guard codigo == v.codigo else { break }
            // Com a tecla gêmea do outro lado também apertada a máscara continua ligada ao soltar;
            // por isso um evento desta tecla com ela já apertada é sempre o soltar.
            let apertou = evento.flags.contains(v.mascara) && !apertadaNoTap
            apertadaNoTap = apertou
            DispatchQueue.main.async { [weak self] in self?.gatilho(apertou, quando) }
        case .keyDown:
            // Tecla cujo aperto foi consumido continua consumida (repetições inclusive) até ser solta,
            // mesmo que o modo tenha mudado no meio: o app em foco nunca recebe metade do gesto.
            if consumidas.contains(codigo) { return nil }
            let repeticao = evento.getIntegerValueField(.keyboardEventAutorepeat) != 0
            let modificadores = evento.flags.intersection([.maskCommand, .maskControl, .maskAlternate, .maskShift])
            if codigo == 53, v.ativo, modificadores.isEmpty || modificadores == v.mascara { // Esc
                consumidas.insert(codigo)
                if !repeticao { DispatchQueue.main.async { [weak self] in self?.cancelarPorTecla() } }
                return nil
            }
            if codigo == 49, v.segurando { // Espaço com o atalho segurado
                consumidas.insert(codigo)
                if !repeticao { DispatchQueue.main.async { [weak self] in self?.entrarEmMaosLivres() } }
                return nil
            }
            if codigo == 9, modificadores == [.maskCommand, .maskControl] { // ⌘⌃V
                consumidas.insert(codigo)
                if !repeticao { DispatchQueue.main.async { [weak self] in self?.aoColarUltimo?() } }
                return nil
            }
            if v.segurando, !repeticao {
                DispatchQueue.main.async { [weak self] in self?.outraTecla(quando) }
            }
        case .keyUp:
            if consumidas.remove(codigo) != nil { return nil }
        default:
            break
        }
        return Unmanaged.passUnretained(evento)
    }

    // MARK: - Thread principal

    private func gatilho(_ apertou: Bool, _ quando: TimeInterval) {
        if apertou {
            switch modo {
            case .parado:
                if aoComecar?() == true {
                    modo = .segurando
                    inicioDoAperto = quando
                } else {
                    ignorarAteSoltar = true
                }
            case .aguardandoSegundoToque:
                temporizador?.invalidate()
                modo = .maosLivres
                ignorarAteSoltar = true
                aoMaosLivres?()
            case .maosLivres:
                modo = .parado
                ignorarAteSoltar = true
                aoTerminar?()
            case .segurando:
                break
            }
            return
        }
        if ignorarAteSoltar {
            ignorarAteSoltar = false
            return
        }
        guard modo == .segurando else { return }
        if quando - inicioDoAperto < toqueMaximo {
            modo = .aguardandoSegundoToque
            temporizador = Timer.scheduledTimer(withTimeInterval: janelaDoSegundoToque, repeats: false) { [weak self] _ in
                guard let self, self.modo == .aguardandoSegundoToque else { return }
                self.modo = .parado
                self.aoCancelar?()
            }
        } else {
            modo = .parado
            aoTerminar?()
        }
    }

    private func entrarEmMaosLivres() {
        guard modo == .segurando else { return }
        modo = .maosLivres
        ignorarAteSoltar = true
        aoMaosLivres?()
    }

    private func cancelarPorTecla() {
        guard modo != .parado else { return }
        temporizador?.invalidate()
        ignorarAteSoltar = modo == .segurando
        modo = .parado
        aoCancelar?()
    }

    private func outraTecla(_ quando: TimeInterval) {
        // ⌥+letra, Fn+seta: a tecla foi usada como modificador, não para ditar. Nada é colado.
        // Logo no começo descarta; depois, o que já foi falado vai para o histórico, sem colar.
        guard modo == .segurando else { return }
        let cedo = quando - inicioDoAperto < janelaDeModificador
        modo = .parado
        ignorarAteSoltar = true
        if cedo { aoCancelar?() } else { aoInterromper?() }
    }

    /// A gravação acabou sem passar pelo atalho (limite de tempo, silêncio, botão da pílula).
    func encerradoPorFora() {
        temporizador?.invalidate()
        if modo == .segurando { ignorarAteSoltar = true }
        modo = .parado
    }

    /// A gravação foi iniciada em mãos livres por outro caminho (botão da interface).
    func iniciadoEmMaosLivres() { modo = .maosLivres }
}
