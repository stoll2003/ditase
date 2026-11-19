# Motor de arquivos e reuniões

A chamada WebKit usa `postMessage({metodo, params})`. A resposta é uma string JSON, lida pelo `chamar` da interface. Falhas de parâmetros, permissões e ações conflitantes rejeitam a chamada. Erros assíncronos do motor aparecem no documento ou no evento de resultado.

## Métodos de lançamento

Cada chamada recebe `params` em formato de objeto. Uma chamada pode usar o método da interface ou o alias na mesma linha.

| Método | Parâmetros | Resposta e efeito |
|---|---|---|
| `lancamento.estado` | `{}` | Metadados de arquivos e histórico, reunião corrente, reuniões recuperáveis e permissões. |
| `lancamento.documento` | `{id}` | Transcrição completa de um item, carregada sob demanda. |
| `lancamento.buscarArquivos` | `{termo}` | IDs de arquivos com texto que corresponde à busca. |
| `lancamento.recuperar` | `{id}` | Reprocessa a gravação preservada de uma reunião interrompida. |
| `lancamento.escolher`, `arquivo.transcrever` | `{}` | Abre seletor nativo múltiplo. Devolve o ID do primeiro arquivo ou `null` ao fechar. |
| `lancamento.adicionar` | `{arquivos: [{nome, suportado}]}` | Alternativa para origens WebKit que só informam nomes. Abre o seletor e devolve o ID do primeiro arquivo ou `null`. O arraste nativo do Finder lê as URLs locais diretamente, enfileira na ordem e emite o ID do primeiro arquivo. |
| `arquivo.listar` | `{}` | Lista de arquivos. |
| `arquivo.cancelar` | `{id}` | Cancela ou retira o item em memória. Preserva uma transcrição anterior salva do mesmo arquivo. |
| `lancamento.remover` | `{id}` | Cancela trabalho de arquivo e oculta a versão salva, quando houver. Também oculta reunião arquivada. |
| `lancamento.desfazer` | `{id}` | Restaura resultado salvo que foi ocultado, inclusive após reabrir o app. |
| `lancamento.repetir` | `{id}` | Refaz arquivo concluído ou que falhou com a URL da sessão. Quando a URL não está disponível, abre o seletor de um único arquivo. Devolve `false` ao fechar o seletor. Substitui a versão salva após sucesso. |
| `lancamento.cancelarFila` | `{}` | Cancela o arquivo em andamento e esvazia a fila. |
| `lancamento.permissao` | `{qual}` | Solicita `audioSistema` ou `microfone`. |
| `lancamento.iniciar`, `reuniao.iniciar` | `{fonte}` | Inicia com `mac` ou `macEMicrofone`. |
| `lancamento.pausar`, `reuniao.pausar` | `{id}` | Alterna pausa e gravação. O tempo pausado fica fora da transcrição. |
| `lancamento.encerrar`, `reuniao.encerrar` | `{id}` | Encerra a captura e inicia passada final e resumo. |
| `reuniao.listar` | `{}` | Lista de reuniões arquivadas. |
| `lancamento.resumir` | `{id}` | Exibe a reunião no estado `finalizando` durante a nova geração. Preserva a transcrição e o resumo anterior até gerar o substituto. |
| `lancamento.cancelarResumo` | `{id}` | Cancela a geração corrente ou dispensa o resumo durante a passada final. |
| `lancamento.titulo` | `{id, nome}` | Altera o título do arquivo ou da reunião. |
| `lancamento.voz`, `falante.renomear` | `{id, voz, nome}` | Altera o nome do rótulo. Nomes iguais segundo a comparação em português unem as duas vozes. |
| `lancamento.trecho` | `{id, trecho, voz}` ou `{id, trecho, nome}` | Reatribui um trecho a uma voz existente ou nome fornecido. |
| `lancamento.exportar` | `{nome, formato, texto}` | Abre o painel nativo Salvar e só grava no destino escolhido. Aceita `txt`, `md`, `srt` e `vtt`. Devolve `true` após salvar ou `false` ao cancelar. |

Mutações sem retorno específico devolvem `true`. Os IDs passados aos métodos são de documentos, trechos ou vozes. O ID de um evento de progresso identifica a execução do trabalho e pode mudar entre tentativas do mesmo documento.

```json
[
  {"metodo":"lancamento.adicionar","params":{"arquivos":[{"nome":"pauta.mp3","suportado":true}]}},
  {"metodo":"lancamento.repetir","params":{"id":"a1"}},
  {"metodo":"lancamento.titulo","params":{"id":"a1","nome":"Pauta revisada"}},
  {"metodo":"lancamento.voz","params":{"id":"r1","voz":"voce","nome":"Ana"}},
  {"metodo":"lancamento.trecho","params":{"id":"r1","trecho":"t1","voz":"voce"}},
  {"metodo":"lancamento.remover","params":{"id":"r1"}},
  {"metodo":"lancamento.desfazer","params":{"id":"r1"}},
  {"metodo":"lancamento.resumir","params":{"id":"r1"}},
  {"metodo":"lancamento.cancelarResumo","params":{"id":"r1"}},
  {"metodo":"lancamento.exportar","params":{"nome":"pauta.vtt","formato":"vtt","texto":"WEBVTT\n\n"}}
]
```

Fases de arquivo: `naFila`, `decodificando`, `transcrevendo`, `diarizando`, `pronto`, `formatoNaoSuportado`, `semFala` e `falhou`. A preparação do texto após os falantes aparece como etapa própria no publicador.

Resposta de exemplo de `lancamento.estado` durante uma reunião. Os campos opcionais ficam ausentes até haver dados. Datas usam segundos Unix e os trechos usam segundos desde o início da mídia ou do tempo ativo da reunião. `atraso` mede do fechamento do bloco de áudio até a chegada do trecho.

```json
{
  "arquivos": [],
  "reuniao": {
    "id":"r1","nome":"Reunião","criadoEm":1760000000,"fase":"gravando","fonte":"macEMicrofone",
    "duracao":3,"atraso":0.36,"baixaConfianca":false,
    "vozes":[{"id":"voce","nome":"Você"}],
    "trechos":[{"id":"t1","inicio":0.4,"fim":2,"voz":"voce","texto":"Vou revisar a pauta.","parcial":true}]
  },
  "anteriores": [],
  "recuperaveis": [],
  "permissoes":{"audioSistema":"concedida","microfone":"concedida"},
  "disponivel":true
}
```

Nas listas `arquivos` e `anteriores` os documentos prontos vêm leves: `trechos` vazio, sem `bruto`, com `palavras` (contagem calculada no SQLite) e `previa` (primeira fala, até 240 caracteres, ou `null`). O documento completo vem por `lancamento.documento`. `recuperaveis` lista todo diário ainda em disco, inclusive um que tenha sobrado depois do arquivamento sem confirmação; `erroProcessamento` explica a última tentativa (por exemplo, "Nenhuma fala foi reconhecida. O áudio continua guardado para nova tentativa.").

```json
{"id":"a1","nome":"pauta.mp3","fase":"pronto","criadoEm":1760000000,"duracao":37.2,"vozes":[{"id":"offline-1","nome":"Pessoa 1"}],"trechos":[],"palavras":58,"previa":"Vamos revisar o plano."}
```

A reunião arquivada tem fase `pronta`, os trechos deixam de ser parciais e as vozes podem mudar após a passada final. `resumo` é um vetor de até quatro tópicos gerados, acompanhado de `resumoGerado: true`. Mantenha o rótulo de texto gerado na interface e exportações. `baixaConfianca: true` sinaliza trechos sem atribuição segura ou gravação parcial. A exclusão de histórico na ponte é reversível.

## Eventos e pílula

O evento existente `ditado` com `detail: "lancamento"` invalida o estado da interface. Os blocos ao vivo só emitem os trechos novos, rótulos e relógio. Uma invalidação completa fica reservada para início, mudanças de etapa, ações de edição e mudanças no acervo. Os eventos levam os dados em `detail`.

```json
{"evento":"ditado:progresso","detail":{"id":"r1","fase":"gravando","etapa":{"id":"gravando","rotulo":"Reunião em andamento"},"estimativa":false,"longo":false,"reuniaoAtiva":true,"nivel":0.21,"decorrido":1.5,"audioChegando":true}}
{"evento":"ditado:progresso","detail":{"id":"r1","fase":"processando","etapa":{"id":"passadaFinal","rotulo":"Passada final"},"progresso":0.18,"estimativa":true,"longo":true,"reuniaoAtiva":false}}
{"evento":"ditado:trechos","detail":{"id":"r1","trechos":[{"id":"t1","inicio":0.4,"fim":2,"voz":"voce","texto":"Vou revisar a pauta.","parcial":true}],"vozes":[{"id":"voce","nome":"Você"}],"duracao":3,"atraso":0.36}}
{"evento":"ditado:arquivos-soltos","detail":"a1"}
{"evento":"ditado:resultado","detail":{"id":"r1","resultado":"concluido"}}
```

O `Controlador` assina `controlador.lancamento.processamento.estados` e `resultados` no ator principal. Projeta o trabalho em `ProgressoDaPilula` e a captura em `CapturaDaPilula`, com pausa, nível, tempo ativo e aviso de falta de áudio. São `AnyPublisher<EstadoProcessamento?, Never>` e `AnyPublisher<ResultadoProcessamento, Never>`. `nil` em estados encerra a atividade e o resultado separado informa `concluido`, `cancelado` ou `erro`, com `motivo` quando cabível. O pulso de 90 ms repete o último estado enquanto há trabalho ou reunião. Na reunião ativa traz `nivel` da mistura em 0 a 1, tempo sem pausas e indicador de áudio. Após 3 s sem nível útil, inclui o motivo. Em etapas de fração desconhecida falta `progresso`, `estimativa` é falsa e o Front mantém a última fração conhecida. Frações emitidas para o mesmo trabalho são não decrescentes.

## Captura, armazenamento e IA local

- ScreenCaptureKit registra somente uma saída de áudio. O filtro seleciona uma tela apenas para vincular a captura do áudio. A configuração exclui a saída de áudio do próprio Dita.se. O macOS gerencia a permissão junto da captura de tela e áudio do sistema.
- O microfone, se habilitado, passa por uma captura única. Um ditado iniciado pelo atalho durante a reunião usa uma derivação desse PCM. A reunião transcreve somente sua captura original e o texto colado do ditado segue o fluxo normal. Se o áudio do sistema reproduzir a voz local, pode haver eco acústico.
- Durante a pausa, o motor descarta os novos buffers da reunião. O dispositivo pode permanecer aberto para o atalho. Na finalização, os trechos provisórios são reatribuídos pelo diarizador offline e edições manuais conhecidas são reaplicadas.
- O SQLite ganha a tabela aditiva `transcricoes_longas`. Gravação e edições usam uma única linha por documento. Só texto, tempos, rótulos, estado e metadados são persistidos. Arquivos ou vídeos de entrada seguem no local original.
- Reuniões em andamento têm um diário privado em `ReunioesPendentes/<id>` na pasta do app. Ele grava blocos de PCM em disco, uma atualização do bloco parcial a cada 500 ms, e os trechos de cada bloco uma única vez. A recuperação é aditiva e idempotente: blocos com trechos gravados são reaproveitados com os mesmos IDs, só os blocos sem trechos voltam ao reconhecedor, e edições manuais (por ID ou, se o trecho foi refeito, pela mesma fonte e intervalo) são mantidas. Resultado vazio ou falha deixa o diário intacto e a reunião em `recuperaveis` com `erroProcessamento`. O diário só sai depois que o SQLite devolve um documento igual ao gravado e que cobre todos os trechos e todo o áudio do diário; fala do diário ausente no resultado é acrescentada antes de gravar. Na abertura, diários já cobertos pelo SQLite são retirados; os demais continuam listados. Falha de gravação em disco termina a reunião com erro, desliga as fontes e deixa o diário para recuperar. A saída normal tem teto rígido de 2 s contado do pedido: o que estiver em memória é gravado fora do ator principal enquanto a captura para; se o prazo vencer, vale o último ponto confirmado (no máximo 0,5 s antes). Reservas temporárias de trabalho têm limpeza separada, depois de 24 h e do fim do processo.
- O resumo roda no Qwen local, via `127.0.0.1`. O prompt pede até quatro tópicos em português, restritos a assuntos, decisões e próximos passos explícitos. Usa até 4.000 caracteres de entrada por chamada, 192 tokens de saída e condensação por blocos para reuniões maiores. A transcrição é confirmada antes de iniciar o resumo. `erroResumo` ou `resumoCancelado` descrevem o desfecho da geração. `erroCaptura`, `erroProcessamento` e `avisoCaptura` representam respectivamente interrupção de entrada, falha no processamento recuperável e indisponibilidade de uma fonte durante a reunião.

## Linha de comando

```sh
Dita.se --transcrever-arquivo exemplo.mp3 --falantes --sem-ia
Dita.se --reuniao-simulada sistema.mp4 microfone.wav --tempo-real
```

Os comandos devolvem um documento JSON e retornam código 1 quando a entrada ou o processamento falha. A simulação aceita um segundo arquivo opcional para a voz local e roda acelerada por padrão. `--sem-resumo` desliga apenas o resumo da simulação. O comando de arquivo só salva se receber `--banco /caminho/teste.sqlite`. Os modos de transcrição seguem pelo motor antes de iniciar o AppKit. Transcrição CLI, reunião simulada, aferição e captura de tela usam uma pasta temporária exclusiva para estado e log. Os modelos instalados continuam disponíveis como insumo de leitura. A opção explícita `--banco` rejeita destinos na pasta de dados de uso diário e links que resolvam para o SQLite ou para o log de uso diário.

O Sortformer Fast v2.1 roda ao vivo em CPU e entrega rótulos provisórios. Em até 120 s, a passada final usa Sortformer Offline v2.1 e exige pelo menos 0,4 s de voz acumulada para publicar um rótulo. Se o modelo atingir a capacidade de quatro rótulos, se falhar ou se a mídia for mais longa, o motor usa o diarizador offline por agrupamento do FluidAudio. Arquivos usam esse mesmo caminho de diarização. A medição sintética embasa os pesos de progresso de 10% para a passada final curta e 25% para a variante por agrupamento, estimativa arredondada. A etapa Qwen mantém a fração estacionada até gerar a saída.

## Validação da integração

- `scripts/compilar.sh Debug` retornou `BUILD SUCCEEDED`.
- `swift test --disable-sandbox --package-path Nucleo` passou sem falhas.
- A validação por linha de comando transcreveu MP3 com duas vozes e MP4 com três, e reabriu as duas saídas no SQLite de teste. A reunião simulada em tempo real entregou os trechos e quatro tópicos do Qwen.
- A validação recusou o caminho do SQLite diário e um link simbólico para ele. Os metadados do banco, do WAL e do log diários coincidiram antes e depois. O registro do servidor Qwen apareceu na pasta temporária.
- O mapeamento das chamadas da interface e os exemplos JSON foram conferidos. O painel nativo, o progresso visual e a captura ao vivo ficam para o aceite manual.
