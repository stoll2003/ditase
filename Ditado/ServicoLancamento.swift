import AppKit
import AVFoundation
import Combine
import CoreGraphics
import Foundation
import Nucleo

@MainActor
final class ServicoLancamento {
    private unowned let controlador: Controlador
    let processamento = PublicadorProcessamento()
    let segmentos = PassthroughSubject<EventoTrechos, Never>()
    struct EventoTrechos: Codable {
        let id: String
        let trechos: [TrechoLongo]
        let vozes: [VozLonga]
        let duracao: Double
        let atraso: Double
    }
    let intervalos = PassthroughSubject<EventoIntervalos, Never>()
    struct EventoIntervalos: Codable {
        let id: String
        let lacunas: [PeriodoDeAudio]
        let pendentes: [PeriodoDeAudio]
    }
    var aoMudar: (() -> Void)?

    private var arquivo: TranscricaoLonga?
    private var idTrabalhoArquivo: String?
    private var fila: [(url: URL, documento: TranscricaoLonga)] = []
    private var origensDaSessao: [String: URL] = [:]
    private var falhos: [String: (url: URL, documento: TranscricaoLonga)] = [:]
    private var trabalho: Task<Void, Never>?
    private var reuniao: TranscricaoLonga?
    private var coletor: ColetorDeReuniao?
    private var processador: ProcessadorDeReuniao?
    private var captura: CapturaReuniao?
    private var capturaIniciada = false
    private var fimSolicitado: Double?
    private var sinalDeFim: AsyncStream<Void>.Continuation?
    private var alimentacaoSistema: Task<Void, Never>?
    private var paradaCaptura: Task<Void, Never>?
    private var pausaPendente: Task<Void, Never>?
    private var pulso: Task<Void, Never>?
    private var mic: Task<Void, Never>?
    private var gravadorMic: Gravador?
    private var distribuidor: DistribuidorDoMicrofone?
    private var audioSistema = CGPreflightScreenCaptureAccess() ? "concedida" : "pendente"
    private var inicioDeCaptura = 0.0
    private var pausaDesde: Double?
    private var tempoPausado = 0.0
    private var nivelPorFonte: [FonteDeAudio: (instante: Double, nivel: Float)] = [:]
    private var ultimoSom = 0.0
    private var ultimaConsultaDeAudio = 0.0
    private var consultandoAudio = false
    private var ultimaMudancaArquivo = Date.distantPast
    private var saindo = false
    private var armazem: ArmazemLancamento?
    private var abertura: Task<(ArmazemLancamento, ArmazemLancamento.RelatorioMigracao), Error>?
    private var avisoMigracao: String?

    var aguardaPersistencia: Bool { reuniao != nil }

    init(controlador: Controlador) { self.controlador = controlador }

    func preparar() async throws -> ArmazemLancamento {
        if let armazem { return armazem }
        if abertura == nil {
            let caminho = controlador.banco.caminho
            let audio = Caminhos.audioDeReuniao
            let diarios = Caminhos.reunioesPendentes
            abertura = Task.detached {
                let banco = try ArmazemLancamento(caminho: caminho)
                return (banco, banco.prepararAbertura(diarios: diarios, disco: DiscoDeBlocos(raiz: audio)))
            }
        }
        guard let abertura else { throw ErroLancamento.ocupado }
        do {
            let (banco, relatorio) = try await abertura.value
            let avisos = relatorio.falhas + (relatorio.invalidos.isEmpty ? [] : ["Há blocos de áudio inválidos preservados para inspeção."])
            avisoMigracao = avisos.isEmpty ? nil : avisos.joined(separator: " ")
            armazem = banco
            return banco
        } catch {
            self.abertura = nil
            throw error
        }
    }

    func estado() throws -> EstadoLancamento {
        var arquivos = try armazem?.lista(tipo: "arquivo").filter { $0.oculto != true && $0.fase != "cancelado" } ?? []
        arquivos += falhos.values.map(\.documento).filter { falha in !arquivos.contains { $0.id == falha.id && $0.fase == "pronto" } }
        if let arquivo { arquivos.append(arquivo) }
        arquivos += fila.map(\.documento)
        var vistos = Set<String>()
        arquivos = arquivos.reversed().filter { vistos.insert($0.id).inserted }
        if audioDoMacDisponivel { audioSistema = "concedida" }
        let recuperaveis = try armazem?.recuperaveis(excluindo: reuniao?.id).filter { $0.oculto != true } ?? []
        let anteriores = try armazem?.lista(tipo: "reuniao") ?? []
        var ativa = reuniao
        ativa?.etapa = processamento.atual?.etapa.id
        return EstadoLancamento(arquivos: arquivos.sorted { $0.criadoEm > $1.criadoEm }, reuniao: ativa,
            anteriores: anteriores.filter { $0.fase == "pronta" && $0.oculto != true && $0.id != reuniao?.id },
            recuperaveis: recuperaveis,
            permissoes: ["audioSistema": audioSistema, "microfone": Gravador.permissao], disponivel: true,
            avisoMigracao: avisoMigracao)
    }

    struct EstadoLancamento: Codable {
        let arquivos: [TranscricaoLonga]
        let reuniao: TranscricaoLonga?
        let anteriores: [TranscricaoLonga]
        let recuperaveis: [TranscricaoLonga]
        let permissoes: [String: String]
        let disponivel: Bool
        let avisoMigracao: String?
    }

    private func motor(limpezaIA: Bool = true) -> (MotorLancamento, PreparadorDeTexto) {
        let termos = (try? controlador.banco.termos()) ?? []
        let dicionario = AplicadorDicionario(termos: termos, ehPalavraComum: Ortografia().verificador)
        let limpador = limpezaIA && controlador.prefs.limpezaIA ? controlador.servidor.limpador(modelo: controlador.modeloEmUso) : nil
        return (MotorLancamento(transcritor: controlador.transcritor), MotorLancamento.preparador(dicionario: dicionario, limpador: limpador))
    }

    func escolher(varios: Bool = true, concluir: @escaping ([URL]) -> Void) {
        let painel = NSOpenPanel()
        painel.allowsMultipleSelection = varios
        painel.canChooseDirectories = false
        painel.message = varios ? "Selecione os arquivos de áudio ou vídeo para transcrever."
                                : "Selecione o arquivo de áudio ou vídeo para transcrever de novo."
        painel.begin { resposta in concluir(resposta == .OK ? painel.urls : []) }
    }

    var audioDoMacDisponivel: Bool { CGPreflightScreenCaptureAccess() }

    func adicionar(_ urls: [URL]) -> String? {
        let itens = urls.filter(\.isFileURL).map {
            ($0, TranscricaoLonga(nome: $0.lastPathComponent, fase: "naFila"))
        }
        fila += itens
        for (url, documento) in itens { origensDaSessao[documento.id] = url }
        processarFila()
        aoMudar?()
        return itens.first?.1.id
    }

    private func processarFila() {
        guard !saindo, !descartando, trabalho == nil, reuniao == nil, !fila.isEmpty else { return }
        let (url, documento) = fila.removeFirst()
        arquivo = documento
        let id = documento.id
        let trabalhoID = UUID().uuidString
        idTrabalhoArquivo = trabalhoID
        let (motor, prepararTexto) = motor()
        let inicio = Date()
        faseArquivo(id: id, etapa: "decodificando", fracao: nil)
        iniciarPulso()
        trabalho = Task {
            defer {
                arquivo = nil
                idTrabalhoArquivo = nil
                trabalho = nil
                pulso?.cancel()
                aoMudar?()
                processarFila()
            }
            do {
                let banco = try await preparar()
                _ = try await motor.arquivo(url, documento: documento, armazem: banco, falantes: true,
                                            preparar: prepararTexto, inicio: inicio) { [weak self] etapa, fracao in
                    await self?.faseArquivo(id: id, etapa: etapa, fracao: fracao)
                }
                faseArquivo(id: id, etapa: "limpando", fracao: 1)
                processamento.finalizar(id: trabalhoID, .concluido)
            } catch is CancellationError {
                processamento.finalizar(id: trabalhoID, .cancelado)
            } catch {
                if Task.isCancelled {
                    processamento.finalizar(id: trabalhoID, .cancelado)
                    return
                }
                var falha = arquivo ?? documento
                switch error {
                case ErroLancamento.formato: falha.fase = "formatoNaoSuportado"
                case ErroLancamento.semFala: falha.fase = "semFala"
                default: falha.fase = "falhou"
                }
                falha.motivo = error.localizedDescription
                falhos[id] = (url, falha)
                processamento.finalizar(id: trabalhoID, .erro, motivo: falha.motivo)
            }
        }
    }

    private func faseArquivo(id: String, etapa: String, fracao: Double?) {
        guard arquivo?.id == id, let idTrabalhoArquivo else { return }
        let pesos: [String: (Double, Double, String)] = [
            "decodificando": (0, 0.08, "Decodificando"),
            "transcrevendo": (0.08, 0.62, "Transcrevendo"),
            "diarizando": (0.70, 0.27, "Identificando falantes"),
            "limpando": (0.97, 0.03, "Preparando transcrição")
        ]
        guard let (base, peso, rotulo) = pesos[etapa] else { return }
        processamento.emitir(id: idTrabalhoArquivo, etapa: etapa, rotulo: rotulo, progresso: fracao.map { base + peso * $0 })
        let fase = etapa == "limpando" ? "diarizando" : etapa
        let mudou = arquivo?.fase != fase
        arquivo?.fase = fase
        arquivo?.progresso = processamento.atual?.progresso
        if mudou || fracao == 1 || Date().timeIntervalSince(ultimaMudancaArquivo) >= 0.25 {
            ultimaMudancaArquivo = Date()
            aoMudar?()
        }
    }

    func cancelar(id: String) {
        if arquivo?.id == id { trabalho?.cancel() }
        fila.removeAll { $0.documento.id == id }
        falhos[id] = nil
        origensDaSessao[id] = nil
        aoMudar?()
    }

    func remover(id: String) async throws {
        guard reuniao?.id != id else { throw ErroLancamento.ocupado }
        cancelar(id: id)
        let tentativa = arquivo?.id == id ? trabalho : nil
        let banco = try await preparar()
        await tentativa?.value
        if try banco.existeDocumento(id) {
            try await Task.detached { try banco.definir(documento: id, .oculto(true)) }.value
        }
        aoMudar?()
    }

    func desfazer(id: String) async throws {
        try await alterar(id: id, campo: .oculto(false))
    }

    func cancelarFila() {
        for item in fila { origensDaSessao[item.documento.id] = nil }
        fila.removeAll()
        if let id = arquivo?.id { cancelar(id: id) }
        else { aoMudar?() }
    }

    func fonteParaRepetir(id: String) throws -> URL? {
        _ = try documentoParaRepetir(id: id)
        guard let origem = origensDaSessao[id], FileManager.default.isReadableFile(atPath: origem.path) else { return nil }
        return origem
    }

    private func documentoParaRepetir(id: String) throws -> TranscricaoLonga {
        guard arquivo?.id != id, !fila.contains(where: { $0.documento.id == id }) else { throw ErroLancamento.ocupado }
        guard let doc = try documento(id: id), doc.fonte == nil, doc.oculto != true else {
            throw ErroLancamento.parametro("id")
        }
        return doc
    }

    func repetir(id: String, url: URL) throws {
        let anterior = try documentoParaRepetir(id: id)
        guard url.isFileURL else { throw ErroLancamento.formato }
        var documento = TranscricaoLonga(nome: anterior.nome, fase: "naFila")
        documento.id = anterior.id
        documento.criadoEm = anterior.criadoEm
        falhos[id] = nil
        origensDaSessao[id] = url
        fila.append((url, documento))
        processarFila()
        aoMudar?()
    }

    func pedirPermissao(_ qual: String) throws {
        switch qual {
        case "microfone":
            Gravador.pedirPermissao { [weak self] _ in self?.aoMudar?() }
        case "audioSistema":
            audioSistema = CGRequestScreenCaptureAccess() ? "concedida" : "negada"
            aoMudar?()
        default: throw ErroLancamento.parametro("qual")
        }
    }

    func iniciarReuniao(fonte: String) throws {
        guard fonte == "mac" || fonte == "macEMicrofone" else { throw ErroLancamento.parametro("fonte") }
        guard !saindo, !descartando, trabalho == nil, reuniao == nil, controlador.fase == .parado else { throw ErroLancamento.ocupado }
        executarReuniao(TranscricaoLonga(nome: "Reunião", fase: "gravando", fonte: fonte), recuperar: false)
    }

    func recuperarReuniao(id: String) throws {
        guard !saindo, !descartando, trabalho == nil, reuniao == nil else { throw ErroLancamento.ocupado }
        guard var documento = try armazem?.documento(id),
              try armazem?.recuperaveis(excluindo: nil).contains(where: { $0.id == id }) == true
        else { throw ErroLancamento.parametro("id") }
        documento.fase = "finalizando"
        executarReuniao(documento, recuperar: true)
    }

    func descartarReuniao(id: String) async throws {
        guard trabalho == nil, reuniao == nil, !saindo else { throw ErroLancamento.ocupado }
        let banco = try await preparar()
        guard !descartando, trabalho == nil, reuniao == nil, !saindo,
              try banco.recuperaveis(excluindo: nil).contains(where: { $0.id == id }) else {
            throw ErroLancamento.parametro("id")
        }
        // Reserve the service while the terminal transition and cleanup run.
        let disco = DiscoDeBlocos(raiz: Caminhos.audioDeReuniao)
        let operacao = Task.detached {
            if try banco.descartar(documento: id) {
                for (bloco, arquivo) in try banco.arquivosParaLimpar() where bloco.documento == id {
                    if DiscoDeBlocos.arquivoValido(arquivo, documento: bloco.documento) { try disco.apagar(arquivo) }
                    try banco.esquecerArquivo(bloco)
                }
            }
        }
        descartando = true
        defer { descartando = false; aoMudar?(); processarFila() }
        try await operacao.value
    }

    private var descartando = false

    private func executarReuniao(_ documento: TranscricaoLonga, recuperar: Bool) {
        let id = documento.id
        let (motor, prepararTexto) = motor(limpezaIA: false)
        let disco = DiscoDeBlocos(raiz: Caminhos.audioDeReuniao)
        let (fim, sinal) = AsyncStream<Void>.makeStream()
        sinalDeFim = sinal
        fimSolicitado = nil
        capturaIniciada = false
        reuniao = documento
        inicioDeCaptura = Relogio.agora()
        pausaDesde = nil
        tempoPausado = 0
        ultimoSom = inicioDeCaptura
        nivelPorFonte.removeAll()
        if recuperar { faseFinal(id: id, etapa: "passadaFinal", fracao: nil) }
        else { publicarReuniao() }
        iniciarPulso()

        trabalho = Task {
            var bancoAberto: ArmazemLancamento?
            defer {
                reuniao = nil
                coletor = nil
                processador = nil
                sinalDeFim = nil
                resumoDispensado = nil
                trabalho = nil
                pausaPendente = nil
                pulso?.cancel()
                aoMudar?()
                processarFila()
            }
            do {
                let banco = try await preparar()
                bancoAberto = banco
                if !recuperar {
                    try await Task.detached {
                        try banco.criarDocumento(id: id, tipo: "reuniao", nome: documento.nome, fonte: documento.fonte, estado: "gravando",
                                                 criadoEm: documento.criadoEm)
                    }.value
                }
                guard !saindo else { return }
                let processador = ProcessadorDeReuniao(documento: id, armazem: banco, disco: disco,
                                                       reconhecedor: motor, preparar: prepararTexto)
                self.processador = processador
                await processador.definirAoConfirmar { [weak self] trechos, atraso in
                    await self?.receberTrechos(id: id, trechos: trechos, atraso: atraso)
                }

                if !recuperar {
                    let coletor = ColetorDeReuniao(documento: id, armazem: banco, disco: disco, inicio: inicioDeCaptura)
                    self.coletor = coletor
                    await coletor.definirAoSelar { fonte, geracao, fim in
                        await processador.blocoSelado(fonte: fonte, geracao: geracao, fim: fim)
                    }
                    if fimSolicitado == nil {
                        do { try await iniciarCaptura(usaMic: documento.fonte == "macEMicrofone", coletor: coletor) }
                        catch { registrarFalhaCaptura(error) }
                    }
                    capturaIniciada = true
                    if fimSolicitado != nil || saindo { await fecharCaptura() }
                    for await _ in fim {}
                }

                guard !saindo else { return }
                _ = try await Task.detached {
                    try banco.mudarEstado(documento: id, para: "processando", se: ["gravando", "pausada", "recuperavel", "interrompida", "finalizada", "processando"])
                }.value
                if let erroCaptura = reuniao?.erroCaptura {
                    try await Task.detached { try banco.definir(documento: id, .erroCaptura(erroCaptura)) }.value
                }
                await gravacaoDeAviso?.value
                faseFinal(id: id, etapa: "passadaFinal", fracao: nil)
                let erroDeDisco = reuniao?.erroProcessamento
                if recuperar { try await processador.recuperar() }
                else { try await processador.fechar() }
                if let erroDeDisco { throw ErroLancamento.processamento(erroDeDisco) }
                try await processador.encerrarCosturas()
                guard !saindo else { return }
                let identificados = try await processador.passadaFinal(motor)
                guard !saindo else { return }
                let atraso = reuniao?.atraso
                if let atraso {
                    try await Task.detached { try banco.definir(documento: id, .atraso(atraso)) }.value
                }
                guard var pronta = try banco.documento(id) else { throw ErroLancamento.parametro("id") }
                pronta.fase = "finalizando"
                reuniao = pronta
                aoMudar?()
                if let erro = pronta.erroCaptura {
                    processamento.finalizar(id: id, .erro, motivo: erro)
                    return
                }
                if !identificados {
                    processamento.finalizar(id: id, .erro, motivo: "Não foi possível identificar os falantes. O áudio continua guardado.")
                    return
                }
                guard pronta.pendentes?.isEmpty != false else {
                    processamento.finalizar(id: id, .erro, motivo: "Há fala pendente. O áudio continua guardado.")
                    return
                }
                if !pronta.trechos.isEmpty {
                    faseFinal(id: id, etapa: "resumindo", fracao: nil)
                    await concluirResumo(id: id, trabalhoID: id, banco: banco)
                } else {
                    processamento.finalizar(id: id, .concluido)
                }
            } catch {
                await fecharCaptura()
                if !saindo {
                    let motivo = error.localizedDescription
                    if let bancoAberto {
                        do {
                            try await Task.detached {
                                try bancoAberto.definir(documento: id, .aviso(motivo))
                                try bancoAberto.mudarEstado(documento: id, para: "recuperavel", se: ["gravando", "pausada", "interrompida", "processando"])
                            }.value
                        } catch { Registro.nota("Não foi possível gravar o estado da reunião: " + error.localizedDescription) }
                    }
                    processamento.finalizar(id: id, .erro, motivo: motivo)
                }
            }
        }
        aoMudar?()
    }

    private func registrarFalhaCaptura(_ erro: Error) {
        guard reuniao?.erroCaptura == nil, reuniao != nil, !saindo else { return }
        reuniao?.erroCaptura = "Falha de captura. " + erro.localizedDescription
        solicitarFim(em: Relogio.agora())
    }

    private func registrarFalhaProcessamento(_ erro: Error) {
        guard reuniao?.erroProcessamento == nil, reuniao != nil, !saindo else { return }
        reuniao?.erroProcessamento = "Falha ao gravar o áudio. " + erro.localizedDescription
        solicitarFim(em: Relogio.agora())
    }

    private func iniciarCaptura(usaMic: Bool, coletor: ColetorDeReuniao) async throws {
        if usaMic {
            do { try await iniciarMicrofone(coletor) }
            catch { avisarMicrofone("Não foi possível iniciar o microfone. A reunião continua com o áudio do Mac. " + error.localizedDescription) }
        }
        let captura = CapturaReuniao()
        try await coletor.abrir(.mac, em: Relogio.agora())
        self.captura = captura
        alimentacaoSistema = Task { [weak self] in
            do {
                for try await dados in captura.amostras {
                    self?.registrarNivel(dados, fonte: .mac)
                    do { try await coletor.receber(.mac, tempo: dados.tempo, amostras: dados.amostras) }
                    catch { self?.registrarFalhaProcessamento(error) }
                }
            } catch { self?.registrarFalhaCaptura(error) }
        }
        do { try await captura.iniciar(); audioSistema = "concedida" }
        catch {
            audioSistema = CGPreflightScreenCaptureAccess() ? "concedida" : "negada"
            throw error
        }
    }

    private func iniciarMicrofone(_ coletor: ColetorDeReuniao) async throws {
        let gravador = Gravador()
        try await coletor.abrir(.mic, em: Relogio.agora())
        gravador.aoMudarConfiguracao = { [weak self, weak gravador] in
            Task { @MainActor in
                guard let self, let gravador, self.gravadorMic === gravador else { return }
                await self.reiniciarMicrofone(coletor)
            }
        }
        gravador.aoNivel = { [weak self] nivel in
            Task { @MainActor in
                if self?.distribuidor?.derivacaoAberta == true { self?.controlador.nivelDoMicrofoneCompartilhado(nivel) }
            }
        }
        gravador.aoEstourarFila = { [weak self, weak gravador] in
            Task { @MainActor in
                guard let self, let gravador, self.gravadorMic === gravador else { return }
                self.avisarMicrofone("A fila do microfone ficou cheia. Confira os trechos pendentes e as lacunas.")
                await self.reiniciarMicrofone(coletor)
            }
        }
        let fluxo = try gravador.iniciarComTempo(microfone: controlador.prefs.microfone, limiteDeBuffers: 512)
        gravadorMic = gravador
        distribuidor = gravador.distribuidorDaReuniao
        mic = Task { [weak self] in
            for await dados in fluxo {
                self?.registrarNivel(dados, fonte: .mic)
                do { try await coletor.receber(.mic, tempo: dados.tempo, amostras: dados.amostras) }
                catch { self?.registrarFalhaProcessamento(error) }
            }
        }
    }

    private var reiniciandoMicrofone = false
    private var gravacaoDeAviso: Task<Void, Never>?

    private func reiniciarMicrofone(_ coletor: ColetorDeReuniao) async {
        guard !reiniciandoMicrofone, fimSolicitado == nil, !saindo else { return }
        reiniciandoMicrofone = true
        defer { reiniciandoMicrofone = false }
        avisarMicrofone("O microfone mudou. Tentando restabelecer a captura.")
        await pararMicrofone()
        guard fimSolicitado == nil, !saindo else { return }
        do {
            try await iniciarMicrofone(coletor)
            avisarMicrofone(nil)
        } catch {
            avisarMicrofone("O microfone foi interrompido. A reunião continua só com o áudio do Mac.")
        }
    }

    private func avisarMicrofone(_ aviso: String?) {
        reuniao?.avisoCaptura = aviso
        if let id = reuniao?.id, let banco = armazem {
            let anterior = gravacaoDeAviso
            gravacaoDeAviso = Task.detached {
                await anterior?.value
                do { try banco.definir(documento: id, .avisoCaptura(aviso)) }
                catch { Registro.nota("Não foi possível gravar o aviso do microfone.") }
            }
        }
        publicarReuniao()
        aoMudar?()
    }

    private func registrarNivel(_ dados: AmostrasComTempo, fonte: FonteDeAudio) {
        let agora = Relogio.agora()
        let nivel = min(1, (dados.amostras.reduce(Float.zero) { $0 + $1 * $1 } / Float(max(1, dados.amostras.count))).squareRoot() * 14)
        nivelPorFonte[fonte] = (agora, nivel)
        if nivel > 0.015 { ultimoSom = agora }

    }

    private func iniciarPulso() {
        pulso?.cancel()
        pulso = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(90))
                guard !Task.isCancelled, let self else { break }
                if reuniao?.fase == "gravando" || reuniao?.fase == "pausada" {
                    publicarReuniao()
                    consultarIntervalos()
                }
                else { processamento.pulso() }
            }
        }
    }

    private func consultarIntervalos() {
        let agora = Relogio.agora()
        guard !consultandoAudio, agora - ultimaConsultaDeAudio >= 1, let id = reuniao?.id, let banco = armazem else { return }
        ultimaConsultaDeAudio = agora
        consultandoAudio = true
        Task {
            defer { consultandoAudio = false }
            do {
                let faixas = try await Task.detached { try banco.periodos(documento: id) }.value
                guard reuniao?.id == id, reuniao?.lacunas ?? [] != faixas.lacunas || reuniao?.pendentes ?? [] != faixas.pendentes else { return }
                reuniao?.lacunas = faixas.lacunas
                reuniao?.pendentes = faixas.pendentes
                intervalos.send(EventoIntervalos(id: id, lacunas: faixas.lacunas, pendentes: faixas.pendentes))
            } catch { Registro.nota("Não foi possível ler os intervalos da reunião: " + error.localizedDescription) }
        }
    }

    private func publicarReuniao() {
        guard let reuniao, ["pausada", "gravando"].contains(reuniao.fase) else { return }
        let agora = Relogio.agora()
        let nivel = pausaDesde == nil ? min(1, nivelPorFonte.values.filter { agora - $0.instante < 0.3 }
            .reduce(Float.zero) { $0 + $1.nivel * $1.nivel }.squareRoot()) : 0
        processamento.emitir(id: reuniao.id, etapa: reuniao.fase,
            rotulo: reuniao.fase == "pausada" ? "Reunião pausada" : "Reunião em andamento",
            progresso: nil, reuniao: true, longo: false, nivel: nivel,
            decorrido: max(0, (pausaDesde ?? agora) - inicioDeCaptura - tempoPausado), audioChegando: nivel > 0.015,
            motivo: reuniao.avisoCaptura ?? (pausaDesde == nil && agora - ultimoSom >= 3 ? "Nenhum áudio detectado nos últimos 3 segundos." : nil))
    }

    func pausar(id: String) throws {
        guard reuniao?.id == id, let coletor, let processador, capturaIniciada,
              let fase = reuniao?.fase, fase == "gravando" || fase == "pausada" else {
            throw ErroLancamento.parametro("id")
        }
        let marco = Relogio.agora()
        let pausando = fase == "gravando"
        if pausando { pausaDesde = marco }
        else if let desde = pausaDesde { tempoPausado += marco - desde; pausaDesde = nil; ultimoSom = marco }
        reuniao?.fase = pausando ? "pausada" : "gravando"
        let anterior = pausaPendente
        pausaPendente = Task {
            await anterior?.value
            do {
                if pausando {
                    try await coletor.pausar(em: marco)
                    Task {
                        do { try await processador.fechar() }
                        catch { Registro.nota("Não foi possível processar a pausa: " + error.localizedDescription) }
                    }
                } else { await coletor.retomar(em: marco) }
            } catch { registrarFalhaProcessamento(error) }
        }
        publicarReuniao()
        aoMudar?()
    }

    func encerrar(id: String) throws {
        guard reuniao?.id == id, let fase = reuniao?.fase, fase == "gravando" || fase == "pausada" else {
            throw ErroLancamento.parametro("id")
        }
        solicitarFim(em: Relogio.agora())
    }

    private func solicitarFim(em marco: Double) {
        guard fimSolicitado == nil, let id = reuniao?.id else { return }
        fimSolicitado = marco
        reuniao?.fase = "finalizando"
        faseFinal(id: id, etapa: "passadaFinal", fracao: nil)
        if capturaIniciada { Task { await fecharCaptura() } }
    }

    private func fecharCaptura() async {
        if let paradaCaptura { await paradaCaptura.value; return }
        let parada = Task {
            let marco = fimSolicitado ?? Relogio.agora()
            await pausaPendente?.value
            await coletor?.marcarFim(em: marco)
            let anterior = captura
            captura = nil
            let alimentacao = alimentacaoSistema
            alimentacaoSistema = nil
            async let sistema: Void = pararSistema(anterior)
            await pararMicrofone()
            await sistema
            await alimentacao?.value
            do { try await coletor?.encerrar(em: marco) }
            catch {
                reuniao?.erroProcessamento = "Falha ao gravar áudio. " + error.localizedDescription
                if let id = reuniao?.id, let banco = armazem {
                    let mensagem = error.localizedDescription
                    _ = try? await Task.detached { try banco.definir(documento: id, .aviso(mensagem)) }.value
                }
            }
            sinalDeFim?.finish()
        }
        paradaCaptura = parada
        await parada.value
        paradaCaptura = nil
    }

    private func pararSistema(_ captura: CapturaReuniao?) async {
        await captura?.parar()
    }

    func abrirDerivacao() -> AsyncStream<[Float]>? {
        guard gravadorMic?.gravando == true else { return nil }
        return distribuidor?.abrirDerivacao()
    }

    func fecharDerivacao() { distribuidor?.fecharDerivacao() }

    private func pararMicrofone() async {
        let gravador = gravadorMic
        gravadorMic = nil
        let alimentacao = mic
        mic = nil
        distribuidor?.fecharDerivacao()
        distribuidor = nil
        if let gravador { await Task.detached { gravador.parar() }.value }
        await alimentacao?.value
    }

    func prepararSaida() async {
        saindo = true
        fimSolicitado = fimSolicitado ?? Relogio.agora()
        pulso?.cancel()
        tarefaResumo?.cancel()
        if capturaIniciada || captura != nil || gravadorMic != nil { await fecharCaptura() }
        trabalho?.cancel()
    }

    func desligar() {
        saindo = true
        trabalho?.cancel()
        tarefaResumo?.cancel()
        pulso?.cancel()
        fecharDerivacao()
    }

    private func receberTrechos(id: String, trechos: [TrechoNovo], atraso: Double) {
        guard reuniao?.id == id else { return }
        let lote = trechos.map { novo in
            var trecho = TrechoLongo(inicio: novo.inicio, fim: novo.fim, voz: novo.voz, texto: novo.texto)
            trecho.id = novo.id
            if novo.fonte == .mic { trecho.origem = "microfone" }
            return trecho
        }
        reuniao?.trechos += lote
        if let existentes = reuniao?.vozes {
            let novos = lote.reduce(into: existentes) { vozes, trecho in
                if !vozes.contains(where: { $0.id == trecho.voz }) {
                    vozes.append(VozLonga(id: trecho.voz, nome: trecho.voz == "voce" ? "Você" : "Chamada"))
                }
            }
            reuniao?.vozes = novos
        }
        let fim = max(reuniao?.duracao ?? 0, lote.map(\.fim).max() ?? 0)
        reuniao?.duracao = fim
        reuniao?.atraso = atraso
        segmentos.send(EventoTrechos(id: id, trechos: lote, vozes: reuniao?.vozes ?? [], duracao: fim, atraso: atraso))
    }

    private func faseFinal(id: String, etapa: String, fracao: Double?) {
        guard reuniao?.id == id else { return }
        if etapa == "falantesParciais" {
            reuniao?.baixaConfianca = true
            return
        }
        let mudouEtapa = processamento.atual?.etapa.id != etapa
        let resumo = etapa == "resumindo"
        let duracao = reuniao?.duracao ?? 0
        let pesoFalantes = duracao <= 120 ? 0.10 : 0.25
        processamento.emitir(id: id, etapa: etapa, rotulo: resumo ? "Resumindo" : "Passada final",
            progresso: fracao.map { resumo ? 1 : $0 * pesoFalantes })
        reuniao?.progresso = processamento.atual?.progresso
        reuniao?.etapa = etapa
        if mudouEtapa { aoMudar?() }
    }

    private var tarefaResumo: Task<[String], Error>?
    private var idResumo: String?
    private var resumoDispensado: String?

    private func gerarResumo(id: String, com resumidor: ResumidorReuniao) async throws -> [String]? {
        if resumoDispensado == id { throw CancellationError() }
        guard tarefaResumo == nil, let doc = try documento(id: id) else { throw ErroLancamento.ocupado }
        let tarefa = Task { try await resumidor.resumir(doc) }
        tarefaResumo = tarefa
        idResumo = id
        defer { tarefaResumo = nil; idResumo = nil }
        do {
            let resumo = try await tarefa.value
            if tarefa.isCancelled { throw CancellationError() }
            return resumo
        } catch {
            if tarefa.isCancelled { throw CancellationError() }
            throw error
        }
    }

    private func concluirResumo(id: String, trabalhoID: String, banco: ArmazemLancamento) async {
        let resumidor = ResumidorReuniao(servidor: controlador.servidor, modelo: controlador.modeloEmUso)
        let campo: ArmazemLancamento.CampoDocumento
        let desfecho: ResultadoProcessamento.Desfecho
        var motivo: String?
        do {
            guard let resumo = try await gerarResumo(id: id, com: resumidor), !resumo.isEmpty
            else { throw ErroDeLimpeza("O Qwen não devolveu tópicos.") }
            campo = .resumo(resumo)
            desfecho = .concluido
        } catch is CancellationError {
            campo = .resumoCancelado
            desfecho = .cancelado
        } catch let falta as ResumoSemModelo {
            motivo = falta.errorDescription
            campo = .erroResumo(motivo ?? "O resumo precisa do modelo de IA.")
            desfecho = .erro
        } catch {
            motivo = "Não foi possível gerar o resumo local. " + error.localizedDescription
            campo = .erroResumo(motivo ?? "Não foi possível gerar o resumo local.")
            desfecho = .erro
        }
        guard !saindo else { return }
        do {
            try await Task.detached { try banco.definir(documento: id, campo) }.value
            processamento.finalizar(id: trabalhoID, desfecho, motivo: motivo)
        } catch { processamento.finalizar(id: trabalhoID, .erro, motivo: error.localizedDescription) }
    }

    func resumir(id: String) throws {
        guard trabalho == nil, tarefaResumo == nil, !descartando, let banco = armazem,
              var doc = try banco.documento(id), doc.fonte != nil, doc.fase == "pronta", !doc.trechos.isEmpty
        else { throw ErroLancamento.ocupado }
        let trabalhoID = id
        doc.fase = "finalizando"
        doc.etapa = "resumindo"
        reuniao = doc
        processamento.emitir(id: id, etapa: "resumindo", rotulo: "Resumindo", progresso: nil)
        iniciarPulso()
        trabalho = Task {
            await concluirResumo(id: id, trabalhoID: trabalhoID, banco: banco)
            reuniao = nil
            resumoDispensado = nil
            pulso?.cancel()
            trabalho = nil
            aoMudar?()
            processarFila()
        }
        aoMudar?()
    }

    func cancelarResumo(id: String) {
        guard reuniao?.id == id, reuniao?.fase == "finalizando" else { return }
        resumoDispensado = id
        if idResumo == id { tarefaResumo?.cancel() }
    }

    func procurarArquivos(_ termo: String) throws -> [String] {
        guard let armazem else { return [] }
        return try armazem.lista(tipo: "arquivo")
            .filter { doc in
                guard doc.oculto != true, let completo = try armazem.documento(doc.id) else { return false }
                return completo.trechos.contains { $0.texto.localizedCaseInsensitiveContains(termo) }
            }.map(\.id)
    }

    func documento(id: String) throws -> TranscricaoLonga? {
        if let salvo = try armazem?.documento(id), salvo.fonte != nil || salvo.fase == "pronto" { return salvo }
        if reuniao?.id == id { return reuniao }
        if arquivo?.id == id { return arquivo }
        if let item = fila.first(where: { $0.documento.id == id }) ?? falhos[id] { return item.documento }
        return try armazem?.documento(id)
    }

    func alterar(id: String, campo: ArmazemLancamento.CampoDocumento) async throws {
        let banco = try await preparar()
        if try banco.existeDocumento(id) {
            try await Task.detached { try banco.definir(documento: id, campo) }.value
        } else if case .nome(let nome) = campo {
            if arquivo?.id == id { arquivo?.nome = nome }
            else if let i = fila.firstIndex(where: { $0.documento.id == id }) { fila[i].documento.nome = nome }
            else if falhos[id] != nil { falhos[id]?.documento.nome = nome }
            else { throw ErroLancamento.parametro("id") }
        } else { throw ErroLancamento.parametro("id") }
        if case .nome(let nome) = campo {
            if reuniao?.id == id { reuniao?.nome = nome }
            if arquivo?.id == id { arquivo?.nome = nome }
        }
        aoMudar?()
    }

    func renomearVoz(id: String, voz: String, nome: String) async throws {
        let banco = try await preparar()
        try await Task.detached { try banco.renomear(documento: id, voz: voz, nome: nome) }.value
        atualizarEdicoes(id: id)
    }

    func reatribuir(id: String, trecho: String, voz: String?, nome: String?) async throws {
        let banco = try await preparar()
        try await Task.detached {
            if let voz { try banco.reatribuir(documento: id, trecho: trecho, voz: voz) }
            else if let nome { try banco.atribuirNovaPessoa(documento: id, trecho: trecho, nome: nome) }
            else { throw ErroLancamento.parametro("voz") }
        }.value
        atualizarEdicoes(id: id)
    }

    private func atualizarEdicoes(id: String) {
        if reuniao?.id == id, let salvo = try? armazem?.documento(id) {
            reuniao?.trechos = salvo.trechos
            reuniao?.vozes = salvo.vozes
        }
        aoMudar?()
    }
}
