import FluidAudio
import Foundation
import Nucleo

private struct PalavraComTempo: Sendable {
    let texto: String
    let inicio: Double
    let fim: Double
}

enum DiarizadorEmFluxo: String, CaseIterable, Sendable {
    case sortformer, lseend
}

/// Adaptador do Parakeet e dos diarizadores do FluidAudio para o núcleo.
actor MotorLancamento: ReconhecedorDeFala, IdentificadorDeFalantes {
    typealias Progresso = @Sendable (_ etapa: String, _ fracao: Double?) async -> Void

    private let transcritor: TranscritorParakeet
    private let cache: URL?
    private var offline: OfflineDiarizerManager?
    private var offlineSortformer: OfflineSortformerDiarizer?
    private var sortformer: SortformerDiarizer?
    private var lseend: LSEENDDiarizer?

    init(transcritor: TranscritorParakeet, cache: URL? = nil) {
        self.transcritor = transcritor
        self.cache = cache
    }

    nonisolated func arquivo(_ origem: URL, documento: TranscricaoLonga, armazem: ArmazemLancamento,
                             falantes: Bool, preparar: @escaping PreparadorDeTexto,
                             inicio: Date = Date(), progresso: @escaping Progresso) async throws -> TranscricaoLonga {
        try Task.checkCancellation()
        let versao = try armazem.criarVersao(documento: documento.id, nome: documento.nome, origem: origem.lastPathComponent)
        do {
            let reserva = try ReservaDeAudio()
            defer { reserva.remover() }
            let acesso = origem.startAccessingSecurityScopedResource()
            defer { if acesso { origem.stopAccessingSecurityScopedResource() } }
            // Copy before decoding. Keep the original extension for AVFoundation's demuxer.
            let snapshot = reserva.pasta.appendingPathComponent("origem").appendingPathExtension(origem.pathExtension)
            let antes = try identidade(origem)
            try FileManager.default.copyItem(at: origem, to: snapshot)
            guard antes == (try identidade(origem)) else {
                throw ErroLancamento.captura("O arquivo de origem mudou durante a leitura. Selecione-o novamente.")
            }
            let wav = reserva.arquivo("decodificado")
            try await AudioLancamento.decodificar(snapshot, para: wav) { await progresso("decodificando", $0) }
            let processador = ProcessadorDeArquivo(armazem: armazem, reconhecedor: self, identificador: falantes ? self : nil, preparar: preparar)
            let desfecho = try await processador.transcrever(versao: versao, pcm: FonteDeWAVLancamento(wav),
                                                             wav: wav, inicio: inicio, progresso: progresso)
            switch desfecho {
            case .publicada:
                guard let pronto = try armazem.documento(documento.id) else { throw ErroLancamento.parametro("id") }
                return pronto
            case .semFala: throw ErroLancamento.semFala
            case .superada: throw CancellationError()
            }
        } catch {
            let cancelado = error is CancellationError || Task.isCancelled
            let codigo = (error as? ErroLancamento).map { erro in
                if case .formato = erro { return ErroDeArquivo.formato }
                return erro.localizedDescription
            } ?? error.localizedDescription
            try armazem.encerrarVersao(versao.id, estado: cancelado ? "cancelada" : "falhou", erro: cancelado ? nil : codigo)
            if cancelado { throw CancellationError() }
            throw error
        }
    }

    private nonisolated func identidade(_ url: URL) throws -> [FileAttributeKey: AnyHashable] {
        let atributos = try FileManager.default.attributesOfItem(atPath: url.path)
        return [.systemFileNumber, .systemNumber, .size, .modificationDate].reduce(into: [:]) {
            $0[$1] = atributos[$1] as? AnyHashable
        }
    }

    /// Regras de fala, dicionário e, se houver, a limpeza local com Guarda e Fidelidade.
    static func preparador(dicionario: AplicadorDicionario, limpador: Limpador?) -> PreparadorDeTexto {
        { texto in
            let base = RegrasFala.arrumarEspacos(dicionario.aplicar(RegrasFala.filtrar(texto)).0)
            guard let limpador, Palavras.contar(base) >= 6 else { return base }
            let pedido = PedidoLimpeza(transcricao: base, vocabulario: dicionario.vocabulario)
            if let saida = try? await limpador.limpar(pedido), Guarda.avaliar(original: base, limpo: saida) == nil {
                return RegrasFala.arrumarEspacos(dicionario.aplicar(Fidelidade.fundir(original: base, modelo: saida)).0)
            }
            return base
        }
    }

    /// Palavras com tempos absolutos. `fimDaFala` fecha a cauda: silêncio depois, sem duração inventada.
    func palavras(_ amostras: [Float], inicio: Double, fimDaFala: Double?) async throws -> [PalavraReconhecida] {
        try await reconhecer(amostras, inicio: inicio, fimDaFala: fimDaFala)
            .map { PalavraReconhecida(texto: $0.texto, inicio: $0.inicio, fim: $0.fim) }
    }

    func faixas(wav: URL) async throws -> [FaixaDeVoz] {
        try await diarizar(wav) { _ in }
    }

    private func reconhecer(_ dados: [Float], inicio: Double, fimDaFala: Double? = nil) async throws -> [PalavraComTempo] {
        guard !dados.isEmpty, dados.contains(where: { abs($0) > 0.002 }) else { return [] }
        try Task.checkCancellation()
        let gerenciador = try await transcritor.carregar()
        var entrada = dados
        let amostrasDeSilencio = fimDaFala == nil ? 0 : 4_800
        let completar = max(amostrasDeSilencio, 17_600 - entrada.count)
        if completar > 0 { entrada += [Float](repeating: 0, count: completar) }
        var estado = TdtDecoderState.make(decoderLayers: await gerenciador.decoderLayerCount)
        let r = try await gerenciador.transcribe(entrada, decoderState: &estado, language: .portuguese)
        let duracao = min(Double(dados.count) / 16_000, fimDaFala ?? .infinity)
        let palavras = buildWordTimings(from: r.tokenTimings ?? [])
        if palavras.isEmpty, !r.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return [PalavraComTempo(texto: r.text, inicio: inicio, fim: inicio + duracao)]
        }
        return palavras.compactMap {
            let de = max(0, $0.startTime)
            let ate = min(duracao, max(de + 0.02, $0.endTime))
            guard ate > de else { return nil }
            return PalavraComTempo(texto: $0.word, inicio: inicio + de, fim: inicio + ate)
        }
    }

    private func prepararOffline() async throws -> OfflineDiarizerManager {
        if let offline { return offline }
        let gerenciador = OfflineDiarizerManager()
        try await gerenciador.prepareModels(directory: cache?.appendingPathComponent("offline"))
        offline = gerenciador
        return gerenciador
    }

    func aquecerOffline() async throws { _ = try await prepararOffline() }

    func aquecerOfflineSortformer() async throws {
        if offlineSortformer != nil { return }
        let diarizador = OfflineSortformerDiarizer()
        let modelos = try await OfflineSortformerModels.loadFromHuggingFace(cacheDirectory: cache, computeUnits: .cpuOnly)
        diarizador.initialize(models: modelos)
        offlineSortformer = diarizador
    }

    func diarizarOfflineSortformer(_ url: URL) async throws -> [FaixaDeVoz] {
        try await aquecerOfflineSortformer()
        guard let offlineSortformer else { throw ErroLancamento.parametro("Sortformer offline") }
        let leitor = try LeitorLancamento(url)
        guard leitor.quantidade <= 16_000 * 120 else { throw ErroLancamento.parametro("áudio acima de 120 segundos") }
        let dados = try leitor.ler(quantidade: leitor.quantidade)
        let timeline = try offlineSortformer.processComplete(dados)
        return faixas(timeline.speakers.values.flatMap(\.finalizedSegments), prefixo: "offline-sortformer")
    }

    func diarizar(_ url: URL, progresso: @escaping @Sendable (Double) async -> Void) async throws -> [FaixaDeVoz] {
        let duracao = try LeitorLancamento(url).quantidade
        if duracao > 0, duracao <= 16_000 * 120 {
            do {
                let curtas = try await diarizarOfflineSortformer(url)
                try Task.checkCancellation()
                let duracoes = Dictionary(grouping: curtas, by: \.voz)
                    .mapValues { $0.reduce(0) { $0 + ($1.fim - $1.inicio) } }
                let ativas = duracoes.filter { $0.value >= 0.4 }
                if !ativas.isEmpty, ativas.count < 4 {
                    await progresso(1)
                    return curtas.filter { ativas[$0.voz] != nil }
                }
            } catch is CancellationError { throw CancellationError() }
            catch { /* O diarizador offline de agrupamento continua disponível como segunda via. */ }
        }
        return try await diarizarPorAgrupamento(url, progresso: progresso)
    }

    func diarizarPorAgrupamento(_ url: URL, progresso: @escaping @Sendable (Double) async -> Void) async throws -> [FaixaDeVoz] {
        let gerenciador = try await prepararOffline()
        let (eventos, emissor) = AsyncStream<Double>.makeStream(bufferingPolicy: .bufferingNewest(1))
        let encaminhar = Task { for await evento in eventos { await progresso(evento) } }
        defer { emissor.finish() }
        let carregado = try AudioSourceFactory().makeDiskBackedSource(from: url, targetSampleRate: 16_000)
        defer { carregado.source.cleanup() }
        let r = try await gerenciador.process(audioSource: FonteCancelavel(base: carregado.source),
                                               audioLoadingSeconds: carregado.loadDuration) { feitos, total in
            if total > 0 { emissor.yield(min(0.99, Double(feitos) / Double(total))) }
        }
        emissor.finish()
        await encaminhar.value
        try Task.checkCancellation()
        await progresso(1)
        return r.segments.map { FaixaDeVoz(voz: "offline-" + $0.speakerId,
                                           inicio: Double($0.startTimeSeconds), fim: Double($0.endTimeSeconds)) }
    }

    func prepararFluxo(_ tipo: DiarizadorEmFluxo, novo: Bool = false) async throws {
        if !novo && ((tipo == .sortformer && sortformer != nil) || (tipo == .lseend && lseend != nil)) { return }
        switch tipo {
        case .sortformer:
            let config = SortformerConfig.fastV2_1
            let modelo = try await SortformerModels.loadFromHuggingFace(config: config, cacheDirectory: cache, computeUnits: .cpuOnly)
            let diarizador = SortformerDiarizer(config: config)
            diarizador.initialize(models: modelo)
            sortformer = diarizador
        case .lseend:
            let diarizador = LSEENDDiarizer()
            try await diarizador.initialize(variant: .dihard3, cacheDirectory: cache)
            lseend = diarizador
        }
    }

    private func faixas(_ segmentos: [DiarizerSegment], prefixo: String = "fluxo") -> [FaixaDeVoz] {
        segmentos.map {
            FaixaDeVoz(voz: "\(prefixo)-\($0.speakerIndex)", inicio: Double($0.startTime), fim: Double($0.endTime))
        }
    }

    func faixaEmFluxo(_ dados: [Float], tipo: DiarizadorEmFluxo, final: Bool = false) throws -> [FaixaDeVoz] {
        switch tipo {
        case .sortformer:
            guard let sortformer else { throw ErroLancamento.parametro("Sortformer") }
            if final {
                let atualizacao = try sortformer.finalizeSession()
                return faixas(atualizacao?.finalizedSegments ?? [])
            }
            // Cada predição do Fast v2.1 finaliza só 480 ms. Drene as predições
            // disponíveis de todo o bloco para o diarizador não ficar para trás.
            try sortformer.addAudio(dados, sourceSampleRate: 16_000)
            var definitivos: [DiarizerSegment] = []
            var ultimosTentativos: [DiarizerSegment] = []
            while let atualizacao = try sortformer.process() {
                definitivos += atualizacao.finalizedSegments
                ultimosTentativos = atualizacao.tentativeSegments
            }
            return faixas(definitivos + ultimosTentativos)
        case .lseend:
            guard let lseend else { throw ErroLancamento.parametro("LS-EEND") }
            let atualizacao = final ? try lseend.finalizeSession() : try lseend.process(samples: dados, sourceSampleRate: 16_000)
            return faixas((atualizacao?.finalizedSegments ?? []) + (atualizacao?.tentativeSegments ?? []))
        }
    }

    func faixasEstaveis(_ tipo: DiarizadorEmFluxo) -> [FaixaDeVoz] {
        let timeline = tipo == .sortformer ? sortformer?.timeline : lseend?.timeline
        return faixas(timeline?.speakers.values.flatMap(\.finalizedSegments) ?? [])
    }

}


private struct FonteCancelavel: AudioSampleSource {
    let base: DiskBackedAudioSampleSource
    var sampleCount: Int { base.sampleCount }

    func copySamples(into destino: UnsafeMutablePointer<Float>, offset: Int, count: Int) throws {
        try Task.checkCancellation()
        try base.copySamples(into: destino, offset: offset, count: count)
    }
}
