/**
 * Conversa com o app nativo. No Mac, cada chamada vai para o Swift por
 * `window.webkit.messageHandlers.ditado`; no Windows, para o Rust pelo `invoke` do Tauri. Fora do app (navegador)
 * as respostas vêm de dados de EXEMPLO em `exemplo.ts`.
 */
import { responderDeExemplo, responderLancamento } from "./exemplo";

export type Ditado = {
  id: number;
  iniciadoEm: number;
  duracao: number;
  duracaoFala: number;
  palavras: number;
  textoBruto: string;
  texto: string;
  app: string;
  appNome: string;
  estado: string;
  latencia: number;
  motor: string;
  limpador: string;
  trocasDicionario: number;
  origem: string;
  audio: string;
};

export type Termo = {
  id: number;
  termo: string;
  ouvidoComo: string[];
  usos: number;
  origem: string;
  criadoEm: number;
};

export type Estatisticas = {
  totalPalavras: number;
  totalDitados: number;
  minutosFalados: number;
  ppmMedio: number;
  ppmDeCadaDitado: number[];
  minutosEconomizados: number;
  diasSeguidos: number;
  maiorSequencia: number;
  diasAtivos: number;
  palavrasHoje: number;
  palavrasNaSemana: number;
  palavrasNaSemanaAnterior: number;
  maiorDitadoPalavras: number;
  maiorDitadoMinutos: number;
  latenciaMediana: number;
  trocasDicionario: number;
  porDia: { dia: string; palavras: number; ditados: number; minutos: number }[];
  porHora: number[];
  porDiaDaSemana: number[];
  porApp: { app: string; nome: string; palavras: number; ditados: number }[];
};

export type Estado = {
  fase: "parado" | "gravando" | "processando";
  maosLivres: boolean;
  reconhecedor: { estado: "parado" | "carregando" | "pronto" | "falhou"; detalhe: string; nome: string };
  limpador: {
    estado: "desligado" | "iniciando" | "pronto" | "semBinario" | "semModelo" | "falhou";
    detalhe: string;
    ligado: boolean;
    modelo: string;
  };
  /** Modelo padrão da IA local: download sob demanda. `feito` e `tamanho` em bytes. */
  modeloIA: {
    estado: "indisponivel" | "ausente" | "baixando" | "conferindo" | "instalado" | "falhou";
    arquivo: string;
    tamanho: number;
    feito?: number;
    detalhe?: string;
    /** Falso depois que a pessoa dispensa o aviso do Início. */
    aviso: boolean;
  };
  permissoes: { microfone: "concedida" | "pendente" | "negada"; acessibilidade: "concedida" | "pendente" };
  atalho: { tecla: string; nome: string; simbolo: string; instalado: boolean };
  wisprAberto: boolean;
  wisprDisponivel: boolean;
  versao: string;
};

export type Ajustes = {
  valores: Record<string, string>;
  teclas: { id: string; nome: string }[];
  microfones: { uid: string; nome: string }[];
  modelos: string[];
  pastaDeModelos: string;
  llamaInstalado: boolean;
};

type Nativo = { postMessage(mensagem: unknown): Promise<string> };
type Invocar = (comando: string, argumentos: Record<string, unknown>) => Promise<unknown>;

/**
 * Windows: o app é Tauri e a chamada vai pelo comando `ponte` (mesmo contrato, mesma resposta
 * em texto JSON). Os avisos chegam pelo mesmo `CustomEvent` do Mac, então `ouvir` não muda.
 */
function tauri(): Nativo | undefined {
  const w = window as unknown as {
    __TAURI__?: { core?: { invoke?: Invocar } };
    __TAURI_INTERNALS__?: { invoke?: Invocar };
  };
  const invoke = w.__TAURI__?.core?.invoke ?? w.__TAURI_INTERNALS__?.invoke;
  if (!invoke) return undefined;
  return {
    postMessage: async (mensagem) => {
      const { metodo, params } = mensagem as { metodo: string; params: Record<string, unknown> };
      return String(await invoke("ponte", { metodo, params }));
    },
  };
}

type Escutar = (evento: string, f: (e: { payload: unknown }) => void) => Promise<unknown>;

/** Eventos do app (mesma lista de `EVENTOS` em `windows/src-tauri/src/ponte.rs`). */
const EVENTOS = ["ditado", "ditado:progresso", "ditado:trechos", "ditado:resultado", "ditado:intervalos", "ditado:arquivos-soltos"];
let ligacaoDoDitado: Promise<void> | undefined;
let ditadoLigado = false;

/**
 * No Windows o app manda eventos estruturados do Tauri, só para a janela principal, em vez de
 * avaliar JavaScript na página. Aqui eles viram o mesmo `CustomEvent` que o Mac dispara, então o
 * resto da interface não muda.
 */
function ligarEventosDoTauri() {
  if (ligacaoDoDitado) return ligacaoDoDitado;
  const escutar = (window as unknown as { __TAURI__?: { event?: { listen?: Escutar } } }).__TAURI__?.event?.listen;
  if (!escutar) return;
  const ligar = (nome: string) => escutar(nome, (e) => window.dispatchEvent(new CustomEvent(nome, { detail: e.payload })));
  ligacaoDoDitado = ligar("ditado").then(() => { ditadoLigado = true; });
  void ligacaoDoDitado.catch(() => console.error("Falha ao assinar atualizações do Dita.se."));
  for (const nome of EVENTOS.slice(1)) {
    void ligar(nome).catch(() => console.error("Falha ao assinar o evento " + nome));
  }
  return ligacaoDoDitado;
}

function nativo(): Nativo | undefined {
  return (window as unknown as { webkit?: { messageHandlers?: { ditado?: Nativo } } }).webkit?.messageHandlers
    ?.ditado ?? tauri();
}

/** Plataforma do app nativo, para textos e atalhos que mudam no Windows. */
export const plataforma = (): "mac" | "windows" | "navegador" =>
  (window as unknown as { webkit?: { messageHandlers?: { ditado?: unknown } } }).webkit?.messageHandlers?.ditado
    ? "mac" : tauri() ? "windows" : "navegador";

export const dentroDoApp = () => nativo() !== undefined;

export async function chamar<T>(metodo: string, params: Record<string, unknown> = {}): Promise<T> {
  const canal = nativo();
  if (!canal) return (/^(lancamento|arquivo|reuniao|falante)\./.test(metodo) ? responderLancamento(metodo, params)
    : responderDeExemplo(metodo, params)) as T;
  return JSON.parse(await canal.postMessage({ metodo, params })) as T;
}

/** Chama `aoMudar` quando o app avisa que o assunto mudou. Devolve a função que cancela. */
export function ouvir(assuntos: string[], aoMudar: () => void): () => void {
  let ativo = true;
  const ligacao = tauri() ? ligarEventosDoTauri() : undefined;
  // O primeiro snapshot pode ser anterior ao registro assíncrono do listener no host.
  if (!ditadoLigado && ligacao && assuntos.length) {
    void ligacao.then(() => { if (ativo) aoMudar(); }, () => {});
  }
  const tratar = (evento: Event) => {
    if (assuntos.includes((evento as CustomEvent<string>).detail)) aoMudar();
  };
  window.addEventListener("ditado", tratar);
  return () => {
    ativo = false;
    window.removeEventListener("ditado", tratar);
  };
}

// App.tsx escuta alguns eventos direto na janela: liga a ponte de eventos já ao carregar.
if (typeof window !== "undefined" && tauri()) ligarEventosDoTauri();
