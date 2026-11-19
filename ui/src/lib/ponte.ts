/**
 * Conversa com o app nativo. Dentro do Ditado, cada chamada vai para o Swift por
 * `window.webkit.messageHandlers.ditado`. Fora dele (navegador, durante o desenvolvimento)
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

function nativo(): Nativo | undefined {
  return (window as unknown as { webkit?: { messageHandlers?: { ditado?: Nativo } } }).webkit?.messageHandlers
    ?.ditado;
}

export const dentroDoApp = () => nativo() !== undefined;

export async function chamar<T>(metodo: string, params: Record<string, unknown> = {}): Promise<T> {
  const canal = nativo();
  if (!canal) return (/^(lancamento|arquivo|reuniao|falante)\./.test(metodo) ? responderLancamento(metodo, params)
    : responderDeExemplo(metodo, params)) as T;
  return JSON.parse(await canal.postMessage({ metodo, params })) as T;
}

/** Chama `aoMudar` quando o app avisa que o assunto mudou. Devolve a função que cancela. */
export function ouvir(assuntos: string[], aoMudar: () => void): () => void {
  const tratar = (evento: Event) => {
    if (assuntos.includes((evento as CustomEvent<string>).detail)) aoMudar();
  };
  window.addEventListener("ditado", tratar);
  return () => window.removeEventListener("ditado", tratar);
}
