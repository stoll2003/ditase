import Foundation

public struct FaixaDeVoz: Codable, Equatable, Sendable {
    public let voz: String
    public let inicio: Double
    public let fim: Double

    public init(voz: String, inicio: Double, fim: Double) {
        self.voz = voz
        self.inicio = inicio
        self.fim = fim
    }
}

public struct TrechoLongo: Codable, Equatable, Sendable {
    public var id = UUID().uuidString
    public var inicio: Double
    public var fim: Double
    public var voz: String
    public var texto: String
    public var parcial: Bool
    public var vozManual: Bool?
    /// "microfone" quando veio da fonte local da reunião. Ausente para o áudio do Mac e arquivos.
    public var origem: String?
    /// Fala preservada de duas origens divergentes na migração.
    public var divergente: Bool?

    public init(inicio: Double, fim: Double, voz: String, texto: String, parcial: Bool = false) {
        self.inicio = inicio
        self.fim = fim
        self.voz = voz
        self.texto = texto
        self.parcial = parcial
    }

    public var doMicrofone: Bool { origem == "microfone" || (origem == nil && voz == "voce" && vozManual != true) }
}

public struct VozLonga: Codable, Equatable, Sendable {
    public var id: String
    public var nome: String
    public var nomeManual: Bool?

    public init(id: String, nome: String, nomeManual: Bool? = nil) {
        self.id = id
        self.nome = nome
        self.nomeManual = nomeManual
    }
}

public struct TranscricaoLonga: Codable, Equatable, Sendable {
    public var id = UUID().uuidString
    public var nome: String
    public var criadoEm = Date().timeIntervalSince1970
    public var duracao: Double?
    public var vozes: [VozLonga] = []
    public var trechos: [TrechoLongo] = []
    public var baixaConfianca = false
    public var bruto: String?
    public var fase: String
    public var progresso: Double?
    public var motivo: String?
    public var processamento: Double?
    public var fonte: String?
    public var atraso: Double?
    public var resumo: [String]?
    public var resumoGerado: Bool?
    public var oculto: Bool?
    public var erroCaptura: String?
    public var erroProcessamento: String?
    public var avisoCaptura: String?
    public var erroResumo: String?
    public var resumoCancelado: Bool?
    public var unioesDeVoz: [String: String]?
    /// Só nas listas leves: contagem de palavras e primeira fala, sem carregar os trechos.
    public var palavras: Int?
    public var previa: String?
    /// Modelo novo: faixas sem áudio (lacuna) e faixas com áudio guardado ainda sem texto (pendente).
    public var lacunas: [PeriodoDeAudio]?
    public var pendentes: [PeriodoDeAudio]?
    /// "aoVivo" (Você/Chamada), "pendente", "identificados", "indisponivel" ou "naoAplica".
    public var falantes: String?
    public var etapa: String?
    public var aviso: String?
    /// Segundos de áudio guardado à espera de transcrição.
    public var segundosPendentes: Double?

    public init(nome: String, fase: String, fonte: String? = nil) {
        self.nome = nome
        self.fase = fase
        self.fonte = fonte
    }

    public mutating func marcarFalhaResumo() {
        erroResumo = "O resumo local falhou. A transcrição foi salva."
        resumoCancelado = nil
    }

    public mutating func atualizarVozes() {
        if let unioesDeVoz {
            let destinos = Set(unioesDeVoz.values)
            for i in trechos.indices {
                let voz = trechos[i].voz
                if let destino = unioesDeVoz[voz] { trechos[i].voz = destino }
                if destinos.contains(trechos[i].voz) { trechos[i].vozManual = true }
            }
        }
        let anteriores = Dictionary(uniqueKeysWithValues: vozes.map { ($0.id, $0) })
        var vistos = Set<String>()
        vozes = trechos.compactMap {
            let id = $0.voz
            guard vistos.insert(id).inserted else { return nil }
            let nome = id == "voce" ? "Você" : id == "desconhecida" ? "Voz não identificada"
                : "Pessoa " + String(vistos.subtracting(["voce", "desconhecida"]).count)
            return anteriores[id] ?? VozLonga(id: id, nome: nome)
        }
    }

    @discardableResult
    public mutating func renomear(voz: String, nome: String) -> Bool {
        guard let i = vozes.firstIndex(where: { $0.id == voz }) else { return false }
        let limpo = nome.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !limpo.isEmpty else { return false }
        let nomeFinal = String(limpo.prefix(120))
        if let destino = vozes.first(where: {
            $0.id != voz && $0.nome.compare(nomeFinal, options: [.caseInsensitive, .diacriticInsensitive], locale: Locale(identifier: "pt_BR")) == .orderedSame
        }) {
            var unioes = unioesDeVoz ?? [:]
            for origem in unioes.keys.filter({ unioes[$0] == voz }) { unioes[origem] = destino.id }
            unioes[voz] = destino.id
            unioesDeVoz = unioes
            atualizarVozes()
        } else {
            vozes[i].nome = nomeFinal
            vozes[i].nomeManual = true
        }
        return true
    }

    public mutating func aplicarPassadaFinal(_ novos: [TrechoLongo]) {
        let antigos = Dictionary(uniqueKeysWithValues: trechos.map { ($0.id, $0) })
        let idsNovos = Set(novos.map(\.id))
        var manuaisSemPar = trechos.filter { $0.vozManual == true && !idsNovos.contains($0.id) }
        var votos: [String: [String: Double]] = [:]
        trechos = novos.map { novo in
            guard let antigo = antigos[novo.id] else {
                return Self.herdarManual(novo, de: &manuaisSemPar) ?? novo
            }
            if antigo.vozManual == true {
                var trecho = novo
                trecho.voz = antigo.voz
                trecho.vozManual = true
                return trecho
            }
            if antigo.voz != "desconhecida", novo.voz != "desconhecida" {
                votos[antigo.voz, default: [:]][novo.voz, default: 0] += max(0, novo.fim - novo.inicio)
            }
            return novo
        }
        let nomesManuais = vozes.filter { $0.nomeManual == true }
        atualizarVozes()
        var usados = Set(vozes.filter { $0.nomeManual == true }.map(\.id))
        for voz in nomesManuais where !usados.contains(voz.id) {
            guard let chances = votos[voz.id],
                  let id = chances.sorted(by: { $0.value == $1.value ? $0.key < $1.key : $0.value > $1.value }).first?.key,
                  usados.insert(id).inserted else { continue }
            renomear(voz: id, nome: voz.nome)
        }
    }

    /// Leva para este documento as edições feitas pelo usuário numa versão já salva.
    public mutating func herdarEdicoes(de salvo: TranscricaoLonga) {
        nome = salvo.nome
        oculto = salvo.oculto
        if salvo.resumo != nil {
            resumo = salvo.resumo
            resumoGerado = salvo.resumoGerado
        }
        var unioes = unioesDeVoz ?? [:]
        for (origem, destino) in salvo.unioesDeVoz ?? [:] { unioes[origem] = destino }
        unioesDeVoz = unioes.isEmpty ? nil : unioes
        var manuais = salvo.trechos.filter { $0.vozManual == true }
        trechos = trechos.map { trecho in
            if let i = manuais.firstIndex(where: { $0.id == trecho.id }) {
                var herdado = trecho
                herdado.voz = manuais[i].voz
                herdado.vozManual = true
                manuais.remove(at: i)
                return herdado
            }
            return Self.herdarManual(trecho, de: &manuais) ?? trecho
        }
        let porID = Dictionary(uniqueKeysWithValues: salvo.vozes.map { ($0.id, $0) })
        vozes = vozes.map { porID[$0.id] ?? $0 } + salvo.vozes.filter { v in !vozes.contains { $0.id == v.id } }
        atualizarVozes()
    }

    /// Mesma fonte e ao menos metade do trecho menor em comum: a edição pertence ao trecho refeito.
    private static func herdarManual(_ novo: TrechoLongo, de manuais: inout [TrechoLongo]) -> TrechoLongo? {
        let candidatos = manuais.indices.filter {
            let antigo = manuais[$0]
            let menor = max(0.02, min(antigo.fim - antigo.inicio, novo.fim - novo.inicio))
            return antigo.doMicrofone == novo.doMicrofone && AlinhamentoDeVoz.sobreposicao(antigo, novo) >= menor * 0.5
        }
        guard let i = candidatos.max(by: {
            AlinhamentoDeVoz.sobreposicao(manuais[$0], novo) < AlinhamentoDeVoz.sobreposicao(manuais[$1], novo)
        }) else { return nil }
        let antigo = manuais.remove(at: i)
        var trecho = novo
        trecho.id = antigo.id
        trecho.voz = antigo.voz
        trecho.vozManual = true
        return trecho
    }
}

public enum AlinhamentoDeVoz {
    public static func sobreposicao(_ a: TrechoLongo, _ b: TrechoLongo) -> Double {
        max(0, min(a.fim, b.fim) - max(a.inicio, b.inicio))
    }

    public static func identificar(inicio: Double, fim: Double, faixas: [FaixaDeVoz]) -> String {
        var sobreposicoes: [String: Double] = [:]
        var fimPorVoz: [String: Double] = [:]
        for faixa in faixas.sorted(by: { $0.inicio < $1.inicio }) {
            let de = max(inicio, faixa.inicio, fimPorVoz[faixa.voz] ?? inicio)
            let ate = min(fim, faixa.fim)
            if ate > de {
                sobreposicoes[faixa.voz, default: 0] += ate - de
                fimPorVoz[faixa.voz] = ate
            }
        }
        let melhores = sobreposicoes.values.sorted(by: >)
        guard let melhor = melhores.first, melhor >= (fim - inicio) * 0.45,
              melhor - (melhores.dropFirst().first ?? 0) > 0.02
        else { return "desconhecida" }
        return sobreposicoes.first { $0.value == melhor }?.key ?? "desconhecida"
    }

    /// Uma janela é dona das palavras cujo centro cai no seu intervalo útil.
    public static func pertence(inicio: Double, fim: Double, a janela: Range<Double>) -> Bool {
        janela.contains((inicio + fim) / 2)
    }

    public static func juntar(_ primeira: [TrechoLongo], _ segunda: [TrechoLongo]) -> [TrechoLongo] {
        (primeira + segunda).sorted {
            if $0.inicio != $1.inicio { return $0.inicio < $1.inicio }
            return $0.fim < $1.fim
        }
    }
}

public enum RetencaoDeReuniao {
    /// Uma pasta órfã pode ser retirada depois que o processo que a criou terminou.
    public static func processoDaReserva(nome: String) -> Int32? {
        let prefixo = "ditase-"
        guard nome.hasPrefix(prefixo) else { return nil }
        let sufixo = nome.dropFirst(prefixo.count).split(separator: "-", maxSplits: 1)
        guard sufixo.count == 2, let pid = Int32(sufixo[0]), pid > 0,
              UUID(uuidString: String(sufixo[1])) != nil else { return nil }
        return pid
    }

    public static func reservaVencida(nome: String, alteracao: Date, agora: Date) -> Bool {
        processoDaReserva(nome: nome) != nil && agora.timeIntervalSince(alteracao) >= 24 * 3600
    }
}
