import Foundation

/// Modelo de leitura: o texto reconhecido nunca muda; a voz efetiva é a edição manual,
/// senão a atribuição final, senão o rótulo ao vivo, e as uniões valem por último.
extension ArmazemLancamento {
    public func documento(_ id: String) throws -> TranscricaoLonga? {
        try ler { c in try Self.montar(id, leve: false, em: c) }
    }

    public func lista(tipo: String) throws -> [TranscricaoLonga] {
        try ler { c in
            try c.consultar("SELECT id FROM lt_documentos WHERE tipo = ? AND estado <> 'descartada' ORDER BY criado_em DESC",
                            [.texto(tipo)]) { $0.texto(0) }
                .compactMap { try Self.montar($0, leve: true, em: c) }
        }
    }

    public func periodos(documento: String) throws -> (lacunas: [PeriodoDeAudio], pendentes: [PeriodoDeAudio]) {
        try ler { c in
            var doc = TranscricaoLonga(nome: "", fase: "")
            doc.id = documento
            try Self.preencherAudio(&doc, em: c)
            return (doc.lacunas ?? [], doc.pendentes ?? [])
        }
    }

    public func existeDocumento(_ id: String) throws -> Bool {
        try ler { c in try c.existe("SELECT 1 FROM lt_documentos WHERE id = ?", [.texto(id)]) }
    }

    static func montar(_ id: String, leve: Bool, em c: ConexaoSQLite) throws -> TranscricaoLonga? {
        struct Linha {
            let tipo, nome, estado, falantes: String
            let criado, duracao: Double
            let fonte, versaoAtiva, resumo, erroResumo, erroCaptura, avisoCaptura, aviso: String?
            let oculto, resumoGerado, resumoCancelado: Bool
            let atraso: Double?
        }
        guard let d = try c.consultar("""
            SELECT tipo, nome, estado, falantes, criado_em, duracao, fonte, versao_ativa, resumo, erro_resumo, erro_captura,
                   aviso_captura, aviso, oculto, resumo_gerado, resumo_cancelado, atraso
            FROM lt_documentos WHERE id = ?
            """, [.texto(id)], { l in
            Linha(tipo: l.texto(0), nome: l.texto(1), estado: l.texto(2), falantes: l.texto(3), criado: l.real(4), duracao: l.real(5),
                  fonte: l.textoOpcional(6), versaoAtiva: l.textoOpcional(7), resumo: l.textoOpcional(8), erroResumo: l.textoOpcional(9),
                  erroCaptura: l.textoOpcional(10), avisoCaptura: l.textoOpcional(11), aviso: l.textoOpcional(12),
                  oculto: l.inteiro(13) == 1, resumoGerado: l.inteiro(14) == 1, resumoCancelado: l.inteiro(15) == 1, atraso: l.realOpcional(16))
        }).first else { return nil }

        var doc = TranscricaoLonga(nome: d.nome, fase: "", fonte: d.tipo == "reuniao" ? (d.fonte ?? "mac") : nil)
        doc.id = id
        doc.criadoEm = d.criado
        doc.duracao = d.duracao > 0 ? d.duracao : nil
        doc.oculto = d.oculto ? true : nil
        doc.erroCaptura = d.erroCaptura
        doc.avisoCaptura = d.avisoCaptura
        doc.aviso = d.aviso
        doc.atraso = d.atraso
        doc.falantes = d.falantes
        if let resumo = d.resumo, let lista = try? JSONDecoder().decode([String].self, from: Data(resumo.utf8)) {
            doc.resumo = lista
            doc.resumoGerado = d.resumoGerado ? true : nil
        }
        doc.erroResumo = d.erroResumo
        doc.resumoCancelado = d.resumoCancelado ? true : nil

        let versao: String?
        if d.tipo == "arquivo" {
            versao = d.versaoAtiva
            try preencherArquivo(&doc, versaoAtiva: d.versaoAtiva, leve: leve, em: c)
        } else {
            versao = ""
            doc.fase = d.estado == "finalizada" ? "pronta" : d.estado == "processando" ? "finalizando" : d.estado
        }

        if let versao {
            let edicoes = try c.consultar("SELECT tipo, alvo, valor FROM lt_edicoes WHERE documento = ? AND versao = ? ORDER BY id",
                                          [.texto(id), .texto(versao)]) { ($0.texto(0), $0.texto(1), $0.texto(2)) }
            var vozTrecho: [String: String] = [:], nomes: [String: String] = [:], unioes: [String: String] = [:]
            for (tipo, alvo, valor) in edicoes {
                switch tipo {
                case "vozTrecho": vozTrecho[alvo] = valor
                case "nomeVoz": nomes[alvo] = valor; unioes[alvo] = nil
                case "uniao": unioes[alvo] = valor
                default: break
                }
            }
            func unida(_ voz: String) -> String {
                var atual = voz
                var vistos: Set<String> = [voz]
                while let proxima = unioes[atual], vistos.insert(proxima).inserted { atual = proxima }
                return atual
            }
            let colunaTexto = leve ? "''" : "texto"
            doc.trechos = try c.consultar("""
                SELECT id, fonte, inicio, fim, \(colunaTexto), voz, voz_final, divergente FROM lt_trechos
                WHERE documento = ? AND versao = ? ORDER BY inicio, fonte, id
                """, [.texto(id), .texto(versao)]) { l in
                let tid = l.texto(0)
                let manual = vozTrecho[tid]
                var t = TrechoLongo(inicio: l.real(2), fim: l.real(3), voz: unida(manual ?? l.textoOpcional(6) ?? l.texto(5)), texto: l.texto(4))
                t.id = tid
                if l.texto(1) == FonteDeAudio.mic.rawValue { t.origem = "microfone" }
                if manual != nil { t.vozManual = true }
                if l.inteiro(7) == 1 { t.divergente = true }
                return t
            }
            var vistos = Set<String>()
            var pessoas = 0
            doc.vozes = doc.trechos.compactMap { t in
                guard vistos.insert(t.voz).inserted else { return nil }
                var nome: String
                switch t.voz {
                case "voce": nome = "Você"
                case "chamada": nome = "Chamada"
                case "desconhecida": nome = "Voz não identificada"
                default:
                    pessoas += 1
                    nome = "Pessoa \(pessoas)"
                }
                var voz = VozLonga(id: t.voz, nome: nome)
                if let manual = nomes[t.voz] { voz.nome = manual; voz.nomeManual = true }
                return voz
            }
            if d.tipo == "reuniao" { doc.baixaConfianca = doc.trechos.contains { $0.voz == "desconhecida" } }
            if leve {
                doc.palavras = try c.consultar("""
                    SELECT COALESCE(SUM(CASE WHEN trim(texto) = '' THEN 0 ELSE length(trim(texto)) - length(replace(trim(texto), ' ', '')) + 1 END), 0)
                    FROM lt_trechos WHERE documento = ? AND versao = ?
                    """, [.texto(id), .texto(versao)]) { Int($0.inteiro(0)) }.first
                doc.previa = try c.consultar("SELECT substr(texto, 1, 240) FROM lt_trechos WHERE documento = ? AND versao = ? ORDER BY inicio LIMIT 1",
                                             [.texto(id), .texto(versao)]) { $0.texto(0) }.first
                doc.trechos = []
            }
        }
        if d.tipo == "reuniao" { try preencherAudio(&doc, em: c) }
        return doc
    }

    private static func preencherArquivo(_ doc: inout TranscricaoLonga, versaoAtiva: String?, leve: Bool, em c: ConexaoSQLite) throws {
        let versoes = try c.consultar("""
            SELECT id, estado, duracao, bruto, processamento, baixa_confianca, erro FROM lt_versoes WHERE documento = ? ORDER BY tentativa DESC
            """, [.texto(doc.id)]) { l in
            (id: l.texto(0), estado: l.texto(1), duracao: l.realOpcional(2), bruto: l.textoOpcional(3),
             processamento: l.realOpcional(4), baixa: l.inteiro(5) == 1, erro: l.textoOpcional(6))
        }
        if let ativa = versoes.first(where: { $0.id == versaoAtiva }) {
            doc.fase = "pronto"
            doc.duracao = ativa.duracao
            doc.bruto = leve ? nil : ativa.bruto
            doc.processamento = ativa.processamento.map { ($0 * 10).rounded() / 10 }
            doc.baixaConfianca = ativa.baixa
            return
        }
        switch versoes.first?.estado {
        case "vazia": doc.fase = "semFala"
        case "cancelada", "superada", nil: doc.fase = "cancelado"
        case "emCurso": doc.fase = "falhou"; doc.motivo = "A transcrição foi interrompida. Tente de novo."
        default:
            let formato = versoes.first?.erro == ErroDeArquivo.formato
            doc.fase = formato ? "formatoNaoSuportado" : "falhou"
            doc.motivo = formato ? nil : versoes.first?.erro
        }
    }

    /// Lacunas e pendências da reunião, em segundos, mescladas por fonte.
    private static func preencherAudio(_ doc: inout TranscricaoLonga, em c: ConexaoSQLite) throws {
        let blocos = try Self.blocos(documento: doc.id, em: c)
        func faixas(_ filtro: (BlocoRegistrado) -> Bool, motivo: (BlocoRegistrado) -> String) -> [PeriodoDeAudio] {
            var saida: [PeriodoDeAudio] = []
            for b in blocos.filter(filtro).sorted(by: { ($0.id.fonte.rawValue, $0.inicio) < ($1.id.fonte.rawValue, $1.inicio) }) {
                let inicio = Double(b.inicio) / Double(taxaLancamento), fim = Double(b.fim) / Double(taxaLancamento)
                if let ultimo = saida.last, ultimo.fonte == b.id.fonte.rawValue, abs(ultimo.fim - inicio) < 0.01, ultimo.motivo == motivo(b) {
                    saida[saida.count - 1] = PeriodoDeAudio(inicio: ultimo.inicio, fim: fim, fonte: ultimo.fonte, motivo: ultimo.motivo)
                } else {
                    saida.append(PeriodoDeAudio(inicio: inicio, fim: fim, fonte: b.id.fonte.rawValue, motivo: motivo(b)))
                }
            }
            return saida
        }
        let lacunas = faixas({ $0.estado == .lacuna }) { $0.erro ?? "Sem áudio neste intervalo." }
        var pendentes = faixas({ $0.estado == .capturado }) { _ in "Áudio guardado, ainda não transcrito." }
        pendentes += try Self.pendencias(documento: doc.id, em: c).filter { $0.estado == "aberta" }.map {
            PeriodoDeAudio(inicio: Double($0.inicio) / Double(taxaLancamento), fim: Double($0.fim) / Double(taxaLancamento),
                           fonte: $0.fonte.rawValue, motivo: $0.motivo)
        }
        doc.lacunas = lacunas.isEmpty ? nil : lacunas
        doc.pendentes = pendentes.isEmpty ? nil : pendentes.sorted { $0.inicio < $1.inicio }
        let segundos = blocos.filter { $0.estado == .capturado || $0.estado == .pendente }.reduce(0) { $0 + $1.quantidade }
        doc.segundosPendentes = segundos > 0 ? Double(segundos) / Double(taxaLancamento) : nil
        if let fim = blocos.map(\.fim).max() {
            doc.duracao = max(doc.duracao ?? 0, Double(fim) / Double(taxaLancamento))
        }
    }

    /// Reuniões fora da sessão ao vivo que ainda guardam áudio ou não terminaram.
    public func recuperaveis(excluindo ativa: String?) throws -> [TranscricaoLonga] {
        try ler { c in
            try c.consultar("""
                SELECT d.id FROM lt_documentos d WHERE d.tipo = 'reuniao' AND d.estado <> 'descartada' AND (
                    d.estado <> 'finalizada'
                    OR EXISTS (SELECT 1 FROM lt_blocos b WHERE b.documento = d.id AND b.estado IN ('capturado','pendente'))
                    OR (d.falantes IN ('pendente','indisponivel') AND EXISTS (
                        SELECT 1 FROM lt_blocos b WHERE b.documento = d.id AND b.audio_liberado = 0 AND b.fonte = 'mac' AND b.estado <> 'lacuna')))
                ORDER BY d.criado_em DESC
                """) { $0.texto(0) }
                .filter { $0 != ativa }
                .compactMap { try Self.montar($0, leve: true, em: c) }
        }
    }
}

public enum ErroDeArquivo {
    public static let formato = "formato"
}
