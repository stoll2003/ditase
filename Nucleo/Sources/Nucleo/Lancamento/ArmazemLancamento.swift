import Foundation

/// Fonte única do texto de reuniões e arquivos. Trechos entram só por inserção; edições ficam em
/// registros à parte e são aplicadas na leitura. Toda escrita passa por uma fila serial com conexão
/// própria (WAL, synchronous=FULL); a leitura usa outra conexão e não espera o escritor.
public final class ArmazemLancamento: @unchecked Sendable {
    let escrita: ConexaoSQLite
    private let leitura: ConexaoSQLite
    private let filaEscrita = DispatchQueue(label: "ditado.armazem.escrita")
    private let filaLeitura = DispatchQueue(label: "ditado.armazem.leitura")
    public let caminho: String

    public init(caminho: String) throws {
        self.caminho = caminho
        if caminho != ":memory:" {
            try FileManager.default.createDirectory(at: URL(fileURLWithPath: caminho).deletingLastPathComponent(),
                                                    withIntermediateDirectories: true)
        }
        escrita = try ConexaoSQLite(caminho: caminho, escrita: true)
        if caminho != ":memory:" {
            // Texto de reunião: só o próprio usuário lê. O SQLite cria WAL e SHM com o modo do banco.
            for sufixo in ["", "-wal", "-shm"] where FileManager.default.fileExists(atPath: caminho + sufixo) {
                chmod(caminho + sufixo, 0o600)
            }
        }
        try Self.criarEsquema(escrita)
        leitura = caminho == ":memory:" ? escrita : try ConexaoSQLite(caminho: caminho, escrita: false)
    }

    /// Tabelas aditivas. O esquema do ditado comum e `transcricoes_longas` não são tocados.
    static func criarEsquema(_ c: ConexaoSQLite) throws {
        try c.executar("""
        CREATE TABLE IF NOT EXISTS lt_documentos (
            id TEXT PRIMARY KEY, tipo TEXT NOT NULL, nome TEXT NOT NULL, criado_em REAL NOT NULL,
            fonte TEXT, estado TEXT NOT NULL, duracao REAL NOT NULL DEFAULT 0, oculto INTEGER NOT NULL DEFAULT 0,
            versao_ativa TEXT, falantes TEXT NOT NULL DEFAULT 'pendente',
            resumo TEXT, resumo_gerado INTEGER NOT NULL DEFAULT 0, erro_resumo TEXT, resumo_cancelado INTEGER NOT NULL DEFAULT 0,
            erro_captura TEXT, aviso_captura TEXT, aviso TEXT, atraso REAL
        );
        CREATE INDEX IF NOT EXISTS lt_documentos_tipo ON lt_documentos (tipo, criado_em DESC);
        CREATE TABLE IF NOT EXISTS lt_blocos (
            id TEXT PRIMARY KEY, documento TEXT NOT NULL, fonte TEXT NOT NULL, geracao INTEGER NOT NULL, seq INTEGER NOT NULL,
            inicio INTEGER NOT NULL, quantidade INTEGER NOT NULL, arquivo TEXT, tamanho INTEGER, soma TEXT,
            estado TEXT NOT NULL, tentativas INTEGER NOT NULL DEFAULT 0, erro TEXT, audio_liberado INTEGER NOT NULL DEFAULT 0,
            UNIQUE (documento, fonte, geracao, seq)
        );
        CREATE INDEX IF NOT EXISTS lt_blocos_doc ON lt_blocos (documento, fonte, geracao, inicio);
        CREATE TABLE IF NOT EXISTS lt_costuras (
            documento TEXT NOT NULL, fonte TEXT NOT NULL, geracao INTEGER NOT NULL,
            confirmado_ate INTEGER NOT NULL, dono TEXT, fechada INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (documento, fonte, geracao)
        );
        CREATE TABLE IF NOT EXISTS lt_pendencias (
            id TEXT PRIMARY KEY, documento TEXT NOT NULL, fonte TEXT NOT NULL, geracao INTEGER NOT NULL,
            inicio INTEGER NOT NULL, fim INTEGER NOT NULL, motivo TEXT NOT NULL,
            tentativas INTEGER NOT NULL DEFAULT 1, estado TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS lt_trechos (
            id TEXT PRIMARY KEY, documento TEXT NOT NULL, versao TEXT NOT NULL DEFAULT '', fonte TEXT NOT NULL,
            inicio REAL NOT NULL, fim REAL NOT NULL, texto TEXT NOT NULL, voz TEXT NOT NULL, voz_final TEXT,
            divergente INTEGER NOT NULL DEFAULT 0, origem_legado TEXT UNIQUE, criado_em REAL NOT NULL
        );
        CREATE INDEX IF NOT EXISTS lt_trechos_doc ON lt_trechos (documento, versao, inicio);
        CREATE TABLE IF NOT EXISTS lt_edicoes (
            id INTEGER PRIMARY KEY AUTOINCREMENT, documento TEXT NOT NULL, versao TEXT NOT NULL DEFAULT '',
            tipo TEXT NOT NULL, alvo TEXT NOT NULL, valor TEXT NOT NULL, criado_em REAL NOT NULL, chave TEXT UNIQUE
        );
        CREATE INDEX IF NOT EXISTS lt_edicoes_doc ON lt_edicoes (documento, versao, id);
        CREATE TABLE IF NOT EXISTS lt_versoes (
            id TEXT PRIMARY KEY, documento TEXT NOT NULL, tentativa INTEGER NOT NULL, origem TEXT NOT NULL,
            identidade TEXT, estado TEXT NOT NULL, criada_em REAL NOT NULL, publicada_em REAL,
            duracao REAL, bruto TEXT, processamento REAL, baixa_confianca INTEGER NOT NULL DEFAULT 0, erro TEXT,
            UNIQUE (documento, tentativa)
        );
        CREATE TABLE IF NOT EXISTS lt_janelas_arquivo (
            versao TEXT NOT NULL, seq INTEGER NOT NULL, inicio INTEGER NOT NULL, fim INTEGER NOT NULL,
            soma TEXT NOT NULL, estado TEXT NOT NULL, PRIMARY KEY (versao, seq)
        );
        CREATE TABLE IF NOT EXISTS lt_descartes (documento TEXT PRIMARY KEY);
        CREATE TABLE IF NOT EXISTS lt_migracoes (chave TEXT PRIMARY KEY, feita_em REAL NOT NULL, detalhe TEXT);
        """)
    }

    /// Escrita serial fora do ator principal.
    func escrever<T>(_ corpo: (ConexaoSQLite) throws -> T) throws -> T {
        try filaEscrita.sync { try corpo(escrita) }
    }

    func ler<T>(_ corpo: (ConexaoSQLite) throws -> T) throws -> T {
        try (caminho == ":memory:" ? filaEscrita : filaLeitura).sync { try corpo(leitura) }
    }

    private static var agora: Double { Date().timeIntervalSince1970 }

    // MARK: - Documentos

    public func criarDocumento(id: String, tipo: String, nome: String, fonte: String?, estado: String,
                               criadoEm: Double = Date().timeIntervalSince1970) throws {
        try escrever { c in
            try c.rodar("""
                INSERT OR IGNORE INTO lt_documentos (id, tipo, nome, criado_em, fonte, estado, falantes)
                VALUES (?,?,?,?,?,?,?)
                """, [.texto(id), .texto(tipo), .texto(nome), .real(criadoEm), fonte.map { .texto($0) } ?? .nulo,
                      .texto(estado), .texto(tipo == "arquivo" ? "naoAplica" : "pendente")])
        }
    }

    /// Muda o estado só se o atual está na lista esperada. Devolve falso se não mudou.
    @discardableResult
    public func mudarEstado(documento: String, para estado: String, se esperados: [String]) throws -> Bool {
        try escrever { c in
            let marcas = esperados.map { _ in "?" }.joined(separator: ",")
            return try c.rodar("UPDATE lt_documentos SET estado = ? WHERE id = ? AND estado IN (\(marcas))",
                               [.texto(estado), .texto(documento)] + esperados.map { .texto($0) }) == 1
        }
    }

    public func estadoDocumento(_ id: String) throws -> String? {
        try ler { c in try c.consultar("SELECT estado FROM lt_documentos WHERE id = ?", [.texto(id)]) { $0.texto(0) }.first }
    }

    public func definir(documento: String, _ campo: CampoDocumento) throws {
        try escrever { c in
            switch campo {
            case .nome(let nome): try c.rodar("UPDATE lt_documentos SET nome = ? WHERE id = ?", [.texto(nome), .texto(documento)])
            case .oculto(let oculto): try c.rodar("UPDATE lt_documentos SET oculto = ? WHERE id = ?", [.inteiro(oculto ? 1 : 0), .texto(documento)])
            case .erroCaptura(let erro): try c.rodar("UPDATE lt_documentos SET erro_captura = ? WHERE id = ?", [erro.map { .texto($0) } ?? .nulo, .texto(documento)])
            case .avisoCaptura(let aviso): try c.rodar("UPDATE lt_documentos SET aviso_captura = ? WHERE id = ?", [aviso.map { .texto($0) } ?? .nulo, .texto(documento)])
            case .aviso(let aviso): try c.rodar("UPDATE lt_documentos SET aviso = ? WHERE id = ?", [aviso.map { .texto($0) } ?? .nulo, .texto(documento)])
            case .duracao(let d): try c.rodar("UPDATE lt_documentos SET duracao = MAX(duracao, ?) WHERE id = ?", [.real(d), .texto(documento)])
            case .atraso(let a): try c.rodar("UPDATE lt_documentos SET atraso = ? WHERE id = ?", [.real(a), .texto(documento)])
            case .resumo(let r):
                let json = String(decoding: try JSONEncoder().encode(r), as: UTF8.self)
                try c.rodar("UPDATE lt_documentos SET resumo = ?, resumo_gerado = 1, erro_resumo = NULL, resumo_cancelado = 0 WHERE id = ?",
                            [.texto(json), .texto(documento)])
            case .erroResumo(let e):
                try c.rodar("UPDATE lt_documentos SET erro_resumo = ?, resumo_cancelado = 0 WHERE id = ?", [.texto(e), .texto(documento)])
            case .resumoCancelado:
                try c.rodar("UPDATE lt_documentos SET resumo_cancelado = 1 WHERE id = ?", [.texto(documento)])
            }
        }
    }

    public enum CampoDocumento {
        case nome(String), oculto(Bool), erroCaptura(String?), avisoCaptura(String?), aviso(String?), duracao(Double), atraso(Double)
        case resumo([String]), erroResumo(String), resumoCancelado
    }

    // MARK: - Blocos

    /// Registra um bloco já gravado de forma durável. Devolve falso se o ID já existia.
    @discardableResult
    public func registrar(_ b: BlocoRegistrado) throws -> Bool {
        try escrever { c in try Self.inserir(b, em: c) }
    }

    static func inserir(_ b: BlocoRegistrado, em c: ConexaoSQLite) throws -> Bool {
        try c.rodar("""
            INSERT OR IGNORE INTO lt_blocos (id, documento, fonte, geracao, seq, inicio, quantidade, arquivo, tamanho, soma, estado, tentativas, erro, audio_liberado)
            VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)
            """, [.texto(b.id.texto), .texto(b.id.documento), .texto(b.id.fonte.rawValue), .inteiro(Int64(b.id.geracao)),
                  .inteiro(Int64(b.id.seq)), .inteiro(Int64(b.inicio)), .inteiro(Int64(b.quantidade)),
                  b.arquivo.map { .texto($0) } ?? .nulo, b.tamanho.map { .inteiro(Int64($0)) } ?? .nulo,
                  b.soma.map { .texto($0) } ?? .nulo, .texto(b.estado.rawValue), .inteiro(Int64(b.tentativas)),
                  b.erro.map { .texto($0) } ?? .nulo, .inteiro(b.audioLiberado ? 1 : 0)]) == 1
    }

    public func blocos(documento: String) throws -> [BlocoRegistrado] {
        try ler { c in try Self.blocos(documento: documento, em: c) }
    }

    static func blocos(documento: String, em c: ConexaoSQLite) throws -> [BlocoRegistrado] {
        try c.consultar("""
            SELECT documento, fonte, geracao, seq, inicio, quantidade, arquivo, tamanho, soma, estado, tentativas, erro, audio_liberado
            FROM lt_blocos WHERE documento = ? ORDER BY fonte, geracao, seq
            """, [.texto(documento)]) { l in
            BlocoRegistrado(id: IdBloco(documento: l.texto(0), fonte: FonteDeAudio(rawValue: l.texto(1)) ?? .mac,
                                        geracao: Int(l.inteiro(2)), seq: Int(l.inteiro(3))),
                            inicio: Int(l.inteiro(4)), quantidade: Int(l.inteiro(5)), arquivo: l.textoOpcional(6),
                            tamanho: l.inteiroOpcional(7).map(Int.init), soma: l.textoOpcional(8),
                            estado: EstadoBloco(rawValue: l.texto(9)) ?? .capturado, tentativas: Int(l.inteiro(10)),
                            erro: l.textoOpcional(11), audioLiberado: l.inteiro(12) == 1)
        }
    }

    /// Referência quebrada (arquivo ausente, incompleto ou com soma diferente) vira lacuna visível.
    public func marcarLacuna(_ id: IdBloco, motivo: String) throws {
        try escrever { c in
            try c.rodar("UPDATE lt_blocos SET estado = 'lacuna', erro = ? WHERE id = ? AND audio_liberado = 0 AND estado <> 'descartado'",
                        [.texto(motivo), .texto(id.texto)])
        }
    }

    public func esquecerArquivo(_ id: IdBloco) throws {
        try escrever { c in try c.rodar("UPDATE lt_blocos SET arquivo = NULL WHERE id = ?", [.texto(id.texto)]) }
    }

    /// Arquivos que podem sair: áudio liberado depois do commit ou bloco descartado.
    public func arquivosParaLimpar() throws -> [(IdBloco, String)] {
        try ler { c in
            try c.consultar("""
                SELECT documento, fonte, geracao, seq, arquivo FROM lt_blocos
                WHERE arquivo IS NOT NULL AND (audio_liberado = 1 OR estado = 'descartado')
                """) { l in
                (IdBloco(documento: l.texto(0), fonte: FonteDeAudio(rawValue: l.texto(1)) ?? .mac,
                         geracao: Int(l.inteiro(2)), seq: Int(l.inteiro(3))), l.texto(4))
            }
        }
    }

    // MARK: - Costura

    public struct EstadoCostura: Equatable, Sendable {
        public let fonte: FonteDeAudio
        public let geracao: Int
        public let confirmadoAte: Int
        public let fechada: Bool
    }

    public func abrirCostura(documento: String, fonte: FonteDeAudio, geracao: Int, inicio: Int) throws {
        try escrever { c in
            try c.rodar("INSERT OR IGNORE INTO lt_costuras (documento, fonte, geracao, confirmado_ate) VALUES (?,?,?,?)",
                        [.texto(documento), .texto(fonte.rawValue), .inteiro(Int64(geracao)), .inteiro(Int64(inicio))])
        }
    }

    public func costuras(documento: String) throws -> [EstadoCostura] {
        try ler { c in
            try c.consultar("SELECT fonte, geracao, confirmado_ate, fechada FROM lt_costuras WHERE documento = ? ORDER BY fonte, geracao",
                            [.texto(documento)]) { l in
                EstadoCostura(fonte: FonteDeAudio(rawValue: l.texto(0)) ?? .mac, geracao: Int(l.inteiro(1)),
                              confirmadoAte: Int(l.inteiro(2)), fechada: l.inteiro(3) == 1)
            }
        }
    }

    /// Toma a costura para uma tentativa. Só um dono confirma de cada vez.
    public func tomarCostura(documento: String, fonte: FonteDeAudio, geracao: Int, dono: String) throws {
        try escrever { c in
            try c.rodar("UPDATE lt_costuras SET dono = ? WHERE documento = ? AND fonte = ? AND geracao = ?",
                        [.texto(dono), .texto(documento), .texto(fonte.rawValue), .inteiro(Int64(geracao))])
        }
    }

    public struct Passo: Sendable {
        public let documento: String
        public let fonte: FonteDeAudio
        public let geracao: Int
        public let dono: String
        public let esperado: Int
        public let novo: Int
        public let trechos: [TrechoNovo]
        /// Blocos com atividade de voz entre os que esta confirmação resolve.
        public let comAtividade: Set<String>
        public let pendencia: (inicio: Int, fim: Int, motivo: String)?
        public let fechar: Bool

        public init(documento: String, fonte: FonteDeAudio, geracao: Int, dono: String, esperado: Int, novo: Int,
                    trechos: [TrechoNovo], comAtividade: Set<String>, pendencia: (inicio: Int, fim: Int, motivo: String)?, fechar: Bool) {
            self.documento = documento
            self.fonte = fonte
            self.geracao = geracao
            self.dono = dono
            self.esperado = esperado
            self.novo = novo
            self.trechos = trechos
            self.comAtividade = comAtividade
            self.pendencia = pendencia
            self.fechar = fechar
        }
    }

    /// Confirma um passo da costura numa só transação: trechos, avanço do ponto confirmado e estado
    /// dos blocos resolvidos. Exige o ponto e o dono esperados; se outra tentativa avançou, nada muda.
    @discardableResult
    public func confirmar(_ p: Passo) throws -> Bool {
        try confirmarPersistindo(p) != nil
    }

    /// Como `confirmar`, devolvendo os trechos com o ID com que de fato ficaram no banco
    /// (sufixo em colisão; trecho idêntico já gravado não volta). Nil se outra tentativa avançou.
    public func confirmarPersistindo(_ p: Passo) throws -> [TrechoNovo]? {
        try escrever { c in
            try c.transacao {
                guard try c.existe("SELECT 1 FROM lt_documentos WHERE id = ? AND estado <> 'descartada'", [.texto(p.documento)]),
                      try c.existe("""
                        SELECT 1 FROM lt_costuras WHERE documento = ? AND fonte = ? AND geracao = ? AND confirmado_ate = ? AND dono = ?
                        """, [.texto(p.documento), .texto(p.fonte.rawValue), .inteiro(Int64(p.geracao)), .inteiro(Int64(p.esperado)), .texto(p.dono)])
                else { return nil }
                let persistidos = try p.trechos.compactMap { try Self.inserir($0, documento: p.documento, versao: "", em: c) }
                if let pend = p.pendencia {
                    try c.rodar("""
                        INSERT OR IGNORE INTO lt_pendencias (id, documento, fonte, geracao, inicio, fim, motivo, estado)
                        VALUES (?,?,?,?,?,?,?,'aberta')
                        """, [.texto("\(p.documento)|\(p.fonte.rawValue)|\(p.geracao)|\(pend.inicio)"), .texto(p.documento),
                              .texto(p.fonte.rawValue), .inteiro(Int64(p.geracao)), .inteiro(Int64(pend.inicio)),
                              .inteiro(Int64(pend.fim)), .texto(pend.motivo)])
                }
                try c.rodar("UPDATE lt_costuras SET confirmado_ate = ?, fechada = ? WHERE documento = ? AND fonte = ? AND geracao = ?",
                            [.inteiro(Int64(p.novo)), .inteiro(p.fechar ? 1 : 0), .texto(p.documento), .texto(p.fonte.rawValue), .inteiro(Int64(p.geracao))])
                try Self.resolverBlocos(documento: p.documento, fonte: p.fonte, geracao: p.geracao, ate: p.novo,
                                        comAtividade: p.comAtividade, em: c)
                return persistidos
            }
        }
    }

    /// Blocos capturados inteiramente antes do ponto confirmado deixam de estar abertos:
    /// pendentes se cruzam uma faixa não reconhecida, transcritos se têm fala ou atividade, sem fala no resto.
    private static func resolverBlocos(documento: String, fonte: FonteDeAudio, geracao: Int, ate: Int,
                                       comAtividade: Set<String>, em c: ConexaoSQLite) throws {
        let abertos = try blocos(documento: documento, em: c).filter {
            $0.id.fonte == fonte && $0.id.geracao == geracao && $0.estado == .capturado && $0.fim <= ate
        }
        for b in abertos {
            let pendente = try c.existe("""
                SELECT 1 FROM lt_pendencias WHERE documento = ? AND fonte = ? AND geracao = ? AND estado = 'aberta' AND inicio < ? AND fim > ?
                """, [.texto(documento), .texto(fonte.rawValue), .inteiro(Int64(geracao)), .inteiro(Int64(b.fim)), .inteiro(Int64(b.inicio))])
            let temFala = try c.existe("""
                SELECT 1 FROM lt_trechos WHERE documento = ? AND versao = '' AND fonte = ? AND (inicio + fim) * 8000 >= ? AND (inicio + fim) * 8000 < ?
                """, [.texto(documento), .texto(fonte.rawValue), .inteiro(Int64(b.inicio)), .inteiro(Int64(b.fim))])
            let estado: EstadoBloco = pendente ? .pendente : (temFala || comAtividade.contains(b.id.texto)) ? .transcrito : .semFala
            try c.rodar("UPDATE lt_blocos SET estado = ?, erro = ? WHERE id = ? AND estado = 'capturado'",
                        [.texto(estado.rawValue), pendente ? .texto("Fala não reconhecida. O áudio continua guardado para nova tentativa.") : .nulo,
                         .texto(b.id.texto)])
        }
    }

    /// Devolve o trecho com o ID gravado, ou nil se um trecho idêntico já estava no banco.
    @discardableResult
    static func inserir(_ t: TrechoNovo, documento: String, versao: String, origemLegado: String? = nil,
                        divergente: Bool = false, em c: ConexaoSQLite) throws -> TrechoNovo? {
        // O ID nasce da primeira amostra do trecho. Dois trechos diferentes podem começar na mesma
        // amostra (passos ou pendências distintos): o segundo ganha sufixo determinístico em vez de
        // travar a transação. O mesmo trecho repetido não duplica.
        var id = t.id
        var n = 0
        while let existente = try c.consultar(
            "SELECT documento, versao, fonte, inicio, fim, texto FROM lt_trechos WHERE id = ?", [.texto(id)], {
                ($0.texto(0), $0.texto(1), $0.texto(2), $0.real(3), $0.real(4), $0.texto(5))
            }).first {
            let (doc, ver, fonte, inicio, fim, texto) = existente
            if doc == documento, ver == versao, fonte == t.fonte.rawValue, inicio == t.inicio, fim == t.fim, texto == t.texto { return nil }
            n += 1
            id = t.id + "#\(n)"
        }
        try c.rodar("""
            INSERT INTO lt_trechos (id, documento, versao, fonte, inicio, fim, texto, voz, divergente, origem_legado, criado_em)
            VALUES (?,?,?,?,?,?,?,?,?,?,?)
            """, [.texto(id), .texto(documento), .texto(versao), .texto(t.fonte.rawValue), .real(t.inicio), .real(t.fim),
                  .texto(t.texto), .texto(t.voz), .inteiro(divergente ? 1 : 0), origemLegado.map { .texto($0) } ?? .nulo, .real(agora)])
        return id == t.id ? t : TrechoNovo(id: id, fonte: t.fonte, inicio: t.inicio, fim: t.fim, texto: t.texto, voz: t.voz)
    }

    // MARK: - Pendências

    public struct Pendencia: Equatable, Sendable {
        public let id: String
        public let fonte: FonteDeAudio
        public let geracao: Int
        public let inicio: Int
        public let fim: Int
        public let motivo: String
        public let tentativas: Int
        public let estado: String
    }

    public func pendencias(documento: String) throws -> [Pendencia] {
        try ler { c in try Self.pendencias(documento: documento, em: c) }
    }

    static func pendencias(documento: String, em c: ConexaoSQLite) throws -> [Pendencia] {
        try c.consultar("""
            SELECT id, fonte, geracao, inicio, fim, motivo, tentativas, estado FROM lt_pendencias WHERE documento = ? ORDER BY inicio
            """, [.texto(documento)]) { l in
            Pendencia(id: l.texto(0), fonte: FonteDeAudio(rawValue: l.texto(1)) ?? .mac, geracao: Int(l.inteiro(2)),
                      inicio: Int(l.inteiro(3)), fim: Int(l.inteiro(4)), motivo: l.texto(5), tentativas: Int(l.inteiro(6)),
                      estado: l.texto(7))
        }
    }

    public enum DesfechoPendencia: Sendable {
        case resolvida([TrechoNovo])
        case semFala
        case continua(String)
    }

    /// Nova tentativa de uma faixa não reconhecida. Exige a pendência aberta com o número de tentativas esperado.
    @discardableResult
    public func resolver(pendencia id: String, documento: String, tentativas: Int, desfecho: DesfechoPendencia) throws -> Bool {
        try escrever { c in
            try c.transacao {
                guard try c.existe("SELECT 1 FROM lt_documentos WHERE id = ? AND estado <> 'descartada'", [.texto(documento)]),
                      let p = try Self.pendencias(documento: documento, em: c).first(where: { $0.id == id }),
                      p.estado == "aberta", p.tentativas == tentativas else { return false }
                switch desfecho {
                case .resolvida(let trechos):
                    for t in trechos { try Self.inserir(t, documento: documento, versao: "", em: c) }
                    try c.rodar("UPDATE lt_pendencias SET estado = 'resolvida', tentativas = tentativas + 1 WHERE id = ?", [.texto(id)])
                case .semFala:
                    try c.rodar("UPDATE lt_pendencias SET estado = 'semFala', tentativas = tentativas + 1 WHERE id = ?", [.texto(id)])
                case .continua(let motivo):
                    try c.rodar("UPDATE lt_pendencias SET motivo = ?, tentativas = tentativas + 1 WHERE id = ?", [.texto(motivo), .texto(id)])
                    return true
                }
                try c.rodar("""
                    UPDATE lt_blocos SET estado = 'transcrito', erro = NULL
                    WHERE documento = ? AND fonte = ? AND geracao = ? AND estado = 'pendente' AND inicio < ? AND inicio + quantidade > ?
                      AND NOT EXISTS (SELECT 1 FROM lt_pendencias q WHERE q.documento = lt_blocos.documento AND q.fonte = lt_blocos.fonte
                        AND q.geracao = lt_blocos.geracao AND q.estado = 'aberta' AND q.inicio < lt_blocos.inicio + lt_blocos.quantidade AND q.fim > lt_blocos.inicio)
                    """, [.texto(documento), .texto(p.fonte.rawValue), .inteiro(Int64(p.geracao)), .inteiro(Int64(p.fim)), .inteiro(Int64(p.inicio))])
                return true
            }
        }
    }

    // MARK: - Passada final, descarte e retenção

    /// Grava a atribuição final por ID de trecho e, se pedido, libera o áudio já resolvido.
    /// Exige que nenhum bloco esteja aberto: o texto precisa estar completo antes.
    @discardableResult
    public func gravarFalantes(documento: String, atribuicao: [String: String], identificados: Bool, liberarAudio: Bool) throws -> Bool {
        try escrever { c in
            try c.transacao {
                guard try c.existe("SELECT 1 FROM lt_documentos WHERE id = ? AND estado NOT IN ('descartada')", [.texto(documento)]),
                      try !c.existe("SELECT 1 FROM lt_blocos WHERE documento = ? AND estado = 'capturado'", [.texto(documento)])
                else { return false }
                for (trecho, voz) in atribuicao {
                    try c.rodar("UPDATE lt_trechos SET voz_final = ? WHERE id = ? AND documento = ?", [.texto(voz), .texto(trecho), .texto(documento)])
                }
                try c.rodar("UPDATE lt_documentos SET falantes = ?, estado = 'finalizada' WHERE id = ?",
                            [.texto(identificados ? "identificados" : "indisponivel"), .texto(documento)])
                if liberarAudio {
                    try c.rodar("UPDATE lt_blocos SET audio_liberado = 1 WHERE documento = ? AND estado IN ('transcrito','semFala','lacuna')",
                                [.texto(documento)])
                }
                return true
            }
        }
    }

    /// Descarte explícito: estado terminal gravado antes de qualquer limpeza de arquivo.
    /// A fala já confirmada continua; reunião sem fala nenhuma sai da lista.
    @discardableResult
    public func descartar(documento: String) throws -> Bool {
        try escrever { c in
            try c.transacao {
                guard try c.existe("SELECT 1 FROM lt_documentos WHERE id = ? AND estado <> 'descartada'", [.texto(documento)]) else { return false }
                try c.rodar("INSERT OR IGNORE INTO lt_descartes (documento) VALUES (?)", [.texto(documento)])
                try c.rodar("""
                    UPDATE lt_blocos SET estado = CASE WHEN estado IN ('capturado','pendente') THEN 'descartado' ELSE estado END,
                        audio_liberado = 1 WHERE documento = ?
                    """, [.texto(documento)])
                try c.rodar("UPDATE lt_pendencias SET estado = 'descartada' WHERE documento = ? AND estado = 'aberta'", [.texto(documento)])
                try c.rodar("UPDATE lt_costuras SET dono = NULL, fechada = 1 WHERE documento = ?", [.texto(documento)])
                let temFala = try c.existe("SELECT 1 FROM lt_trechos WHERE documento = ?", [.texto(documento)])
                try c.rodar("""
                    UPDATE lt_documentos SET estado = ?, falantes = CASE WHEN falantes = 'identificados' THEN falantes ELSE 'indisponivel' END WHERE id = ?
                    """, [.texto(temFala ? "finalizada" : "descartada"), .texto(documento)])
                return true
            }
        }
    }

    // MARK: - Edições (registros à parte, aplicados na leitura)

    public func reatribuir(documento: String, trecho: String, voz: String) throws {
        try escrever { c in
            let versao = try Self.versaoDoTrecho(trecho, documento: documento, em: c)
            try c.rodar("INSERT INTO lt_edicoes (documento, versao, tipo, alvo, valor, criado_em) VALUES (?,?, 'vozTrecho', ?, ?, ?)",
                        [.texto(documento), .texto(versao), .texto(trecho), .texto(voz), .real(Self.agora)])
        }
    }

    /// Renomear para o nome de outra voz une as duas.
    public func renomear(documento: String, voz: String, nome: String) throws {
        let limpo = String(nome.trimmingCharacters(in: .whitespacesAndNewlines).prefix(120))
        guard !limpo.isEmpty else { throw ErroDeBanco(description: "nome vazio") }
        guard let doc = try self.documento(documento) else { throw ErroDeBanco(description: "documento inexistente") }
        let versao = try ler { c in try Self.versaoAtiva(documento, em: c) } ?? ""
        let destino = doc.vozes.first { $0.id != voz && Self.mesmoNome($0.nome, limpo) }
        try escrever { c in
            try c.rodar("INSERT INTO lt_edicoes (documento, versao, tipo, alvo, valor, criado_em) VALUES (?,?,?,?,?,?)",
                        [.texto(documento), .texto(versao), .texto(destino == nil ? "nomeVoz" : "uniao"), .texto(voz),
                         .texto(destino?.id ?? limpo), .real(Self.agora)])
        }
    }

    /// "Nova pessoa" a partir de um trecho: usa uma voz com o mesmo nome se existir.
    public func atribuirNovaPessoa(documento: String, trecho: String, nome: String) throws {
        let limpo = String(nome.trimmingCharacters(in: .whitespacesAndNewlines).prefix(120))
        guard !limpo.isEmpty, let doc = try self.documento(documento) else { throw ErroDeBanco(description: "nome vazio") }
        if let existente = doc.vozes.first(where: { Self.mesmoNome($0.nome, limpo) }) {
            try reatribuir(documento: documento, trecho: trecho, voz: existente.id)
            return
        }
        let voz = "manual-" + UUID().uuidString
        try escrever { c in
            let versao = try Self.versaoDoTrecho(trecho, documento: documento, em: c)
            try c.transacao {
                try c.rodar("INSERT INTO lt_edicoes (documento, versao, tipo, alvo, valor, criado_em) VALUES (?,?, 'nomeVoz', ?, ?, ?)",
                            [.texto(documento), .texto(versao), .texto(voz), .texto(limpo), .real(Self.agora)])
                try c.rodar("INSERT INTO lt_edicoes (documento, versao, tipo, alvo, valor, criado_em) VALUES (?,?, 'vozTrecho', ?, ?, ?)",
                            [.texto(documento), .texto(versao), .texto(trecho), .texto(voz), .real(Self.agora)])
            }
        }
    }

    static func mesmoNome(_ a: String, _ b: String) -> Bool {
        a.compare(b, options: [.caseInsensitive, .diacriticInsensitive], locale: Locale(identifier: "pt_BR")) == .orderedSame
    }

    private static func versaoDoTrecho(_ trecho: String, documento: String, em c: ConexaoSQLite) throws -> String {
        guard let v = try c.consultar("SELECT versao FROM lt_trechos WHERE id = ? AND documento = ?", [.texto(trecho), .texto(documento)],
                                      { $0.texto(0) }).first else { throw ErroDeBanco(description: "trecho inexistente") }
        return v
    }

    static func versaoAtiva(_ documento: String, em c: ConexaoSQLite) throws -> String? {
        try c.consultar("SELECT versao_ativa FROM lt_documentos WHERE id = ?", [.texto(documento)]) { $0.textoOpcional(0) }.first ?? nil
    }

    // MARK: - Versões de arquivo

    public struct Versao: Equatable, Sendable {
        public let id: String
        public let documento: String
        public let tentativa: Int
        public let origem: String
        public let estado: String
        public let identidade: String?
    }

    /// Cada tentativa tem identidade própria e um número crescente por documento.
    public func criarVersao(documento: String, nome: String, origem: String) throws -> Versao {
        try escrever { c in
            try c.transacao {
                try c.rodar("INSERT OR IGNORE INTO lt_documentos (id, tipo, nome, criado_em, estado, falantes) VALUES (?, 'arquivo', ?, ?, 'ativo', 'naoAplica')",
                            [.texto(documento), .texto(nome), .real(Self.agora)])
                let n = try c.consultar("SELECT COALESCE(MAX(tentativa), 0) + 1 FROM lt_versoes WHERE documento = ?", [.texto(documento)]) { Int($0.inteiro(0)) }.first ?? 1
                let v = Versao(id: UUID().uuidString, documento: documento, tentativa: n, origem: origem, estado: "emCurso", identidade: nil)
                try c.rodar("INSERT INTO lt_versoes (id, documento, tentativa, origem, estado, criada_em) VALUES (?,?,?,?, 'emCurso', ?)",
                            [.texto(v.id), .texto(documento), .inteiro(Int64(n)), .texto(origem), .real(Self.agora)])
                return v
            }
        }
    }

    public func versoes(documento: String) throws -> [Versao] {
        try ler { c in
            try c.consultar("SELECT id, documento, tentativa, origem, estado, identidade FROM lt_versoes WHERE documento = ? ORDER BY tentativa",
                            [.texto(documento)]) { l in
                Versao(id: l.texto(0), documento: l.texto(1), tentativa: Int(l.inteiro(2)), origem: l.texto(3),
                       estado: l.texto(4), identidade: l.textoOpcional(5))
            }
        }
    }

    public func versaoAtiva(documento: String) throws -> String? {
        try ler { c in try Self.versaoAtiva(documento, em: c) }
    }

    /// Cancelar só vale para tentativa em curso. A versão ativa não muda.
    @discardableResult
    public func encerrarVersao(_ id: String, estado: String, erro: String? = nil) throws -> Bool {
        try escrever { c in
            try c.rodar("UPDATE lt_versoes SET estado = ?, erro = ? WHERE id = ? AND estado = 'emCurso'",
                        [.texto(estado), erro.map { .texto($0) } ?? .nulo, .texto(id)]) == 1
        }
    }

    /// Confirma a decisão de uma janela sob a identidade da tentativa e do PCM que ela leu.
    @discardableResult
    public func confirmarJanela(versao: String, seq: Int, inicio: Int, fim: Int, soma: String, estado: EstadoBloco) throws -> Bool {
        try escrever { c in
            try c.transacao {
                guard try c.existe("SELECT 1 FROM lt_versoes WHERE id = ? AND estado = 'emCurso'", [.texto(versao)]) else { return false }
                try c.rodar("""
                    INSERT INTO lt_janelas_arquivo (versao,seq,inicio,fim,soma,estado) VALUES (?,?,?,?,?,?)
                    """, [.texto(versao), .inteiro(Int64(seq)), .inteiro(Int64(inicio)), .inteiro(Int64(fim)), .texto(soma), .texto(estado.rawValue)])
                return true
            }
        }
    }

    public struct Publicacao: Sendable {
        public let versao: String
        public let identidade: String
        public let trechos: [TrechoNovo]
        public let duracao: Double
        public let bruto: String
        public let processamento: Double
        public let baixaConfianca: Bool
        public let quantidadePCM: Int?

        public init(versao: String, identidade: String, trechos: [TrechoNovo], duracao: Double, bruto: String,
                    processamento: Double, baixaConfianca: Bool, quantidadePCM: Int? = nil) {
            self.versao = versao
            self.identidade = identidade
            self.trechos = trechos
            self.duracao = duracao
            self.bruto = bruto
            self.processamento = processamento
            self.baixaConfianca = baixaConfianca
            self.quantidadePCM = quantidadePCM
        }
    }

    /// Insere os trechos da versão e troca a versão ativa numa só transação. Exige a tentativa
    /// ainda em curso e mais nova que a ativa; senão desfaz e a versão anterior continua.
    @discardableResult
    public func publicar(_ p: Publicacao) throws -> Bool {
        try escrever { c in
            try c.transacao {
                guard let (documento, tentativa) = try c.consultar(
                    "SELECT documento, tentativa FROM lt_versoes WHERE id = ? AND estado = 'emCurso'", [.texto(p.versao)],
                    { ($0.texto(0), Int($0.inteiro(1))) }).first else { return false }
                if try c.existe("SELECT 1 FROM lt_versoes WHERE documento = ? AND tentativa > ? AND estado IN ('emCurso','pronta')",
                                [.texto(documento), .inteiro(Int64(tentativa))]) {
                    try c.rodar("UPDATE lt_versoes SET estado = 'superada' WHERE id = ?", [.texto(p.versao)])
                    return false
                }
                if let quantidade = p.quantidadePCM {
                    let janelas = try c.consultar("SELECT inicio,fim,estado FROM lt_janelas_arquivo WHERE versao = ? ORDER BY seq",
                                                 [.texto(p.versao)]) { ($0.inteiro(0), $0.inteiro(1), $0.texto(2)) }
                    var fim: Int64 = 0
                    for (inicio, ate, estado) in janelas {
                        guard inicio == fim, ate > inicio, estado == "transcrito" || estado == "semFala" else {
                            throw ErroDeBanco(description: "Há uma janela de áudio ainda sem transcrição.")
                        }
                        fim = ate
                    }
                    guard fim == Int64(quantidade) else { throw ErroDeBanco(description: "Nem todo o arquivo foi processado.") }
                }
                guard !p.trechos.isEmpty else {
                    try c.rodar("UPDATE lt_versoes SET estado = 'vazia', identidade = ? WHERE id = ?", [.texto(p.identidade), .texto(p.versao)])
                    return false
                }
                for t in p.trechos { try Self.inserir(t, documento: documento, versao: p.versao, em: c) }
                try c.rodar("""
                    UPDATE lt_versoes SET estado = 'pronta', identidade = ?, publicada_em = ?, duracao = ?, bruto = ?, processamento = ?, baixa_confianca = ?
                    WHERE id = ?
                    """, [.texto(p.identidade), .real(Self.agora), .real(p.duracao), .texto(p.bruto), .real(p.processamento),
                          .inteiro(p.baixaConfianca ? 1 : 0), .texto(p.versao)])
                try c.rodar("UPDATE lt_documentos SET versao_ativa = ?, duracao = ?, oculto = 0 WHERE id = ?",
                            [.texto(p.versao), .real(p.duracao), .texto(documento)])
                return true
            }
        }
    }
}
