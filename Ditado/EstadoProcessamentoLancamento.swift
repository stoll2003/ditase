import Combine
import Foundation
import Nucleo

struct EstadoProcessamento: Codable, Equatable {
    struct Etapa: Codable, Equatable { let id: String; let rotulo: String }
    var id: String
    var etapa: Etapa
    var fase: String
    var progresso: Double?
    var estimativa: Bool
    var longo: Bool
    var reuniaoAtiva: Bool
    var nivel: Float?
    var decorrido: Double?
    var audioChegando: Bool?
    var motivo: String?
}

struct ResultadoProcessamento: Codable, Equatable {
    enum Desfecho: String, Codable { case concluido, cancelado, erro }
    let id: String
    let resultado: Desfecho
    let motivo: String?
}

@MainActor
final class PublicadorProcessamento {
    private let estado = CurrentValueSubject<EstadoProcessamento?, Never>(nil)
    private let resultado = PassthroughSubject<ResultadoProcessamento, Never>()
    var estados: AnyPublisher<EstadoProcessamento?, Never> { estado.eraseToAnyPublisher() }
    var resultados: AnyPublisher<ResultadoProcessamento, Never> { resultado.eraseToAnyPublisher() }
    var atual: EstadoProcessamento? { estado.value }
    private var maximo: Double = 0

    func emitir(id: String, etapa: String, rotulo: String, progresso: Double?,
                reuniao: Bool = false, longo: Bool = true, nivel: Float? = nil,
                decorrido: Double? = nil, audioChegando: Bool? = nil, motivo: String? = nil) {
        if atual?.id != id { maximo = 0 }
        let valor = progresso.map { max(maximo, min(1, max(0, $0))) }
        if let valor { maximo = valor }
        estado.send(EstadoProcessamento(id: id, etapa: .init(id: etapa, rotulo: rotulo), fase: reuniao ? etapa : "processando", progresso: valor,
                                         estimativa: valor != nil, longo: longo, reuniaoAtiva: reuniao, nivel: nivel,
                                         decorrido: decorrido, audioChegando: audioChegando, motivo: motivo))
    }

    func pulso() { if let atual { estado.send(atual) } }

    func finalizar(id: String, _ desfecho: ResultadoProcessamento.Desfecho, motivo: String? = nil) {
        guard atual?.id == id else { return }
        resultado.send(.init(id: id, resultado: desfecho, motivo: motivo))
        estado.send(nil)
        maximo = 0
    }
}
