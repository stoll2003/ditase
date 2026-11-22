import SwiftUI

/// Projeção local dos eventos do motor. A fase da pílula pode mudar antes do resultado.
struct ProgressoDaPilula: Equatable {
    enum Resultado: Equatable { case concluido, cancelado, erro(String) }
    let id: String
    let progresso: Double?
    let etapa: String
    let estimativa: Bool
    let longo: Bool
    var resultado: Resultado? = nil
}

struct CapturaDaPilula: Equatable {
    var pausada = false
    var segundos = 0
    var nivel = 0.0
    var motivoSemAudio: String?
}

/// Começa às 12h e segue para a direita, com semicírculos reais nas extremidades.
private struct CapsulaHoraria: Shape {
    func path(in rect: CGRect) -> Path {
        let r = min(rect.width, rect.height) / 2
        let direita = rect.maxX - r
        let esquerda = rect.minX + r
        var caminho = Path()
        caminho.move(to: CGPoint(x: rect.midX, y: rect.minY))
        caminho.addLine(to: CGPoint(x: direita, y: rect.minY))
        caminho.addArc(center: CGPoint(x: direita, y: rect.midY), radius: r,
                       startAngle: .degrees(-90), endAngle: .degrees(90), clockwise: false)
        caminho.addLine(to: CGPoint(x: esquerda, y: rect.maxY))
        caminho.addArc(center: CGPoint(x: esquerda, y: rect.midY), radius: r,
                       startAngle: .degrees(90), endAngle: .degrees(270), clockwise: false)
        caminho.addLine(to: CGPoint(x: rect.midX, y: rect.minY))
        return caminho
    }
}

struct ContornoDaPilula: View {
    let evento: ProgressoDaPilula?
    let largura: CGFloat
    let altura: CGFloat
    let reduzido: Bool
    @Binding var trilho: Bool
    @State private var trabalho: String?
    @State private var alvo = 0.0
    @State private var opacidade = 0.0
    @State private var ultimo: ContinuousClock.Instant?
    @State private var inicio: ContinuousClock.Instant?
    @State private var tarefa: Task<Void, Never>?
    @State private var semEstimativa = false
    @State private var vermelho = false
    @State private var halo = false
    private static let azul = Color(red: 10 / 255, green: 132 / 255, blue: 1)
    private static let corErro = Color(red: 1, green: 69 / 255, blue: 58 / 255)
    private static let suavizar = Animation.timingCurve(0.23, 1, 0.32, 1, duration: 0.4)

    var body: some View {
        ZStack {
            if trilho {
                let cor = vermelho ? Self.corErro : Self.azul
                let raio = max(0, altura - 2) / 2
                let perimetro = max(1, 2 * max(0, largura - altura) + 2 * .pi * raio)
                let inicioDaPonta = max(0, alvo - 12 / perimetro)
                let pontaAtiva = semEstimativa && !reduzido
                CapsulaHoraria().trim(from: 0, to: pontaAtiva ? inicioDaPonta : alvo)
                    .stroke(cor, style: StrokeStyle(lineWidth: 2, lineCap: .round))
                    .padding(1)
                if pontaAtiva {
                    PontaDeProgresso(inicio: inicioDaPonta, fim: alvo, cor: cor)
                        .padding(1)
                }
                if halo {
                    Capsule().strokeBorder(Self.azul.opacity(0.35), lineWidth: 1)
                        .shadow(color: Self.azul.opacity(0.55), radius: 7)
                        .transition(.opacity)
                }
            }
        }
        .frame(width: largura, height: altura)
        .opacity(opacidade)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
        .onChange(of: evento, initial: true) { _, novo in atualizar(novo) }
        .onDisappear { tarefa?.cancel() }
    }

    private func atualizar(_ novo: ProgressoDaPilula?) {
        guard let novo else {
            tarefa?.cancel()
            limpar()
            trabalho = nil
            return
        }
        let agora = ContinuousClock.now
        if novo.id != trabalho {
            tarefa?.cancel()
            limpar()
            trabalho = novo.id
            inicio = agora
        }
        if let resultado = novo.resultado {
            tarefa?.cancel()
            guard trilho else { return }
            semEstimativa = false
            tarefa = Task { @MainActor in await terminar(resultado) }
            return
        }

        let dif = ultimo.map { Double($0.duration(to: agora).components.attoseconds) / 1e18 + Double($0.duration(to: agora).components.seconds) }
        ultimo = agora
        semEstimativa = !novo.estimativa || novo.progresso == nil
        guard let fracao = novo.progresso, fracao.isFinite else { return }
        let valor = min(1, max(0, fracao))
        if trilho && valor < alvo - 0.02 {
            tarefa?.cancel()
            tarefa = Task { @MainActor in
                withAnimation(reduzido ? nil : .easeIn(duration: 0.2)) { alvo = 0 }
                guard await esperar(reduzido ? 0 : 0.2) else { return }
                limpar()
                iniciarTraco(valor)
            }
            return
        }
        guard valor >= 0.015 else { return }
        if trilho {
            withAnimation(reduzido ? nil : .linear(duration: min(1.2, max(0.2, dif ?? 0.4)))) { alvo = max(alvo, valor) }
        } else {
            tarefa?.cancel()
            let decorrido = inicio.map { $0.duration(to: agora) } ?? .zero
            let faltam = novo.longo ? Duration.zero : max(.zero, .milliseconds(800) - decorrido)
            tarefa = Task { @MainActor in
                do { try await Task.sleep(for: faltam) } catch { return }
                iniciarTraco(valor)
            }
        }
    }

    private func iniciarTraco(_ valor: Double) {
        guard valor >= 0.015 else { return }
        withAnimation(reduzido ? nil : .easeOut(duration: 0.15)) { trilho = true; opacidade = 1 }
        withAnimation(reduzido ? nil : Self.suavizar) { alvo = valor }
    }

    private func terminar(_ resultado: ProgressoDaPilula.Resultado) async {
        switch resultado {
        case .concluido:
            withAnimation(reduzido ? nil : .timingCurve(0.23, 1, 0.32, 1, duration: 0.25)) { alvo = 1 }
            if reduzido {
                guard await esperar(0.3) else { return }
            } else {
                guard await esperar(0.25) else { return }
                withAnimation(.easeOut(duration: 0.18)) { halo = true }
                guard await esperar(0.3) else { return }
                withAnimation(.easeIn(duration: 0.25)) { halo = false }
            }
        case .cancelado:
            if !reduzido {
                let duracao = min(0.3, max(0.12, alvo * 0.35))
                withAnimation(.timingCurve(0.77, 0, 0.175, 1, duration: duracao)) { alvo = 0 }
                guard await esperar(duracao) else { return }
            }
        case .erro:
            withAnimation(reduzido ? nil : .easeInOut(duration: 0.2)) { vermelho = true }
            guard await esperar(0.5) else { return }
        }
        let fade = reduzido ? 0.12 : resultado == .cancelado ? 0.1 : 0.25
        withAnimation(.easeOut(duration: fade)) { opacidade = 0 }
        guard await esperar(fade) else { return }
        limpar()
    }

    private func limpar() {
        trilho = false
        alvo = 0
        opacidade = 0
        ultimo = nil
        semEstimativa = false
        vermelho = false
        halo = false
    }

    private func esperar(_ segundos: Double) async -> Bool {
        do { try await Task.sleep(for: .seconds(segundos)); return !Task.isCancelled }
        catch { return false }
    }
}

private struct PontaDeProgresso: View {
    let inicio: Double
    let fim: Double
    let cor: Color
    @State private var respirar = false

    var body: some View {
        CapsulaHoraria().trim(from: inicio, to: fim)
            .stroke(cor, style: StrokeStyle(lineWidth: 2, lineCap: .round))
            .opacity(respirar ? 0.45 : 1)
            .animation(.easeInOut(duration: 0.6).repeatForever(autoreverses: true), value: respirar)
            .onAppear { respirar = true }
    }
}

/// Segmento azul do processamento sem fração conhecida (ditado e retranscrição), pela §2.5 de
/// SPEC-windows-identidade.md, a mesma regra do Windows: 22% do perímetro fechado da cápsula,
/// 2 pt, pontas redondas, uma volta linear a cada 1,2 s no sentido horário a partir do centro do
/// topo, enquanto durar o estado real. Sem estimativa nem percentual. Com redução de movimento,
/// o perímetro inteiro fica azul parado, 1,5 pt a 70%.
struct ArcoDeProcessamento: View {
    let largura: CGFloat
    let altura: CGFloat
    let reduzido: Bool
    @State private var inicio = Date()
    static let comprimento = 0.22
    static let volta = 1.2
    private static let azul = Color(red: 10 / 255, green: 132 / 255, blue: 1)

    /// Início e fim do segmento no instante `t` (segundos), em frações do contorno. Passando de 1,
    /// o segmento vira dois trechos para atravessar o ponto das 12h sem salto.
    static func trechos(em t: Double) -> [(Double, Double)] {
        let inicio = (t / volta).truncatingRemainder(dividingBy: 1)
        let fim = inicio + comprimento
        return fim <= 1 ? [(inicio, fim)] : [(inicio, 1), (0, fim - 1)]
    }

    var body: some View {
        Group {
            if reduzido {
                CapsulaHoraria()
                    .stroke(Self.azul, style: StrokeStyle(lineWidth: 1.5, lineCap: .round))
                    .opacity(0.7)
            } else {
                // TimelineView(.animation) só existe enquanto este arco está na tela (processando).
                TimelineView(.animation) { contexto in
                    let t = max(0, contexto.date.timeIntervalSince(inicio))
                    ZStack {
                        ForEach(Array(Self.trechos(em: t).enumerated()), id: \.offset) { _, trecho in
                            CapsulaHoraria().trim(from: trecho.0, to: trecho.1)
                                .stroke(Self.azul, style: StrokeStyle(lineWidth: 2, lineCap: .round))
                        }
                    }
                }
            }
        }
        .padding(1)
        .frame(width: largura, height: altura)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}
