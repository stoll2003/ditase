import AppKit
import Foundation
import Nucleo
import SQLite3

/// Traz para o Ditado o histórico e o dicionário do Wispr Flow instalado neste Mac.
/// Só lê o banco do Wispr; nada é alterado lá. Importar de novo não duplica.
enum ImportadorWispr {
    struct Resumo: Codable {
        var ditados = 0
        var termos = 0
        var encontrado = true
    }

    static var bancoDoWispr: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("Wispr Flow/flow.sqlite")
    }

    static var disponivel: Bool { FileManager.default.fileExists(atPath: bancoDoWispr.path) }

    static func importar(para banco: Banco) -> Resumo {
        var resumo = Resumo()
        guard disponivel else {
            resumo.encontrado = false
            return resumo
        }
        var db: OpaquePointer?
        let uri = "file:\(bancoDoWispr.path.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? "")?mode=ro"
        guard sqlite3_open_v2(uri, &db, SQLITE_OPEN_READONLY | SQLITE_OPEN_URI, nil) == SQLITE_OK else {
            resumo.encontrado = false
            sqlite3_close(db)
            return resumo
        }
        defer { sqlite3_close(db) }
        sqlite3_busy_timeout(db, 2000)

        let formato = DateFormatter()
        formato.locale = Locale(identifier: "en_US_POSIX")
        formato.dateFormat = "yyyy-MM-dd HH:mm:ss.SSS XXXXX"
        var nomes: [String: String] = [:]
        func nomeDoApp(_ id: String) -> String {
            if let n = nomes[id] { return n }
            let n = NSWorkspace.shared.urlForApplication(withBundleIdentifier: id).map { FileManager.default.displayName(atPath: $0.path).replacingOccurrences(of: ".app", with: "") } ?? id
            nomes[id] = n
            return n
        }
        func texto(_ s: OpaquePointer?, _ i: Int32) -> String { sqlite3_column_text(s, i).map { String(cString: $0) } ?? "" }

        var stmt: OpaquePointer?
        let sql = "SELECT transcriptEntityId, COALESCE(formattedText,''), COALESCE(asrText,''), timestamp, COALESCE(duration,0), COALESCE(speechDuration,0), COALESCE(numWords,0), COALESCE(app,''), COALESCE(e2eLatency,0) FROM History WHERE numWords > 0"
        if sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK {
            while sqlite3_step(stmt) == SQLITE_ROW {
                let chave = "wispr:" + texto(stmt, 0)
                guard (try? banco.existe(chaveExterna: chave)) == false, let data = formato.date(from: texto(stmt, 3)) else { continue }
                let app = texto(stmt, 7)
                let d = Ditado(iniciadoEm: data.timeIntervalSince1970, duracao: sqlite3_column_double(stmt, 4), duracaoFala: sqlite3_column_double(stmt, 5),
                               palavras: Int(sqlite3_column_int64(stmt, 6)), textoBruto: texto(stmt, 2), texto: texto(stmt, 1), app: app,
                               appNome: app.isEmpty ? "" : nomeDoApp(app), estado: "concluido", latencia: sqlite3_column_double(stmt, 8) / 1000,
                               motor: "wispr", limpador: "wispr", origem: "wispr")
                if (try? banco.inserir(d, chaveExterna: chave)) != nil { resumo.ditados += 1 }
            }
        }
        sqlite3_finalize(stmt)

        let existentes = Set(((try? banco.termos()) ?? []).map { $0.termo.lowercased() })
        if sqlite3_prepare_v2(db, "SELECT phrase, COALESCE(replacement,'') FROM Dictionary WHERE isDeleted = 0 AND isSnippet = 0", -1, &stmt, nil) == SQLITE_OK {
            while sqlite3_step(stmt) == SQLITE_ROW {
                let frase = texto(stmt, 0), troca = texto(stmt, 1)
                let termo = troca.isEmpty ? frase : troca
                guard !termo.isEmpty, !existentes.contains(termo.lowercased()) else { continue }
                if (try? banco.salvar(termo: Termo(termo: termo, ouvidoComo: troca.isEmpty ? [] : [frase], origem: "wispr"))) != nil { resumo.termos += 1 }
            }
        }
        sqlite3_finalize(stmt)
        return resumo
    }
}
