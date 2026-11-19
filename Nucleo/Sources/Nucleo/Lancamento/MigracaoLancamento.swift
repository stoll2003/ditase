import Foundation

/// Migração aditiva do formato anterior: documento JSON de `transcricoes_longas`, diários em
/// `ReunioesPendentes/<id>` (trechos e metadados em plist) e PCM pendente. Cada origem tem chave
/// estável; as linhas e a marca de migração entram na mesma transação, e a conversão é relida e
/// conferida antes do commit. Divergência preserva as duas versões. O JSON original nunca é apagado;
/// o diário só sai depois do commit conferido.
extension ArmazemLancamento {
    public struct RelatorioMigracao: Equatable, Sendable {
        public var documentos = 0
        public var diarios = 0
        public var divergencias = 0
        public var falhas: [String] = []
        public var adotados = 0
        public var lacunas = 0
        public var invalidos: [String] = []
    }

    /// Roda na abertura, antes de listar recuperáveis. Repetir não duplica nada.
    @discardableResult
    public func prepararAbertura(diarios: URL?, disco: DiscoDeBlocos) -> RelatorioMigracao {
        var relatorio = RelatorioMigracao()
        migrarDocumentos(&relatorio)
        if let diarios { migrarDiarios(diarios, disco: disco, &relatorio) }
        reconciliar(disco: disco, &relatorio)
        return relatorio
    }

    // MARK: - Documentos JSON

    private func migrarDocumentos(_ relatorio: inout RelatorioMigracao) {
        let legados: [(String, String)]
        do {
            legados = try ler { c in
                guard try c.existe("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'transcricoes_longas'") else { return [] }
                return try c.consultar("""
                    SELECT id, dados FROM transcricoes_longas t
                    WHERE NOT EXISTS (SELECT 1 FROM lt_migracoes m WHERE m.chave = 'doc:' || t.id) ORDER BY criado_em
                    """) { ($0.texto(0), $0.texto(1)) }
            }
        } catch { relatorio.falhas.append("leitura do histórico: \(error)"); return }
        for (id, json) in legados {
            do {
                let antigo = try JSONDecoder().decode(TranscricaoLonga.self, from: Data(json.utf8))
                try escrever { c in try c.transacao { try Self.importar(antigo, em: c) } }
                relatorio.documentos += 1
            } catch { relatorio.falhas.append("documento \(id): \(error)") }
        }
    }

    private static func importar(_ antigo: TranscricaoLonga, em c: ConexaoSQLite) throws {
        let reuniao = antigo.fonte != nil
        let pronto = antigo.fase == "pronta" || antigo.fase == "pronto"
        let id = antigo.id
        try c.rodar("""
            INSERT OR IGNORE INTO lt_documentos (id, tipo, nome, criado_em, fonte, estado, duracao, oculto, falantes,
                resumo, resumo_gerado, erro_resumo, resumo_cancelado, erro_captura, aviso_captura, atraso)
            VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
            """, [.texto(id), .texto(reuniao ? "reuniao" : "arquivo"), .texto(antigo.nome), .real(antigo.criadoEm),
                  antigo.fonte.map { .texto($0) } ?? .nulo,
                  .texto(reuniao ? (pronto ? "finalizada" : "interrompida") : "ativo"), .real(antigo.duracao ?? 0),
                  .inteiro(antigo.oculto == true ? 1 : 0), .texto(reuniao ? (pronto ? "identificados" : "pendente") : "naoAplica"),
                  try antigo.resumo.map { .texto(String(decoding: try JSONEncoder().encode($0), as: UTF8.self)) } ?? .nulo,
                  .inteiro(antigo.resumoGerado == true ? 1 : 0), antigo.erroResumo.map { .texto($0) } ?? .nulo,
                  .inteiro(antigo.resumoCancelado == true ? 1 : 0), antigo.erroCaptura.map { .texto($0) } ?? .nulo,
                  antigo.avisoCaptura.map { .texto($0) } ?? .nulo, antigo.atraso.map { .real($0) } ?? .nulo])
        var versao = ""
        if !reuniao {
            versao = "legado-" + id
            try c.rodar("""
                INSERT OR IGNORE INTO lt_versoes (id, documento, tentativa, origem, identidade, estado, criada_em, publicada_em, duracao, bruto, processamento, baixa_confianca, erro)
                VALUES (?,?,1,'legado','legado',?,?,?,?,?,?,?,?)
                """, [.texto(versao), .texto(id), .texto(pronto ? "pronta" : antigo.fase == "semFala" ? "vazia" : "falhou"),
                      .real(antigo.criadoEm), .real(antigo.criadoEm), antigo.duracao.map { .real($0) } ?? .nulo,
                      antigo.bruto.map { .texto($0) } ?? .nulo, antigo.processamento.map { .real($0) } ?? .nulo,
                      .inteiro(antigo.baixaConfianca ? 1 : 0),
                      antigo.fase == "formatoNaoSuportado" ? .texto(ErroDeArquivo.formato) : antigo.motivo.map { .texto($0) } ?? .nulo])
            if pronto { try c.rodar("UPDATE lt_documentos SET versao_ativa = ? WHERE id = ?", [.texto(versao), .texto(id)]) }
        }
        for t in antigo.trechos where try !c.existe("SELECT 1 FROM lt_trechos WHERE origem_legado = ?", [.texto("doc:\(id):\(t.id)")]) {
            let fonte: FonteDeAudio = !reuniao ? .arquivo : t.doMicrofone ? .mic : .mac
            try inserir(TrechoNovo(id: t.id, fonte: fonte, inicio: t.inicio, fim: t.fim, texto: t.texto, voz: t.voz),
                        documento: id, versao: versao, origemLegado: "doc:\(id):\(t.id)", divergente: t.divergente == true, em: c)
            if t.vozManual == true {
                try c.rodar("INSERT OR IGNORE INTO lt_edicoes (documento, versao, tipo, alvo, valor, criado_em, chave) VALUES (?,?, 'vozTrecho', ?, ?, ?, ?)",
                            [.texto(id), .texto(versao), .texto(t.id), .texto(t.voz), .real(antigo.criadoEm), .texto("legado:doc:\(id):vt:\(t.id)")])
            }
        }
        // Nomes: grava como edição todo nome que difere do padrão da leitura, para a releitura bater.
        let padrao = try montar(id, leve: false, em: c)
        for voz in antigo.vozes {
            guard let atual = padrao?.vozes.first(where: { $0.id == voz.id }), atual.nome != voz.nome || voz.nomeManual == true else { continue }
            try c.rodar("INSERT OR IGNORE INTO lt_edicoes (documento, versao, tipo, alvo, valor, criado_em, chave) VALUES (?,?, 'nomeVoz', ?, ?, ?, ?)",
                        [.texto(id), .texto(versao), .texto(voz.id), .texto(voz.nome), .real(antigo.criadoEm), .texto("legado:doc:\(id):nv:\(voz.id)")])
        }
        try c.rodar("INSERT INTO lt_migracoes (chave, feita_em, detalhe) VALUES (?,?,?)",
                    [.texto("doc:" + id), .real(Date().timeIntervalSince1970), .texto("\(antigo.trechos.count) trechos")])
        try conferir(antigo, em: c)
    }

    /// Releitura da conversão dentro da transação: textos, IDs, ordem, vozes, resumo e ocultação.
    private static func conferir(_ antigo: TranscricaoLonga, em c: ConexaoSQLite) throws {
        guard let novo = try montar(antigo.id, leve: false, em: c) else { throw ErroDeBanco(description: "conversão não relida") }
        let esperado = antigo.trechos.sorted { ($0.inicio, $0.id) < ($1.inicio, $1.id) }
        let obtido = novo.trechos.sorted { ($0.inicio, $0.id) < ($1.inicio, $1.id) }
        guard esperado.map(\.id) == obtido.map(\.id), esperado.map(\.texto) == obtido.map(\.texto),
              esperado.map(\.voz) == obtido.map(\.voz), zip(esperado, obtido).allSatisfy({ abs($0.inicio - $1.inicio) < 1e-9 && abs($0.fim - $1.fim) < 1e-9 }),
              novo.nome == antigo.nome, (novo.resumo ?? []) == (antigo.resumo ?? []), (novo.oculto == true) == (antigo.oculto == true)
        else { throw ErroDeBanco(description: "conversão divergente do original") }
        let nomesAntigos = Dictionary(antigo.vozes.map { ($0.id, $0.nome) }, uniquingKeysWith: { a, _ in a })
        for voz in novo.vozes where nomesAntigos[voz.id].map({ $0 != voz.nome }) == true {
            throw ErroDeBanco(description: "nome de voz divergente na conversão")
        }
    }

    // MARK: - Diários antigos

    private func migrarDiarios(_ raiz: URL, disco: DiscoDeBlocos, _ relatorio: inout RelatorioMigracao) {
        let diarios: [DiarioReuniao]
        do { diarios = try DiarioReuniao.listar(raiz: raiz) }
        catch { relatorio.falhas.append("leitura dos diários: \(error)"); return }
        for diario in diarios {
            do {
                if try ler({ c in try c.existe("SELECT 1 FROM lt_migracoes WHERE chave = ?", [.texto("diario:" + diario.id)]) }) {
                    try? diario.remover()
                    continue
                }
                let divergencias = try importar(diario, disco: disco)
                relatorio.diarios += 1
                relatorio.divergencias += divergencias
                try? diario.remover()
            } catch { relatorio.falhas.append("diário \(diario.id): \(error)") }
        }
    }

    private func importar(_ diario: DiarioReuniao, disco: DiscoDeBlocos) throws -> Int {
        let meta = try diario.documento()
        let id = diario.id
        let porBloco = try diario.audios().map { entrada in (inicio: entrada.inicio, trechos: try diario.trechosSalvos(inicio: entrada.inicio),
                                                             pcm: try DiarioReuniao.audio(em: entrada.url)) }
        let deBlocos = try diario.trechos()
        let idsDeBlocos = Set(deBlocos.map(\.id))
        let trechosDeArquivo = deBlocos + meta.trechos.filter { !idsDeBlocos.contains($0.id) && !$0.texto.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        // PCM primeiro, durável e com nomes determinísticos: repetir sobrescreve com o mesmo conteúdo.
        var blocos: [(BlocoRegistrado, transcrito: Bool)] = []
        for (seq, b) in porBloco.enumerated() {
            for (fonte, amostras) in [(FonteDeAudio.mac, b.pcm.sistema), (.mic, b.pcm.microfone)] where !amostras.isEmpty {
                let registro = try disco.gravar(IdBloco(documento: id, fonte: fonte, geracao: 0, seq: seq), inicio: b.inicio, amostras: amostras)
                blocos.append((registro, b.trechos != nil))
            }
        }
        return try escrever { c in
            try c.transacao {
                let existia = try c.existe("SELECT 1 FROM lt_documentos WHERE id = ?", [.texto(id)])
                if !existia {
                    try c.rodar("INSERT INTO lt_documentos (id, tipo, nome, criado_em, fonte, estado, falantes) VALUES (?, 'reuniao', ?, ?, ?, 'interrompida', 'pendente')",
                                [.texto(id), .texto(meta.nome), .real(meta.criadoEm), .texto(meta.fonte ?? "mac")])
                }
                var divergencias = 0
                var inseridos = 0
                for t in trechosDeArquivo {
                    let fonte: FonteDeAudio = t.doMicrofone ? .mic : .mac
                    let existente = try c.consultar("SELECT texto, inicio, fim, fonte FROM lt_trechos WHERE id = ?", [.texto(t.id)]) {
                        ($0.texto(0), $0.real(1), $0.real(2), $0.texto(3))
                    }.first
                    let origem = "diario:\(id):\(t.id)"
                    guard try !c.existe("SELECT 1 FROM lt_trechos WHERE origem_legado = ?", [.texto(origem)]) else { continue }
                    if let existente {
                        if existente.0 == t.texto && abs(existente.1 - t.inicio) < 1e-6 && abs(existente.2 - t.fim) < 1e-6 && existente.3 == fonte.rawValue { continue }
                        divergencias += 1
                        try Self.inserir(TrechoNovo(id: t.id + "~diario", fonte: fonte, inicio: t.inicio, fim: t.fim, texto: t.texto, voz: t.voz),
                                         documento: id, versao: "", origemLegado: origem, divergente: true, em: c)
                    } else {
                        try Self.inserir(TrechoNovo(id: t.id, fonte: fonte, inicio: t.inicio, fim: t.fim, texto: t.texto, voz: t.voz),
                                         documento: id, versao: "", origemLegado: origem, em: c)
                    }
                    inseridos += 1
                }
                // Edições do metadado do diário entram só onde o histórico não tem edição própria.
                for t in meta.trechos where t.vozManual == true {
                    let temEdicao = try c.existe("SELECT 1 FROM lt_edicoes WHERE documento = ? AND tipo = 'vozTrecho' AND alvo = ?", [.texto(id), .texto(t.id)])
                    if temEdicao { divergencias += 1; continue }
                    try c.rodar("INSERT OR IGNORE INTO lt_edicoes (documento, versao, tipo, alvo, valor, criado_em, chave) VALUES (?, '', 'vozTrecho', ?, ?, ?, ?)",
                                [.texto(id), .texto(t.id), .texto(t.voz), .real(meta.criadoEm), .texto("legado:diario:\(id):vt:\(t.id)")])
                }
                for voz in meta.vozes where voz.nomeManual == true {
                    if try c.existe("SELECT 1 FROM lt_edicoes WHERE documento = ? AND tipo IN ('nomeVoz','uniao') AND alvo = ?", [.texto(id), .texto(voz.id)]) { continue }
                    try c.rodar("INSERT OR IGNORE INTO lt_edicoes (documento, versao, tipo, alvo, valor, criado_em, chave) VALUES (?, '', 'nomeVoz', ?, ?, ?, ?)",
                                [.texto(id), .texto(voz.id), .texto(voz.nome), .real(meta.criadoEm), .texto("legado:diario:\(id):nv:\(voz.id)")])
                }
                var fins: [FonteDeAudio: Int] = [:]
                for (registro, transcrito) in blocos {
                    var b = registro
                    b.estado = transcrito ? .transcrito : .pendente
                    if !transcrito { b.erro = "Áudio de uma versão anterior ainda não transcrito." }
                    _ = try Self.inserir(b, em: c)
                    fins[b.id.fonte] = max(fins[b.id.fonte] ?? 0, b.fim)
                    if !transcrito {
                        try c.rodar("INSERT OR IGNORE INTO lt_pendencias (id, documento, fonte, geracao, inicio, fim, motivo, estado) VALUES (?,?,?,0,?,?,?, 'aberta')",
                                    [.texto("\(id)|\(b.id.fonte.rawValue)|0|\(b.inicio)"), .texto(id), .texto(b.id.fonte.rawValue),
                                     .inteiro(Int64(b.inicio)), .inteiro(Int64(b.fim)), .texto("Áudio de uma versão anterior ainda não transcrito.")])
                    }
                }
                for (fonte, fim) in fins {
                    try c.rodar("INSERT OR IGNORE INTO lt_costuras (documento, fonte, geracao, confirmado_ate, fechada) VALUES (?,?,0,?,1)",
                                [.texto(id), .texto(fonte.rawValue), .inteiro(Int64(fim))])
                }
                if divergencias > 0 {
                    try c.rodar("UPDATE lt_documentos SET aviso = ? WHERE id = ?",
                                [.texto("Há falas divergentes entre o histórico e a gravação interrompida. As duas versões foram mantidas."), .texto(id)])
                }
                try c.rodar("INSERT INTO lt_migracoes (chave, feita_em, detalhe) VALUES (?,?,?)",
                            [.texto("diario:" + id), .real(Date().timeIntervalSince1970), .texto("\(inseridos) trechos, \(blocos.count) blocos")])
                // Conferência por origem antes de liberar o diário.
                let contados = try c.consultar("SELECT COUNT(*) FROM lt_trechos WHERE origem_legado LIKE ?", [.texto("diario:\(id):%")]) { Int($0.inteiro(0)) }.first ?? 0
                let registrados = try c.consultar("SELECT COUNT(*) FROM lt_blocos WHERE documento = ? AND arquivo IS NOT NULL", [.texto(id)]) { Int($0.inteiro(0)) }.first ?? 0
                guard contados >= inseridos, registrados >= blocos.count else { throw ErroDeBanco(description: "conferência do diário falhou") }
                return divergencias
            }
        }
    }

    // MARK: - Reconciliação de abertura

    /// Órfão adotado numa costura que já existia e cujo ponto confirmado passa à frente dele.
    /// A parte anterior ao ponto sempre vira uma pendência própria, com identidade do intervalo
    /// inteiro (fonte, geração, início e fim): o PCM fica retido e a recuperação transcreve esse
    /// intervalo de novo. Não se tenta inferir cobertura pelo texto existente (sobreposição não
    /// prova cobertura); a nova transcrição idêntica não duplica, porque `inserir` ignora trecho igual.
    /// A costura não recua, para não refazer o texto já confirmado depois do intervalo.
    static func protegerAdotadoAtrasado(_ b: BlocoRegistrado, costuraNova: Bool, em c: ConexaoSQLite) throws {
        guard !costuraNova else { return }
        let chave: [ConexaoSQLite.Valor] = [.texto(b.id.documento), .texto(b.id.fonte.rawValue), .inteiro(Int64(b.id.geracao))]
        guard let confirmado = try c.consultar("SELECT confirmado_ate FROM lt_costuras WHERE documento = ? AND fonte = ? AND geracao = ?",
                                               chave, { Int($0.inteiro(0)) }).first,
              b.inicio < confirmado else { return }
        // Pendências abertas sobrepostas são absorvidas pela nova, que passa a cobrir a união:
        // duas tentativas sobre o mesmo áudio duplicariam a fala.
        var inicio = b.inicio, ate = min(b.fim, confirmado)
        let sobrepostas = try c.consultar("""
            SELECT id, inicio, fim FROM lt_pendencias WHERE documento = ? AND fonte = ? AND geracao = ? AND estado = 'aberta' AND inicio < ? AND fim > ?
            """, chave + [.inteiro(Int64(ate)), .inteiro(Int64(inicio))]) { ($0.texto(0), Int($0.inteiro(1)), Int($0.inteiro(2))) }
        for (_, i, f) in sobrepostas { inicio = min(inicio, i); ate = max(ate, f) }
        for (id, _, _) in sobrepostas {
            try c.rodar("UPDATE lt_pendencias SET estado = 'absorvida' WHERE id = ? AND estado = 'aberta'", [.texto(id)])
        }
        let motivo = "Áudio recuperado depois de uma interrupção, ainda não transcrito. O áudio continua guardado."
        let id = "adocao|\(b.id.documento)|\(b.id.fonte.rawValue)|\(b.id.geracao)|\(inicio)|\(ate)"
        try c.rodar("""
            INSERT OR IGNORE INTO lt_pendencias (id, documento, fonte, geracao, inicio, fim, motivo, estado)
            VALUES (?,?,?,?,?,?,?,'aberta')
            """, [.texto(id)] + chave + [.inteiro(Int64(inicio)), .inteiro(Int64(ate)), .texto(motivo)])
        try c.rodar("UPDATE lt_pendencias SET estado = 'aberta' WHERE id = ?", [.texto(id)])
        if b.fim <= confirmado {
            try c.rodar("UPDATE lt_blocos SET estado = 'pendente', erro = ? WHERE id = ? AND estado = 'capturado'",
                        [.texto(motivo), .texto(b.id.texto)])
        }
    }

    /// Adota PCM completo sem registro, marca lacuna onde o registro aponta para arquivo ausente
    /// e repete a limpeza de arquivos já liberados que foi interrompida.
    private func reconciliar(disco: DiscoDeBlocos, _ relatorio: inout RelatorioMigracao) {
        do {
            let registrados = try ler { c in
                Set(try c.consultar("SELECT arquivo FROM lt_blocos WHERE arquivo IS NOT NULL") { $0.texto(0) })
            }
            disco.marcarSemBackup()
            let (adotaveis, invalidos) = disco.orfaos(registrados: registrados)
            relatorio.invalidos = invalidos
            // Costuras criadas por esta adoção: o ponto confirmado nunca passa à frente de áudio adotado.
            var nascidas = Set<String>()
            for b in adotaveis {
                let chave = "\(b.id.documento)|\(b.id.fonte.rawValue)|\(b.id.geracao)"
                let descartado = try ler { c in
                    try c.existe("SELECT 1 FROM lt_descartes WHERE documento = ?", [.texto(b.id.documento)])
                }
                if descartado {
                    if let arquivo = b.arquivo { try disco.apagar(arquivo) }
                    continue
                }
                try escrever { c in
                    try c.transacao {
                        if try !c.existe("SELECT 1 FROM lt_documentos WHERE id = ?", [.texto(b.id.documento)]) {
                            try c.rodar("INSERT INTO lt_documentos (id, tipo, nome, criado_em, fonte, estado, falantes) VALUES (?, 'reuniao', 'Reunião recuperada', ?, 'macEMicrofone', 'interrompida', 'pendente')",
                                        [.texto(b.id.documento), .real(Date().timeIntervalSince1970)])
                        }
                        if try c.rodar("INSERT OR IGNORE INTO lt_costuras (documento, fonte, geracao, confirmado_ate) VALUES (?,?,?,?)",
                                       [.texto(b.id.documento), .texto(b.id.fonte.rawValue), .inteiro(Int64(b.id.geracao)), .inteiro(Int64(b.inicio))]) > 0 {
                            nascidas.insert(chave)
                        } else if nascidas.contains(chave) {
                            try c.rodar("UPDATE lt_costuras SET confirmado_ate = MIN(confirmado_ate, ?) WHERE documento = ? AND fonte = ? AND geracao = ?",
                                        [.inteiro(Int64(b.inicio)), .texto(b.id.documento), .texto(b.id.fonte.rawValue), .inteiro(Int64(b.id.geracao))])
                        }
                        try c.rodar("UPDATE lt_costuras SET fechada = 0 WHERE documento = ? AND fonte = ? AND geracao = ? AND confirmado_ate < ?",
                                    [.texto(b.id.documento), .texto(b.id.fonte.rawValue), .inteiro(Int64(b.id.geracao)), .inteiro(Int64(b.fim))])
                        guard try Self.inserir(b, em: c) else { return }
                        relatorio.adotados += 1
                        try Self.protegerAdotadoAtrasado(b, costuraNova: nascidas.contains(chave), em: c)
                    }
                }
            }
            for doc in try ler({ c in try c.consultar("SELECT DISTINCT documento FROM lt_blocos WHERE arquivo IS NOT NULL AND audio_liberado = 0 AND estado <> 'lacuna'") { $0.texto(0) } }) {
                for b in try blocos(documento: doc) where !b.audioLiberado && b.estado != .lacuna && b.arquivo != nil && !disco.existe(b) {
                    try marcarLacuna(b.id, motivo: DiscoDeBlocos.Falha.incompleto(b.id.texto).localizedDescription)
                    relatorio.lacunas += 1
                }
            }
            for (id, arquivo) in try arquivosParaLimpar() {
                // Caminho inválido vindo do banco não é seguido: só a referência sai.
                if DiscoDeBlocos.arquivoValido(arquivo, documento: id.documento) { try disco.apagar(arquivo) }
                try esquecerArquivo(id)
            }
            for doc in try ler({ c in try c.consultar("SELECT id FROM lt_documentos WHERE estado = 'descartada'") { $0.texto(0) } }) {
                disco.removerPasta(documento: doc)
            }
        } catch { relatorio.falhas.append("reconciliação: \(error)") }
    }
}
