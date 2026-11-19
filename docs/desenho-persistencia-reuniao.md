# Persistência da reunião: modelo aditivo

Motivo: quando a transcrição da reunião é tratada como um documento único substituído por inteiro, a recuperação precisa reconciliar o diário de áudio e o SQLite por sobreposição de intervalo e texto. Toda heurística de reconciliação abre casos de perda ou duplicação. Este desenho remove a reconciliação.

## Princípios
1. **Uma única fonte da verdade para o texto: o SQLite.** O diário em disco guarda só áudio ainda não transcrito, nunca texto.
2. **Escrita só por inserção.** Trecho reconhecido vira linha nova e nunca é substituído nem apagado pelo motor. Edição do usuário, renomeação e reatribuição de voz são registros à parte (colunas de edição ou tabela de edições) aplicados na leitura, nunca reescrevendo o texto reconhecido.
3. **Unidade de idempotência: o bloco de áudio.** Cada bloco capturado tem ID estável (reunião, fonte, número sequencial). O estado do bloco vive no SQLite: `capturado` → `transcrito` (com 0 ou mais trechos) → áudio liberado. A transcrição de um bloco e a mudança para `transcrito` acontecem na mesma transação. Se o bloco já está `transcrito`, recuperar não faz nada com ele.
4. **Áudio só sai do disco depois do commit do bloco correspondente.** Nada de confirmação por sobreposição ou por texto: a confirmação é o estado do bloco no banco.
5. **Resultado vazio é resultado.** Bloco sem fala vira `transcrito` com 0 trechos; tentativa com erro deixa o bloco em `capturado` para nova tentativa (com contador e erro visível), nunca em estado que impeça retentativa.
6. **Passada final de falantes não reescreve texto.** Ela grava um mapa voz provisória → voz final (ou rótulo por trecho numa coluna própria), aplicado na leitura. O resumo é linha separada.
7. **Recuperar é o mesmo caminho de transcrever.** Na abertura, blocos `capturado` com áudio em disco são listados como recuperáveis; Recuperar transcreve esses blocos pelo caminho normal. Repetir não duplica nada. Descartar marca os blocos como descartados e só então libera o áudio (ação explícita do usuário).
8. **Escrita fora do ator principal.** Um escritor serial com conexão SQLite própria (WAL) recebe inserções e mudanças de estado; a interface só lê. Saída do app: o escritor drena o que já recebeu dentro do teto de 2 s; o resto continua em disco como blocos `capturado`.

## Arquivo transcrito
O mesmo modelo vale para arquivo: blocos por janela, trechos por inserção, reprocessamento cria uma nova versão (novo conjunto de trechos ligado a uma versão), e a versão anterior só é ocultada quando a nova termina.

## Migração
Aditiva: tabelas de blocos, trechos e edições novas (a tabela `trechos` existente pode ser reaproveitada se compatível); documentos salvos no formato atual continuam legíveis e são convertidos para linhas na primeira abertura, sem apagar o original até a conversão ser relida e conferida.

## Testes obrigatórios (regressão)
- mesmo intervalo com fala diferente no áudio pendente e no banco: nada apagado;
- partes exclusivas no banco e no áudio pendente: o resultado é a união; repetir a recuperação não muda nada;
- bloco com resultado vazio, depois com fala em nova tentativa;
- saída durante gravação, pausa e passada final; queda simulada; reabertura;
- escrita lenta (1,2 s e 3,0 s) com teto de 2 s;
- edição manual, renomeação e união de vozes depois da recuperação e depois da passada final;
- ditado comum e migração do banco legado intactos.

## Garantias obrigatórias e separação de falantes

Durante a reunião não há separação de falantes ao vivo. O texto ao vivo aparece por origem: "Você" (microfone) e "Chamada" (áudio do Mac). As pessoas da chamada são identificadas só na passada final, ao encerrar, sobre o áudio retido. Com isso, todo o PCM da reunião é retido até a passada final confirmar (com limite de disco e descarte explícito), sem diarizador em fluxo para recompor; a atribuição final é por ID de trecho, com a edição manual prevalecendo; a linha de tempo comum das duas fontes e o atalho continuam isolados. Na interface: durante a reunião, rótulos "Você" e "Chamada"; ao encerrar, "Pessoa 1, 2…" na chamada.

Resumo das garantias:
1. Bloco durável antes da fila: identidade selada (geração da fonte, sequência, faixa de amostras), PCM gravado de forma durável antes do registro, adoção de arquivos órfãos completos na abertura, referência quebrada como lacuna visível, transação de reconhecimento condicionada ao estado e à tentativa esperados, descarte com estado terminal antes da limpeza, `synchronous=FULL` no WAL para o commit de bloco.
2. Costura entre blocos com dono único por fonte: prefixo confirmado e cauda provisória; janela de reconhecimento com contexto; só confirma trechos com fronteira resolvida; mesma regra na pausa, no fim e na recuperação.
3. Zero trechos: "sem fala confirmada" é terminal; vazio com atividade de voz ou cobertura incompleta fica pendente e repetível.
4. PCM retido até a passada final (ver simplificação).
5. Linha de tempo comum pelos timestamps da captura e índices de amostra, lacuna explícita, nova geração ao trocar dispositivo; atalho por derivação própria, sem entrar como fonte da reunião; filas separadas.
6. Migração importa documento salvo, diários antigos e PCM pendente, com chaves estáveis, na mesma transação da marca de migração, preservando ambos quando divergirem.
7. Versões de arquivo com identidade de conteúdo, tentativa própria e publicação transacional da versão ativa.
Teto de saída: nenhum caminho longo no ator principal; ao vencer os 2 s, responde sem esperar a fila; o que estava só em RAM é declarado como não garantido.
