import Foundation

/// Um trecho de fala recortado do microfone, pronto para o reconhecedor.
public struct Segmento: Sendable {
    /// Posição da primeira amostra na gravação inteira.
    public let inicio: Int
    public let amostras: [Float]
    /// Segundos de silêncio antes deste trecho. Pausa longa indica mudança de assunto.
    public let pausaAntes: Double
    /// O trecho foi cortado por tamanho, sem pausa: a frase continua no próximo.
    public let cortadoNoMeio: Bool

    public var duracao: Double { Double(amostras.count) / 16_000 }
}

/// Corta o áudio em trechos nas pausas da fala, enquanto a pessoa ainda dita.
///
/// O reconhecedor trabalha em janelas de 15 segundos. Cortar nas pausas mantém cada
/// trecho dentro de uma janela, sem emenda no meio de palavra, e permite transcrever
/// durante a gravação: ao soltar a tecla só falta o último trecho.
public struct Segmentador: Sendable {
    public struct Config: Sendable {
        public var taxa = 16_000
        /// Pausa que fecha um trecho que já tem o tamanho mínimo.
        public var pausaCorte = 0.55
        /// Pausa que fecha qualquer trecho, mesmo curto.
        public var pausaLonga = 1.1
        public var minimoParaCorte = 5.0
        /// Limite do trecho. Acima disso corta no ponto mais quieto dos últimos segundos.
        public var maximo = 13.5
        /// Silêncio mantido nas pontas para o reconhecedor não comer a primeira sílaba.
        public var margem = 0.2
        public var falaMinima = 0.16
        public init() {}
    }

    static let quadro = 320 // 20 ms a 16 kHz
    static let limiarTeto: Float = 0.008
    static let limiarPiso: Float = 0.0018

    let config: Config
    private var resto: [Float] = []
    private var atual: [Float] = []
    private var niveisDoAtual: [Float] = []
    private var preRolo: [Float] = []
    private var emFala = false
    private var quadrosDeFala = 0
    private var quadrosQuietos = 0
    private var quadrosParado = 0
    private var posicao = 0
    private var inicioDoAtual = 0
    private var pausaAntesDoAtual = 0.0
    private var pico: Float = 0
    private var continuaDoAnterior = false

    /// Nível do último quadro, de 0 a 1, para a animação da pílula.
    public private(set) var nivel: Float = 0
    /// Segundos seguidos sem fala. Serve para encerrar gravação esquecida ligada.
    public var silencioCorrido: Double { emFala ? Double(quadrosQuietos) * 0.02 : Double(quadrosParado) * 0.02 }
    /// Segundos de fala detectados desde o começo.
    public private(set) var falaTotal = 0.0

    public init(config: Config = Config()) { self.config = config }

    private var limiar: Float { min(Self.limiarTeto, max(Self.limiarPiso, pico * 0.1)) }

    public mutating func alimentar(_ novas: [Float]) -> [Segmento] {
        var prontos: [Segmento] = []
        resto.append(contentsOf: novas)
        var i = 0
        while resto.count - i >= Self.quadro {
            if let s = processar(quadro: resto[i..<(i + Self.quadro)]) { prontos.append(s) }
            i += Self.quadro
        }
        resto.removeFirst(i)
        return prontos
    }

    public mutating func finalizar() -> Segmento? {
        if !resto.isEmpty {
            if emFala { atual.append(contentsOf: resto) }
            posicao += resto.count
            resto.removeAll()
        }
        defer { reiniciarTrecho() }
        guard emFala, Double(quadrosDeFala) * 0.02 >= config.falaMinima else { return nil }
        return fechar(ate: atual.count, cortado: false)
    }

    private mutating func processar(quadro: ArraySlice<Float>) -> Segmento? {
        var soma: Float = 0
        for a in quadro { soma += a * a }
        let rms = (soma / Float(Self.quadro)).squareRoot()
        pico = max(pico * 0.9995, rms)
        nivel = min(1, rms * 12)
        let fala = rms >= limiar
        defer { posicao += Self.quadro }

        guard emFala else {
            if fala {
                emFala = true
                inicioDoAtual = posicao - preRolo.count
                pausaAntesDoAtual = continuaDoAnterior ? 0 : Double(quadrosParado) * 0.02
                atual = preRolo + quadro
                niveisDoAtual = Array(repeating: 0, count: preRolo.count / Self.quadro) + [rms]
                preRolo.removeAll(keepingCapacity: true)
                quadrosDeFala = 1
                quadrosQuietos = 0
                falaTotal += 0.02
            } else {
                quadrosParado += 1
                preRolo.append(contentsOf: quadro)
                let teto = Int(config.margem * Double(config.taxa))
                if preRolo.count > teto { preRolo.removeFirst(preRolo.count - teto) }
            }
            return nil
        }

        atual.append(contentsOf: quadro)
        niveisDoAtual.append(rms)
        if fala {
            quadrosDeFala += 1
            quadrosQuietos = 0
            falaTotal += 0.02
        } else {
            quadrosQuietos += 1
        }
        let duracao = Double(atual.count) / Double(config.taxa)
        let quieto = Double(quadrosQuietos) * 0.02

        if quieto >= config.pausaLonga || (quieto >= config.pausaCorte && duracao >= config.minimoParaCorte) {
            let sobra = max(0, quadrosQuietos * Self.quadro - Int(config.margem * Double(config.taxa)))
            let valido = Double(quadrosDeFala) * 0.02 >= config.falaMinima
            let segmento = valido ? fechar(ate: atual.count - sobra, cortado: false) : nil
            let parado = quadrosQuietos
            reiniciarTrecho()
            quadrosParado = parado
            return segmento
        }
        if duracao >= config.maximo {
            let corte = pontoMaisQuieto()
            let segmento = fechar(ate: corte, cortado: true)
            let sobra = Array(atual[corte...])
            let niveisSobra = Array(niveisDoAtual[(corte / Self.quadro)...])
            let novoInicio = inicioDoAtual + corte
            reiniciarTrecho()
            emFala = true
            continuaDoAnterior = true
            atual = sobra
            niveisDoAtual = niveisSobra
            inicioDoAtual = novoInicio
            pausaAntesDoAtual = 0
            quadrosDeFala = niveisSobra.filter { $0 >= limiar }.count
            return segmento
        }
        return nil
    }

    /// O meio da janela mais quieta de 100 ms nos últimos 4 segundos do trecho.
    private func pontoMaisQuieto() -> Int {
        let total = niveisDoAtual.count
        let janela = 5
        let inicioBusca = max(janela, total - 200)
        let fimBusca = total - 10
        guard fimBusca > inicioBusca else { return (total - 1) * Self.quadro }
        var melhor = inicioBusca
        var menor = Float.greatestFiniteMagnitude
        for i in inicioBusca..<fimBusca {
            var soma: Float = 0
            for j in (i - janela / 2)...(i + janela / 2) { soma += niveisDoAtual[j] }
            if soma < menor {
                menor = soma
                melhor = i
            }
        }
        return melhor * Self.quadro
    }

    private func fechar(ate fim: Int, cortado: Bool) -> Segmento {
        Segmento(inicio: inicioDoAtual, amostras: Array(atual[..<max(0, min(fim, atual.count))]), pausaAntes: pausaAntesDoAtual, cortadoNoMeio: cortado)
    }

    private mutating func reiniciarTrecho() {
        emFala = false
        continuaDoAnterior = false
        atual.removeAll(keepingCapacity: true)
        niveisDoAtual.removeAll(keepingCapacity: true)
        preRolo.removeAll(keepingCapacity: true)
        quadrosDeFala = 0
        quadrosQuietos = 0
        quadrosParado = 0
    }
}
