import Foundation
import SQLite3

/// Conexão SQLite usada só pelo armazém de transcrições longas.
/// Não é segura entre threads: cada conexão vive numa fila serial do armazém.
final class ConexaoSQLite {
    enum Valor: Equatable {
        case texto(String), inteiro(Int64), real(Double), nulo
    }

    struct Linha {
        fileprivate let stmt: OpaquePointer
        func texto(_ i: Int32) -> String { sqlite3_column_text(stmt, i).map { String(cString: $0) } ?? "" }
        func textoOpcional(_ i: Int32) -> String? {
            sqlite3_column_type(stmt, i) == SQLITE_NULL ? nil : texto(i)
        }
        func inteiro(_ i: Int32) -> Int64 { sqlite3_column_int64(stmt, i) }
        func inteiroOpcional(_ i: Int32) -> Int64? { sqlite3_column_type(stmt, i) == SQLITE_NULL ? nil : inteiro(i) }
        func real(_ i: Int32) -> Double { sqlite3_column_double(stmt, i) }
        func realOpcional(_ i: Int32) -> Double? { sqlite3_column_type(stmt, i) == SQLITE_NULL ? nil : real(i) }
    }

    private var db: OpaquePointer?
    private static let transitorio = unsafeBitCast(-1, to: sqlite3_destructor_type.self)

    init(caminho: String, escrita: Bool) throws {
        let flags = escrita ? (SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE) : SQLITE_OPEN_READONLY
        guard sqlite3_open_v2(caminho, &db, flags | SQLITE_OPEN_NOMUTEX, nil) == SQLITE_OK else {
            throw ErroDeBanco(description: "não abriu o armazém em \(caminho)")
        }
        // WAL para leitores concorrentes. FULL com fullfsync: o commit de um bloco sobrevive a queda de energia.
        try executar("PRAGMA busy_timeout=5000;")
        if escrita {
            try executar("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA fullfsync=ON; PRAGMA checkpoint_fullfsync=ON; PRAGMA foreign_keys=ON;")
        }
    }

    deinit { sqlite3_close(db) }

    func executar(_ sql: String) throws {
        var erro: UnsafeMutablePointer<CChar>?
        guard sqlite3_exec(db, sql, nil, nil, &erro) == SQLITE_OK else {
            let msg = erro.map { String(cString: $0) } ?? "erro"
            sqlite3_free(erro)
            throw ErroDeBanco(description: msg)
        }
    }

    private func preparar(_ sql: String, _ valores: [Valor]) throws -> OpaquePointer {
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let stmt else {
            throw ErroDeBanco(description: String(cString: sqlite3_errmsg(db)) + " em: " + sql)
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

    /// Executa e devolve o número de linhas alteradas.
    @discardableResult
    func rodar(_ sql: String, _ valores: [Valor] = []) throws -> Int {
        let stmt = try preparar(sql, valores)
        defer { sqlite3_finalize(stmt) }
        let r = sqlite3_step(stmt)
        guard r == SQLITE_DONE || r == SQLITE_ROW else {
            throw ErroDeBanco(description: String(cString: sqlite3_errmsg(db)))
        }
        return Int(sqlite3_changes(db))
    }

    func consultar<T>(_ sql: String, _ valores: [Valor] = [], _ ler: (Linha) throws -> T) throws -> [T] {
        let stmt = try preparar(sql, valores)
        defer { sqlite3_finalize(stmt) }
        var saida: [T] = []
        while true {
            let r = sqlite3_step(stmt)
            if r == SQLITE_ROW { saida.append(try ler(Linha(stmt: stmt))) }
            else if r == SQLITE_DONE { break }
            else { throw ErroDeBanco(description: String(cString: sqlite3_errmsg(db))) }
        }
        return saida
    }

    func existe(_ sql: String, _ valores: [Valor] = []) throws -> Bool {
        try !consultar(sql, valores) { _ in true }.isEmpty
    }

    /// Transação imediata: confirma se o bloco termina, desfaz se lança.
    func transacao<T>(_ corpo: () throws -> T) throws -> T {
        try executar("BEGIN IMMEDIATE")
        do {
            let r = try corpo()
            try executar("COMMIT")
            return r
        } catch {
            try? executar("ROLLBACK")
            throw error
        }
    }
}
