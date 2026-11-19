import Foundation

/// Amostras por segundo de todo PCM do lançamento (mono, Float32).
public let taxaLancamento = 16_000

public enum FonteDeAudio: String, Codable, Sendable, CaseIterable {
    /// Microfone local ("Você"), áudio do Mac ("Chamada") e arquivo importado.
    case mic, mac, arquivo

    public var vozAoVivo: String {
        switch self {
        case .mic: return "voce"
        case .mac: return "chamada"
        case .arquivo: return "desconhecida"
        }
    }
}

public enum EstadoBloco: String, Codable, Sendable {
    /// Áudio durável e registrado, ainda não resolvido pela costura.
    case capturado
    /// Resolvido pela costura, com zero ou mais trechos.
    case transcrito
    /// Resolvido sem fala e sem atividade de voz: terminal.
    case semFala
    /// Faz parte de uma faixa com voz que o reconhecedor não transcreveu: repetível.
    case pendente
    /// Áudio ausente: lacuna da captura ou referência quebrada.
    case lacuna
    /// Descartado pelo usuário: terminal, o arquivo pode sair.
    case descartado
}

/// Identidade selada de um bloco: reunião, fonte, geração da fonte e sequência.
public struct IdBloco: Hashable, Sendable, Codable {
    public let documento: String
    public let fonte: FonteDeAudio
    public let geracao: Int
    public let seq: Int

    public init(documento: String, fonte: FonteDeAudio, geracao: Int, seq: Int) {
        self.documento = documento
        self.fonte = fonte
        self.geracao = geracao
        self.seq = seq
    }

    public var texto: String { "\(documento)|\(fonte.rawValue)|\(geracao)|\(seq)" }
}

public struct BlocoRegistrado: Equatable, Sendable {
    public let id: IdBloco
    /// Primeira amostra na linha de tempo comum da reunião.
    public let inicio: Int
    public let quantidade: Int
    public var arquivo: String?
    public var tamanho: Int?
    public var soma: String?
    public var estado: EstadoBloco
    public var tentativas: Int
    public var erro: String?
    public var audioLiberado: Bool

    public init(id: IdBloco, inicio: Int, quantidade: Int, arquivo: String?, tamanho: Int?, soma: String?,
                estado: EstadoBloco, tentativas: Int = 0, erro: String? = nil, audioLiberado: Bool = false) {
        self.id = id
        self.inicio = inicio
        self.quantidade = quantidade
        self.arquivo = arquivo
        self.tamanho = tamanho
        self.soma = soma
        self.estado = estado
        self.tentativas = tentativas
        self.erro = erro
        self.audioLiberado = audioLiberado
    }

    public var fim: Int { inicio + quantidade }
}

/// Palavra devolvida pelo reconhecedor, em segundos absolutos da linha de tempo.
public struct PalavraReconhecida: Equatable, Sendable {
    public let texto: String
    public let inicio: Double
    public let fim: Double

    public init(texto: String, inicio: Double, fim: Double) {
        self.texto = texto
        self.inicio = inicio
        self.fim = fim
    }

    public var centro: Double { (inicio + fim) / 2 }
}

/// Linha de trecho a inserir. O ID é determinístico: repetir o mesmo trabalho não duplica.
public struct TrechoNovo: Equatable, Sendable {
    public let id: String
    public let fonte: FonteDeAudio
    public let inicio: Double
    public let fim: Double
    public let texto: String
    public let voz: String

    public init(id: String, fonte: FonteDeAudio, inicio: Double, fim: Double, texto: String, voz: String) {
        self.id = id
        self.fonte = fonte
        self.inicio = inicio
        self.fim = fim
        self.texto = texto
        self.voz = voz
    }
}

/// Faixa de tempo mostrada como lacuna ou pendência na transcrição.
public struct PeriodoDeAudio: Codable, Equatable, Sendable {
    public let inicio: Double
    public let fim: Double
    public let fonte: String
    public let motivo: String

    public init(inicio: Double, fim: Double, fonte: String, motivo: String) {
        self.inicio = inicio
        self.fim = fim
        self.fonte = fonte
        self.motivo = motivo
    }
}

/// O reconhecedor de fala. `fimDaFala` fecha a cauda: o modelo recebe silêncio depois,
/// mas nenhuma palavra ganha duração além do áudio real.
public protocol ReconhecedorDeFala: Sendable {
    func palavras(_ amostras: [Float], inicio: Double, fimDaFala: Double?) async throws -> [PalavraReconhecida]
}

/// Separação de falantes sobre um WAV da faixa inteira.
public protocol IdentificadorDeFalantes: Sendable {
    func faixas(wav: URL) async throws -> [FaixaDeVoz]
}

/// Regras de fala, dicionário e, para arquivos, a limpeza local com Guarda e Fidelidade.
public typealias PreparadorDeTexto = @Sendable (String) async -> String
