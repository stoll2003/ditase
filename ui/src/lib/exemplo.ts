/**
 * DADOS DE EXEMPLO, usados só quando a interface roda fora do app (navegador).
 * Nada aqui é dado real do usuário.
 */
import { plataforma } from "./plataforma";
import type { Ajustes, Ditado, Estado, Estatisticas, Termo } from "./ponte";

const agora = Math.floor(Date.now() / 1000);
const textos = [
  "É o seguinte: eu preciso que você revise o módulo de exportação e me diga se o relatório está saindo com as colunas na ordem certa.",
  "Capítulo três. A manhã começou devagar, com aquela luz meio amarelada entrando pela janela da cozinha.\n\nEla ainda não tinha decidido se ia ou não ia, e o café esfriava na xícara.",
  "Dá uma olhada nesse trem aí e vê se o servidor subiu direito depois da atualização.",
  "Monta um resumo curto da reunião de ontem, com os três pontos que ficaram pendentes.",
  "Lembrar de comprar pilha para o controle e ligar para a oficina antes das cinco.",
];
const apps = [
  ["com.exemplo.editor", "Editor de Texto"],
  ["com.exemplo.chat", "Chat de IA"],
  ["com.exemplo.notas", "Notas"],
];

let ditados: Ditado[] = Array.from({ length: 42 }, (_, i) => {
  const texto = textos[i % textos.length];
  const palavras = texto.split(/\s+/).length * (1 + (i % 4));
  const [app, appNome] = apps[i % apps.length];
  return {
    id: 100 - i,
    iniciadoEm: agora - i * 9_400 - (i % 3) * 1_700,
    duracao: palavras / 2.6,
    duracaoFala: palavras / 2.75,
    palavras,
    textoBruto: texto.toLowerCase(),
    texto,
    app,
    appNome,
    estado: "concluido",
    latencia: 0.6 + (i % 5) * 0.3,
    motor: "parakeet-ultra",
    limpador: i % 4 === 0 ? "regras" : "Qwen3.5-4B-Q4_K_M",
    trocasDicionario: i % 6 === 0 ? 1 : 0,
    origem: "ditado",
    audio: "",
  };
});

let termos: Termo[] = [
  { id: 1, termo: "VitePress", ouvidoComo: ["Vite Press"], usos: 12, origem: "manual", criadoEm: agora },
  { id: 2, termo: "Svelte", ouvidoComo: ["Suelti", "Svelt"], usos: 31, origem: "manual", criadoEm: agora },
  { id: 3, termo: "Vitest", ouvidoComo: [], usos: 3, origem: "manual", criadoEm: agora },
  { id: 4, termo: "Fulano de Tal", ouvidoComo: [], usos: 0, origem: "wispr", criadoEm: agora },
];

const teclasMac = [
  { id: "fn", nome: "Fn (🌐)", simbolo: "Fn" },
  { id: "opcaoDireita", nome: "Opção direita (⌥)", simbolo: "⌥ direita" },
  { id: "comandoDireito", nome: "Comando direito (⌘)", simbolo: "⌘ direito" },
  { id: "controleDireito", nome: "Controle direito (⌃)", simbolo: "⌃ direito" },
];
const teclasWindows = [
  { id: "controleDireito", nome: "Ctrl direito", simbolo: "Ctrl direito" },
  { id: "shiftDireito", nome: "Shift direito", simbolo: "Shift direito" },
  { id: "altDireito", nome: "Alt direito (AltGr)", simbolo: "Alt direito" },
];
const noWindows = plataforma() === "windows";
const atalhoInicial = noWindows ? teclasWindows[0] : teclasMac[1];

const valores: Record<string, string> = {
  atalho: atalhoInicial.id,
  limpezaIA: "1",
  modelo: "Qwen3.5-4B-Q4_K_M.gguf",
  sons: "1",
  limiteMinutos: "120",
  pararEmSilencioMinutos: "5",
  guardarAudioDias: "7",
  microfone: "",
  pilulaSempreVisivel: "1",
  espacoNoFim: "1",
};

function estatisticas(dias: number): Estatisticas {
  const porDia = Array.from({ length: dias }, (_, i) => {
    const data = new Date(Date.now() - (dias - 1 - i) * 86_400_000);
    const palavras = i % 7 === 3 ? 0 : Math.round(600 + 900 * Math.abs(Math.sin(i * 1.7)) + (i % 5) * 140);
    return { dia: data.toISOString().slice(0, 10), palavras, ditados: palavras ? 4 + (i % 6) : 0, minutos: palavras / 158 };
  });
  const total = 48_230;
  return {
    totalPalavras: total,
    totalDitados: 612,
    minutosFalados: total / 158,
    ppmMedio: 158,
    ppmDeCadaDitado: Array.from({ length: 160 }, (_, i) => Math.round(158 + 34 * Math.sin(i * 2.1) + 22 * Math.cos(i * 0.7))),
    minutosEconomizados: total / 40 - total / 158,
    diasSeguidos: 6,
    maiorSequencia: 14,
    diasAtivos: 38,
    palavrasHoje: porDia[porDia.length - 1].palavras,
    palavrasNaSemana: porDia.slice(-7).reduce((s, d) => s + d.palavras, 0),
    palavrasNaSemanaAnterior: porDia.slice(-14, -7).reduce((s, d) => s + d.palavras, 0),
    maiorDitadoPalavras: 2931,
    maiorDitadoMinutos: 19.3,
    latenciaMediana: 1.2,
    trocasDicionario: 46,
    porDia,
    porHora: Array.from({ length: 24 }, (_, h) => (h < 7 ? 0 : Math.round(2600 * Math.exp(-((h - 15) ** 2) / 22) + 300))),
    porDiaDaSemana: [7200, 8100, 6900, 9400, 7700, 4300, 4630],
    porApp: [
      { app: "com.exemplo.editor", nome: "Editor de Texto", palavras: 21_400, ditados: 88 },
      { app: "com.exemplo.chat", nome: "Chat de IA", palavras: 18_900, ditados: 402 },
      { app: "com.exemplo.notas", nome: "Notas", palavras: 5_300, ditados: 97 },
      { app: "com.exemplo.correio", nome: "Correio", palavras: 2_630, ditados: 25 },
    ],
  };
}

const estado: Estado = {
  fase: "parado",
  maosLivres: false,
  reconhecedor: { estado: "pronto", detalhe: "", nome: "Parakeet Ultra" },
  limpador: { estado: "pronto", detalhe: "", ligado: true, modelo: "Qwen3.5-4B-Q4_K_M" },
  modeloIA: { estado: "instalado", arquivo: "Qwen3.5-2B-Q4_K_M.gguf", tamanho: 1_280_835_840, aviso: true },
  permissoes: { microfone: "concedida", acessibilidade: "concedida" },
  atalho: { tecla: atalhoInicial.id, nome: atalhoInicial.nome, simbolo: atalhoInicial.simbolo, instalado: true },
  wisprAberto: false,
  wisprDisponivel: !noWindows,
  versao: "0.1.0 (exemplo)",
};

// Download fictício do modelo: ?modelo=ausente|baixando|conferindo|falhou|indisponivel escolhe o
// ponto de partida; "Baixar modelo" avança sozinho até instalado.
type ModeloDemo = Estado["modeloIA"];
let modeloDemo: ModeloDemo | undefined;
let downloadDemo: ReturnType<typeof setInterval> | undefined;

function situacaoInicialDoModelo(): ModeloDemo {
  const pedido = new URLSearchParams(location.search).get("modelo");
  const base = estado.modeloIA;
  switch (pedido) {
    case "ausente":
      return { ...base, estado: "ausente", feito: 0 };
    case "baixando":
      return { ...base, estado: "baixando", feito: Math.round(base.tamanho * 0.42) };
    case "conferindo":
      return { ...base, estado: "conferindo", feito: base.tamanho };
    case "falhou":
      return { ...base, estado: "falhou", feito: 312_000_000, detalhe: "Sem conexão com a internet. Confira a rede e tente de novo; o download continua de onde parou." };
    case "indisponivel":
      return { ...base, estado: "indisponivel" };
    default:
      return base;
  }
}

function avisarEstado() {
  window.dispatchEvent(new CustomEvent("ditado", { detail: "estado" }));
}

function estadoComModelo(atual: Estado): Estado {
  modeloDemo ??= situacaoInicialDoModelo();
  const m = modeloDemo;
  const limpador: Estado["limpador"] =
    m.estado === "indisponivel" ? { ...atual.limpador, estado: "semBinario", modelo: "Qwen3.5-2B-Q4_K_M" }
      : m.estado === "instalado" ? atual.limpador
        : { ...atual.limpador, estado: "semModelo", modelo: "Qwen3.5-2B-Q4_K_M" };
  return { ...atual, limpador, modeloIA: m };
}

function baixarDemo() {
  if (!modeloDemo || downloadDemo) return;
  modeloDemo = { ...modeloDemo, estado: "baixando", detalhe: undefined, feito: modeloDemo.feito ?? 0 };
  avisarEstado();
  downloadDemo = setInterval(() => {
    const m = modeloDemo!;
    if (m.estado === "conferindo") {
      clearInterval(downloadDemo);
      downloadDemo = undefined;
      modeloDemo = { ...m, estado: "instalado" };
    } else {
      const feito = Math.min(m.tamanho, (m.feito ?? 0) + 48_000_000);
      modeloDemo = { ...m, feito, estado: feito >= m.tamanho ? "conferindo" : "baixando" };
    }
    avisarEstado();
  }, 200);
}

export function responderDeExemplo(metodo: string, params: Record<string, unknown>): unknown {
  const consulta = new URLSearchParams(location.search);
  switch (metodo) {
    case "modelo.baixar":
      baixarDemo();
      return true;
    case "modelo.cancelar":
      clearInterval(downloadDemo);
      downloadDemo = undefined;
      if (modeloDemo) modeloDemo = { ...modeloDemo, estado: "ausente" };
      avisarEstado();
      return true;
    case "estado":
      if (consulta.get("exemplo") === "novo") {
        return {
          ...estadoComModelo(estado),
          reconhecedor: { ...estado.reconhecedor, estado: "carregando" },
          permissoes: { microfone: "pendente", acessibilidade: noWindows ? "concedida" : "pendente" },
          atalho: { ...estado.atalho, instalado: false },
        };
      }
      return estadoComModelo(estado);
    case "historico": {
      if (consulta.get("exemplo") === "novo") return [];
      const busca = String(params.busca ?? "").toLowerCase();
      return ditados.filter((d) => d.texto.toLowerCase().includes(busca));
    }
    case "estatisticas":
      if (consulta.get("exemplo") === "novo") {
        return { ...estatisticas(Number(params.dias ?? 30)), totalPalavras: 0, totalDitados: 0, porApp: [], ppmDeCadaDitado: [] };
      }
      return estatisticas(Number(params.dias ?? 30));
    case "dicionario.listar":
      return consulta.get("exemplo") === "novo" ? [] : termos;
    case "dicionario.salvar": {
      const id = Number(params.id ?? 0);
      const novo: Termo = {
        id: id || Math.max(0, ...termos.map((t) => t.id)) + 1,
        termo: String(params.termo),
        ouvidoComo: (params.ouvidoComo as string[]) ?? [],
        usos: termos.find((t) => t.id === id)?.usos ?? 0,
        origem: "manual",
        criadoEm: agora,
      };
      termos = id ? termos.map((t) => (t.id === id ? novo : t)) : [...termos, novo];
      return novo;
    }
    case "dicionario.apagar":
      termos = termos.filter((t) => t.id !== params.id);
      return true;
    case "ditado.apagar":
      ditados = ditados.filter((d) => d.id !== params.id);
      window.dispatchEvent(new CustomEvent("ditado", { detail: "historico" }));
      return true;
    case "ajustes.ler":
      return {
        valores,
        teclas: (noWindows ? teclasWindows : teclasMac).map(({ id, nome }) => ({ id, nome })),
        microfones: [{ uid: "embutido", nome: noWindows ? "Microfone integrado" : "Microfone do MacBook" }],
        modelos: ["Qwen3.5-2B-Q4_K_M.gguf", "Qwen3.5-4B-Q4_K_M.gguf"],
        pastaDeModelos: noWindows ? "%APPDATA%\\Dita.se\\Modelos" : "~/Library/Application Support/Ditado/Models",
        llamaInstalado: situacaoInicialDoModelo().estado !== "indisponivel",
      } satisfies Ajustes;
    case "ajustes.definir":
      valores[String(params.chave)] = String(params.valor);
      if (params.chave === "atalho") {
        const escolhida = (noWindows ? teclasWindows : teclasMac).find((t) => t.id === String(params.valor));
        if (escolhida) {
          estado.atalho = { ...estado.atalho, tecla: escolhida.id, nome: escolhida.nome, simbolo: escolhida.simbolo };
          avisarEstado();
        }
      }
      if (params.chave === "avisoModelo" && modeloDemo) {
        modeloDemo = { ...modeloDemo, aviso: params.valor === "1" };
        avisarEstado();
      }
      return true;
    case "wispr.importar":
      return { ditados: 44, termos: 2, encontrado: true };
    case "relatorio.copiar":
      return [
        "Relatório de teste do Dita.se para Windows",
        `Versão: ${estado.versao}`,
        "Windows: Windows 11 (exemplo)",
        `Atalho: ${estado.atalho.nome} (gancho instalado)`,
        "Microfones: 1 encontrado(s); escolhido: padrão",
        `Fase do ditado: ${estado.fase}`,
      ].join("\n");
    default:
      return true;
  }
}

// Dados fictícios usados apenas no navegador. O áudio escolhido na demonstração não é lido.
type ArquivoDemo = import("./lancamento").Arquivo;
type ReuniaoDemo = import("./lancamento").Reuniao;
type LanceDemo = import("./lancamento").Lancamento;

const vozesDemo = [{ id: "a", nome: "Pessoa 1" }, { id: "b", nome: "Pessoa 2" }, { id: "c", nome: "Pessoa 3" }];
const falasDemo = [
  { id: "1", voz: "a", inicio: 14, fim: 24, texto: "Vamos reservar as primeiras duas semanas para testar o piloto com uma turma pequena. Assim conseguimos ajustar o material antes de abrir novas vagas." },
  { id: "2", voz: "b", inicio: 26, fim: 39, texto: "Eu preparo o guia de boas-vindas e a lista de equipamentos. Na quinta, posso compartilhar a primeira versão para comentários." },
  { id: "3", voz: "c", inicio: 40, fim: 54, texto: "Fechado. Fico com a agenda dos encontros e confirmo a sala antes de divulgar as datas." },
  { id: "4", voz: "a", inicio: 62, fim: 81, texto: "Combinamos uma revisão rápida na sexta às dez. Até lá, vamos reunir as dúvidas para deixar o primeiro encontro mais direto." },
];
const agoraDemo = Math.floor(Date.now() / 1000);
const baseDemo = (id: string, nome: string) => ({ id, nome, criadoEm: agoraDemo, vozes: [], trechos: [] });
const reuniaoDemo = (id: string, fase: ReuniaoDemo["fase"]): ReuniaoDemo => ({
  ...baseDemo(id, "Planejamento da oficina de fotografia"), fase, fonte: "macEMicrofone", duracao: 868, atraso: 2,
  falantes: fase === "pronta" ? "identificados" : "pendente",
  vozes: fase === "pronta"
    ? [{ id: "a", nome: "Você" }, { id: "b", nome: "Pessoa 1" }, { id: "c", nome: "Pessoa 2" }]
    : [{ id: "a", nome: "Você" }, { id: "chamada", nome: "Chamada" }],
  trechos: fase === "pronta" ? falasDemo : falasDemo.map((t) => t.voz === "a" ? t : { ...t, voz: "chamada" }),
  ...(fase === "pronta" ? { baixaConfianca: true, resumo: [
    "O piloto terá duas semanas e uma turma pequena.",
    "Lia prepara o guia de boas-vindas e a lista de equipamentos para quinta.",
    "A revisão da agenda ficou para sexta às 10h.",
  ] } : {}),
});
const consultaLancamento = new URLSearchParams(location.search);
const previa = consultaLancamento.get("reuniao");
let lancamento: LanceDemo = {
  disponivel: true,
  permissoes: { audioSistema: "concedida", microfone: "concedida" },
  arquivos: consultaLancamento.get("arquivos") === "vazio" ? [] : [
    { ...baseDemo("em-curso", "Entrevista sobre o jardim comunitário.m4a"), fase: "transcrevendo", progresso: 0.48, duracao: 726 },
    { ...baseDemo("na-fila", "Revisão do roteiro da exposição.mov"), fase: "naFila" },
    { ...baseDemo("concluido", "Oficina de fotografia.mp4"), fase: "pronto", duracao: 2538, processamento: 19, vozes: vozesDemo, trechos: falasDemo },
  ],
  reuniao: previa === "durante" ? reuniaoDemo("demo-viva", "gravando") : undefined,
  anteriores: previa === "depois" ? [reuniaoDemo("demo-final", "pronta")] : [],
  recuperaveis: previa === "pendente" ? [{ ...reuniaoDemo("demo-pendente", "recuperavel"),
    pendentes: [{ inicio: 26, fim: 39, fonte: "mac", motivo: "Áudio guardado, ainda não transcrito." }],
    lacunas: [{ inicio: 54, fim: 62, fonte: "mic", motivo: "A fonte não entregou áudio neste intervalo." }] }] : [],
};
if (consultaLancamento.get("arquivos") === "estados") {
  const fases: ArquivoDemo["fase"][] = ["decodificando", "diarizando", "formatoNaoSuportado", "semFala", "falhou"];
  lancamento.arquivos.push(...fases.map((fase) => ({ ...baseDemo(fase, `Exemplo ${fase}.wav`), fase,
    progresso: fase === "decodificando" ? 0.08 : fase === "diarizando" ? 0.88 : undefined })));
}
const removidos = new Map<string, ReuniaoDemo>();
function atualizarLancamento() {
  lancamento = structuredClone(lancamento);
  window.dispatchEvent(new CustomEvent("ditado", { detail: "lancamento" }));
}
let filaTimer: number | undefined;
function moverFila() {
  window.clearTimeout(filaTimer);
  const proximo = lancamento.arquivos.find((a) => ["decodificando", "transcrevendo", "diarizando"].includes(a.fase)) ?? lancamento.arquivos.find((a) => a.fase === "naFila");
  if (!proximo) return;
  const estados: ArquivoDemo["fase"][] = ["naFila", "decodificando", "transcrevendo", "diarizando", "pronto"];
  const indice = estados.indexOf(proximo.fase) + 1;
  proximo.fase = estados[indice] ?? "pronto";
  proximo.progresso = [0, 0.08, 0.54, 0.88, 1][indice];
  if (proximo.fase === "pronto") Object.assign(proximo, { vozes: vozesDemo, trechos: falasDemo, duracao: 81, processamento: 4 });
  atualizarLancamento();
  filaTimer = window.setTimeout(moverFila, 850);
}
/** Nomes do contrato da ponte e seus aliases, levados ao nome usado aqui na demonstração. */
const aliasesDoContrato: Record<string, string> = {
  "arquivo.transcrever": "lancamento.adicionar", "arquivo.cancelar": "lancamento.remover",
  "reuniao.iniciar": "lancamento.iniciar", "reuniao.pausar": "lancamento.pausar", "reuniao.encerrar": "lancamento.encerrar",
  "falante.renomear": "lancamento.voz", "lancamento.escolher": "lancamento.adicionar",
};

export function responderLancamento(nome: string, p: Record<string, unknown>) {
  const metodo = aliasesDoContrato[nome] ?? nome;
  if (metodo === "lancamento.estado") return structuredClone(lancamento);
  if (metodo === "arquivo.listar") return structuredClone(lancamento.arquivos);
  if (metodo === "reuniao.listar") return structuredClone(lancamento.anteriores);
  const arq = lancamento.arquivos.find((a) => a.id === p.id);
  const reuniao = lancamento.reuniao;
  const doc = arq ?? (reuniao?.id === p.id ? reuniao : lancamento.anteriores.find((r) => r.id === p.id));
  switch (metodo) {
    case "lancamento.documento": return doc ?? null;
    case "lancamento.buscarArquivos": return lancamento.arquivos
      .filter((a) => a.trechos.some((t) => t.texto.toLocaleLowerCase("pt-BR").includes(String(p.termo)))).map((a) => a.id);
    case "lancamento.adicionar": {
      const arquivos = ((p.arquivos ?? []) as { nome: string; suportado: boolean }[]).map((f) => ({
        ...baseDemo(crypto.randomUUID(), f.nome), fase: f.suportado ? "naFila" as const : "formatoNaoSuportado" as const,
      }));
      lancamento.arquivos.push(...arquivos);
      window.clearTimeout(filaTimer);
      filaTimer = window.setTimeout(moverFila, 450);
      atualizarLancamento();
      return arquivos[0]?.id;
    }
    case "arquivo.cancelar":
    case "lancamento.remover":
      if (arq) lancamento.arquivos = lancamento.arquivos.filter((a) => a.id !== arq.id);
      else {
        const salva = lancamento.anteriores.find((r) => r.id === p.id);
        if (salva) removidos.set(salva.id, salva);
        lancamento.anteriores = lancamento.anteriores.filter((r) => r.id !== p.id);
      }
      if (arq && ["decodificando", "transcrevendo", "diarizando"].includes(arq.fase)) moverFila();
      break;
    case "lancamento.desfazer": {
      const id = String(p.id);
      const salva = removidos.get(id);
      if (salva) {
        removidos.delete(id);
        lancamento.anteriores.push(salva);
        lancamento.anteriores.sort((a, b) => b.criadoEm - a.criadoEm);
      }
      break;
    }
    case "lancamento.cancelarFila":
      window.clearTimeout(filaTimer);
      lancamento.arquivos = lancamento.arquivos.filter((a) => !["naFila", "decodificando", "transcrevendo", "diarizando"].includes(a.fase));
      break;
    case "lancamento.repetir":
      if (arq) { arq.fase = "naFila"; arq.progresso = undefined; moverFila(); }
      break;
    case "lancamento.iniciar": {
      if (reuniao) break;
      const nova = reuniaoDemo(crypto.randomUUID(), "gravando");
      lancamento.reuniao = { ...nova, nome: "Reunião de hoje, " + new Date().toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" }),
        fonte: p.fonte as ReuniaoDemo["fonte"], duracao: 0, criadoEm: Math.floor(Date.now() / 1000) };
      break;
    }
    case "lancamento.pausar":
      if (reuniao && reuniao.fase !== "finalizando") reuniao.fase = reuniao.fase === "pausada" ? "gravando" : "pausada";
      break;
    case "lancamento.encerrar":
      if (reuniao && reuniao.fase !== "finalizando") {
        reuniao.fase = "finalizando";
        const id = reuniao.id;
        window.setTimeout(() => {
          if (lancamento.reuniao?.id !== id) return;
          const encerrada = lancamento.reuniao;
          const final = reuniaoDemo(id, "pronta");
          lancamento.anteriores.unshift({ ...encerrada, fase: "pronta", resumo: final.resumo,
            falantes: final.falantes, vozes: final.vozes, trechos: final.trechos });
          lancamento.reuniao = undefined;
          atualizarLancamento();
        }, 1600);
      }
      break;
    case "lancamento.descartar":
      lancamento.recuperaveis = lancamento.recuperaveis?.filter((r) => r.id !== p.id);
      break;
    case "lancamento.recuperar": {
      const recuperada = lancamento.recuperaveis?.find((r) => r.id === p.id);
      if (recuperada) {
        lancamento.recuperaveis = lancamento.recuperaveis?.filter((r) => r.id !== p.id);
        lancamento.anteriores.unshift({ ...recuperada, ...reuniaoDemo(recuperada.id, "pronta"), pendentes: undefined });
      }
      break;
    }
    case "lancamento.cancelarResumo":
      if (reuniao?.fase === "finalizando") { lancamento.anteriores.unshift({ ...reuniao, fase: "pronta" }); lancamento.reuniao = undefined; }
      break;
    case "lancamento.resumir":
      if (doc && "fonte" in doc) (doc as ReuniaoDemo).resumo = reuniaoDemo(doc.id, "pronta").resumo;
      break;
    case "lancamento.titulo":
      if (doc && String(p.nome).trim()) doc.nome = String(p.nome).trim();
      break;
    case "lancamento.voz":
      if (doc) {
        const nome = String(p.nome).trim();
        const voz = doc.vozes.find((v) => v.id === p.voz);
        const existente = doc.vozes.find((v) => v.id !== p.voz && v.nome.localeCompare(nome, "pt-BR", { sensitivity: "base" }) === 0);
        if (voz && nome && existente) {
          doc.trechos = doc.trechos.map((t) => t.voz === voz.id ? { ...t, voz: existente.id } : t);
          doc.vozes = doc.vozes.filter((v) => v.id !== voz.id);
        } else if (voz && nome) voz.nome = nome;
      }
      break;
    case "lancamento.trecho":
      if (doc) {
        const trecho = doc.trechos.find((t) => t.id === p.trecho);
        if (trecho && typeof p.voz === "string") trecho.voz = p.voz;
        else if (trecho && typeof p.nome === "string" && p.nome.trim()) {
          const nome = p.nome.trim();
          const voz = doc.vozes.find((v) => v.nome === nome) ?? { id: crypto.randomUUID(), nome };
          if (!doc.vozes.some((v) => v.id === voz.id)) doc.vozes.push(voz);
          trecho.voz = voz.id;
        }
      }
      break;
    case "lancamento.permissao":
      if (p.qual === "audioSistema" || p.qual === "microfone") lancamento.permissoes[p.qual] = "concedida";
      break;
    default: return undefined;
  }
  atualizarLancamento();
  return true;
}
if (!(window as unknown as { webkit?: { messageHandlers?: { ditado?: unknown } } }).webkit?.messageHandlers?.ditado) window.setInterval(() => {
  if (lancamento.reuniao?.fase !== "gravando") return;
  lancamento.reuniao.duracao = (lancamento.reuniao.duracao ?? 0) + 1;
  atualizarLancamento();
}, 1000);
