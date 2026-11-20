# Integração do núcleo Rust

O crate `nucleo` expõe os tipos abaixo na raiz. Os quatro traits dos motores e `TAXA` ficam em `nucleo::motores`. Todo PCM é mono, `f32`, a 16.000 Hz. A posição do bloco é um índice de amostra. Os tempos do reconhecedor e dos falantes são segundos absolutos.

| Fluxo | Entrada pública | Resultado |
|---|---|---|
| Ditado | `SessaoDitado::new(transcritor, limpador, dicionario, config, ao_trecho)`, `alimentar(&[f32])`, `finalizar()` | `ResultadoDitado` com texto, bruto, duração de fala, falhas e trocas. `cancelar()` encerra a entrada. O callback recebe ordem e bruto para a reserva no banco. |
| Texto | `RegrasFala::filtrar`, `AplicadorDicionario::aplicar`, `Guarda::avaliar`, `Fidelidade::fundir` | Texto por regras, trocas de vocabulário, motivo de recusa e texto com as palavras preservadas. |
| Histórico | `Banco::new(caminho)`, `inserir`, `gravar_trecho`, `concluir`, `historico`, `interrompidos`, `salvar(Termo)` | Identificadores SQLite de 64 bits. `Estatisticas::calcular(&ditados, agora, dias)` usa o fuso local. |
| Modelo | `BaixadorDeModelo::new(pasta, ModeloRemoto::qwen_padrao())`, `baixar(transporte, livre, cancelado, progresso)` | Caminho do arquivo conferido. A casca fornece HTTP com Range por `TransporteDeDownload`, espaço livre e sinal de cancelamento. Execute em trabalhador. |
| Captura | `ArmazemLancamento::new(caminho)`, `criar_documento`, `DiscoDeBlocos::new(raiz)`, `ColetorDeReuniao::new(documento, armazem, disco, inicio)` | `abrir(fonte, t)` cria a geração. `receber(fonte, tempo, amostras)`, `pausar`, `retomar` e `encerrar` usam o mesmo relógio monotônico. O ID de reunião é um UUID. |
| Reconhecimento da reunião | `ProcessadorDeReuniao { documento, armazem, disco, reconhecedor }`, `bloco_selado()`, `fechar()`, `recuperar()` | Vetor incremental com os IDs gravados. `passada_final(identificador)` identifica as vozes da chamada. Use outra conexão para o processador e serialize as chamadas a cada instância. |
| Arquivo | `criar_versao`, `ProcessadorDeArquivo::transcrever`, `encerrar_versao` | A casca implementa `FonteDePCM` ou entrega um vetor, fornece WAV para falantes e confere a origem antes da publicação. Erro volta ao chamador para marcar a tentativa como falha. |
| Leitura e edição | `documento`, `lista`, `recuperaveis`, `reatribuir`, `renomear`, `atribuir_nova_pessoa`, `descartar`, `limpar` | `TranscricaoLonga` serializa os nomes de campos em camelCase para a ponte. A lista inclui contagem e prévia. |

`regex` e `unicode-normalization` implementam os limites de palavras e a comparação sem acento do Foundation. `chrono` fornece calendário e fuso local no Windows e no Mac. `tempfile` mantém o áudio excedente do ditado em arquivo privado temporário com limpeza ao fechar. SQLite vem embutido pelo `rusqlite`. Os testes de download usam transporte em memória.

Para as assinaturas completas, execute `cargo doc -p nucleo --no-deps`.

