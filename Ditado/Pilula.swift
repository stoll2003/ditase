import AppKit
import Combine
import SwiftUI

/// Estado mostrado na pílula.
final class ModeloDaPilula: ObservableObject {
    enum Fase: Equatable {
        case ociosa
        case gravando
        case maosLivres
        case processando
        case aviso(String)
    }

    @Published var fase: Fase = .ociosa
    @Published private(set) var progresso: ProgressoDaPilula?
    @Published private(set) var reuniao: CapturaDaPilula?
    let nivelReuniao = CurrentValueSubject<Double, Never>(0)
    let segundosReuniao = CurrentValueSubject<Int, Never>(0)

    func receberReuniao(_ captura: CapturaDaPilula?) {
        var cabecalho = captura
        cabecalho?.nivel = 0
        cabecalho?.segundos = 0
        if reuniao != cabecalho { reuniao = cabecalho }
        let segundos = captura?.segundos ?? 0
        if segundos != segundosReuniao.value { segundosReuniao.send(segundos) }
        let nivel = captura?.nivel ?? 0
        let amostra = nivel.isFinite ? min(1, max(0, nivel)) : 0
        if abs(amostra - nivelReuniao.value) >= 0.025 || amostra == 0 && nivelReuniao.value != 0 {
            nivelReuniao.send(amostra)
        }
    }
    var aoComecarReuniao: (() -> Void)?
    var aoAbrirReuniao: (() -> Void)?
    var aoEncerrarReuniao: (() -> Void)?
    var aoPausarReuniao: (() -> Void)?
    var aoEscolherArquivo: (() -> Void)?
    var aoAbrirDitase: (() -> Void)?
    private var encerrados: [String] = []

    func suspenderProgresso() { progresso = nil }

    func receber(_ evento: ProgressoDaPilula) {
        guard !encerrados.contains(evento.id) else { return }
        if evento.resultado != nil {
            guard progresso?.id == evento.id else { return }
            encerrados.append(evento.id)
            if encerrados.count > 32 { encerrados.removeFirst() }
        }
        progresso = evento
    }
    /// Últimos níveis do microfone, do mais antigo ao mais novo, de 0 a 1.
    /// Não é publicado de propósito: chega umas 20 vezes por segundo e quem desenha (as barras)
    /// lê o valor a cada quadro, então a pílula inteira não é recalculada a cada amostra.
    private(set) var niveis: [CGFloat] = Array(repeating: 0, count: ModeloDaPilula.barras)
    @Published var segundos: Int = 0
    @Published var sempreVisivel = true
    /// O ponteiro está sobre a pílula em repouso: ela cresce e vira o botão de ditar.
    @Published var sobre = false
    /// Nome curto da tecla de atalho, mostrado no botão ("⌥ direita").
    @Published var atalho = ""
    /// Clique na pílula em repouso: começa o ditado em mãos livres.
    var aoAlternar: (() -> Void)?
    var aoParar: (() -> Void)?
    var aoCancelar: (() -> Void)?

    static let barras = 9

    func empurrar(nivel: Float) {
        var novos = niveis
        novos.removeFirst()
        novos.append(CGFloat(nivel))
        niveis = novos
    }

    func zerarNiveis() { niveis = Array(repeating: 0, count: Self.barras) }
}

/// Valores de movimento da pílula. A especificação (gatilho, duração, curva e versão com
/// movimento reduzido de cada transição) está em `docs/motion.md`.
private enum Mov {
    /// Troca de estado: tamanho da cápsula, sem sobra.
    static let fase = Animation.spring(duration: 0.38, bounce: 0)
    /// O ponteiro chega: a marca cresce e vira o botão, com um resto de elasticidade.
    static let abrir = Animation.spring(duration: 0.34, bounce: 0.16)
    static let fechar = Animation.spring(duration: 0.26, bounce: 0)
    static let pressao = Animation.spring(duration: 0.16, bounce: 0)
    /// O conteúdo novo espera a cápsula começar a se mexer; o antigo sai antes.
    static let entrada = Animation.smooth(duration: 0.22).delay(0.07)
    static let saida = Animation.easeIn(duration: 0.1)
    /// Movimento reduzido: só um fade curto, sem deslocamento nem escala.
    static let reduzida = Animation.easeOut(duration: 0.12)
    static let reduzidaSaida = Animation.easeOut(duration: 0.08)

    static let alturaMarca: CGFloat = 8
    static let alturaBotao: CGFloat = 32
    static let alturaEstado: CGFloat = 30
}

/// Conteúdo que entra: nasce um pouco menor e desfocado, e se firma enquanto a cápsula cresce.
private struct Brotar: ViewModifier {
    let visivel: Double
    func body(content: Content) -> some View {
        content
            .opacity(visivel)
            .scaleEffect(0.86 + 0.14 * visivel)
            .blur(radius: (1 - visivel) * 3)
    }
}

private extension AnyTransition {
    static var brotar: AnyTransition {
        .asymmetric(
            insertion: AnyTransition.modifier(active: Brotar(visivel: 0), identity: Brotar(visivel: 1)).animation(Mov.entrada),
            removal: AnyTransition.opacity.animation(Mov.saida)
        )
    }

    static var surgir: AnyTransition {
        .asymmetric(insertion: AnyTransition.opacity.animation(Mov.reduzida.delay(0.05)), removal: AnyTransition.opacity.animation(Mov.reduzidaSaida))
    }
}

/// A cápsula escura na base da tela: mostra que o app está ouvindo, processando ou avisando.
///
/// A cápsula tem um tamanho por estado (`largura` e `altura`) e o conteúdo de cada estado entra e sai
/// por cima dela. Assim a troca de estado é um único movimento contínuo, sem salto de tamanho.
struct VistaDaPilula: View {
    @ObservedObject var modelo: ModeloDaPilula
    @Environment(\.accessibilityReduceMotion) private var semMovimentoDoSistema
    @State private var pressionado = false
    @State private var trilho = false
    /// Só para teste: força o modo de movimento reduzido sem mexer nos Ajustes do Sistema.
    private let forcarSemMovimento: Bool?

    init(modelo: ModeloDaPilula, forcarSemMovimento: Bool? = nil) {
        _modelo = ObservedObject(wrappedValue: modelo)
        self.forcarSemMovimento = forcarSemMovimento
    }

    private var semMovimento: Bool { forcarSemMovimento ?? semMovimentoDoSistema }

    var body: some View {
        ZStack(alignment: .bottom) {
            Color.clear
            pilula
                .scaleEffect(pressionado && !semMovimento ? 0.95 : 1)
                .animation(semMovimento ? nil : Mov.pressao, value: pressionado)
                .opacity(visivel ? 1 : 0)
                .padding(.bottom, 10)
        }
        .animation(semMovimento ? Mov.reduzida : Mov.fase, value: modelo.fase)
        .animation(semMovimento ? Mov.reduzida : Mov.fase, value: modelo.reuniao != nil)
        .animation(semMovimento ? nil : (modelo.sobre ? Mov.abrir : Mov.fechar), value: modelo.sobre)
        .onChange(of: modelo.fase) { _, _ in pressionado = false }
    }

    /// O conteúdo vive num palco de tamanho fixo, centrado na base; só a forma da cápsula (fundo,
    /// borda e máscara) muda de tamanho. Assim o conteúdo não é reposicionado durante o crescimento.
    private var pilula: some View {
        ZStack(alignment: .bottom) { conteudo }
            .frame(width: Self.palco, height: Mov.alturaBotao, alignment: .bottom)
            .mask(alignment: .bottom) { Capsule().frame(width: largura, height: altura) }
            .background(alignment: .bottom) {
                Capsule()
                    .fill(Color(white: botaoAberto ? 0.1 : 0.07))
                    .frame(width: largura, height: altura)
                    .shadow(color: .black.opacity(0.35), radius: 7, y: 3)
            }
            .overlay(alignment: .bottom) {
                Capsule()
                    .strokeBorder(Color.white.opacity(trilho ? 0.22 : botaoAberto ? 0.26 : 0.14), lineWidth: 1)
                    .animation(semMovimento ? nil : .easeOut(duration: 0.2), value: trilho)
                    .frame(width: largura, height: altura)
            }
            .overlay(alignment: .bottom) {
                ContornoDaPilula(evento: modelo.progresso, largura: largura, altura: altura,
                                 reduzido: semMovimento, trilho: $trilho)
            }
            .overlay(alignment: .bottom) {
                // Ditado (ou retranscrição) processando sem fração conhecida: o motor não manda
                // progresso, e o contorno acima só desenha com fração. O arco gira até a fase mudar.
                if modelo.fase == .processando && modelo.progresso == nil {
                    // Entrada em 120 ms e saída em 160 ms da posição atual (§2.5).
                    ArcoDeProcessamento(largura: largura, altura: altura, reduzido: semMovimento)
                        .transition(semMovimento ? .opacity.animation(.easeOut(duration: 0.12)) : .asymmetric(
                            insertion: .opacity.animation(.easeOut(duration: 0.12)),
                            removal: .opacity.animation(.easeIn(duration: 0.16))))
                }
            }
    }

    /// Largura do palco: a do maior aviso possível.
    private static let palco: CGFloat = 344

    private var reuniaoVisivel: CapturaDaPilula? { modelo.fase == .ociosa ? modelo.reuniao : nil }
    private var botaoAberto: Bool { modelo.fase == .ociosa && modelo.sobre && reuniaoVisivel == nil }

    private var visivel: Bool { modelo.fase != .ociosa || modelo.sempreVisivel || modelo.sobre || reuniaoVisivel != nil }

    private var altura: CGFloat {
        if reuniaoVisivel != nil { return modelo.sobre ? 30 : 26 }
        switch modelo.fase {
        case .ociosa: return botaoAberto ? Mov.alturaBotao : Mov.alturaMarca
        default: return Mov.alturaEstado
        }
    }

    private var largura: CGFloat {
        if let reuniaoVisivel {
            if modelo.sobre { return 140 }
            return reuniaoVisivel.motivoSemAudio == nil ? 88 : 160
        }
        switch modelo.fase {
        case .ociosa: return botaoAberto ? 176 : 40
        case .gravando: return 84
        case .processando: return 64
        case .maosLivres: return 172
        case .aviso(let texto): return Self.larguraDoAviso(texto)
        }
    }

    private static func larguraDoAviso(_ texto: String) -> CGFloat {
        let fonte = NSFont.systemFont(ofSize: 12, weight: .medium)
        let medido = ceil((texto as NSString).size(withAttributes: [.font: fonte]).width)
        return min(344, max(96, medido + 32))
    }

    private var entrada: AnyTransition { semMovimento ? .surgir : .brotar }

    @ViewBuilder private var conteudo: some View {
        if botaoAberto {
            HStack(spacing: 0) {
                BotaoDitar(modelo: modelo, pressionado: $pressionado)
                Rectangle().fill(.white.opacity(0.16)).frame(width: 1, height: 14).accessibilityHidden(true)
                Button { modelo.aoComecarReuniao?() } label: {
                    Label("Reunião", systemImage: "waveform.and.mic")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(.white)
                        .frame(width: 83, height: Mov.alturaBotao)
                        .contentShape(Rectangle())
                }
                .buttonStyle(RastrearPressao(pressionado: $pressionado))
                .accessibilityLabel("Começar a transcrever uma reunião")
                .help("Começar a transcrever uma reunião")
                .disabled(modelo.aoComecarReuniao == nil)
            }
                .contextMenu {
                    Button("Ditar em mãos livres") { modelo.aoAlternar?() }
                    Button("Começar reunião") { modelo.aoComecarReuniao?() }.disabled(modelo.aoComecarReuniao == nil)
                    Button("Transcrever arquivo…") { modelo.aoEscolherArquivo?() }.disabled(modelo.aoEscolherArquivo == nil)
                    Button("Abrir o Dita.se") { modelo.aoAbrirDitase?() }.disabled(modelo.aoAbrirDitase == nil)
                }
                .frame(height: Mov.alturaBotao)
                .transition(entrada)
        }
        if let reuniaoVisivel {
            ReuniaoNaPilula(captura: reuniaoVisivel, nivel: modelo.nivelReuniao, segundos: modelo.segundosReuniao,
                           sobre: modelo.sobre, reduzido: semMovimento,
                           abrir: modelo.aoAbrirReuniao, encerrar: modelo.aoEncerrarReuniao)
                .frame(height: altura)
                .contextMenu {
                    Button("Abrir a reunião") { modelo.aoAbrirReuniao?() }.disabled(modelo.aoAbrirReuniao == nil)
                    Button(reuniaoVisivel.pausada ? "Retomar reunião" : "Pausar reunião") { modelo.aoPausarReuniao?() }
                        .disabled(modelo.aoPausarReuniao == nil)
                    Button("Encerrar a reunião") { modelo.aoEncerrarReuniao?() }.disabled(modelo.aoEncerrarReuniao == nil)
                }
                .transition(entrada)
        }
        if modelo.fase == .gravando {
            Barras(modelo: modelo, semMovimento: semMovimento)
                .fixedSize()
                .frame(height: Mov.alturaEstado)
                .transition(entrada)
        }
        if modelo.fase == .maosLivres {
            ControlesMaosLivres(modelo: modelo, semMovimento: semMovimento)
                .fixedSize()
                .frame(height: Mov.alturaEstado)
                .transition(semMovimento ? .surgir : .asymmetric(insertion: AnyTransition.opacity.animation(Mov.entrada), removal: AnyTransition.opacity.animation(Mov.saida)))
        }
        if modelo.fase == .processando {
            Pontos(semMovimento: semMovimento)
                .fixedSize()
                .frame(height: Mov.alturaEstado)
                .accessibilityLabel(modelo.progresso?.etapa ?? "Processando")
                .accessibilityValue(modelo.progresso?.progresso.map { "\(Int(max(0, min(1, $0)) * 10) * 10)%" } ?? "")
                .transition(entrada)
        }
        if case .aviso(let texto) = modelo.fase {
            Text(texto)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.white)
                .lineLimit(1)
                .fixedSize()
                .frame(height: Mov.alturaEstado)
                .id(texto)
                .transition(entrada)
        }
    }
}

/// O botão em repouso. Ocupa a cápsula toda, então qualquer ponto dela é clicável.
private struct BotaoDitar: View {
    @ObservedObject var modelo: ModeloDaPilula
    @Binding var pressionado: Bool

    var body: some View {
        Button(action: { modelo.aoAlternar?() }) {
            HStack(spacing: 6) {
                Image(systemName: "mic.fill").font(.system(size: 11, weight: .semibold))
                Text("Ditar").font(.system(size: 12, weight: .semibold))
            }
            .foregroundStyle(.white)
            .fixedSize()
            .frame(width: 92, height: Mov.alturaBotao)
            .contentShape(Capsule())
        }
        .buttonStyle(RastrearPressao(pressionado: $pressionado))
        .accessibilityLabel("Começar a ditar em mãos livres")
        .help(modelo.atalho.isEmpty ? "Clique para ditar" : "Clique para ditar, ou segure \(modelo.atalho)")
    }
}

/// Estilo sem aparência própria: só avisa a pílula quando o botão está pressionado.
private struct RastrearPressao: ButtonStyle {
    @Binding var pressionado: Bool
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .onChange(of: configuration.isPressed) { _, novo in pressionado = novo }
    }
}

private struct ControlesMaosLivres: View {
    @ObservedObject var modelo: ModeloDaPilula
    let semMovimento: Bool
    @State private var entrou = false

    var body: some View {
        HStack(spacing: 10) {
            BotaoRedondo(simbolo: "xmark", rotulo: "Cancelar o ditado", semMovimento: semMovimento) { modelo.aoCancelar?() }
                .nascer(entrou, atraso: 0.05, semMovimento: semMovimento)
            Barras(modelo: modelo, semMovimento: semMovimento)
            Text(tempo)
                .font(.system(size: 11, weight: .medium).monospacedDigit())
                .foregroundStyle(Color.white.opacity(0.7))
                .lineLimit(1)
                .minimumScaleFactor(0.8)
                .frame(width: 34)
                .contentTransition(semMovimento ? .identity : .numericText())
                .animation(semMovimento ? nil : .smooth(duration: 0.25), value: modelo.segundos)
            BotaoRedondo(simbolo: "stop.fill", rotulo: "Terminar o ditado", destaque: true, semMovimento: semMovimento) { modelo.aoParar?() }
                .nascer(entrou, atraso: 0.09, semMovimento: semMovimento)
        }
        .onAppear { entrou = true }
    }

    private var tempo: String { String(format: "%d:%02d", modelo.segundos / 60, modelo.segundos % 60) }
}

private extension View {
    /// Os botões laterais nascem pequenos e depois da cápsula, um atrás do outro.
    func nascer(_ visivel: Bool, atraso: Double, semMovimento: Bool) -> some View {
        self
            .scaleEffect(semMovimento || visivel ? 1 : 0.5)
            .opacity(semMovimento || visivel ? 1 : 0)
            .animation(semMovimento ? nil : .spring(duration: 0.3, bounce: 0.12).delay(atraso), value: visivel)
    }
}

private struct BotaoRedondo: View {
    let simbolo: String
    let rotulo: String
    var destaque = false
    let semMovimento: Bool
    let acao: () -> Void

    var body: some View {
        Button(action: acao) {
            Image(systemName: simbolo)
                .font(.system(size: 9, weight: .bold))
                .frame(width: 20, height: 20)
        }
        .buttonStyle(EstiloRedondo(destaque: destaque, semMovimento: semMovimento))
        .accessibilityLabel(rotulo)
        .help(rotulo)
    }
}

private struct EstiloRedondo: ButtonStyle {
    let destaque: Bool
    let semMovimento: Bool
    @State private var sobre = false

    func makeBody(configuration: Configuration) -> some View {
        let pressionado = configuration.isPressed
        let base = destaque ? Color(red: 1, green: 0.27, blue: 0.23) : Color.white
        let fundo = destaque ? (pressionado ? 0.78 : sobre ? 1 : 0.92) : (pressionado ? 0.3 : sobre ? 0.22 : 0.14)
        return configuration.label
            .foregroundStyle(destaque ? Color.white : Color.white.opacity(sobre || pressionado ? 1 : 0.75))
            .background(Circle().fill(base.opacity(fundo)))
            .scaleEffect(pressionado && !semMovimento ? 0.86 : 1)
            .animation(semMovimento ? nil : .spring(duration: 0.16, bounce: 0), value: pressionado)
            .animation(semMovimento ? nil : .easeOut(duration: 0.12), value: sobre)
            .onHover { sobre = $0 }
    }
}

/// Barras da voz. O app manda o nível umas 20 vezes por segundo; aqui cada barra persegue o
/// valor com subida rápida e descida lenta, redesenhada a 60 quadros por segundo. A barra do
/// meio mostra o nível de agora e as vizinhas mostram o passado, então a voz "abre" a partir do centro.
/// A linha do tempo só existe enquanto esta vista está na tela (gravando).
private struct Barras: View {
    let modelo: ModeloDaPilula
    let semMovimento: Bool
    @State private var suavizador = SuavizadorDeBarras()

    private static let largura: CGFloat = 3
    private static let espaco: CGFloat = 3
    private static let minimo: CGFloat = 4
    private static let maximo: CGFloat = 20

    var body: some View {
        TimelineView(.animation(minimumInterval: semMovimento ? 0.05 : 1.0 / 60.0)) { linha in
            let agora = linha.date.timeIntervalSinceReferenceDate
            Canvas { contexto, tamanho in
                let alturas = suavizador.alturas(em: agora, niveis: modelo.niveis, suave: !semMovimento)
                for (i, v) in alturas.enumerated() {
                    let h = Self.minimo + (Self.maximo - Self.minimo) * v
                    let x = CGFloat(i) * (Self.largura + Self.espaco)
                    let retangulo = CGRect(x: x, y: (tamanho.height - h) / 2, width: Self.largura, height: h)
                    contexto.fill(Path(roundedRect: retangulo, cornerRadius: Self.largura / 2), with: .color(.white.opacity(0.55 + 0.45 * v)))
                }
            }
        }
        .frame(width: CGFloat(ModeloDaPilula.barras) * Self.largura + CGFloat(ModeloDaPilula.barras - 1) * Self.espaco, height: 22)
        .accessibilityElement()
        .accessibilityLabel("Ouvindo")
    }
}

/// Transforma o nível cru do microfone em altura de barra (0 a 1) sem tremor.
final class SuavizadorDeBarras {
    private var valores = [Double](repeating: 0, count: ModeloDaPilula.barras)
    private var anterior: TimeInterval?

    /// Quanto cada distância do centro pesa: o meio é o mais vivo.
    private static let pesos: [Double] = [1, 0.92, 0.78, 0.62, 0.48]

    /// Nível cru (0 a 1) em forma de barra: ignora o chiado de fundo e levanta a fala baixa.
    static func formatar(_ nivel: CGFloat) -> Double {
        let v = max(0, min(1, (Double(nivel) - 0.03) / 0.97))
        return pow(v, 0.65)
    }

    func alturas(em agora: TimeInterval, niveis: [CGFloat], suave: Bool) -> [Double] {
        let n = valores.count
        let centro = n / 2
        let dt = min(0.1, max(0, agora - (anterior ?? agora)))
        anterior = agora
        for i in 0..<n {
            let d = abs(i - centro)
            let indice = max(0, niveis.count - 1 - 2 * d)
            var alvo = niveis.isEmpty ? 0 : Self.formatar(niveis[min(indice, niveis.count - 1)]) * Self.pesos[min(d, Self.pesos.count - 1)]
            guard suave else {
                valores[i] = alvo
                continue
            }
            // Em silêncio a barra respira um pouco: mostra que o microfone está aberto.
            alvo = max(alvo, 0.045 * (0.5 + 0.5 * sin(agora * 2.4 + Double(d) * 0.7)))
            let tau = alvo > valores[i] ? 0.045 : 0.16 + 0.025 * Double(d)
            valores[i] += (alvo - valores[i]) * (1 - exp(-dt / tau))
        }
        return valores
    }
}

/// Três pontos em onda, enquanto o texto é transcrito. Só existe na tela durante o processamento.
private struct Pontos: View {
    let semMovimento: Bool
    @State private var pulsar = false

    var body: some View {
        Group {
            if semMovimento {
                HStack(spacing: 5) {
                    ForEach(0..<3, id: \.self) { _ in
                        Circle().fill(Color.white).frame(width: 5, height: 5)
                    }
                }
                .opacity(pulsar ? 1 : 0.45)
                .animation(.easeInOut(duration: 0.9).repeatForever(autoreverses: true), value: pulsar)
                .onAppear { pulsar = true }
            } else {
                TimelineView(.animation(minimumInterval: 1.0 / 30.0)) { linha in
                    let t = linha.date.timeIntervalSinceReferenceDate
                    Canvas { contexto, tamanho in
                        for i in 0..<3 {
                            let onda = pow(max(0, sin(t / 1.15 * 2 * .pi - Double(i) * 0.9)), 2)
                            let raio = 2.5 + 0.7 * onda
                            let x = tamanho.width / 2 + CGFloat(i - 1) * 10
                            let y = tamanho.height / 2 - 2.5 * onda
                            let retangulo = CGRect(x: x - raio, y: y - raio, width: raio * 2, height: raio * 2)
                            contexto.fill(Path(ellipseIn: retangulo), with: .color(.white.opacity(0.3 + 0.7 * onda)))
                        }
                    }
                }
            }
        }
        .frame(width: 30, height: 16)
        .accessibilityElement()
        .accessibilityLabel("Processando")
    }
}

/// Aceita o primeiro clique mesmo com outro app em primeiro plano.
private final class VistaClicavel<Conteudo: View>: NSHostingView<Conteudo> {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
}

/// Janela da pílula: sem borda, acima de tudo, em todos os espaços, sem tomar o foco do app em uso.
///
/// A janela é maior que a pílula e transparente. Para não bloquear cliques no que está atrás,
/// ela ignora o mouse o tempo todo e só passa a recebê-lo enquanto o ponteiro está em cima da
/// pílula (em repouso, para o botão de ditar; em mãos livres, para parar e cancelar).
final class JanelaDaPilula: NSPanel {
    let modelo = ModeloDaPilula()
    private static let tamanho = NSSize(width: 360, height: 64)
    private var monitores: [Any] = []

    init() {
        super.init(contentRect: NSRect(origin: .zero, size: Self.tamanho), styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        isOpaque = false
        backgroundColor = .clear
        hasShadow = false
        level = .statusBar
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        hidesOnDeactivate = false
        isMovable = false
        ignoresMouseEvents = true
        let vista = VistaClicavel(rootView: VistaDaPilula(modelo: modelo))
        vista.frame = NSRect(origin: .zero, size: Self.tamanho)
        contentView = vista
        posicionar()
        NotificationCenter.default.addObserver(self, selector: #selector(posicionar), name: NSApplication.didChangeScreenParametersNotification, object: nil)
        let tipos: NSEvent.EventTypeMask = [.mouseMoved, .leftMouseDragged]
        if let global = NSEvent.addGlobalMonitorForEvents(matching: tipos, handler: { [weak self] _ in self?.conferirPonteiro() }) { monitores.append(global) }
        if let local = NSEvent.addLocalMonitorForEvents(matching: tipos, handler: { [weak self] evento in
            self?.conferirPonteiro()
            return evento
        }) { monitores.append(local) }
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }

    /// Área da tela, em volta da pílula, em que ela reage ao mouse. Maior quando já está aberta,
    /// para não fechar com um tremor do ponteiro. Mantém folga sobre os tamanhos de `VistaDaPilula`
    /// (marca 40x8, botão 96x32, mãos livres 172x30).
    private var zona: NSRect? {
        let (largura, altura): (CGFloat, CGFloat)
        switch modelo.fase {
        case .ociosa:
            if let reuniao = modelo.reuniao {
                (largura, altura) = modelo.sobre ? (170, 50) : (reuniao.motivoSemAudio == nil ? 104 : 176, 46)
            } else {
                guard modelo.sempreVisivel || modelo.sobre else { return nil }
                (largura, altura) = modelo.sobre ? (196, 52) : (56, 24)
            }
        case .maosLivres: (largura, altura) = (190, 50)
        default: return nil
        }
        return NSRect(x: frame.midX - largura / 2, y: frame.minY + 2, width: largura, height: altura)
    }

    private func conferirPonteiro() {
        let dentro = zona.map { NSMouseInRect(NSEvent.mouseLocation, $0, false) } ?? false
        if ignoresMouseEvents == dentro { ignoresMouseEvents = !dentro }
        let sobre = dentro && modelo.fase == .ociosa
        if modelo.sobre != sobre { modelo.sobre = sobre }
    }

    /// Base da tela onde está o cursor, centralizada, logo acima do Dock.
    @objc func posicionar() {
        let tela = NSScreen.screens.first { NSMouseInRect(NSEvent.mouseLocation, $0.frame, false) } ?? NSScreen.main
        guard let area = tela?.visibleFrame else { return }
        setFrameOrigin(NSPoint(x: area.midX - Self.tamanho.width / 2, y: area.minY))
    }

    func mostrar(_ fase: ModeloDaPilula.Fase) {
        if fase != .ociosa, modelo.fase == .ociosa { posicionar() }
        modelo.fase = fase
        if fase != .ociosa { modelo.sobre = false }
        orderFrontRegardless()
        conferirPonteiro()
    }

    func mostrarReuniao(_ captura: CapturaDaPilula?) {
        let mudou = (captura == nil) != (modelo.reuniao == nil)
        modelo.receberReuniao(captura)
        if mudou {
            if captura != nil && modelo.fase == .ociosa { posicionar() }
            orderFrontRegardless()
            conferirPonteiro()
        }
    }

    /// Mostra um aviso curto e volta ao repouso.
    func avisar(_ texto: String, por segundos: Double = 2.2) {
        mostrar(.aviso(texto))
        DispatchQueue.main.asyncAfter(deadline: .now() + segundos) { [weak self] in
            guard let self, self.modelo.fase == .aviso(texto) else { return }
            self.mostrar(.ociosa)
        }
    }
}

private struct ReuniaoNaPilula: View {
    let captura: CapturaDaPilula
    let nivel: CurrentValueSubject<Double, Never>
    let segundos: CurrentValueSubject<Int, Never>
    let sobre: Bool
    let reduzido: Bool
    let abrir: (() -> Void)?
    let encerrar: (() -> Void)?
    @State private var entrou = false

    var body: some View {
        let alerta = captura.pausada || captura.motivoSemAudio != nil
        HStack(spacing: 6) {
            PontoDeCaptura(nivel: nivel, alerta: alerta, reduzido: reduzido)
                .frame(width: 7, height: 7)
                .accessibilityHidden(true)
            if let motivo = captura.motivoSemAudio, !sobre {
                Text(motivo).foregroundStyle(.white).font(.system(size: 11)).lineLimit(1)
            } else {
                RelogioDaReuniao(segundos: segundos, reduzido: reduzido)
            }
            if sobre {
                Button("Abrir") { abrir?() }
                    .buttonStyle(.plain)
                    .foregroundStyle(.white)
                    .font(.system(size: 11, weight: .semibold))
                    .help("Abrir a transcrição da reunião no Dita.se")
                    .nascer(entrou, atraso: 0.05, semMovimento: reduzido)
                BotaoRedondo(simbolo: "stop.fill", rotulo: "Encerrar a reunião", destaque: true, semMovimento: reduzido) { encerrar?() }
                    .nascer(entrou, atraso: 0.09, semMovimento: reduzido)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(captura.pausada ? "Reunião pausada" : "Reunião em andamento")
        .onAppear { entrou = sobre }
        .onChange(of: sobre) { _, valor in entrou = valor }
    }
}

private struct RelogioDaReuniao: View {
    let segundos: CurrentValueSubject<Int, Never>
    let reduzido: Bool
    @State private var valor = 0

    private var relogio: String {
        let s = max(0, valor)
        if s >= 3600 { return String(format: "%d:%02d:%02d", s / 3600, s / 60 % 60, s % 60) }
        return String(format: "%d:%02d", s / 60, s % 60)
    }

    var body: some View {
        Text(relogio)
            .font(.system(size: 11, weight: .medium).monospacedDigit())
            .foregroundStyle(.white.opacity(0.7))
            .frame(width: 40)
            .minimumScaleFactor(0.8)
            // Sem transição numérica: animar os dígitos a cada segundo custava ~1,7% de CPU em horas de reunião.
            .onReceive(segundos) { novos in
                guard novos != valor else { return }
                var semAnimacao = Transaction()
                semAnimacao.disablesAnimations = true
                withTransaction(semAnimacao) { valor = novos }
            }
    }
}

/// O nível atualiza só a opacidade da camada. O relógio e o restante da vista ficam intactos.
private struct PontoDeCaptura: NSViewRepresentable {
    let nivel: CurrentValueSubject<Double, Never>
    let alerta: Bool
    let reduzido: Bool

    func makeNSView(context: Context) -> Marcador {
        let marcador = Marcador()
        marcador.ligar(nivel)
        return marcador
    }

    func updateNSView(_ view: Marcador, context: Context) {
        view.configurar(alerta: alerta, reduzido: reduzido, nivel: nivel.value)
    }

    final class Marcador: NSView {
        private var assinante: AnyCancellable?
        private var alerta = false
        private var reduzido = false
        private var suavizado = 0.0
        private var ultimaAtualizacao = ContinuousClock.now

        init() {
            super.init(frame: .zero)
            wantsLayer = true
            layer?.cornerRadius = 3.5
        }

        required init?(coder: NSCoder) { nil }

        func ligar(_ nivel: CurrentValueSubject<Double, Never>) {
            assinante = nivel.sink { [weak self] amostra in self?.atualizar(amostra) }
        }

        func configurar(alerta: Bool, reduzido: Bool, nivel: Double) {
            guard self.alerta != alerta || self.reduzido != reduzido || layer?.backgroundColor == nil else { return }
            self.alerta = alerta
            self.reduzido = reduzido
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            layer?.backgroundColor = alerta ? NSColor(srgbRed: 1, green: 159 / 255, blue: 10 / 255, alpha: 1).cgColor
                : NSColor(srgbRed: 1, green: 69 / 255, blue: 58 / 255, alpha: 1).cgColor
            layer?.removeAnimation(forKey: "nivel")
            layer?.opacity = alerta || reduzido ? 1 : Float(0.5 + nivel * 0.5)
            CATransaction.commit()
        }

        private func atualizar(_ nivel: Double) {
            guard !alerta, !reduzido, let layer else { return }
            let agora = ContinuousClock.now
            let intervalo = ultimaAtualizacao.duration(to: agora)
            guard intervalo >= .milliseconds(85) else { return }
            ultimaAtualizacao = agora
            let dt = Double(intervalo.components.seconds) + Double(intervalo.components.attoseconds) / 1e18
            let tau = nivel >= suavizado ? 0.1 : 0.5
            suavizado += (nivel - suavizado) * (1 - exp(-dt / tau))
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            layer.opacity = Float(0.5 + suavizado * 0.5)
            CATransaction.commit()
        }
    }
}
