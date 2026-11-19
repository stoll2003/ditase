import Foundation
import XCTest
@testable import Nucleo

/// Reconhecedor dublê com roteiro de palavras em segundos absolutos. Devolve só as palavras
/// inteiras dentro da janela; a palavra cortada pela borda volta truncada ("pal~"), como um
/// reconhecedor real erra numa fronteira. Pode devolver vazio nas primeiras chamadas.
final class Oraculo: ReconhecedorDeFala, @unchecked Sendable {
    let roteiro: [PalavraReconhecida]
    private let trava = NSLock()
    private var vaziasRestantes: Int
    private(set) var chamadas = 0

    private func consumirVazia() -> Bool {
        trava.withLock {
            chamadas += 1
            let vazio = vaziasRestantes > 0
            if vazio { vaziasRestantes -= 1 }
            return vazio
        }
    }

    init(_ roteiro: [PalavraReconhecida], vaziasPrimeiro: Int = 0) {
        self.roteiro = roteiro
        vaziasRestantes = vaziasPrimeiro
    }

    func palavras(_ amostras: [Float], inicio: Double, fimDaFala: Double?) async throws -> [PalavraReconhecida] {
        if consumirVazia() { return [] }
        let fim = inicio + (fimDaFala ?? Double(amostras.count) / Double(taxaLancamento))
        return roteiro.compactMap { p in
            guard p.fim > inicio, p.inicio < fim else { return nil }
            if p.inicio >= inicio - 1e-9, p.fim <= fim + 1e-9 { return p }
            return PalavraReconhecida(texto: String(p.texto.prefix(3)) + "~", inicio: max(p.inicio, inicio), fim: min(p.fim, fim))
        }
    }
}

final class Identificador: IdentificadorDeFalantes, @unchecked Sendable {
    let faixasFixas: [FaixaDeVoz]
    let falhar: Bool
    init(_ faixas: [FaixaDeVoz], falhar: Bool = false) { faixasFixas = faixas; self.falhar = falhar }
    func faixas(wav: URL) async throws -> [FaixaDeVoz] {
        if falhar { throw CocoaError(.fileReadUnknown) }
        XCTAssertTrue(FileManager.default.fileExists(atPath: wav.path))
        return faixasFixas
    }
}

struct PCMEmMemoria: FonteDePCM {
    let amostras: [Float]
    var quantidade: Int { amostras.count }
    func ler(inicio: Int, quantidade: Int) throws -> [Float] {
        Array(amostras[max(0, inicio)..<min(amostras.count, inicio + quantidade)])
    }
}

enum Sintetico {
    /// Roteiro de palavras com duração e pausa fixas.
    static func roteiro(_ textos: [String], inicio: Double = 0.5, duracao: Double = 0.35, pausa: Double = 0.4) -> [PalavraReconhecida] {
        var t = inicio
        return textos.map { texto in
            defer { t += duracao + pausa }
            return PalavraReconhecida(texto: texto, inicio: t, fim: t + duracao)
        }
    }

    /// PCM com tom de 0,1 onde há palavra e zero no resto.
    static func pcm(_ roteiro: [PalavraReconhecida], segundos: Double, amplitude: Float = 0.1) -> [Float] {
        var saida = [Float](repeating: 0, count: Int(segundos * Double(taxaLancamento)))
        for p in roteiro {
            let a = Int(p.inicio * Double(taxaLancamento)), z = min(saida.count, Int(p.fim * Double(taxaLancamento)))
            for i in a..<max(a, z) { saida[i] = amplitude * Float(sin(Double(i) * 0.3)) }
        }
        return saida
    }
}

/// Ambiente de um teste: banco, pasta de áudio e diários antigos numa pasta temporária.
final class Ambiente {
    let raiz: URL
    let armazem: ArmazemLancamento
    let disco: DiscoDeBlocos
    var diarios: URL { raiz.appendingPathComponent("ReunioesPendentes", isDirectory: true) }
    var banco: String { raiz.appendingPathComponent("teste.sqlite").path }

    init(escrita: DiscoDeBlocos.Escrita? = nil, legado: ((String) throws -> Void)? = nil) throws {
        let pasta = FileManager.default.temporaryDirectory.appendingPathComponent("persistencia-" + UUID().uuidString, isDirectory: true)
        raiz = pasta
        try FileManager.default.createDirectory(at: pasta, withIntermediateDirectories: true)
        try legado?(pasta.appendingPathComponent("teste.sqlite").path)
        armazem = try ArmazemLancamento(caminho: pasta.appendingPathComponent("teste.sqlite").path)
        let audio = pasta.appendingPathComponent("Audio")
        disco = escrita.map { DiscoDeBlocos(raiz: audio, escrita: $0) } ?? DiscoDeBlocos(raiz: audio)
    }

    deinit { try? FileManager.default.removeItem(at: raiz) }

    /// Nova abertura do app: outra conexão sobre o mesmo banco.
    func reabrir() throws -> ArmazemLancamento { try ArmazemLancamento(caminho: banco) }

    func novaReuniao(fonte: String = "mac") throws -> String {
        let id = UUID().uuidString
        try armazem.criarDocumento(id: id, tipo: "reuniao", nome: "Reunião sintética", fonte: fonte, estado: "gravando")
        return id
    }

    /// Captura contínua: buffers de 100 ms carimbados a partir de t=0.
    func capturar(_ coletor: ColetorDeReuniao, _ fonte: FonteDeAudio, _ pcm: [Float], desde: Double = 0,
                  aoSelar processador: ProcessadorDeReuniao? = nil) async throws {
        let passo = taxaLancamento / 10
        for i in stride(from: 0, to: pcm.count, by: passo) {
            try await coletor.receber(fonte, tempo: desde + Double(i) / Double(taxaLancamento),
                                      amostras: Array(pcm[i..<min(pcm.count, i + passo)]))
            if let processador, (i / passo) % 10 == 9 {
                let g = await coletor.geracao(fonte) ?? 0
                await processador.blocoSelado(fonte: fonte, geracao: g, fim: 0)
            }
        }
    }

    func textos(_ doc: String) throws -> [String] {
        try armazem.documento(doc)?.trechos.map(\.texto) ?? []
    }

    func palavras(_ doc: String) throws -> [String] {
        try textos(doc).flatMap { $0.split(separator: " ").map(String.init) }
    }
}
