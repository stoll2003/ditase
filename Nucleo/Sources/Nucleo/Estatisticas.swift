import Foundation

/// Números da página de Insights, calculados do histórico.
public struct Estatisticas: Codable, Equatable, Sendable {
    public struct Dia: Codable, Equatable, Sendable {
        /// AAAA-MM-DD no fuso do usuário.
        public var dia: String
        public var palavras: Int
        public var ditados: Int
        public var minutos: Double
    }

    public struct App: Codable, Equatable, Sendable {
        public var app: String
        public var nome: String
        public var palavras: Int
        public var ditados: Int
    }

    public var totalPalavras = 0
    public var totalDitados = 0
    /// Minutos de fala.
    public var minutosFalados = 0.0
    /// Palavras por minuto de fala, média ponderada pelo tempo.
    public var ppmMedio = 0.0
    public var ppmDeCadaDitado: [Double] = []
    /// Minutos economizados em relação a digitar na velocidade de referência.
    public var minutosEconomizados = 0.0
    public var diasSeguidos = 0
    public var maiorSequencia = 0
    public var diasAtivos = 0
    public var palavrasHoje = 0
    public var palavrasNaSemana = 0
    public var palavrasNaSemanaAnterior = 0
    public var maiorDitadoPalavras = 0
    public var maiorDitadoMinutos = 0.0
    /// Mediana, em segundos, da espera entre soltar a tecla e o texto pronto (só ditados deste app).
    public var latenciaMediana = 0.0
    public var trocasDicionario = 0
    public var porDia: [Dia] = []
    /// 24 posições, palavras por hora do dia.
    public var porHora: [Int] = Array(repeating: 0, count: 24)
    /// 7 posições, segunda a domingo.
    public var porDiaDaSemana: [Int] = Array(repeating: 0, count: 7)
    public var porApp: [App] = []

    public init() {}

    /// Velocidade de digitação usada para estimar o tempo economizado.
    public static let ppmDigitando = 40.0

    public static func calcular(_ ditados: [Ditado], agora: Date = Date(), calendario: Calendar = .current, diasNoGrafico: Int = 30) -> Estatisticas {
        var e = Estatisticas()
        let formato = DateFormatter()
        formato.calendar = calendario
        formato.timeZone = calendario.timeZone
        formato.locale = Locale(identifier: "en_US_POSIX")
        formato.dateFormat = "yyyy-MM-dd"

        var dias: [String: Dia] = [:]
        var apps: [String: App] = [:]
        var segundosDeFala = 0.0
        var palavrasComTempo = 0
        var latencias: [Double] = []
        let hoje = calendario.startOfDay(for: agora)
        let inicioDaSemana = calendario.date(byAdding: .day, value: -6, to: hoje) ?? hoje
        let inicioDaSemanaAnterior = calendario.date(byAdding: .day, value: -13, to: hoje) ?? hoje

        for d in ditados where d.palavras > 0 {
            let data = Date(timeIntervalSince1970: d.iniciadoEm)
            let fala = d.duracaoFala > 0 ? d.duracaoFala : d.duracao
            e.totalPalavras += d.palavras
            e.totalDitados += 1
            e.trocasDicionario += d.trocasDicionario
            if fala >= 2 {
                segundosDeFala += fala
                palavrasComTempo += d.palavras
                let ppm = Double(d.palavras) / (fala / 60)
                if ppm > 20, ppm < 400 { e.ppmDeCadaDitado.append(ppm.rounded()) }
            }
            if d.origem == "ditado", d.latencia > 0 { latencias.append(d.latencia) }
            if d.palavras > e.maiorDitadoPalavras {
                e.maiorDitadoPalavras = d.palavras
                e.maiorDitadoMinutos = d.duracao / 60
            }
            let chave = formato.string(from: data)
            var dia = dias[chave] ?? Dia(dia: chave, palavras: 0, ditados: 0, minutos: 0)
            dia.palavras += d.palavras
            dia.ditados += 1
            dia.minutos += fala / 60
            dias[chave] = dia
            let partes = calendario.dateComponents([.hour, .weekday], from: data)
            e.porHora[partes.hour ?? 0] += d.palavras
            // Calendar: domingo = 1. Aqui segunda = 0.
            e.porDiaDaSemana[((partes.weekday ?? 2) + 5) % 7] += d.palavras
            let idApp = d.app.isEmpty ? "desconhecido" : d.app
            var app = apps[idApp] ?? App(app: idApp, nome: d.appNome.isEmpty ? idApp : d.appNome, palavras: 0, ditados: 0)
            app.palavras += d.palavras
            app.ditados += 1
            if !d.appNome.isEmpty { app.nome = d.appNome }
            apps[idApp] = app
            if data >= hoje { e.palavrasHoje += d.palavras }
            if data >= inicioDaSemana { e.palavrasNaSemana += d.palavras }
            else if data >= inicioDaSemanaAnterior { e.palavrasNaSemanaAnterior += d.palavras }
        }

        e.minutosFalados = segundosDeFala / 60
        e.ppmMedio = segundosDeFala > 0 ? Double(palavrasComTempo) / (segundosDeFala / 60) : 0
        e.minutosEconomizados = max(0, Double(e.totalPalavras) / ppmDigitando - e.minutosFalados)
        e.diasAtivos = dias.count
        latencias.sort()
        e.latenciaMediana = latencias.isEmpty ? 0 : latencias[latencias.count / 2]
        e.porApp = apps.values.sorted { $0.palavras > $1.palavras }

        // Gráfico: os últimos N dias, inclusive os vazios.
        for recuo in stride(from: diasNoGrafico - 1, through: 0, by: -1) {
            guard let data = calendario.date(byAdding: .day, value: -recuo, to: hoje) else { continue }
            let chave = formato.string(from: data)
            e.porDia.append(dias[chave] ?? Dia(dia: chave, palavras: 0, ditados: 0, minutos: 0))
        }

        // Sequência atual: dias seguidos até hoje (ou até ontem, se hoje ainda não houve ditado).
        var cursor = hoje
        if dias[formato.string(from: cursor)] == nil, let ontem = calendario.date(byAdding: .day, value: -1, to: cursor) { cursor = ontem }
        while dias[formato.string(from: cursor)] != nil {
            e.diasSeguidos += 1
            guard let antes = calendario.date(byAdding: .day, value: -1, to: cursor) else { break }
            cursor = antes
        }
        // Maior sequência do histórico.
        let ordenados = dias.keys.sorted().compactMap { formato.date(from: $0) }
        var corrida = 0
        var anterior: Date?
        for data in ordenados {
            if let a = anterior, let esperado = calendario.date(byAdding: .day, value: 1, to: a), calendario.isDate(esperado, inSameDayAs: data) { corrida += 1 }
            else { corrida = 1 }
            e.maiorSequencia = max(e.maiorSequencia, corrida)
            anterior = data
        }
        return e
    }
}
