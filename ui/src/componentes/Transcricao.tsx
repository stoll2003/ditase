import { CaretDown, Copy, DownloadSimple, Info, PencilSimple } from "@phosphor-icons/react";
import { useRef, useState } from "react";
import { Button, IconButton } from "@/components/ui/action-button";
import { Menu, type MenuGroup } from "@/components/ui/action-menu";
import { Inset } from "@/components/ui/surface";
import { copiarTexto, exportarDocumento, documentoCompleto, nomeDaVoz, tempo, textoDoDocumento, type Documento, type Formato, type Trecho } from "@/lib/lancamento";
import { executar, acaoLocal } from "@/lib/acoes-lancamento";
import { chamar } from "@/lib/ponte";
import { LinhaAnimada } from "@/componentes/Movimento";
import { cn } from "@/lib/utils";
import { plural } from "@/lib/formato";

export function Copiar({ texto, rotulo = "Copiar" }: { texto: string | (() => Promise<string>); rotulo?: string }) {
  const [copiado, mudar] = useState(false);
  return <Button size="sm" icon={Copy} onClick={() => void executar(async () => {
    await copiarTexto(typeof texto === "string" ? texto : await texto()); mudar(true);
  }, "Transcrição copiada")}>{copiado ? "Copiado" : rotulo}</Button>;
}

export function Exportar({ doc, resumo }: { doc: Documento; resumo?: string[] }) {
  const grupos: MenuGroup[] = [{ items: [
    { id: "copiar", label: resumo ? "Copiar só a transcrição" : "Copiar com falantes e tempos", onSelect: () =>
      void executar(async () => copiarTexto(textoDoDocumento({ ...await documentoCompleto(doc), resumo: undefined } as Documento)), "Transcrição copiada") },
    ...(resumo?.length ? [{ id: "resumo", label: "Copiar só o resumo", onSelect: () => void executar(() => copiarTexto(resumo.join("\n")), "Resumo copiado") }] : []),
    ...(resumo ? [] : [{ id: "corrido", label: "Copiar só o texto corrido", onSelect: () => void executar(async () => copiarTexto(textoDoDocumento(await documentoCompleto(doc), "txt", true)), "Transcrição copiada") }]),
  ] }, { items: ([
    ["txt", "Salvar texto (.txt)"], ["md", "Salvar documento Markdown (.md)"], ["srt", "Salvar legendas SubRip (.srt)"],
    ["vtt", "Salvar legendas WebVTT (.vtt)"],
  ] as [Formato, string][]).map(([formato, label]) => ({ id: formato, label, onSelect: () => void executar(async () => exportarDocumento(await documentoCompleto(doc), formato)) })) }];
  return <Menu groups={grupos} trigger={<Button size="sm" icon={DownloadSimple}>Exportar</Button>} />;
}

export function EditarNome({ nome, rotulo, salvar, classe, editarAoMontar = false, aoFechar }: {
  nome: string; rotulo: string; salvar: (nome: string) => void; classe?: string; editarAoMontar?: boolean; aoFechar?: () => void;
}) {
  const [aberto, abrir] = useState(editarAoMontar);
  const [valor, mudar] = useState(nome);
  const ignorar = useRef(false);
  function fechar() {
    if (!ignorar.current && valor.trim() && valor.trim() !== nome) salvar(valor.trim());
    abrir(false); aoFechar?.();
  }
  return aberto ? <input aria-label={rotulo} autoFocus maxLength={100} value={valor}
    onFocus={(event) => event.target.select()} onChange={(event) => mudar(event.target.value)} onBlur={fechar}
    className="min-w-0 max-w-full rounded-lg bg-surface px-2 py-1 text-[0.8125rem] ring-2 ring-ring outline-none"
    onKeyDown={(event) => {
      if (event.key === "Escape") { ignorar.current = true; fechar(); }
      if (event.key === "Enter") event.currentTarget.blur();
    }} />
    : <button type="button" aria-label={rotulo} title={rotulo} className={cn("rounded-md text-left focus-visible:ring-2 focus-visible:ring-ring", classe)}
      onClick={() => { ignorar.current = false; mudar(nome); abrir(true); }}>{nome}</button>;
}

export function Transcricao({ doc, trechos = doc.trechos, aoVivo = false, aoEditar, filhosAbaixo }: {
  doc: Documento; trechos?: Trecho[]; aoVivo?: boolean; aoEditar?: () => void; filhosAbaixo?: React.ReactNode;
}) {
  const [editando, editar] = useState("");
  const [novaPessoa, criarPessoa] = useState("");
  const cor = (id: string) => {
    const indice = doc.vozes.findIndex((v) => v.id === id);
    return indice < 0 || indice > 4 ? 5 : indice;
  };
  const nomeNovo = (voz: string, nome: string) => void executar(() => chamar("falante.renomear", { id: doc.id, voz, nome }), `Renomeado para “${nome}”`);
  const editarVoz = (voz: string) => { aoEditar?.(); editar(voz); };
  return <>
    {doc.baixaConfianca && <Inset className="mb-3 flex gap-2 bg-warning/[0.08] px-3.5 py-2.5 text-[0.8125rem]">
      <Info className="mt-0.5 size-4 shrink-0 text-warning" />
      {aoVivo ? "Não foi possível separar bem as vozes em alguns trechos. Ao encerrar, o Dita.se fará uma revisão final da gravação."
        : "Não foi possível separar bem as vozes nesta gravação (vozes sobrepostas ou áudio distante). Clique no nome acima de qualquer trecho para indicar quem falou."}
    </Inset>}
    {doc.vozes.length > 0 && <div className="mb-3 flex flex-wrap items-center gap-1.5 border-b border-foreground/[0.06] pb-3">
      <span className="mr-1 text-xs font-medium text-muted-foreground">{doc.vozes.length === 1 ? "1 voz detectada" : "Vozes:"}</span>
      {doc.vozes.map((voz) => editando === voz.id
        ? <EditarNome key={voz.id} editarAoMontar nome={voz.nome} rotulo={`Renomear ${voz.nome}`} salvar={(nome) => nomeNovo(voz.id, nome)} aoFechar={() => editar("")} />
        : <button key={voz.id} type="button" className="voz-chip h-7 px-2.5" data-voz={cor(voz.id)}
          aria-label={`Renomear ${voz.nome}`} onClick={() => editarVoz(voz.id)}>
          <span className="voz-ponto" />{voz.nome}<span className="font-normal text-muted-foreground">{plural(doc.trechos.filter((t) => t.voz === voz.id).length, "fala", "falas")}</span>
          <PencilSimple className="size-3 opacity-60" />
        </button>)}
    </div>}
    <ol className="grid gap-1">
      {trechos.map((trecho) => <LinhaAnimada key={trecho.id} entrada={aoVivo ? "nova" : "nenhuma"}><div className="group rounded-xl px-3 py-2.5 hover:bg-foreground/[0.025]">
        <div className="mb-1 flex items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            {novaPessoa === trecho.id ? <EditarNome nome="" editarAoMontar rotulo="Nova pessoa" aoFechar={() => criarPessoa("")}
              salvar={(nome) => void acaoLocal("trecho", { id: doc.id, trecho: trecho.id, nome })} />
              : <Menu trigger={<button className="voz-chip min-h-6 px-2" data-voz={cor(trecho.voz)} type="button"
              aria-label={`Falante do trecho: ${nomeDaVoz(doc, trecho.voz)}`}><span className="voz-ponto" />{nomeDaVoz(doc, trecho.voz)}<CaretDown className="size-3" /></button>}
              groups={[{ label: "Mudar só este trecho para:", items: doc.vozes.map((voz) => ({
                id: voz.id, label: voz.nome, onSelect: () => void acaoLocal("trecho", { id: doc.id, trecho: trecho.id, voz: voz.id }),
              })) }, { items: [
                { id: "nova", label: "Nova pessoa…", onSelect: () => { aoEditar?.(); criarPessoa(trecho.id); } },
                { id: "renomear", label: "Renomear todas as falas", disabled: !doc.vozes.some((v) => v.id === trecho.voz), onSelect: () => editarVoz(trecho.voz) },
              ] }]} />}
            <span className="font-mono text-[0.6875rem] tabular-nums text-muted-foreground">{tempo(trecho.inicio)}</span>
          </div>
          <IconButton size="sm" icon={Copy} label="Copiar este trecho" tooltip="Copiar trecho"
            className="opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
            onClick={() => void executar(() => copiarTexto(trecho.texto), "Trecho copiado")} />
        </div>
        <p className={cn("selecionavel whitespace-pre-wrap text-sm leading-6", trecho.parcial && "text-muted-foreground")}>{trecho.texto}</p>
      </div></LinhaAnimada>)}
    </ol>
    {filhosAbaixo}
  </>;
}
