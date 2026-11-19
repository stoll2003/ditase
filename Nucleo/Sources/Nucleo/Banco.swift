import Foundation
import SQLite3

public struct ErroDeBanco: Error, CustomStringConvertible {
    public let description: String
}

/// Um ditado no histórico.
public struct Ditado: Codable, Equatable, Identifiable, Sendable {
    public var id: Int64
    public var iniciadoEm: Double
    /// Segundos entre apertar e soltar.
    public var duracao: Double
    /// Segundos em que houve fala.
    public var duracaoFala: Double
    public var palavras: Int
    public var textoBruto: String
    public var texto: String
    public var app: String
    public var appNome: String
    /// gravando, concluido, parcial (algum trecho não foi transcrito), recuperado ou falhou.
    public var estado: String
    /// Segundos entre soltar a tecla e o texto ficar pronto.
    public var latencia: Double
    public var motor: String
    public var limpador: String
    public var trocasDicionario: Int
    /// "ditado" ou "wispr" (importado).
    public var origem: String
    public var audio: String

    public init(id: Int64 = 0, iniciadoEm: Double, duracao: Double = 0, duracaoFala: Double = 0, palavras: Int = 0, textoBruto: String = "", texto: String = "",
                app: String = "", appNome: String = "", estado: String = "gravando", latencia: Double = 0, motor: String = "", limpador: String = "",
                trocasDicionario: Int = 0, origem: String = "ditado", audio: String = "") {
        self.id = id
        self.iniciadoEm = iniciadoEm
        self.duracao = duracao
        self.duracaoFala = duracaoFala
        self.palavras = palavras
        self.textoBruto = textoBruto
        self.texto = texto
        self.app = app
        self.appNome = appNome
        self.estado = estado
        self.latencia = latencia
        self.motor = motor
        self.limpador = limpador
        self.trocasDicionario = trocasDicionario
        self.origem = origem
        self.audio = audio
    }
}

/// Banco SQLite do app: histórico, trechos em andamento, dicionário e ajustes.
/// Todas as chamadas passam por uma fila própria; pode ser usado de qualquer thread.
public final class Banco: @unchecked Sendable {
    public let caminho: String
    private var db: OpaquePointer?
    private let fila = DispatchQueue(label: "ditado.banco")
    private static let transitorio = unsafeBitCast(-1, to: sqlite3_destructor_type.self)

    public init(caminho: String) throws {
        self.caminho = caminho
        if caminho != ":memory:" {
            try FileManager.default.createDirectory(at: URL(fileURLWithPath: caminho).deletingLastPathComponent(), withIntermediateDirectories: true)
        }
        guard sqlite3_open_v2(caminho, &db, SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_FULLMUTEX, nil) == SQLITE_OK else {
            throw ErroDeBanco(description: "não abriu o banco em \(caminho)")
        }
        try executar("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=3000;")
        try migrar()
    }

    deinit { sqlite3_close(db) }

    private func migrar() throws {
        try executar("""
        CREATE TABLE IF NOT EXISTS ditados (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            iniciado_em REAL NOT NULL, duracao REAL NOT NULL DEFAULT 0, duracao_fala REAL NOT NULL DEFAULT 0,
            palavras INTEGER NOT NULL DEFAULT 0, texto_bruto TEXT NOT NULL DEFAULT '', texto TEXT NOT NULL DEFAULT '',
            app TEXT NOT NULL DEFAULT '', app_nome TEXT NOT NULL DEFAULT '', estado TEXT NOT NULL DEFAULT 'gravando',
            latencia REAL NOT NULL DEFAULT 0, motor TEXT NOT NULL DEFAULT '', limpador TEXT NOT NULL DEFAULT '',
            trocas_dicionario INTEGER NOT NULL DEFAULT 0, origem TEXT NOT NULL DEFAULT 'ditado', audio TEXT NOT NULL DEFAULT '',
            chave_externa TEXT UNIQUE
        );
        CREATE INDEX IF NOT EXISTS ditados_por_data ON ditados (iniciado_em);
        CREATE TABLE IF NOT EXISTS trechos (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ditado_id INTEGER NOT NULL REFERENCES ditados(id) ON DELETE CASCADE,
            ordem INTEGER NOT NULL, bruto TEXT NOT NULL, limpo TEXT
        );
        CREATE INDEX IF NOT EXISTS trechos_por_ditado ON trechos (ditado_id, ordem);
        CREATE TABLE IF NOT EXISTS dicionario (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            termo TEXT NOT NULL UNIQUE COLLATE NOCASE, ouvido_como TEXT NOT NULL DEFAULT '[]',
            usos INTEGER NOT NULL DEFAULT 0, origem TEXT NOT NULL DEFAULT 'manual', criado_em REAL NOT NULL
        );
        CREATE TABLE IF NOT EXISTS transcricoes_longas (
            id TEXT PRIMARY KEY, tipo TEXT NOT NULL, criado_em REAL NOT NULL, dados TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS transcricoes_longas_por_tipo ON transcricoes_longas (tipo, criado_em DESC);
        CREATE TABLE IF NOT EXISTS config (chave TEXT PRIMARY KEY, valor TEXT NOT NULL);
        """)
    }

    // MARK: - Acesso cru

    private func executar(_ sql: String) throws {
        var erro: UnsafeMutablePointer<CChar>?
        guard sqlite3_exec(db, sql, nil, nil, &erro) == SQLITE_OK else {
            let msg = erro.map { String(cString: $0) } ?? "erro"
            sqlite3_free(erro)
            throw ErroDeBanco(description: msg)
        }
    }

    enum Valor {
        case texto(String), inteiro(Int64), real(Double), nulo
    }

    private func preparar(_ sql: String, _ valores: [Valor]) throws -> OpaquePointer {
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let stmt else {
            throw ErroDeBanco(description: String(cString: sqlite3_errmsg(db)))
        }
        for (i, v) in valores.enumerated() {
            let n = Int32(i + 1)
            switch v {
            case .texto(let s): sqlite3_bind_text(stmt, n, s, -1, Self.transitorio)
            case .inteiro(let x): sqlite3_bind_int64(stmt, n, x)
            case .real(let x): sqlite3_bind_double(stmt, n, x)
            case .nulo: sqlite3_bind_null(stmt, n)
            }
        }
        return stmt
    }

    @discardableResult
    private func rodar(_ sql: String, _ valores: [Valor] = []) throws -> Int64 {
        let stmt = try preparar(sql, valores)
        defer { sqlite3_finalize(stmt) }
        guard sqlite3_step(stmt) == SQLITE_DONE else { throw ErroDeBanco(description: String(cString: sqlite3_errmsg(db))) }
        return sqlite3_last_insert_rowid(db)
    }

    private func consultar<T>(_ sql: String, _ valores: [Valor] = [], _ ler: (OpaquePointer) -> T) throws -> [T] {
        let stmt = try preparar(sql, valores)
        defer { sqlite3_finalize(stmt) }
        var saida: [T] = []
        while true {
            let r = sqlite3_step(stmt)
            if r == SQLITE_ROW { saida.append(ler(stmt)) }
            else if r == SQLITE_DONE { break }
            else { throw ErroDeBanco(description: String(cString: sqlite3_errmsg(db))) }
        }
        return saida
    }

    private static func texto(_ s: OpaquePointer, _ i: Int32) -> String {
        sqlite3_column_text(s, i).map { String(cString: $0) } ?? ""
    }

    // MARK: - Ditados

    private static let colunas = "id, iniciado_em, duracao, duracao_fala, palavras, texto_bruto, texto, app, app_nome, estado, latencia, motor, limpador, trocas_dicionario, origem, audio"

    private static func lerDitado(_ s: OpaquePointer) -> Ditado {
        Ditado(id: sqlite3_column_int64(s, 0), iniciadoEm: sqlite3_column_double(s, 1), duracao: sqlite3_column_double(s, 2),
               duracaoFala: sqlite3_column_double(s, 3), palavras: Int(sqlite3_column_int64(s, 4)), textoBruto: texto(s, 5), texto: texto(s, 6),
               app: texto(s, 7), appNome: texto(s, 8), estado: texto(s, 9), latencia: sqlite3_column_double(s, 10), motor: texto(s, 11),
               limpador: texto(s, 12), trocasDicionario: Int(sqlite3_column_int64(s, 13)), origem: texto(s, 14), audio: texto(s, 15))
    }

    /// Grava um ditado novo e devolve o id. `chaveExterna` evita importar duas vezes o mesmo registro.
    @discardableResult
    public func inserir(_ d: Ditado, chaveExterna: String? = nil) throws -> Int64 {
        try fila.sync {
            try rodar("""
            INSERT INTO ditados (iniciado_em, duracao, duracao_fala, palavras, texto_bruto, texto, app, app_nome, estado, latencia, motor, limpador, trocas_dicionario, origem, audio, chave_externa)
            VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
            """, [.real(d.iniciadoEm), .real(d.duracao), .real(d.duracaoFala), .inteiro(Int64(d.palavras)), .texto(d.textoBruto), .texto(d.texto),
                  .texto(d.app), .texto(d.appNome), .texto(d.estado), .real(d.latencia), .texto(d.motor), .texto(d.limpador),
                  .inteiro(Int64(d.trocasDicionario)), .texto(d.origem), .texto(d.audio), chaveExterna.map { .texto($0) } ?? .nulo])
        }
    }

    public func existe(chaveExterna: String) throws -> Bool {
        try fila.sync { try !consultar("SELECT 1 FROM ditados WHERE chave_externa = ?", [.texto(chaveExterna)]) { _ in true }.isEmpty }
    }

    public func atualizar(_ d: Ditado) throws {
        try fila.sync {
            try rodar("""
            UPDATE ditados SET duracao=?, duracao_fala=?, palavras=?, texto_bruto=?, texto=?, app=?, app_nome=?, estado=?, latencia=?, motor=?, limpador=?, trocas_dicionario=?, audio=? WHERE id=?
            """, [.real(d.duracao), .real(d.duracaoFala), .inteiro(Int64(d.palavras)), .texto(d.textoBruto), .texto(d.texto), .texto(d.app), .texto(d.appNome),
                  .texto(d.estado), .real(d.latencia), .texto(d.motor), .texto(d.limpador), .inteiro(Int64(d.trocasDicionario)), .texto(d.audio), .inteiro(d.id)])
            _ = ()
        }
    }

    /// Grava o resultado final e só então apaga os trechos de reserva, numa transação:
    /// se a gravação falhar, os trechos continuam lá e o ditado segue recuperável.
    public func concluir(_ d: Ditado) throws {
        try fila.sync {
            try executar("BEGIN IMMEDIATE")
            do {
                try rodar("""
                UPDATE ditados SET duracao=?, duracao_fala=?, palavras=?, texto_bruto=?, texto=?, app=?, app_nome=?, estado=?, latencia=?, motor=?, limpador=?, trocas_dicionario=?, audio=? WHERE id=?
                """, [.real(d.duracao), .real(d.duracaoFala), .inteiro(Int64(d.palavras)), .texto(d.textoBruto), .texto(d.texto), .texto(d.app), .texto(d.appNome),
                      .texto(d.estado), .real(d.latencia), .texto(d.motor), .texto(d.limpador), .inteiro(Int64(d.trocasDicionario)), .texto(d.audio), .inteiro(d.id)])
                guard sqlite3_changes(db) == 1 else { throw ErroDeBanco(description: "ditado \(d.id) não encontrado ao concluir") }
                try rodar("DELETE FROM trechos WHERE ditado_id = ?", [.inteiro(d.id)])
                try executar("COMMIT")
            } catch {
                try? executar("ROLLBACK")
                throw error
            }
        }
    }

    public func ditado(id: Int64) throws -> Ditado? {
        try fila.sync { try consultar("SELECT \(Self.colunas) FROM ditados WHERE id = ?", [.inteiro(id)], Self.lerDitado).first }
    }

    /// Histórico do mais novo para o mais antigo, sem os cancelados. `busca` filtra pelo texto.
    public func historico(limite: Int = 100, antesDe: Double? = nil, busca: String = "") throws -> [Ditado] {
        try fila.sync {
            var sql = "SELECT \(Self.colunas) FROM ditados WHERE estado IN ('concluido','parcial','recuperado','falhou')"
            var valores: [Valor] = []
            if let antesDe {
                sql += " AND iniciado_em < ?"
                valores.append(.real(antesDe))
            }
            if !busca.isEmpty {
                sql += " AND texto LIKE ? ESCAPE '\\'"
                let escapada = busca.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "%", with: "\\%").replacingOccurrences(of: "_", with: "\\_")
                valores.append(.texto("%\(escapada)%"))
            }
            sql += " ORDER BY iniciado_em DESC LIMIT ?"
            valores.append(.inteiro(Int64(limite)))
            return try consultar(sql, valores, Self.lerDitado)
        }
    }

    public func ultimoConcluido() throws -> Ditado? {
        try fila.sync {
            try consultar("SELECT \(Self.colunas) FROM ditados WHERE estado IN ('concluido','parcial','recuperado') AND origem='ditado' ORDER BY iniciado_em DESC LIMIT 1", [], Self.lerDitado).first
        }
    }

    public func apagarDitado(id: Int64) throws {
        try fila.sync { _ = try rodar("DELETE FROM ditados WHERE id = ?", [.inteiro(id)]) }
    }

    /// Ditados para as estatísticas: só os que produziram texto.
    public func paraEstatisticas() throws -> [Ditado] {
        try fila.sync {
            try consultar("SELECT \(Self.colunas) FROM ditados WHERE estado IN ('concluido','parcial','recuperado') AND palavras > 0 ORDER BY iniciado_em", [], Self.lerDitado)
        }
    }

    // MARK: - Trechos (proteção contra queda durante ditado longo)

    public func gravarTrecho(ditado: Int64, ordem: Int, bruto: String) throws {
        try fila.sync { _ = try rodar("INSERT INTO trechos (ditado_id, ordem, bruto) VALUES (?,?,?)", [.inteiro(ditado), .inteiro(Int64(ordem)), .texto(bruto)]) }
    }

    public func trechos(ditado: Int64) throws -> [String] {
        try fila.sync { try consultar("SELECT bruto FROM trechos WHERE ditado_id = ? ORDER BY ordem", [.inteiro(ditado)]) { Self.texto($0, 0) } }
    }

    public func apagarTrechos(ditado: Int64) throws {
        try fila.sync { _ = try rodar("DELETE FROM trechos WHERE ditado_id = ?", [.inteiro(ditado)]) }
    }

    /// Ditados que ficaram "gravando" porque o app fechou no meio.
    public func interrompidos() throws -> [Ditado] {
        try fila.sync { try consultar("SELECT \(Self.colunas) FROM ditados WHERE estado = 'gravando'", [], Self.lerDitado) }
    }

    // MARK: - Dicionário

    private static func lerTermo(_ s: OpaquePointer) -> Termo {
        let formas = (try? JSONDecoder().decode([String].self, from: Data(texto(s, 2).utf8))) ?? []
        return Termo(id: sqlite3_column_int64(s, 0), termo: texto(s, 1), ouvidoComo: formas, usos: Int(sqlite3_column_int64(s, 3)),
                     origem: texto(s, 4), criadoEm: sqlite3_column_double(s, 5))
    }

    public func termos() throws -> [Termo] {
        try fila.sync { try consultar("SELECT id, termo, ouvido_como, usos, origem, criado_em FROM dicionario ORDER BY termo COLLATE NOCASE", [], Self.lerTermo) }
    }

    /// Cria o termo ou, se já existe com o mesmo nome, junta as formas "ouvido como".
    @discardableResult
    public func salvar(termo: Termo) throws -> Termo {
        try fila.sync {
            let nome = termo.termo.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !nome.isEmpty else { throw ErroDeBanco(description: "termo vazio") }
            func limpar(_ formas: [String]) -> [String] {
                var vistos = Set<String>()
                return formas.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                    .filter { !$0.isEmpty && $0.caseInsensitiveCompare(nome) != .orderedSame && vistos.insert($0.lowercased()).inserted }
            }
            func json(_ formas: [String]) -> String { String(data: (try? JSONEncoder().encode(formas)) ?? Data("[]".utf8), encoding: .utf8) ?? "[]" }
            if termo.id != 0 {
                try rodar("UPDATE dicionario SET termo=?, ouvido_como=? WHERE id=?", [.texto(nome), .texto(json(limpar(termo.ouvidoComo))), .inteiro(termo.id)])
                var t = termo
                t.termo = nome
                t.ouvidoComo = limpar(termo.ouvidoComo)
                return t
            }
            if let existente = try consultar("SELECT id, termo, ouvido_como, usos, origem, criado_em FROM dicionario WHERE termo = ? COLLATE NOCASE", [.texto(nome)], Self.lerTermo).first {
                var t = existente
                t.termo = nome
                t.ouvidoComo = limpar(existente.ouvidoComo + termo.ouvidoComo)
                try rodar("UPDATE dicionario SET termo=?, ouvido_como=? WHERE id=?", [.texto(nome), .texto(json(t.ouvidoComo)), .inteiro(t.id)])
                return t
            }
            var t = termo
            t.termo = nome
            t.ouvidoComo = limpar(termo.ouvidoComo)
            t.id = try rodar("INSERT INTO dicionario (termo, ouvido_como, usos, origem, criado_em) VALUES (?,?,?,?,?)",
                             [.texto(nome), .texto(json(t.ouvidoComo)), .inteiro(Int64(t.usos)), .texto(t.origem), .real(t.criadoEm)])
            return t
        }
    }

    public func apagarTermo(id: Int64) throws {
        try fila.sync { _ = try rodar("DELETE FROM dicionario WHERE id = ?", [.inteiro(id)]) }
    }

    public func contarUso(termos ids: [Int64]) throws {
        try fila.sync { for id in ids { try rodar("UPDATE dicionario SET usos = usos + 1 WHERE id = ?", [.inteiro(id)]) } }
    }

    // MARK: - Ajustes

    public func config(_ chave: String) -> String? {
        fila.sync { (try? consultar("SELECT valor FROM config WHERE chave = ?", [.texto(chave)]) { Self.texto($0, 0) })?.first }
    }

    public func definir(config chave: String, valor: String) throws {
        try fila.sync { _ = try rodar("INSERT INTO config (chave, valor) VALUES (?,?) ON CONFLICT(chave) DO UPDATE SET valor = excluded.valor", [.texto(chave), .texto(valor)]) }
    }
}

extension Banco {
    public func salvar(transcricao: TranscricaoLonga) throws {
        let dados = String(decoding: try JSONEncoder().encode(transcricao), as: UTF8.self)
        try fila.sync {
            _ = try rodar("""
                INSERT INTO transcricoes_longas (id, tipo, criado_em, dados) VALUES (?, ?, ?, ?)
                ON CONFLICT(id) DO UPDATE SET dados = excluded.dados
                """, [.texto(transcricao.id), .texto(transcricao.fonte == nil ? "arquivo" : "reuniao"),
                      .real(transcricao.criadoEm), .texto(dados)])
        }
    }

    public func transcricao(id: String) throws -> TranscricaoLonga? {
        let linha = try fila.sync {
            try consultar("SELECT dados FROM transcricoes_longas WHERE id = ?", [.texto(id)]) { Self.texto($0, 0) }.first
        }
        return try linha.map { try JSONDecoder().decode(TranscricaoLonga.self, from: Data($0.utf8)) }
    }

    public func transcricoes(tipo: String, detalhes: Bool = true) throws -> [TranscricaoLonga] {
        // A lista leve não leva trechos, mas calcula no SQLite a contagem de palavras e a primeira fala.
        let palavras = """
            (SELECT coalesce(sum(CASE WHEN trim(json_extract(value, '$.texto')) = '' THEN 0 ELSE
              length(trim(json_extract(value, '$.texto'))) - length(replace(trim(json_extract(value, '$.texto')), ' ', '')) + 1 END), 0)
             FROM json_each(dados, '$.trechos'))
            """
        let selecao = detalhes ? "dados" : """
            json_set(json_remove(dados, '$.bruto'), '$.trechos', json('[]'), '$.palavras', \(palavras),
                     '$.previa', substr(json_extract(dados, '$.trechos[0].texto'), 1, 240))
            """
        let linhas = try fila.sync {
            try consultar("SELECT \(selecao) FROM transcricoes_longas WHERE tipo = ? ORDER BY criado_em DESC",
                          [.texto(tipo)]) { Self.texto($0, 0) }
        }
        return try linhas.map { try JSONDecoder().decode(TranscricaoLonga.self, from: Data($0.utf8)) }
    }
}
