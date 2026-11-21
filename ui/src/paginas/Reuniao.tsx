import { useDados } from "@/lib/dados";
import { ArrowDown, Clock, Copy, DotsThree, Microphone, Pause, Play, Stop, UsersThree, Warning } from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";
import { useReducedMotion } from "motion/react";
import { Cabecalho, Secao } from "@/componentes/Cabecalho";
import { ErroDeLeitura } from "@/componentes/ErroDeLeitura";
import { Copiar, EditarNome, Exportar, Transcricao } from "@/componentes/Transcricao";
import { acaoLocal, executar } from "@/lib/acoes-lancamento";
import { chamar } from "@/lib/ponte";
import { toast } from "@/components/ui/toast-stack";
import { Button, IconButton } from "@/components/ui/action-button";
import { Dialog } from "@/components/ui/modal-sheet";
import { Menu } from "@/components/ui/action-menu";
import { ProgressBar } from "@/components/ui/progress-bar";
import { StatusPill, Tag } from "@/components/ui/status-pill";
import { Inset, Panel, Shell, ShellHeader } from "@/components/ui/surface";
import { SearchField } from "@/components/ui/text-field";
import { blocos, copiarTexto, tempo, textoDoDocumento, type FonteReuniao, type Lancamento, type Reuniao as DadosReuniao } from "@/lib/lancamento";
import { cn } from "@/lib/utils";
import { plural } from "@/lib/formato";
import { plataforma } from "@/lib/plataforma";

const fonteTexto = (fonte: FonteReuniao, win: boolean) =>
  fonte === "mac"
    ? (win ? "Só o áudio do computador" : "Só o áudio do Mac")
    : (win ? "Áudio do computador + microfone (Você)" : "Áudio do Mac + microfone (Você)");
const dia = (segundos: number) => {
  const data = new Date(segundos * 1000);
  const hoje = new Date();
  const ontem = new Date(); ontem.setDate(hoje.getDate() - 1);
  if (data.toDateString() === hoje.toDateString()) return "Hoje";
  if (data.toDateString() === ontem.toDateString()) return "Ontem";
  return data.toLocaleDateString("pt-BR", { day: "numeric", month: "short" });
};
const mudarTitulo = (reuniao: DadosReuniao, nome: string) => void acaoLocal("titulo", { id: reuniao.id, nome });
function AvisosDeAudio({ reuniao }: { reuniao: DadosReuniao }) {
  const avisos = [
    ...(reuniao.lacunas ?? []).map((faixa) => ({ ...faixa, estado: "Lacuna de áudio" })),
    ...(reuniao.pendentes ?? []).map((faixa) => ({ ...faixa, estado: "Transcrição pendente" })),
  ];
  return <>{reuniao.aviso && <p className="mb-2 text-sm text-warning">{reuniao.aviso}</p>}
    {avisos.length > 0 && <details className="mb-3 text-sm">
      <summary className="cursor-pointer text-warning">{plural(avisos.length, "intervalo com aviso", "intervalos com avisos")}</summary>
      <ul className="mt-1 space-y-1 pl-3 text-muted-foreground">
        {avisos.map((a, i) => <li key={i}>{a.estado} · {a.fonte === "mic" ? "Você" : "Chamada"} · {tempo(a.inicio)} a {tempo(a.fim)}. {a.motivo}</li>)}
      </ul>
    </details>}
  </>;
}

function AoVivo({ reuniao }: { reuniao: DadosReuniao }) {
  const painel = useRef<HTMLDivElement>(null);
  const [acompanhar, mudarAcompanhar] = useState(true);
  const reduzido = useReducedMotion();
  const pausada = reuniao.fase === "pausada";
  const ultimo = reuniao.trechos.at(-1);
  const ponto = pausada || reuniao.avisoCaptura ? "bg-warning" : "bg-negative motion-safe:animate-pulse";
  useEffect(() => {
    if (acompanhar && painel.current) painel.current.scrollTo({ top: painel.current.scrollHeight, behavior: reduzido ? "instant" : "smooth" });
  }, [acompanhar, ultimo?.id, ultimo?.texto, reduzido]);
  return <Shell>
    <ShellHeader className="flex-wrap" title={<span className="flex flex-wrap items-center gap-2">
      <span aria-hidden="true" className={cn("size-2.5 shrink-0 rounded-full", ponto)} />
      <EditarNome nome={reuniao.nome} rotulo="Dar nome à reunião" salvar={(nome) => mudarTitulo(reuniao, nome)} />
      <span className="font-mono tabular-nums text-muted-foreground">{tempo(reuniao.duracao ?? 0)}</span>
    </span>} subtitle={fonteTexto(reuniao.fonte, plataforma() === "windows")}
      aside={<><Button icon={pausada ? Play : Pause} onClick={() => void acaoLocal("pausar", { id: reuniao.id })}>{pausada ? "Retomar" : "Pausar"}</Button>
        <Button variant="danger" icon={Stop} onClick={() => void acaoLocal("encerrar", { id: reuniao.id })}>Encerrar</Button></>} />
    {reuniao.avisoCaptura && <p role="status" className="px-3 pb-2 text-sm text-warning">{reuniao.avisoCaptura}</p>}
    <div className="px-3 pb-3"><AvisosDeAudio reuniao={reuniao} /><StatusPill tone={pausada || (reuniao.atraso ?? 0) > 4 ? "warning" : "neutral"} icon={(reuniao.atraso ?? 0) > 4 ? Clock : undefined}>
      {pausada ? "Pausada" : reuniao.atraso == null ? "Quase ao vivo" : reuniao.atraso <= 4
        ? `Quase ao vivo · ~${Math.round(reuniao.atraso)} s de atraso` : `Atraso de ~${Math.round(reuniao.atraso)} s · alcançando o áudio`}
    </StatusPill></div>
    <Panel ref={painel} className="rolagem max-h-[28rem] p-4" onWheel={(event) => { if (event.deltaY < 0) mudarAcompanhar(false); }}
      onScroll={(event) => {
        const el = event.currentTarget;
        if (el.scrollHeight - el.scrollTop - el.clientHeight <= 20) mudarAcompanhar(true);
      }}>
      {reuniao.trechos.length
        ? <Transcricao doc={reuniao} aoVivo aoEditar={() => mudarAcompanhar(false)} trechos={reuniao.baixaConfianca ? reuniao.trechos : blocos(reuniao.trechos)} />
        : <p role="status" className="py-4 text-sm text-muted-foreground">Ouvindo a chamada… Os primeiros trechos aparecem alguns segundos depois que alguém fala.</p>}
    </Panel>
    {!acompanhar && <div className="flex justify-center p-2"><Button size="sm" icon={ArrowDown} onClick={() => mudarAcompanhar(true)}>Acompanhar ao vivo</Button></div>}
  </Shell>;
}

function ReuniaoSalva({ reuniao, finalizando = false }: { reuniao: DadosReuniao; finalizando?: boolean }) {
  const [busca, pesquisar] = useState("");
  const [bruto, mostrarBruto] = useState(false);
  const todos = reuniao.baixaConfianca ? reuniao.trechos : blocos(reuniao.trechos);
  const trechos = todos.filter((t) => t.texto.toLocaleLowerCase("pt-BR").includes(busca.trim().toLocaleLowerCase("pt-BR")));
  return <Shell>
    <ShellHeader className="flex-wrap" icon={UsersThree} title={<EditarNome nome={reuniao.nome} rotulo="Dar nome à reunião" salvar={(nome) => mudarTitulo(reuniao, nome)} />}
      subtitle={`${dia(reuniao.criadoEm)} · ${tempo(reuniao.duracao ?? 0)} · ${plural(reuniao.vozes.length, "voz", "vozes")}`}
      aside={<><Copiar texto={textoDoDocumento(reuniao)} rotulo="Copiar tudo" /><Exportar doc={reuniao} resumo={reuniao.resumo ?? []} />
        {!finalizando && <Menu trigger={<IconButton size="sm" label="Mais ações" icon={DotsThree} />} groups={[{ items: [
          { id: "resumir", label: "Gerar resumo de novo", onSelect: () => void acaoLocal("resumir", { id: reuniao.id }) },
          { id: "bruto", label: "Ver o que foi ouvido", disabled: !reuniao.bruto, onSelect: () => mostrarBruto(true) },
          { id: "remover", label: "Apagar reunião", destructive: true, onSelect: () => void executar(async () => {
            await chamar("lancamento.remover", { id: reuniao.id });
            toast.success("Reunião apagada", { action: { label: "Desfazer", onClick: () => void acaoLocal("desfazer", { id: reuniao.id }) } });
          }) },
        ] }]} />}</>} />
    <Panel className="p-4">
      {(reuniao.erroCaptura || reuniao.avisoCaptura) && <p role="status" className="mb-3 text-sm text-warning">{reuniao.erroCaptura ?? reuniao.avisoCaptura}</p>}
      <AvisosDeAudio reuniao={reuniao} />
      {reuniao.falantes === "indisponivel" && <p className="mb-3 text-sm text-warning">Os falantes da chamada não foram identificados.</p>}
      <Inset className="mb-4 p-4">
        <div className="mb-2 flex flex-wrap items-center gap-2">
          <h3 className="text-[0.8125rem] font-semibold">Resumo</h3>
          {reuniao.resumo?.length ? <><Tag>Gerado localmente · Qwen</Tag><IconButton className="ml-auto" size="sm" icon={Copy} label="Copiar só o resumo"
            onClick={() => void executar(() => copiarTexto(reuniao.resumo?.join("\n") ?? ""), "Resumo copiado")} /></> : null}
        </div>
        {reuniao.erroResumo && <p role="alert" className="mb-2 text-sm text-negative">{reuniao.erroResumo}</p>}
        {reuniao.resumoCancelado && <p role="status" className="mb-2 text-sm text-muted-foreground">Resumo cancelado. A transcrição foi salva.</p>}
        {finalizando ? <>
          <p role="status" className="text-sm text-muted-foreground">{reuniao.etapa === "resumindo" ? "Gerando o resumo no Qwen local…" : "Identificando as pessoas da chamada…"}</p>
          {reuniao.progresso != null && <ProgressBar className="mt-2" size="sm" label="Progresso da reunião" value={reuniao.progresso * 100} />}
          {reuniao.etapa === "resumindo" && <Button className="mt-2" size="sm" variant="quiet" onClick={() => void acaoLocal("cancelarResumo", { id: reuniao.id })}>Cancelar resumo</Button>}
        </> : reuniao.resumo?.length ? <>
          <ul className="selecionavel list-disc space-y-1 pl-5 text-sm leading-6">{reuniao.resumo.map((item, i) => <li key={i}>{item}</li>)}</ul>
          <p className="mt-2 text-xs leading-5 text-muted-foreground">Resumo gerado por modelo local a partir da transcrição. Para trechos exatos, consulte as falas abaixo.</p>
        </> : <><p className="text-sm text-muted-foreground">Resumo local não gerado.</p><Button className="mt-2" size="sm" variant="quiet" onClick={() => void acaoLocal("resumir", { id: reuniao.id })}>Tentar gerar resumo</Button></>}
      </Inset>
      <h3 className="mb-2 text-[0.8125rem] font-semibold">Transcrição</h3>
      {todos.length > 10 && <SearchField className="mb-3" placeholder="Buscar nesta reunião" value={busca} onChange={(e) => pesquisar(e.target.value)} onClear={() => pesquisar("")} />}
      {bruto && reuniao.bruto ? <><Button size="sm" variant="quiet" onClick={() => mostrarBruto(false)}>Ver transcrição</Button><p className="selecionavel whitespace-pre-wrap py-3 text-sm">{reuniao.bruto}</p></>
        : <Transcricao key={reuniao.id} doc={reuniao} trechos={trechos} />}
    </Panel>
  </Shell>;
}

function ReuniaoArquivada({ reuniao }: { reuniao: DadosReuniao }) {
  const { dados, erro, recarregar } = useDados<DadosReuniao | null>("lancamento.documento", { id: reuniao.id }, ["lancamento"]);
  if (erro) return <ErroDeLeitura erro={erro} recarregar={recarregar} />;
  return dados ? <ReuniaoSalva reuniao={dados} /> : <p role="status">Carregando transcrição…</p>;
}

export function Reuniao({ dados, ditando, erro, recarregar }: {
  dados?: Lancamento; ditando: boolean; erro?: string; recarregar: () => void;
}) {
  const win = plataforma() === "windows";
  const [fonte, mudarFonte] = useState<FonteReuniao>("macEMicrofone");
  const [selecionada, selecionar] = useState<string>();
  const [descarte, confirmarDescarte] = useState<DadosReuniao>();
  const atual = dados?.reuniao;
  const audio = win || dados?.permissoes.audioSistema === "concedida";
  const mic = dados?.permissoes.microfone === "concedida";
  const pronta = audio && (fonte === "mac" || mic);
  const anteriores = dados?.anteriores ?? [];
  const aberta = anteriores.find((r) => r.id === selecionada) ?? anteriores[0];
  return <>
    <Cabecalho titulo="Reunião" apoio={win ? "Transcreve chamadas deste computador junto com a sua voz e gera um resumo local ao terminar, sem entrar como participante na chamada." : "Transcreve chamadas deste Mac junto com a sua voz e gera um resumo local ao terminar, sem entrar como participante na chamada."}
      lado={!atual && <Button variant="primary" icon={Microphone} disabled={!!erro || !dados?.disponivel || !pronta || ditando} onClick={() => { selecionar(undefined); void acaoLocal("iniciar", { fonte }); }}>Começar reunião</Button>} />
    {erro && <ErroDeLeitura erro={erro} recarregar={recarregar} />}
    {atual ? atual.fase === "finalizando" ? <ReuniaoSalva reuniao={atual} finalizando /> : <AoVivo key={atual.id} reuniao={atual} />
      : erro ? null
      : !dados?.disponivel ? <p role="status" className="text-sm text-muted-foreground">
        {dados ? dados.motivoIndisponibilidade ?? "A captura de reuniões está indisponível nesta versão do motor." : "Carregando reuniões…"}
      </p>
        : <>
          {(!audio || (fonte !== "mac" && !mic)) && <div className="mb-4 grid gap-2">
            {!audio && <Permissao qual="audioSistema" />}
            {fonte !== "mac" && !mic && <Permissao qual="microfone" />}
          </div>}
          <Shell><Panel className="flex flex-wrap items-center justify-between gap-4 p-4">
            <label className="grid gap-1 text-xs font-medium text-muted-foreground">Fontes de áudio
              <select value={fonte} onChange={(e) => mudarFonte(e.target.value as FonteReuniao)}
                className="max-w-full rounded-lg bg-surface px-3 py-2 text-sm text-foreground focus-visible:ring-2 focus-visible:ring-ring">
                <option value="macEMicrofone">{win ? "Áudio do computador + meu microfone (recomendado)" : "Áudio do Mac + meu microfone (recomendado)"}</option>
                <option value="mac">{win ? "Só o áudio do computador" : "Só o áudio do Mac"}</option>
              </select>
            </label>
            <StatusPill tone={pronta ? "positive" : "warning"}>{pronta ? "Pronto para capturar" : !audio ? (win ? "Falta permissão do áudio do computador" : "Falta permissão do áudio do Mac") : "Falta permissão do microfone"}</StatusPill>
          </Panel></Shell>
        </>}
    {dados?.avisoMigracao && <p role="alert" className="my-3 text-sm text-warning">Alguns dados antigos precisam de nova tentativa de importação. {dados.avisoMigracao}</p>}
    {!!dados?.recuperaveis?.length && <Secao titulo="Reuniões interrompidas">
      {dados.recuperaveis.map((r) => <Inset key={r.id} className="mb-2 flex flex-wrap items-center gap-3 p-3">
        <div className="flex-1"><p className="font-medium">{r.nome} · {dia(r.criadoEm)}</p>
          {r.erroCaptura && <p className="text-sm text-warning">{r.erroCaptura}</p>}
          {r.erroProcessamento && <p className="text-sm text-warning">{r.erroProcessamento}</p>}
          <AvisosDeAudio reuniao={r} /></div>
        <Button disabled={!!atual} onClick={() => void acaoLocal("recuperar", { id: r.id })}>Recuperar transcrição</Button>
        <Button variant="quiet" disabled={!!atual} onClick={() => confirmarDescarte(r)}>Descartar áudio</Button>
      </Inset>)}
    </Secao>}
    <Dialog open={!!descarte} onOpenChange={(aberto) => { if (!aberto) confirmarDescarte(undefined); }}
      title="Descartar áudio pendente?" description="O áudio será apagado. Os trechos já transcritos permanecem no histórico."
      footer={<><Button variant="quiet" onClick={() => confirmarDescarte(undefined)}>Voltar</Button>
        <Button variant="danger" disabled={!!atual} onClick={() => {
          if (!descarte) return;
          const id = descarte.id;
          confirmarDescarte(undefined);
          void acaoLocal("descartar", { id });
        }}>Descartar áudio</Button></>} />
    {anteriores.length > 0 && <Secao titulo="Reuniões anteriores"><div className="grid gap-3">
      {anteriores.map((r, i) => <div key={r.id}>
        {(i === 0 || dia(anteriores[i - 1].criadoEm) !== dia(r.criadoEm)) && <p className="mb-2 text-xs text-muted-foreground">{dia(r.criadoEm)}</p>}
        {r.id === aberta?.id ? <ReuniaoArquivada key={r.id} reuniao={r} /> :
          <button className="w-full rounded-xl bg-foreground/[0.04] p-3 text-left hover:bg-foreground/[0.08] focus-visible:ring-2 focus-visible:ring-ring"
            onClick={() => selecionar(r.id)} type="button">
            <span className="block text-sm font-medium">{r.nome}</span>
            <span className="line-clamp-2 text-[0.8125rem] text-muted-foreground">{r.resumo?.[0] ?? r.trechos[0]?.texto ?? r.previa}</span>
            <span className="text-xs tabular-nums text-muted-foreground">{tempo(r.duracao ?? 0)} · {r.vozes.length} vozes</span>
          </button>}
      </div>)}
    </div></Secao>}
  </>;
}

function Permissao({ qual }: { qual: "audioSistema" | "microfone" }) {
  const win = plataforma() === "windows";
  return <Inset className="flex items-center gap-3 p-4">
    <Warning className="size-4 shrink-0 text-warning" weight="fill" />
    <div className="min-w-0 flex-1 text-[0.8125rem]"><p className="font-medium">{qual === "audioSistema" ? (win ? "Permita capturar o áudio do computador" : "Permita capturar o áudio do Mac") : "Permita o microfone"}</p>
      <p className="text-muted-foreground">{qual === "audioSistema" ? (win ? "É o que deixa o Dita.se ouvir outras pessoas da chamada. Nenhuma imagem da tela é gravada e o áudio não sai deste computador." : "É o que deixa o Dita.se ouvir outras pessoas da chamada. Nenhuma imagem da tela é gravada e o áudio não sai deste Mac.") : "Para incluir a sua voz na transcrição da reunião como Você."}</p></div>
    <Button size="sm" onClick={() => void acaoLocal("permissao", { qual })}>Permitir</Button>
  </Inset>;
}
