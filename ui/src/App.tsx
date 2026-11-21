import { BookBookmark, ChartBar, FileAudio, GearSix, House, UsersThree, type Icon } from "@phosphor-icons/react";
import { MotionConfig, motion, useReducedMotion } from "motion/react";
import { useEffect, useRef, useState, type DragEvent } from "react";
import { Toaster, toast } from "@/components/ui/toast-stack";
import { useDados, useHash } from "@/lib/dados";
import { indicador, pagina as entradaDaPagina } from "@/lib/motion-tokens";
import { plataforma } from "@/lib/plataforma";
import { chamar, dentroDoApp, type Estado } from "@/lib/ponte";
import { aceite, aplicarLote, arquivoEmCurso, extensao, extensoes, mesclarLancamento, tempo, type Lancamento, type IntervalosReuniao, type LoteReuniao } from "@/lib/lancamento";
import { executar } from "@/lib/acoes-lancamento";
import { IconTile } from "@/components/ui/surface";
import { Arquivos } from "@/paginas/Arquivos";
import { Reuniao } from "@/paginas/Reuniao";
import { cn } from "@/lib/utils";
import { Ajustes } from "@/paginas/Ajustes";
import { Dicionario } from "@/paginas/Dicionario";
import { Inicio } from "@/paginas/Inicio";
import { Insights } from "@/paginas/Insights";

const paginas: { id: string; nome: string; icone: Icon }[] = [
  { id: "inicio", nome: "Início", icone: House },
  { id: "reuniao", nome: "Reunião", icone: UsersThree },
  { id: "arquivos", nome: "Arquivos", icone: FileAudio },
  { id: "insights", nome: "Insights", icone: ChartBar },
  { id: "dicionario", nome: "Dicionário", icone: BookBookmark },
  { id: "ajustes", nome: "Ajustes", icone: GearSix },
];

export default function App() {
  const [pagina, irPara] = useHash("inicio");
  const { dados: estado } = useDados<Estado>("estado", {}, ["estado"]);
  const { dados: lance, erro: erroLancamento, recarregar: recarregarLancamento, setDados: atualizarLancamento } =
    useDados<Lancamento>("lancamento.estado", {}, ["lancamento"], mesclarLancamento);
  const win = plataforma() === "windows";
  const estadoDaFala = estado?.reconhecedor.estado;
  const permissaoDoMicrofone = estado?.permissoes.microfone;
  // No Windows, a carga do reconhecedor e a volta dos ajustes de privacidade publicam "estado".
  // Só as mudanças que interessam às duas telas pedem outra leitura do acervo.
  useEffect(() => {
    if (win && estadoDaFala) recarregarLancamento();
  }, [win, estadoDaFala, permissaoDoMicrofone, recarregarLancamento]);
  const [destaque, destacar] = useState<string>();
  useEffect(() => {
    const receber = (evento: Event) => {
      const lote = (evento as CustomEvent<LoteReuniao>).detail;
      atualizarLancamento((atual) => aplicarLote(atual, lote));
    };
    const intervalos = (evento: Event) => {
      const { id, lacunas, pendentes } = (evento as CustomEvent<IntervalosReuniao>).detail;
      atualizarLancamento((atual) => {
        if (!atual || atual.reuniao?.id !== id) return atual;
        return { ...atual, reuniao: { ...atual.reuniao, lacunas, pendentes } };
      });
    };
    const progresso = (evento: Event) => {
      const pauta = (evento as CustomEvent<{ id: string; progresso?: number; decorrido?: number; etapa: { id: string } }>).detail;
      atualizarLancamento((atual) => {
        const reuniao = atual?.reuniao;
        if (!atual || reuniao?.id !== pauta.id) return atual;
        if (reuniao.fase === "finalizando" && (reuniao.progresso !== pauta.progresso || reuniao.etapa !== pauta.etapa.id)) {
          return { ...atual, reuniao: { ...reuniao, progresso: pauta.progresso, etapa: pauta.etapa.id } };
        }
        if ((reuniao.fase === "gravando" || reuniao.fase === "pausada") && pauta.decorrido != null &&
            Math.floor(pauta.decorrido) > Math.floor(reuniao.duracao ?? 0)) {
          return { ...atual, reuniao: { ...reuniao, duracao: pauta.decorrido } };
        }
        return atual;
      });
    };
    const abriu = (evento: Event) => {
      location.hash = "#/arquivos";
      destacar((evento as CustomEvent<string>).detail);
    };
    window.addEventListener("ditado:trechos", receber);
    window.addEventListener("ditado:progresso", progresso);
    window.addEventListener("ditado:intervalos", intervalos);
    window.addEventListener("ditado:arquivos-soltos", abriu);
    return () => {
      window.removeEventListener("ditado:trechos", receber);
      window.removeEventListener("ditado:progresso", progresso);
      window.removeEventListener("ditado:intervalos", intervalos);
      window.removeEventListener("ditado:arquivos-soltos", abriu);
    };
  }, [atualizarLancamento]);
  const entrada = useRef<HTMLInputElement>(null);
  const ninho = useRef(0);
  const mola = useRef<number | undefined>(undefined);
  const [arrastando, arrastar] = useState(false);
  const disponivel = !erroLancamento && lance?.disponivel === true;
  const ativo = lance?.arquivos.find(arquivoEmCurso);
  const reuniao = lance?.reuniao;
  const fila = lance?.arquivos.filter((a) => arquivoEmCurso(a) || a.fase === "naFila").length ?? 0;
  const atual = paginas.some((p) => p.id === pagina) ? pagina : "inicio";
  const rolagem = useRef<HTMLElement>(null);
  // Página nova começa do topo, não do ponto de rolagem da anterior.
  useEffect(() => { void rolagem.current?.scrollTo({ top: 0 }); }, [atual]);

  /**
   * No app, o WebKit entrega só o nome do arquivo arrastado, sem acesso ao arquivo. O motor então
   * abre o seletor nativo para a pessoa confirmar; avisamos antes, para não parecer que o arraste falhou.
   */
  function adicionar(arquivos: File[], doArraste = false) {
    const noApp = dentroDoApp();
    if (!disponivel || (!arquivos.length && !(noApp && doArraste))) return;
    irPara("arquivos");
    if (noApp && doArraste) {
      toast("Confirme os arquivos na janela que abriu", {
        description: "Esta origem só informou os nomes. Selecione os arquivos para pôr na fila.",
        duration: 8000,
      });
    }
    void executar(async () => {
      const descritores = arquivos.map((f) => ({
        nome: f.name,
        suportado: extensoes.includes(extensao(f.name)),
      }));
      const id = await chamar<string | null>("lancamento.adicionar", { arquivos: descritores });
      if (noApp && !id) toast("Nenhum arquivo adicionado", { description: "A janela foi fechada sem escolher arquivos." });
      destacar(id ?? undefined);
    });
  }

  function escolher() {
    if (!disponivel) return;
    if (!dentroDoApp()) { entrada.current?.click(); return; }
    void executar(async () => destacar(await chamar<string | undefined>("lancamento.escolher")));
  }

  useEffect(() => {
    if (atual !== "arquivos") return;
    const tecla = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "o" && disponivel) {
        e.preventDefault();
        if (dentroDoApp()) void executar(async () => destacar(await chamar<string | undefined>("lancamento.escolher")));
        else entrada.current?.click();
      }
    };
    window.addEventListener("keydown", tecla);
    return () => window.removeEventListener("keydown", tecla);
  }, [atual, disponivel]);
  useEffect(() => () => window.clearTimeout(mola.current), []);
  const deArquivo = (event: DragEvent) => event.dataTransfer.types.includes("Files");

  return (
    // Com "user", quem pediu movimento reduzido no sistema fica sem deslocamento e sem escala: só fades.
    <MotionConfig reducedMotion="user">
    <div className={cn("flex h-full bg-surface", win ? "p-0" : "p-1.5 pl-0")}
      onDragEnter={(event) => { if (deArquivo(event)) { event.preventDefault(); ninho.current++; arrastar(true); } }}
      onDragOver={(event) => { if (deArquivo(event)) { event.preventDefault(); event.dataTransfer.dropEffect = disponivel ? "copy" : "none"; } }}
      onDragLeave={() => { ninho.current = Math.max(0, ninho.current - 1); if (!ninho.current) arrastar(false); }}
      onDrop={(event) => {
        if (!deArquivo(event)) return;
        event.preventDefault(); window.clearTimeout(mola.current); ninho.current = 0; arrastar(false);
        adicionar(Array.from(event.dataTransfer.files), true);
      }}>
      <input ref={entrada} type="file" hidden multiple accept={aceite}
        onChange={(event) => { adicionar(Array.from(event.target.files ?? [])); event.target.value = ""; }} />
      {/* O topo da lateral fica livre no Mac (botões da janela); no Windows a barra de título é nativa. */}
      <nav aria-label="Seções" className={cn("flex w-[13rem] shrink-0 flex-col px-3 pb-3", win ? "pt-1" : "pt-12")}>
        {!win && <p className="px-2 pb-4 text-[0.9375rem] font-semibold tracking-tight">Dita.se</p>}
        <ul className="flex flex-col gap-0.5">
          {paginas.map(({ id, nome, icone: Icone }) => (
            <li key={id}>
              <button
                type="button"
                aria-current={atual === id ? "page" : undefined}
                onClick={() => irPara(id)}
                onDragEnter={(event) => {
                  if (id === "arquivos" && atual !== id && disponivel && deArquivo(event)) {
                    window.clearTimeout(mola.current);
                    mola.current = window.setTimeout(() => irPara(id), 350);
                  }
                }}
                onDragLeave={() => window.clearTimeout(mola.current)}
                className={cn(
                  "relative isolate flex w-full items-center gap-2.5 font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring",
                  win ? "h-9 rounded-[4px] px-3 text-[0.875rem]" : "h-8 rounded-lg px-2 text-[0.8125rem]",
                  atual === id ? "text-foreground" : "text-muted-foreground hover:bg-foreground/[0.04] hover:text-foreground",
                )}
              >
                {atual === id && (
                  <motion.span
                    layoutId="nav-ativo"
                    transition={indicador}
                    className={cn(
                      "absolute inset-0 -z-10",
                      win ? "rounded-[4px] bg-foreground/[0.06]" : "rounded-lg bg-foreground/[0.08]",
                    )}
                  >
                    {win && (
                      <span
                        aria-hidden="true"
                        className="absolute top-1/2 left-0 h-4 w-[3px] -translate-y-1/2 rounded-[1.5px] bg-brand"
                      />
                    )}
                  </motion.span>
                )}
                <Icone weight={atual === id ? "fill" : "regular"} className="size-4" />
                {nome}
                {id === "arquivos" && ativo && <span className="ml-auto text-[0.6875rem] tabular-nums text-brand">{ativo.progresso == null ? `1/${fila}` : `${Math.round(ativo.progresso * 100)}%`}</span>}
                {id === "reuniao" && reuniao && reuniao.fase !== "finalizando" && <span className="ml-auto flex items-center gap-1 text-[0.6875rem] tabular-nums">
                  <span className={cn("size-1.5 rounded-full", reuniao.fase === "pausada" ? "bg-warning" : "bg-negative")} />
                  {reuniao.fase === "pausada" ? "Pausada" : tempo(reuniao.duracao ?? 0)}
                </span>}
              </button>
            </li>
          ))}
        </ul>
        <div className="mt-auto">{estado && <Situacao estado={estado} lance={lance} />}</div>
      </nav>

      <div className="relative flex min-w-0 flex-1">
      <main
        ref={rolagem}
        className={cn(
          "rolagem min-w-0 flex-1 bg-surface-inset",
          win
            ? "rounded-none rounded-tl-lg border-t border-l border-(--traco-camada)"
            : "rounded-2xl",
        )}
      >
        {/* A página nova entra subindo 6 px; a antiga some na hora, então navegar não espera animação. */}
        <motion.div
          key={atual}
          initial={{ opacity: 0, y: 6 }}
          animate={{ opacity: 1, y: 0 }}
          transition={entradaDaPagina}
          className="mx-auto max-w-[52rem] px-8 pt-9 pb-12"
        >
          {atual === "inicio" && <Inicio estado={estado} irPara={irPara} />}
          {atual === "reuniao" && <Reuniao dados={lance} ditando={estado?.fase === "gravando"}
            erro={erroLancamento} recarregar={recarregarLancamento} />}
          {atual === "arquivos" && <Arquivos dados={lance} selecionar={escolher} arrastando={arrastando}
            destaque={destaque} erro={erroLancamento} recarregar={recarregarLancamento} />}
          {atual === "insights" && <Insights />}
          {atual === "dicionario" && <Dicionario />}
          {atual === "ajustes" && <Ajustes estado={estado} />}
        </motion.div>
      </main>
      {arrastando && disponivel && atual !== "arquivos" && <div className="pointer-events-none absolute inset-2 z-40 flex flex-col items-center justify-center gap-2 rounded-2xl border-2 border-dashed border-brand bg-surface-inset/95 p-6 text-center">
        <IconTile icon={FileAudio} className="text-brand" />
        <p className="text-[0.9375rem] font-medium">Solte para transcrever com separação de vozes</p>
        <p className="text-[0.8125rem] text-muted-foreground">Áudio ou vídeo ({extensoes.join(", ")})</p>
        {dentroDoApp() && <p className="text-[0.8125rem] text-muted-foreground">Ao soltar, abre a janela de escolha para você confirmar os arquivos.</p>}
      </div>}
      </div>
      <Toaster />
    </div>
    </MotionConfig>
  );
}

/** Rodapé da lateral: o que o app está fazendo agora e se os motores estão prontos. */
function Situacao({ estado, lance }: { estado: Estado; lance?: Lancamento }) {
  const win = plataforma() === "windows";
  const semPermissao = estado.permissoes.microfone !== "concedida" || (!win && estado.permissoes.acessibilidade !== "concedida");
  let cor = "bg-positive";
  let texto = "Pronto para ditar";
  const reuniao = lance?.reuniao;
  const ativo = lance?.arquivos.find(arquivoEmCurso);
  let apoio = `Segure ${estado.atalho.simbolo}`;
  if (reuniao?.fase === "gravando" || reuniao?.fase === "pausada" || reuniao?.fase === "finalizando") {
    const gravando = reuniao.fase === "gravando";
    cor = gravando ? "bg-negative" : reuniao.fase === "pausada" ? "bg-warning" : "bg-brand";
    texto = reuniao.fase === "finalizando" ? "Fechando a reunião" : `${gravando ? "Em reunião" : "Reunião pausada"} · ${tempo(reuniao.duracao ?? 0)}`;
    apoio = gravando
      ? (reuniao.fonte === "mac"
          ? (win ? "Só áudio do computador" : "Só áudio do Mac")
          : (win ? "Áudio do computador e microfone" : "Áudio do Mac e microfone"))
      : reuniao.fase === "pausada" ? "Clique em Reunião para retomar" : "Separando vozes e resumindo";
  } else if (estado.fase === "gravando") {
    cor = "bg-negative";
    texto = estado.maosLivres ? "Ouvindo, mãos livres" : "Ouvindo";
  } else if (estado.fase === "processando") {
    cor = "bg-brand";
    texto = "Transcrevendo";
  } else if (ativo) {
    cor = "bg-brand";
    texto = "Transcrevendo arquivo" + (ativo.progresso == null ? "" : ` · ${Math.round(ativo.progresso * 100)}%`);
    apoio = ativo.nome;
  } else if (semPermissao) {
    cor = "bg-warning";
    texto = "Falta permissão";
  } else if (estado.reconhecedor.estado === "carregando" || estado.reconhecedor.estado === "parado") {
    cor = "bg-warning";
    texto = "Carregando o reconhecedor";
  } else if (estado.reconhecedor.estado === "falhou") {
    cor = "bg-negative";
    texto = "Reconhecedor com erro";
  }
  const reduzido = useReducedMotion();
  const ouvindo = reuniao ? reuniao.fase === "gravando" : estado.fase === "gravando";
  return (
    <div className="px-2 text-[0.75rem] leading-5 text-muted-foreground" role="status">
      <p className="flex items-center gap-2 font-medium text-foreground">
        {/* O ponto pulsa só enquanto grava (e só sem movimento reduzido): em repouso nada se mexe. */}
        <motion.span
          className={cn("size-1.5 rounded-full transition-colors duration-300", cor)}
          aria-hidden="true"
          animate={ouvindo && !reduzido ? { opacity: [1, 0.35, 1] } : { opacity: 1 }}
          transition={ouvindo && !reduzido ? { duration: 1.4, ease: "easeInOut", repeat: Infinity } : { duration: 0.15 }}
        />
        <motion.span key={texto} initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ duration: 0.18 }}>
          {texto}
        </motion.span>
      </p>
      <p className="truncate pl-3.5" title={apoio}>{apoio}</p>
    </div>
  );
}
