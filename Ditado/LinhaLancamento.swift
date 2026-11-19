import Foundation
import Nucleo

@MainActor
enum LinhaLancamento {
    static var solicitado: Bool {
        ["--transcrever-arquivo", "--reuniao-simulada", "--medir-falantes"].contains(where: ModoLinha.tem)
    }

    static func executar() async -> Int32 {
        let args = CommandLine.arguments
        let transcritor = TranscritorParakeet()
        let dicionario = AplicadorDicionario(termos: [], ehPalavraComum: { _ in false })
        let instrucoes = Bundle.main.url(forResource: "prompt-limpeza", withExtension: "md", subdirectory: "Recursos")
            .flatMap { try? String(contentsOf: $0, encoding: .utf8) } ?? ""
        let servidor = ServidorLLM(instrucoes: instrucoes)
        defer { servidor.parar() }
        let modelo = ModoLinha.argumento("--modelo") ?? Preferencias.padroes["modelo"]!
        let cache = ModoLinha.argumento("--cache-falantes").map { URL(fileURLWithPath: $0, isDirectory: true) }
        let limpador = ModoLinha.tem("--transcrever-arquivo") && !ModoLinha.tem("--sem-ia") && !instrucoes.isEmpty
            ? servidor.limpador(modelo: modelo) : nil
        let motor = MotorLancamento(transcritor: transcritor, cache: cache)
        let preparar = MotorLancamento.preparador(dicionario: dicionario, limpador: limpador)
        let inicio = Date()
        do {
            if let caminho = ModoLinha.argumento("--transcrever-arquivo") {
                let banco = try abrirBanco()
                let url = URL(fileURLWithPath: caminho)
                let doc = TranscricaoLonga(nome: url.lastPathComponent, fase: "naFila")
                let resultado = try await motor.arquivo(url, documento: doc, armazem: banco,
                                                        falantes: ModoLinha.tem("--falantes"),
                                                        preparar: preparar, inicio: inicio) { _, _ in }
                try imprimir(resultado)
            } else if let caminho = ModoLinha.argumento("--reuniao-simulada"),
                      let i = args.firstIndex(of: "--reuniao-simulada") {
                let mic = args.indices.contains(i + 2) && !args[i + 2].hasPrefix("--") ? args[i + 2] : nil
                try await simular(motor: motor, preparar: preparar, sistema: caminho, mic: mic, inicio: inicio,
                                  resumidor: ModoLinha.tem("--sem-resumo") ? nil : ResumidorReuniao(servidor: servidor, modelo: modelo))
            } else if let caminho = ModoLinha.argumento("--medir-falantes") {
                try await medir(motor: motor, caminho: caminho)
            } else { throw ErroLancamento.parametro("arquivo de entrada") }
            return 0
        } catch {
            FileHandle.standardError.write(Data(("erro: " + error.localizedDescription + "\n").utf8))
            return 1
        }
    }

    private static func abrirBanco() throws -> ArmazemLancamento {
        let caminho = ModoLinha.argumento("--banco")
        let banco = try Caminhos.bancoDoUtilitario(caminho == ":memory:" ? nil : caminho)
        return try ArmazemLancamento(caminho: banco)
    }

    nonisolated private static func simular(motor: MotorLancamento, preparar: @escaping PreparadorDeTexto,
                                           sistema: String, mic: String?, inicio: Date,
                                           resumidor: ResumidorReuniao?) async throws {
        let banco = try await abrirBanco()
        let disco = await DiscoDeBlocos(raiz: Caminhos.audioDeReuniao)
        let reserva = try ReservaDeAudio()
        defer { reserva.remover() }
        let sistemaPCM = reserva.arquivo("origem-mac")
        let micPCM = mic.map { _ in reserva.arquivo("origem-mic") }
        try await AudioLancamento.decodificar(URL(fileURLWithPath: sistema), para: sistemaPCM) { _ in }
        if let mic, let micPCM {
            try await AudioLancamento.decodificar(URL(fileURLWithPath: mic), para: micPCM) { _ in }
        }
        let doc = TranscricaoLonga(nome: "Reunião simulada", fase: "gravando", fonte: mic == nil ? "mac" : "macEMicrofone")
        try banco.criarDocumento(id: doc.id, tipo: "reuniao", nome: doc.nome, fonte: doc.fonte, estado: "gravando", criadoEm: doc.criadoEm)
        let coletor = ColetorDeReuniao(documento: doc.id, armazem: banco, disco: disco, inicio: 0)
        let processador = ProcessadorDeReuniao(documento: doc.id, armazem: banco, disco: disco, reconhecedor: motor, preparar: preparar)
        let tempos = TemposSimulacao()
        await processador.definirAoConfirmar { _, atraso in await tempos.bloco(atraso) }
        await coletor.definirAoSelar { fonte, geracao, fim in
            await processador.blocoSelado(fonte: fonte, geracao: geracao, fim: fim)
        }
        try await coletor.abrir(.mac, em: 0)
        if micPCM != nil { try await coletor.abrir(.mic, em: 0) }
        let leitor = try LeitorLancamento(sistemaPCM)
        let microfone = try micPCM.map(LeitorLancamento.init)
        let total = max(leitor.quantidade, microfone?.quantidade ?? 0)
        let relogio = Date()
        let tempoReal = ModoLinha.tem("--tempo-real")
        for posicao in stride(from: 0, to: total, by: 1_600) {
            let quantidade = min(1_600, total - posicao)
            let segundo = Double(posicao) / 16_000
            try await coletor.receber(.mac, tempo: segundo, amostras: leitor.ler(quantidade: quantidade))
            if let microfone {
                try await coletor.receber(.mic, tempo: segundo, amostras: microfone.ler(quantidade: quantidade))
            }
            if tempoReal {
                let restante = Double(posicao + quantidade) / 16_000 - Date().timeIntervalSince(relogio)
                if restante > 0 { try await Task.sleep(for: .seconds(restante)) }
            }
        }
        try await coletor.encerrar(em: Double(total) / 16_000)
        try await processador.fechar()
        try await processador.encerrarCosturas()
        let iniciouFinal = Date()
        let identificados = try await processador.passadaFinal(motor)
        let duracaoFinal = Date().timeIntervalSince(iniciouFinal)
        let atraso = await tempos.pico
        try banco.definir(documento: doc.id, .atraso(atraso))
        if let resumidor, let atual = try banco.documento(doc.id), !atual.trechos.isEmpty {
            let iniciouResumo = Date()
            do {
                let resumo = try await resumidor.resumir(atual)
                guard !resumo.isEmpty else { throw ErroDeLimpeza("O Qwen não devolveu tópicos.") }
                try banco.definir(documento: doc.id, .resumo(resumo))
            } catch is CancellationError { try banco.definir(documento: doc.id, .resumoCancelado) }
            catch { try banco.definir(documento: doc.id, .erroResumo(error.localizedDescription)) }
            FileHandle.standardError.write(Data("resumo_s=\(Date().timeIntervalSince(iniciouResumo))\n".utf8))
        }
        guard var pronto = try banco.documento(doc.id) else { throw ErroLancamento.parametro("id") }
        FileHandle.standardError.write(Data("passada_final_s=\(duracaoFinal)\n".utf8))
        pronto.processamento = Date().timeIntervalSince(inicio)
        try await imprimir(pronto)
        if !identificados { throw ErroLancamento.captura("Não foi possível identificar os falantes.") }
        if pronto.pendentes?.isEmpty == false { throw ErroLancamento.captura("Há fala pendente. O áudio continua guardado.") }
    }

    private static func medir(motor: MotorLancamento, caminho: String) async throws {
        let reserva = try ReservaDeAudio()
        defer { reserva.remover() }
        let wav = reserva.arquivo("teste")
        try await AudioLancamento.decodificar(URL(fileURLWithPath: caminho), para: wav) { _ in }
        for nome in ModoLinha.argumento("--motor-medicao").map({ [$0] }) ?? ["offline", "offline-sortformer", "sortformer", "lseend"] {
            do {
                let tipo = DiarizadorEmFluxo(rawValue: nome)
                if let tipo { try await motor.prepararFluxo(tipo) }
                else if nome == "offline-sortformer" { try await motor.aquecerOfflineSortformer() }
                else { try await motor.aquecerOffline() }
                let inicio = Date()
                let faixas: [FaixaDeVoz]
                if let tipo {
                    let leitor = try LeitorLancamento(wav)
                    for _ in stride(from: 0, to: leitor.quantidade, by: 48_000) {
                        let dados = try leitor.ler(quantidade: 48_000)
                        _ = try await motor.faixaEmFluxo(dados, tipo: tipo)
                    }
                    _ = try await motor.faixaEmFluxo([], tipo: tipo, final: true)
                    faixas = await motor.faixasEstaveis(tipo)
                } else if nome == "offline-sortformer" { faixas = try await motor.diarizarOfflineSortformer(wav) }
                else { faixas = try await motor.diarizarPorAgrupamento(wav) { _ in } }
                try imprimir(Medicao(motor: nome, segundos: Date().timeIntervalSince(inicio), faixas: faixas))
            } catch {
                let aviso = nome + ": " + error.localizedDescription + "\n"
                FileHandle.standardError.write(Data(aviso.utf8))
            }
        }
    }

    private struct Medicao: Encodable { let motor: String; let segundos: Double; let faixas: [FaixaDeVoz] }

    private static func imprimir<T: Encodable>(_ valor: T) throws {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        FileHandle.standardOutput.write(try encoder.encode(valor) + Data("\n".utf8))
    }
}

private actor TemposSimulacao {
    private(set) var pico = 0.0
    func bloco(_ atraso: Double) { pico = max(pico, atraso) }
}
