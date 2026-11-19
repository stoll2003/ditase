# Dita.se: especificação de movimento

Código: `Ditado/Pilula.swift` (pílula) e `ui/src/{App.tsx,paginas,componentes,lib/motion-tokens.ts,index.css}` (janela principal).

## Princípios

1. Movimento tem função: orientar (de onde veio), confirmar (a ação foi aceita) ou mostrar estado (ouvindo, processando). Nada decorativo.
2. Repouso é parado. Animação contínua só enquanto grava ou processa; fora disso nada roda (0% de CPU preservado).
3. Nunca bloqueia: navegar, clicar e apagar não esperam animação (exceto o recolhimento de 200 ms da linha apagada).
4. Só `opacity`, `transform` e, na pílula, o tamanho da cápsula. Sem curva com overshoot na janela principal.
5. Movimento reduzido do sistema: sem deslocamento, escala, desfoque nem laços. Sobram fades curtos e mudanças de cor.
6. Estado nunca depende só de movimento: texto, cor e rótulo de acessibilidade continuam.

## Pílula (SwiftUI)

Modelo: cada fase tem um tamanho de cápsula (`largura`, `altura`). O conteúdo vive num palco fixo de 344 x 32 centrado na base, e só a forma (máscara, fundo e borda) muda de tamanho. O conteúdo não é reposicionado durante o crescimento, então a troca é um único movimento contínuo.

Tamanhos: marca 40 x 8, botão 96 x 32, gravando 84 x 30, processando 64 x 30, mãos livres 172 x 30, aviso entre 96 e 344 de largura conforme o texto (30 de altura). A área do mouse (`JanelaDaPilula.zona`) já cobre esses tamanhos com folga (124 x 52 aberta, 190 x 50 em mãos livres), então não mudou.

| Transição | Gatilho | O que se move | Duração e curva | Movimento reduzido |
|---|---|---|---|---|
| Marca para botão | ponteiro entra na zona | cápsula cresce 40x8 para 96x32; borda e fundo clareiam; "Ditar" entra com escala 0,86 a 1, desfoque 3 a 0 e fade | mola 0,34 s, bounce 0,16; conteúdo 0,22 s com atraso de 0,07 s | só fade de 0,12 s, sem crescimento animado de conteúdo |
| Botão para marca | ponteiro sai | cápsula encolhe; conteúdo some antes | mola 0,26 s sem bounce; conteúdo fade 0,1 s | fade 0,08 s |
| Pressão do botão | mouse pressionado | pílula inteira a 95% de escala | mola 0,16 s | sem escala |
| Botão para mãos livres ou gravando | clique ou tecla | cápsula passa ao novo tamanho partindo do botão; "Ditar" sai (0,1 s), conteúdo novo entra a partir de 0,07 s | mola 0,38 s sem bounce | fade 0,12 s, entrada com atraso 0,05 s |
| Mãos livres: botões laterais | entrada da fase | cancelar e parar nascem com escala 0,5 a 1, escalonados 0,05 s e 0,09 s | mola 0,3 s, bounce 0,12 | aparecem prontos |
| Mãos livres: contador | a cada segundo | dígitos trocam por `numericText` | 0,25 s | troca direta |
| Botões redondos | hover e pressão | fundo mais claro no hover; escala 0,86 na pressão | 0,12 s e mola 0,16 s | só cor |
| Gravando ou mãos livres para processando | fim da fala | cápsula encolhe para 64x30; barras saem em fade; pontos entram | mola 0,38 s | fade 0,12 s |
| Processando para aviso | resultado | cápsula muda de largura conforme o texto; texto entra com a mesma entrada | mola 0,38 s | fade |
| Aviso, processando ou gravando para repouso | fim | cápsula volta a 40x8 e o conteúdo some primeiro | mola 0,38 s | fade 0,08 s |

Quando muda de fase no meio de outra transição, o SwiftUI parte do valor atual da mola (nada reinicia nem salta).

### Barras da voz

Desenho em `Canvas` dentro de `TimelineView(.animation)` a 60 quadros por segundo; só existe na tela enquanto grava (em mãos livres ou segurando a tecla).

- Entrada: `niveis` (9 valores, 0 a 1, cerca de 20 por segundo). O mais novo fica na barra do meio; as vizinhas mostram o passado, dois passos por barra, então a voz abre a partir do centro.
- Forma: ignora o chiado (`nivel - 0,03`) e levanta fala baixa (potência 0,65). Peso por distância do centro: 1, 0,92, 0,78, 0,62, 0,48.
- Suavização por barra, independente do fps: subida rápida (constante de tempo 0,045 s), descida lenta (0,16 s mais 0,025 s por posição). Isso tira o tremor do dado a 20 Hz e dá a sensação de voz subindo e baixando.
- Silêncio: a barra "respira" a 4,5% da altura (seno de 2,4 rad/s) para mostrar que o microfone está aberto.
- Altura de 4 a 20 pontos; opacidade acompanha o nível (0,55 a 1).
- Movimento reduzido: sem suavização, sem respiração, redesenho a 20 Hz; a altura segue o nível direto.
- `niveis` deixou de ser `@Published` de propósito: a pílula não é recalculada a cada amostra; só o `Canvas` lê o valor a cada quadro.

### Processando

Três pontos em onda, `Canvas` a 30 quadros por segundo, ciclo de 1,15 s, existe só na fase `processando`. Movimento reduzido: três pontos fixos com respiração de opacidade lenta (0,9 s), sem deslocamento.

### Custo (medido)

Banco de provas com a `JanelaDaPilula` real e voz sintética: repouso 0,1% de CPU (ruído de medição; nenhuma animação agendada); gravando 3 a 4%; mãos livres 4%; processando 3%; ciclo completo volta a 0,1 a 0,2% depois do aviso.

## Janela principal (React + motion)

Aproveitado do Tuff UI sem duplicar: gráficos, números rolantes, menus, toasts, diálogos, disclosure, botões (`active:scale`). `MotionConfig reducedMotion="user"` na raiz, mais `prefers-reduced-motion` no CSS próprio.

| Transição | Gatilho | O que se move | Duração e curva | Movimento reduzido |
|---|---|---|---|---|
| Página nova | troca de aba | conteúdo sobe 6 px com fade; a página antiga some na hora; rolagem volta ao topo | 0,22 s, `cubic-bezier(0.23, 1, 0.32, 1)` | só fade (`MotionConfig` remove o deslocamento) |
| Indicador da navegação | troca de aba | fundo do item ativo desliza entre itens (`layoutId`) | mola 0,34 s sem bounce | sem deslizar |
| Ponto de estado (lateral) | `fase` gravando | pulsa opacidade 1 para 0,35, 1,4 s, só enquanto grava; troca de cor em 0,3 s; texto troca com fade 0,18 s | linear suave | sem pulso, cor muda |
| Histórico e dicionário: carga | abrir a página | linhas entram em cascata: sobem 6 px com fade, 24 ms entre linhas, teto de 8 linhas | 0,26 s, mesma curva | sem animação |
| Item novo (ditado ou termo salvo) | chegou depois da primeira carga | linha cresce em altura e opacidade; fundo acende em azul e apaga em 1,4 s | 0,2 s; acender 1,4 s | altura e opacidade em 0 s; o acender continua (é só cor) |
| Termo editado | salvar | mesmo acender de fundo, 1,4 s | 1,4 s | continua |
| Apagar (ditado ou termo) | menu ou lixeira | linha recolhe (altura, opacidade, margem) e só então a ação de apagar roda | 0,2 s | corte imediato |
| Copiar | clique | ícone vira "Check" com escala 0,6 a 1 e fica 1,6 s, depois volta; toast do Tuff UI continua | 0,18 s | troca sem escala |
| Resultado de busca | digitar | sem animação (resposta imediata) | n/a | n/a |

Regras do histórico e do dicionário: busca nova reinicia a base de "itens vistos", então resultado de busca nunca conta como item novo. Apagar não depende de resposta lenta: se a ponte falhar, a linha volta (`setSaindo(false)` no ditado).

## Interrupção, teclado e foco

- Pílula: o alvo de cada mola é o estado atual; mudar de fase no meio redireciona sem salto.
- Páginas: trocar de aba durante a animação reinicia só a página nova; a antiga já tinha saído.
- Teclado: toda ação animada continua com o mesmo botão e foco; nada depende de hover. Anéis de foco do Tuff UI não são animados.
- Apagar: o recolhimento de 0,2 s não bloqueia outras linhas.

## Verificação

Rodado:
- Banco de provas (`swiftc` sobre `Pilula.swift` com a janela real e voz sintética): 490 quadros capturados do roteiro completo (marca, hover, clique, mãos livres, processando, aviso, gravando, repouso, mouse que sai) e conferidos em folhas de contato: cápsula encadeia tamanho, conteúdo e opacidade sem salto.
- Mesmo banco com `forcarSemMovimento: true` (157 quadros): sem escala nem deslocamento, só fades.
- CPU por fase (valores acima).
- `cd ui && npx tsc -b` e `npm run build`: passam.

Não coberto pelo banco de provas isolado: app completo ao vivo com microfone real e navegação da janela principal em tempo real. Conferir no app instalado.

---

# Contorno de progresso e reunião em andamento

Durante o processamento, junto da onda, um contorno azul preenche a borda da pílula, dá uma volta e, ao fechar, está pronto. Esta seção especifica o comportamento visual, a implementação na interface e os sinais emitidos pelo motor.

## Princípios desta seção

1. O contorno é uma barra de progresso, então só existe quando o progresso é real. Progresso desconhecido mostra só a onda, nunca um arco inventado.
2. Concluir nunca atrasa a entrega. O texto cola e a pílula troca de estado na hora; o contorno é uma camada própria que termina de fechar e sumir por até 0,55 s depois.
3. Cor não é o único sinal: erro e cancelamento têm texto na pílula (aviso) e rótulo de acessibilidade.
4. Repouso continua em 0% de CPU: nada anima fora de processamento, reunião (orçamento próprio, abaixo) ou dos 0,55 s finais.

## Contorno de progresso

### Forma e parâmetros

| Item | Valor |
|---|---|
| Trajeto | retângulo arredondado de raio igual à metade da altura (cápsula), no centro da linha de borda, recuado 1 pt para o traço de 2 pt ficar dentro do quadro da pílula; acompanha a largura e a altura animadas da pílula |
| Ponto de partida | topo, no centro (na pílula de processamento, 64 x 30, o meio do trecho reto de cima) |
| Sentido | horário; fecha no mesmo ponto de partida |
| Comprimento | proporcional ao progresso, medido ao longo do trajeto (velocidade uniforme, sem acelerar nas curvas); perímetro de cerca de 156 pt na pílula de processamento |
| Espessura | 2 pt, ponta arredondada; some enquanto o progresso for menor que 1,5% (evita pontinho solto) |
| Cor | azul do sistema do modo escuro, `#0A84FF`, nos dois temas: a pílula é escura em claro e escuro. Contraste 5,1:1 contra a pílula (`#121212`), acima dos 3:1 de elementos gráficos (WCAG 1.4.11) |
| Trilho | a borda de 1 pt que já existe, sem camada nova; enquanto o contorno está visível ela sobe de 14% para 22% de branco (0,2 s) para a parte ainda vazia ler como trilho |
| Camada | própria, acima da borda e fora da máscara do conteúdo, com estado próprio (não depende da fase), para sobreviver à troca de fase |

### Quando aparece

- Só com progresso conhecido. Sem progresso: nenhum arco, trilho inalterado (14%), só a onda.
- Atraso de 0,8 s depois de entrar em processamento, para um ditado curto (cerca de 1,2 s) não piscar um arco. Trabalhos que o motor marca como longos (arquivo, passada final da reunião) pulam o atraso.
- Entrada: fade de 0,15 s, `ease-out`. O primeiro valor recebido cresce de 0 até ele em 0,4 s com `cubic-bezier(0.23, 1, 0.32, 1)`: se o primeiro valor já for 40%, o traço "alcança" o ponto, e é um valor real.

### Suavização (o progresso chega em saltos)

- Cada atualização leva o traço ao novo valor por interpolação linear em tempo igual ao intervalo entre as duas últimas atualizações, limitado a 0,2 s no mínimo e 1,2 s no máximo. O traço anda sem parar e chega quando a próxima chega. Custo: ele fica atrasado um intervalo em relação ao valor real, e isso é aceito.
- Nunca passa do último valor recebido (sem extrapolar). Se o motor atrasar, o traço para onde está: parada real, não falsa.
- Nunca recua. Valor menor que o exibido por mais de 2% significa trabalho novo: o traço recolhe em 0,2 s e recomeça.
- Atualização no meio de uma interpolação reparte do ponto atual (sem salto).

### Etapa sem estimativa

Quando uma etapa não sabe a fração (resumo por modelo local, por exemplo), o traço fica parado no valor real da etapa e os últimos 12 pt dele respiram: opacidade de 1 para 0,45 e volta, período 1,2 s, `ease-in-out`. Mostra "ainda trabalhando" sem inventar progresso. Termina quando a etapa seguinte devolve progresso ou o trabalho acaba. Movimento reduzido: sem respiração (a onda já indica atividade).

### Conclusão

| Passo | O que acontece | Duração e curva |
|---|---|---|
| 1 | O traço fecha a volta, mesmo que falte pouco | até 0,25 s, `cubic-bezier(0.23, 1, 0.32, 1)` |
| 2 | Pulso: halo azul ao redor da pílula (raio 0 a 7 pt, opacidade 0 a 0,55 e volta a 0) | sobe 0,18 s `ease-out`, desce 0,32 s `ease-in`, começa quando a volta fecha |
| 3 | O traço some | fade de 0,25 s, a partir de 0,3 s depois de fechar |

- Total do fechamento ao fim: cerca de 0,55 s. Não empurra nada: a troca de fase da pílula (aviso ou repouso, mola de 0,38 s) acontece quando o motor termina, e o contorno a acompanha porque usa a mesma largura e altura animadas.
- Se o trabalho acabar antes do contorno aparecer (menos de 0,8 s), não há contorno nem pulso.
- O halo cabe na janela da pílula (360 x 64, pílula com 10 pt de folga embaixo); não muda `JanelaDaPilula.zona`.

### Cancelamento

O traço recolhe no sentido contrário até 0 em `clamp(0,35 s x progresso, 0,12 s, 0,3 s)` com `cubic-bezier(0.77, 0, 0.175, 1)`, depois fade de 0,1 s. Sem pulso. A cor continua azul (cancelar não é erro). A pílula segue a troca de fase normal (aviso "Cancelado" ou repouso).

### Erro

A cor vai de azul para vermelho do sistema `#FF453A` em 0,2 s (contraste 5,5:1), o comprimento fica onde parou, segura 0,5 s e some em 0,25 s. Sem tremor. A pílula mostra o aviso com o texto do erro: o vermelho nunca é o único sinal.

### Convivência com a onda

A onda de três pontos (30 quadros por segundo, ciclo de 1,15 s) fica no centro e não muda. A pílula de processamento continua 64 x 30 em todos os trabalhos. Onda e contorno são independentes: a onda para quando a fase muda, o contorno termina os seus 0,55 s.

### Movimento reduzido

O contorno é informação de estado, não enfeite, então permanece, mas sem animação:

| Momento | Com movimento reduzido |
|---|---|
| Progresso | o traço pula para cada valor novo, sem interpolação |
| Entrada | aparece direto (sem fade, mantém o atraso de 0,8 s) |
| Etapa sem estimativa | sem respiração |
| Conclusão | fecha na hora, sem pulso, segura 0,3 s e some em fade de 0,12 s |
| Cancelamento | fade de 0,12 s, sem recolher |
| Erro | troca de cor imediata, segura 0,5 s, fade de 0,12 s |

### Custo

- Processando sem contorno: 3% de CPU medido (onda a 30 quadros por segundo). O contorno é só uma linha de 2 pt, e o orçamento é no máximo 6% no mesmo banco de provas (hipótese, medir). A animação de cada atualização termina sozinha; com atualizações a cada 0,5 s ela fica quase contínua, então só vale enquanto há processamento.
- Fora de processamento e passados os 0,55 s finais, nenhuma animação agendada: repouso medido em 0,1% deve continuar.
- Sem desfoque nem sombra animados em contínuo: a única sombra é o halo de 0,5 s da conclusão.

## Reunião em andamento

### Ideia

Durante uma chamada a pílula tem de ser discreta e, ao mesmo tempo, deixar claro para a pessoa que o áudio está sendo captado. Por isso: pequena, parada, sem barras e sem pulso. O único movimento é o ponto de captura acompanhando o nível do áudio devagar, que serve de prova de que o áudio chega.

A reunião é um estado de fundo: ocupa o lugar do repouso. Ditado (segurar a tecla, mãos livres, processando) e avisos passam por cima e, ao terminar, a pílula volta para a reunião, não para a marca de repouso. Em reunião ela fica visível sempre, mesmo com "sempre visível" desligado, enquanto houver captura.

### Estado "reunião"

| Item | Valor |
|---|---|
| Tamanho | 88 x 26 (uma linha mais baixa que os estados de ditado, que têm 30) |
| Conteúdo | ponto de captura de 7 pt, vermelho do sistema `#FF453A`, e o tempo decorrido (`m:ss`, depois `h:mm:ss`, texto de 11 pt a 70% de branco, 40 pt de largura com redução de até 80%) |
| Fundo e borda | os mesmos da pílula, sem realce |
| Nível no ponto | opacidade do ponto vai de 0,5 (silêncio) a 1 (fala) seguindo o nível do áudio misturado; sobe em 0,1 s e desce em 0,5 s; redesenho a 10 quadros por segundo, não 60 |
| Silêncio | o ponto fica em 0,5, visível: silêncio da chamada não é falha |
| Sem áudio chegando (3 s ou mais sem blocos) | o ponto vira âmbar `#FF9F0A` fixo e a pílula mostra o aviso "Sem áudio da chamada" (texto), voltando à reunião quando o áudio volta |
| Tempo | atualiza 1 vez por segundo; os dígitos trocam com `numericText` de 0,25 s |
| Entrada | a pílula cresce de 40 x 8 (ou do botão) para 88 x 26 por mola de 0,38 s sem bounce; conteúdo entra com a mesma entrada dos outros estados; o ponto nasce com escala de 0,5 a 1 em mola de 0,3 s sem bounce |
| Movimento reduzido | ponto fixo em opacidade 1 (sem acompanhar nível), tempo sem `numericText`, entrada por fade de 0,12 s |

### Hover: controles

Passar o mouse sobre a pílula de reunião (mesma regra do botão em repouso) a expande para 140 x 30 com o mesmo movimento do botão (mola de 0,34 s, bounce 0,16; volta em 0,26 s sem bounce). Mostra o ponto com o tempo, um botão "Abrir" (leva à aba Reunião) e o botão redondo de encerrar, vermelho, igual ao de parar do mãos livres (20 pt). Os dois botões entram escalonados em 0,05 s e 0,09 s, como no mãos livres, e têm hover e pressão iguais aos botões redondos já especificados. Encerrar só fica ao alcance depois de expandir (dois passos), para um clique distraído no meio da chamada não encerrar a reunião. Confirmação extra de encerrar é decisão do Designer.

Consequência para o Front: `JanelaDaPilula.zona` precisa de um caso para reunião (repouso de reunião cerca de 104 x 34, expandida cerca de 170 x 50), e a regra que hoje só considera `sobre` na fase de repouso passa a valer também na reunião.

### Fim da reunião: passada final e resumo

1. Clique em encerrar: pressão do botão (escala 0,86, 0,16 s), a pílula vai para o estado de processamento (64 x 30, mola de 0,38 s sem bounce), o ponto de captura some em fade de 0,1 s e a onda de três pontos entra.
2. O contorno azul é reutilizado, com progresso global do trabalho todo (volta única):
   - passada final (corrige falantes), com a fração real do áudio reprocessado;
   - resumo pelo modelo local: se o motor estimar (tokens gerados sobre o limite), a fração segue; senão a etapa é marcada sem estimativa e o traço respira nos últimos 12 pt.
   Os pesos das etapas na volta (quanto da volta cada uma ocupa) são decisão do motor, medidos com a reunião simulada, não chutados.
3. Fechamento: conclusão normal (volta fecha, pulso, some) e aviso "Reunião pronta" (texto final é do Designer). Sugestão, não pedida: tornar esse aviso clicável para abrir a aba Reunião; isso exige incluir a fase de aviso em `JanelaDaPilula.zona`.
4. Cancelamento e erro seguem as regras do contorno. Erro na passada final ou no resumo mantém a transcrição ao vivo já gravada, e a pílula avisa com texto.

### Ditado durante a reunião

Se a pessoa ditar com a reunião em curso, a pílula faz as transições normais de ditado, e quando o ditado acaba (aviso incluído) volta ao estado de reunião. Conflito de microfone entre ditado e captura é tratado pelo motor.

### Orçamento de CPU na reunião

Reunião pode durar horas, então o estado de fundo tem orçamento próprio, bem menor que o ditado: no máximo 1% sustentado no banco de provas, sem animação de 60 quadros por segundo, sem `Canvas` de barras e sem sombra animada. Só 10 quadros por segundo no ponto e 1 atualização por segundo no tempo. Medir também a memória depois de alguns minutos, para checar que nada se acumula.

## Implementação em SwiftUI

- Trajeto próprio, não `Capsule`: usa `trim(from:to:)` sobre uma forma com ponto de partida no topo centro e sentido horário, porque o ponto inicial do `Capsule` do sistema não está garantido. Uma forma própria (cápsula de dois semicírculos e dois trechos retos) elimina a dúvida.
- Estado próprio do contorno no modelo da pílula, separado de `fase`: valor exibido, se há progresso conhecido, se a etapa tem estimativa, se o trabalho é longo, e o resultado final (concluído, cancelado ou erro). Sem esse resultado a pílula não consegue distinguir sucesso de cancelamento quando `fase` muda para aviso, pois hoje todos viram `aviso`.
- Interpolação por animação nativa de SwiftUI a cada atualização (duração igual ao intervalo, como acima), sem timer próprio: a animação termina sozinha, e a camada sai da hierarquia depois dos 0,55 s finais para nada continuar agendado.
- Largura e altura do contorno vêm do mesmo estado que anima a cápsula, para o laço acompanhar o encolhimento ou o crescimento sem descolar.
- Movimento reduzido: `accessibilityReduceMotion` já existente decide entre as colunas "normal" e "reduzido" das tabelas acima.
- Acessibilidade: a pílula de processamento expõe o percentual como valor, atualizado em degraus de 10% (para o VoiceOver não falar a cada atualização), e anuncia só a troca de etapa, a conclusão e o erro.
- Janela principal: onde mostrar o mesmo progresso (arquivo, passada final), usar o `ProgressBar` do Tuff UI com o mesmo `progresso` e `etapa`, sem animação própria.

## O que o motor emite

| Dado | Regra |
|---|---|
| `progresso` | fração de 0 a 1 do trabalho todo (volta única), nunca decrescente dentro do mesmo trabalho; ausente quando for desconhecido |
| Cadência | pelo menos uma emissão a cada 0,5 s enquanto processa; mais de 10 por segundo pode ser agrupado, a interface ignora o excesso |
| `etapa` | identificador e rótulo em português ("Transcrevendo", "Identificando falantes", "Passada final", "Resumindo") |
| `estimativa` | verdadeiro se a fração da etapa é estimada ou medida; falso quando a etapa não sabe (faz o traço respirar) |
| `longo` | verdadeiro para arquivo e passada final da reunião (pula o atraso de 0,8 s); falso ou ausente para ditado |
| Identidade | um id por trabalho, para ignorar progresso atrasado de um trabalho cancelado |
| Resultado | evento final separado da fase, com `concluido`, `cancelado` ou `erro` e o motivo em texto |
| Reunião: nível | nível de 0 a 1 do áudio misturado, pelo menos 10 vezes por segundo, mais o tempo decorrido |
| Reunião: captura | indicador de que há áudio chegando e, quando falta por 3 s ou mais, um motivo em texto |
| Pesos | quando houver mais de uma etapa, o peso de cada uma na volta, medido na reunião simulada |

## Verificação com dados sintéticos

Rodar com o banco de provas da pílula real e dados sintéticos:

1. Contorno em 0, 25, 50, 75 e 100% com atualizações em saltos de intervalos irregulares: folha de contato por quadro e checagem de que o traço nunca para e nunca recua.
2. Progresso desconhecido: nenhum arco. Etapa sem estimativa: traço parado e ponta respirando.
3. Conclusão, cancelamento e erro: sequência de quadros de cada um, com a troca de fase da pílula no meio, conferindo que o laço acompanha o encolhimento.
4. Movimento reduzido forçado: sem interpolação, sem pulso, sem respiração.
5. CPU: processando com e sem contorno (meta de até 6%), repouso depois do fim (meta 0,1%), reunião parada por vários minutos (meta de até 1%, com memória estável).
6. Trabalho curto (menos de 0,8 s): nenhum contorno aparece.

Notas de implementação:
- No estado de reunião ativa vale esta seção, não o estado de 212×30 pt de `docs/design/lancamento.md`; o repouso com cápsula dividida "Ditar | Reunião" no hover vem da especificação de telas.
- O relógio da pílula em reunião troca de valor sem a transição `numericText`, que sozinha custava 1,68% de CPU; com isso a reunião fica em 0,3%.
