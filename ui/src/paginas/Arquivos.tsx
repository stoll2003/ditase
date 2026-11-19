import { useDados } from "@/lib/dados";
import { chamar } from "@/lib/ponte";
import { ArrowClockwise, CaretDown, CaretUp, DotsThree, FileAudio, FileVideo, FolderOpen, Trash, X } from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";
import { Cabecalho } from "@/componentes/Cabecalho";
import { Copiar, Exportar, Transcricao } from "@/componentes/Transcricao";
import { acaoLocal, executar } from "@/lib/acoes-lancamento";
import { Button, IconButton } from "@/components/ui/action-button";
import { Menu } from "@/components/ui/action-menu";
import { ProgressBar } from "@/components/ui/progress-bar";
import { StatusPill, type StatusTone } from "@/components/ui/status-pill";
import { IconTile, Panel, Shell, ShellHeader } from "@/components/ui/surface";
import { SearchField } from "@/components/ui/text-field";
import { arquivoEmCurso, blocos, documentoCompleto, extensao, tempo, textoDoDocumento, type Arquivo, type FaseArquivo } from "@/lib/lancamento";
import { cn } from "@/lib/utils";
import { plural } from "@/lib/formato";

const fases: Record<FaseArquivo, [string, string, StatusTone]> = {
  naFila: ["Na fila", "Aguardando o arquivo anterior terminar", "neutral"],
  decodificando: ["Lendo o áudio", "Convertendo o arquivo para 16 kHz mono", "brand"],
  transcrevendo: ["Transcrevendo", "Reconhecendo a fala no Parakeet Ultra", "brand"],
  diarizando: ["Separando vozes", "Identificando quem falou em cada trecho", "brand"],
  pronto: ["Pronto", "", "positive"],
  formatoNaoSuportado: ["Formato não suportado", "O macOS não conseguiu abrir este formato. Use mp3, m4a, wav, aac, flac, mp4 ou mov.", "warning"],
  semFala: ["Sem fala detectada", "O áudio foi lido até o fim, mas o reconhecedor não encontrou voz audível.", "neutral"],
  falhou: ["Falhou", "Não foi possível concluir a transcrição deste arquivo.", "negative"],
};
function ArquivoCard({ arquivo, aberto, alternar }: { arquivo: Arquivo; aberto: boolean; alternar: () => void }) {
  const [bruto, mostrarBruto] = useState(false);
  const props = { arquivo, aberto, alternar, bruto, mostrarBruto };
  return arquivo.fase === "pronto" && aberto ? <ArquivoAberto {...props} /> : <ConteudoArquivo {...props} />;
}

type Cartao = { arquivo: Arquivo; aberto: boolean; alternar: () => void; bruto: boolean; mostrarBruto: (valor: boolean) => void };

function ArquivoAberto(props: Cartao) {
  const { dados } = useDados<Arquivo | null>("lancamento.documento", { id: props.arquivo.id }, ["lancamento"]);
  return <ConteudoArquivo {...props} arquivo={dados ?? props.arquivo} />;
}

function ConteudoArquivo({ arquivo, aberto, alternar, bruto, mostrarBruto }: Cartao) {
  const [titulo, descricao, tom] = fases[arquivo.fase];
  const pronto = arquivo.fase === "pronto";
  const trabalhando = arquivoEmCurso(arquivo);
  const fila = arquivo.fase === "naFila";
  const apagar = () => void acaoLocal("remover", { id: arquivo.id });
  const parar = () => void executar(() => chamar("arquivo.cancelar", { id: arquivo.id }));
  const repetir = () => void acaoLocal("repetir", { id: arquivo.id });
  const palavras = arquivo.trechos.length > 0
    ? arquivo.trechos.reduce((n, trecho) => n + trecho.texto.trim().split(/\s+/).filter(Boolean).length, 0)
    : arquivo.palavras ?? 0;
  const meta = [new Date(arquivo.criadoEm * 1000).toLocaleDateString("pt-BR", { day: "numeric", month: "short" }),
    arquivo.duracao != null && tempo(arquivo.duracao), plural(arquivo.vozes.length, "voz", "vozes"), palavras > 0 && plural(palavras, "palavra", "palavras"),
    arquivo.processamento != null && `pronto em ${arquivo.processamento} s`].filter(Boolean).join(" · ");
  const percentual = arquivo.progresso == null ? undefined : Math.round(arquivo.progresso * 100);
  const nomeCurto = arquivo.nome.length > 48 ? arquivo.nome.slice(0, 27) + "…" + arquivo.nome.slice(-20) : arquivo.nome;
  return <Shell id={`arquivo-${arquivo.id}`} aria-label={arquivo.nome} tabIndex={-1}>
    <ShellHeader icon={["mp4", "mov", "m4v", "webm"].includes(extensao(arquivo.nome)) ? FileVideo : FileAudio}
      className="flex-wrap" title={<span title={arquivo.nome}>{nomeCurto}</span>} subtitle={pronto ? meta : [arquivo.duracao != null && tempo(arquivo.duracao), descricao, arquivo.motivo].filter(Boolean).join(" · ")}
      aside={pronto ? <>
        <Copiar texto={async () => textoDoDocumento(await documentoCompleto(arquivo))} /><Exportar doc={arquivo} />
        <IconButton size="sm" icon={aberto ? CaretUp : CaretDown} label={aberto ? "Recolher transcrição" : "Mostrar transcrição"} aria-expanded={aberto} onClick={alternar} />
        <Menu trigger={<IconButton size="sm" icon={DotsThree} label="Mais ações" />} groups={[{ items: [
          { id: "repetir", label: "Transcrever de novo", onSelect: repetir },
          { id: "bruto", label: "Ver o que foi ouvido", onSelect: () => { mostrarBruto(true); if (!aberto) alternar(); } },
          { id: "apagar", label: "Apagar da lista", destructive: true, onSelect: apagar },
        ] }]} />
      </> : <>
        <StatusPill tone={tom}>{trabalhando && percentual != null ? `${titulo} · ${percentual}%` : titulo}</StatusPill>
        {(arquivo.fase === "falhou" || arquivo.fase === "semFala") && <Button size="sm" variant="quiet" icon={ArrowClockwise} onClick={repetir}>{arquivo.fase === "falhou" ? "Tentar de novo" : "Transcrever de novo"}</Button>}
        <IconButton size="sm" icon={fila || trabalhando ? X : Trash} label={fila ? "Tirar da fila" : trabalhando ? "Cancelar este arquivo" : "Remover da lista"} onClick={fila || trabalhando ? parar : apagar} />
      </>} />
    {trabalhando && percentual != null && <div className="px-3 pb-2.5"><ProgressBar label="Progresso do arquivo" value={percentual} size="sm" /></div>}
    {pronto && aberto && <Panel className="p-4">
      {bruto && arquivo.bruto ? <><Button size="sm" variant="quiet" onClick={() => mostrarBruto(false)}>Ver transcrição</Button><p className="selecionavel whitespace-pre-wrap px-3 py-3 text-sm">{arquivo.bruto}</p></>
        : <Transcricao doc={arquivo} trechos={arquivo.baixaConfianca ? arquivo.trechos : blocos(arquivo.trechos)} />}
    </Panel>}
  </Shell>;
}

export function Arquivos({ arquivos, selecionar, arrastando, destaque, disponivel }: {
  arquivos: Arquivo[]; selecionar: () => void; arrastando: boolean; destaque?: string; disponivel: boolean;
}) {
  const [abertos, atualizar] = useState(() => new Set<string>());
  const [busca, pesquisar] = useState("");
  const fila = arquivos.filter((a) => arquivoEmCurso(a) || a.fase === "naFila");
  const primeiroPronto = arquivos.find((a) => a.fase === "pronto")?.id;
  const [recolhidos, recolher] = useState(() => new Set<string>());
  const focado = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (destaque && focado.current !== destaque && arquivos.some((a) => a.id === destaque)) {
      document.getElementById(`arquivo-${destaque}`)?.focus();
      focado.current = destaque;
    }
  }, [destaque, arquivos]);
  const termo = busca.trim().toLocaleLowerCase("pt-BR");
  const { dados: encontrados } = useDados<string[]>("lancamento.buscarArquivos", { termo }, ["lancamento"]);
  const filtrados = arquivos.filter((a) => !termo || a.nome.toLocaleLowerCase("pt-BR").includes(termo) || encontrados?.includes(a.id));
  return <>
    <Cabecalho titulo="Arquivos" apoio="Transcreva gravações de áudio ou vídeo e separe quem falou em cada trecho. Tudo roda neste Mac."
      lado={<Button icon={FolderOpen} variant="primary" onClick={selecionar} disabled={!disponivel}>Escolher arquivo</Button>} />
    {!disponivel && <p role="status" className="mb-4 text-sm text-muted-foreground">A transcrição de arquivos está indisponível nesta versão do motor.</p>}
    <button type="button" onClick={selecionar} disabled={!disponivel} className={cn(
      "mb-6 flex w-full items-center gap-3 rounded-xl border border-dashed border-foreground/15 bg-foreground/[0.02] px-4 text-left hover:border-foreground/25 focus-visible:ring-2 focus-visible:ring-ring",
      arquivos.length ? "min-h-13 justify-between py-2" : "min-h-54 flex-col justify-center p-8 text-center",
      arrastando && "border-brand bg-brand/[0.06]",
    )}>
      {arquivos.length ? <><span className="flex items-center gap-2 text-[0.8125rem] font-medium"><FileAudio className="size-4 text-muted-foreground" />Arraste mais arquivos para cá para pôr na fila</span>
        <span className="hidden text-xs text-muted-foreground lg:inline">mp3 · m4a · wav · flac · mp4 · mov</span></>
        : <><IconTile icon={FileAudio} className="size-10 text-brand" /><span className="text-[0.9375rem] font-medium">Arraste arquivos de áudio ou vídeo para cá</span>
          <span className="max-w-md text-[0.8125rem] leading-5 text-muted-foreground">Aceita mp3, m4a, wav, aac, flac, ogg, mp4, mov, m4v e webm. Você pode soltar vários de uma vez ou usar Escolher arquivo.</span></>}
    </button>
    {arquivos.filter((a) => a.fase === "pronto").length >= 2 && <SearchField aria-label="Buscar nos arquivos" placeholder="Buscar nos arquivos" value={busca} onChange={(e) => pesquisar(e.target.value)} onClear={() => pesquisar("")} className="mb-3" />}
    {fila.length >= 2 && <div className="mb-2 flex items-center justify-between px-1 text-xs text-muted-foreground"><span>Processando 1 de {fila.length} arquivos</span>
      <Button size="sm" variant="quiet" onClick={() => void acaoLocal("cancelarFila")}>Cancelar fila</Button></div>}
    <div className="grid gap-2.5">
      {filtrados.map((arquivo) => {
        const aberto = !recolhidos.has(arquivo.id) && (abertos.has(arquivo.id) || arquivo.id === primeiroPronto || arquivo.id === destaque);
        return <ArquivoCard key={arquivo.id} arquivo={arquivo} aberto={aberto} alternar={() => {
          if (aberto) recolher(new Set([...recolhidos, arquivo.id]));
          else { recolher(new Set([...recolhidos].filter((id) => id !== arquivo.id))); atualizar(new Set([...abertos, arquivo.id])); }
        }} />;
      })}
      {termo && !filtrados.length && <p className="py-8 text-center text-sm text-muted-foreground">Nenhum arquivo encontrado.</p>}
    </div>
  </>;
}
