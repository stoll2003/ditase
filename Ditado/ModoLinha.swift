import AVFoundation
import Foundation
import Nucleo

/// Modos de linha de comando, para conferir o motor sem microfone:
///
///     Ditado --transcrever audio.wav [--sem-ia] [--tempo-real] [--modelo arquivo.gguf]
///
/// Passa o arquivo pela mesma sessão usada no ditado real (segmentação, reconhecedor,
/// regras, dicionário do usuário e modelo de limpeza) e imprime o texto e os tempos.
enum ModoLinha {
    static func argumento(_ nome: String) -> String? {
        let args = CommandLine.arguments
        guard let i = args.firstIndex(of: nome), i + 1 < args.count else { return nil }
        return args[i + 1]
    }

    static func tem(_ nome: String) -> Bool { CommandLine.arguments.contains(nome) }

    static func lerAudio(_ caminho: String) throws -> [Float] {
        let arquivo = try AVAudioFile(forReading: URL(fileURLWithPath: caminho))
        let origem = arquivo.processingFormat
        guard let entrada = AVAudioPCMBuffer(pcmFormat: origem, frameCapacity: AVAudioFrameCount(arquivo.length)) else { throw Gravador.Erro.formato }
        try arquivo.read(into: entrada)
        let alvo = Gravador.formatoAlvo
        if origem.sampleRate == alvo.sampleRate, origem.channelCount == 1, let canal = entrada.floatChannelData?[0] {
            return Array(UnsafeBufferPointer(start: canal, count: Int(entrada.frameLength)))
        }
        guard let conversor = AVAudioConverter(from: origem, to: alvo),
              let saida = AVAudioPCMBuffer(pcmFormat: alvo, frameCapacity: AVAudioFrameCount(Double(entrada.frameLength) * alvo.sampleRate / origem.sampleRate) + 1024)
        else { throw Gravador.Erro.formato }
        var entregue = false
        var erro: NSError?
        conversor.convert(to: saida, error: &erro) { _, estado in
            if entregue {
                estado.pointee = .endOfStream
                return nil
            }
            entregue = true
            estado.pointee = .haveData
            return entrada
        }
        if let erro { throw erro }
        guard let canal = saida.floatChannelData?[0] else { throw Gravador.Erro.formato }
        return Array(UnsafeBufferPointer(start: canal, count: Int(saida.frameLength)))
    }

    static func transcrever(_ caminho: String) async -> Int32 {
        func erro(_ m: String) -> Int32 {
            FileHandle.standardError.write(Data("erro: \(m)\n".utf8))
            return 1
        }
        let amostras: [Float]
        do { amostras = try lerAudio(caminho) } catch { return erro("não li o áudio: \(error)") }
        let duracao = Double(amostras.count) / 16_000

        let termos = (try? Banco(caminho: Caminhos.banco.path).termos()) ?? []
        let dicionario = AplicadorDicionario(termos: termos, ehPalavraComum: Ortografia().verificador)
        let transcritor = TranscritorParakeet()
        let t0 = Date()
        do { try await transcritor.carregar() } catch { return erro("reconhecedor: \(error)") }
        let cargaDoReconhecedor = Date().timeIntervalSince(t0)

        var limpador: Limpador?
        var servidor: ServidorLLM?
        var cargaDoModelo = 0.0
        if !tem("--sem-ia") {
            let modelo = argumento("--modelo") ?? Preferencias.padroes["modelo"]!
            let instrucoes = Bundle.main.url(forResource: "prompt-limpeza", withExtension: "md", subdirectory: "Recursos").flatMap { try? String(contentsOf: $0, encoding: .utf8) }
                ?? Bundle.main.url(forResource: "prompt-limpeza", withExtension: "md").flatMap { try? String(contentsOf: $0, encoding: .utf8) } ?? ""
            let s = ServidorLLM(instrucoes: instrucoes)
            let t1 = Date()
            guard await s.garantir(modelo: modelo) else { return erro("modelo de limpeza indisponível: \(s.estado)") }
            cargaDoModelo = Date().timeIntervalSince(t1)
            servidor = s
            limpador = s.limpador(modelo: modelo)
        }
        defer { servidor?.parar() }

        let sessao = SessaoDitado(transcritor: transcritor, limpador: limpador, dicionario: dicionario)
        let tempoReal = tem("--tempo-real")
        let bloco = 1600 // 100 ms
        let inicio = Date()
        var i = 0
        while i < amostras.count {
            let fim = min(i + bloco, amostras.count)
            await sessao.alimentar(Array(amostras[i..<fim]))
            i = fim
            if tempoReal {
                let alvo = inicio.addingTimeInterval(Double(i) / 16_000)
                let falta = alvo.timeIntervalSinceNow
                if falta > 0 { try? await Task.sleep(nanoseconds: UInt64(falta * 1_000_000_000)) }
            }
        }
        let soltou = Date()
        let r = await sessao.finalizar()
        let espera = Date().timeIntervalSince(soltou)

        print(r.texto)
        let resumo: [String: Any] = [
            "audio_s": (duracao * 10).rounded() / 10, "fala_s": (r.duracaoFala * 10).rounded() / 10, "palavras": r.palavras,
            "espera_ao_soltar_s": (espera * 100).rounded() / 100, "total_s": (Date().timeIntervalSince(inicio) * 100).rounded() / 100,
            "limpador": r.limpador, "recusas": r.recusas, "falhas": r.falhas, "trocas_dicionario": r.trocas.count,
            "carga_reconhecedor_s": (cargaDoReconhecedor * 100).rounded() / 100, "carga_modelo_s": (cargaDoModelo * 100).rounded() / 100,
            "tempo_real": tempoReal,
        ]
        if let dados = try? JSONSerialization.data(withJSONObject: resumo, options: [.sortedKeys]) {
            FileHandle.standardError.write(dados + Data("\n".utf8))
        }
        if let bruto = argumento("--bruto") { try? r.textoBruto.write(toFile: bruto, atomically: true, encoding: .utf8) }
        return 0
    }
}
