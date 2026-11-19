import { chamar, dentroDoApp } from "./ponte";

export type Trecho = { id: string; inicio: number; fim: number; voz: string; texto: string; parcial?: boolean };
export type Voz = { id: string; nome: string };
export type Documento = { id: string; nome: string; criadoEm: number; duracao?: number; vozes: Voz[]; trechos: Trecho[]; baixaConfianca?: boolean; bruto?: string;
  /** Só nas listas leves, quando os trechos não vêm. */
  palavras?: number; previa?: string | null };
export type FaseArquivo = "naFila" | "decodificando" | "transcrevendo" | "diarizando" | "pronto" | "formatoNaoSuportado" | "semFala" | "falhou";
export type Arquivo = Documento & { fase: FaseArquivo; progresso?: number; motivo?: string; processamento?: number };
export type FonteReuniao = "macEMicrofone" | "mac";
export type PeriodoDeAudio = { inicio: number; fim: number; fonte: "mic" | "mac"; motivo: string };
export type Reuniao = Documento & {
  fase: "gravando" | "pausada" | "finalizando" | "pronta" | "recuperavel" | "interrompida";
  falantes?: "pendente" | "identificados" | "indisponivel";
  etapa?: string; aviso?: string; pendentes?: PeriodoDeAudio[]; lacunas?: PeriodoDeAudio[];
  fonte: FonteReuniao; erroCaptura?: string; erroProcessamento?: string; avisoCaptura?: string; erroResumo?: string; resumoCancelado?: boolean; atraso?: number; progresso?: number; resumo?: string[]; resumoGerado?: boolean; oculto?: boolean;
};
export type Lancamento = {
  arquivos: Arquivo[]; reuniao?: Reuniao; anteriores: Reuniao[]; recuperaveis?: Reuniao[];
  permissoes: { audioSistema: string; microfone: string }; disponivel: boolean; avisoMigracao?: string;
};
export type IntervalosReuniao = Pick<Reuniao, "id" | "lacunas" | "pendentes">;
export type LoteReuniao = Pick<Reuniao, "id" | "trechos" | "vozes" | "duracao" | "atraso">;

export function aplicarLote(estado: Lancamento | undefined, lote: LoteReuniao): Lancamento | undefined {
  const reuniao = estado?.reuniao;
  if (!estado || !reuniao || reuniao.id !== lote.id) return estado;
  const recebidos = new Set(reuniao.trechos.map((trecho) => trecho.id));
  const novos = lote.trechos.filter((trecho) => !recebidos.has(trecho.id));
  if (!novos.length) return estado;
  return { ...estado, reuniao: { ...reuniao, vozes: lote.vozes, duracao: Math.max(reuniao.duracao ?? 0, lote.duracao ?? 0), atraso: lote.atraso,
    trechos: [...reuniao.trechos, ...novos].sort((a, b) => a.inicio - b.inicio) } };
}

export function mesclarLancamento(novo: Lancamento, anterior?: Lancamento): Lancamento {
  const atual = novo.reuniao;
  const vivo = anterior?.reuniao;
  if (atual && vivo && atual.id === vivo.id && atual.fase === vivo.fase && atual.fase !== "finalizando") {
    const ids = new Set(atual.trechos.map((trecho) => trecho.id));
    const novos = vivo.trechos.filter((trecho) => !ids.has(trecho.id));
    const vozes = new Map(vivo.vozes.map((voz) => [voz.id, voz]));
    for (const voz of atual.vozes) vozes.set(voz.id, voz);
    return { ...novo, reuniao: { ...atual, duracao: Math.max(vivo.duracao ?? 0, atual.duracao ?? 0), atraso: atual.atraso ?? vivo.atraso,
      vozes: [...vozes.values()], trechos: [...atual.trechos, ...novos].sort((a, b) => a.inicio - b.inicio) } };
  }
  return novo;
}

export async function documentoCompleto<T extends Documento>(doc: T): Promise<T> {
  return dentroDoApp() ? (await chamar<T | null>("lancamento.documento", { id: doc.id })) ?? doc : doc;
}

export type Formato = "txt" | "md" | "srt" | "vtt";
export const extensoes = "mp3,m4a,wav,aac,flac,ogg,mp4,mov,m4v,webm".split(",");
export const aceite = extensoes.map((ext) => "." + ext).join(",");
export const extensao = (nome: string) => nome.split(".").pop()?.toLowerCase() ?? "";
export const arquivoEmCurso = (arquivo: Arquivo) => ["decodificando", "transcrevendo", "diarizando"].includes(arquivo.fase);
export const tempo = (segundos: number) => {
  const total = Math.max(0, Math.floor(segundos));
  const minutos = Math.floor(total / 60);
  const s = String(total % 60).padStart(2, "0");
  return minutos >= 60 ? `${Math.floor(minutos / 60)}:${String(minutos % 60).padStart(2, "0")}:${s}` : `${String(minutos).padStart(2, "0")}:${s}`;
};
export const nomeDaVoz = (doc: Documento, id: string) => doc.vozes.find((v) => v.id === id)?.nome ?? "Voz não identificada";
export function blocos(trechos: Trecho[]): Trecho[] {
  const resultado: Trecho[] = [];
  for (const trecho of trechos) {
    const anterior = resultado.at(-1);
    if (anterior && !anterior.parcial && !trecho.parcial && anterior.voz === trecho.voz && trecho.inicio - anterior.fim < 8 && trecho.inicio >= anterior.inicio) {
      resultado[resultado.length - 1] = { ...anterior, fim: trecho.fim, texto: anterior.texto + " " + trecho.texto };
    } else resultado.push(trecho);
  }
  return resultado;
}
const carimbo = (s: number, separador: string) => {
  const ms = Math.max(0, Math.round(s * 1000));
  return [Math.floor(ms / 3_600_000), Math.floor(ms / 60_000) % 60, Math.floor(ms / 1000) % 60]
    .map((n) => String(n).padStart(2, "0")).join(":") + separador + String(ms % 1000).padStart(3, "0");
};
const escapar = (s: string) => s.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
export function textoDoDocumento(doc: Documento, formato: Formato = "txt", corrido = false): string {
  const resumo = "resumo" in doc ? (doc as Reuniao).resumo : undefined;
  if (formato === "srt" || formato === "vtt") {
    const vtt = formato === "vtt";
    return (vtt ? "WEBVTT\n\n" : "") + doc.trechos.filter((t) => t.fim > t.inicio && !t.parcial).map((t, i) => {
      const nome = nomeDaVoz(doc, t.voz);
      const texto = t.texto.replace(/\n{2,}/g, "\n");
      return `${vtt ? "" : i + 1 + "\n"}${carimbo(t.inicio, vtt ? "." : ",")} --> ${carimbo(t.fim, vtt ? "." : ",")}\n${vtt ? `<v ${escapar(nome)}>${escapar(texto)}` : (doc.vozes.length > 1 ? `[${nome}] ` : "") + texto}`;
    }).join("\n\n") + "\n";
  }
  const corpo = blocos(doc.trechos).map((t) => {
    const rotulo = `[${tempo(t.inicio)}] ${nomeDaVoz(doc, t.voz)}`;
    return corrido ? t.texto : formato === "md" ? `**${rotulo}:** ${t.texto}` : `${rotulo}\n${t.texto}`;
  }).join("\n\n");
  if (corrido) return corpo;
  const cabecalho = formato === "md" ? `# ${doc.nome}\n\n${new Date(doc.criadoEm * 1000).toLocaleString("pt-BR")} · ${tempo(doc.duracao ?? 0)}\n\n` : "";
  const blocoResumo = resumo?.length ? "Resumo gerado localmente pelo Qwen\n\n" + resumo.map((item) => "- " + item).join("\n") + "\n\nTranscrição\n\n" : "";
  return cabecalho + blocoResumo + corpo;
}
export async function copiarTexto(texto: string) {
  if (dentroDoApp()) { await chamar("copiar", { texto }); return; }
  await navigator.clipboard.writeText(texto);
}
export async function exportarDocumento(doc: Documento, formato: Formato) {
  const texto = textoDoDocumento(doc, formato);
  // O nome vai ao diálogo nativo ou ao atributo download, sem controles do sistema de arquivos.
  // oxlint-disable-next-line eslint/no-control-regex
  const nome = doc.nome.replace(/\.[a-z0-9]{1,6}$/i, "").replace(/[/\\:\x00-\x1f]/g, "_") + "." + formato;
  if (dentroDoApp()) { await chamar("lancamento.exportar", { nome, formato, texto }); return; }
  const url = URL.createObjectURL(new Blob([texto], { type: "text/plain;charset=utf-8" }));
  const link = document.createElement("a");
  link.href = url; link.download = nome; link.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}
