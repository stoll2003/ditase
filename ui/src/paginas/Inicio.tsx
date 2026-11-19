import { ArrowClockwise, Check, Copy, DotsThree, Sparkle, Microphone, MicrophoneSlash, Stop, TextAa, Trash, Warning } from "@phosphor-icons/react";
import { useEffect, useMemo, useState } from "react";
import { Cabecalho, Secao } from "@/componentes/Cabecalho";
import { type EntradaDaLinha, LinhaAnimada, useNovos } from "@/componentes/Movimento";
import { Button, IconButton } from "@/components/ui/action-button";
import { Menu } from "@/components/ui/action-menu";
import { EmptyState } from "@/components/ui/empty-state";
import { Kbd } from "@/components/ui/key-hint";
import { Skeleton, SkeletonGroup } from "@/components/ui/loading-skeleton";
import { Dialog } from "@/components/ui/modal-sheet";
import { StatGrid } from "@/components/ui/stat-grid";
import { Inset } from "@/components/ui/surface";
import { SearchField } from "@/components/ui/text-field";
import { toast } from "@/components/ui/toast-stack";
import { useDados } from "@/lib/dados";
import { compacto, dia, duracao, espera, hora, numero, plural, segundos, tamanhoDeArquivo } from "@/lib/formato";
import { plataforma } from "@/lib/plataforma";
import { chamar, type Ditado, type Estado, type Estatisticas } from "@/lib/ponte";

export function Inicio({ estado, irPara }: { estado?: Estado; irPara: (pagina: string) => void }) {
  const [busca, setBusca] = useState("");
  const { dados: historico } = useDados<Ditado[]>("historico", { busca, limite: 300 }, ["historico"]);
  const { dados: numeros } = useDados<Estatisticas>("estatisticas", { dias: 14 }, ["historico"]);
  const [bruto, setBruto] = useState<Ditado | undefined>();

  const grupos = useMemo(() => {
    const saida: { titulo: string; itens: Ditado[] }[] = [];
    for (const d of historico ?? []) {
      const titulo = dia(d.iniciadoEm);
      const ultimo = saida[saida.length - 1];
      if (ultimo?.titulo === titulo) ultimo.itens.push(d);
      else saida.push({ titulo, itens: [d] });
    }
    return saida;
  }, [historico]);

  const novos = useNovos(historico?.map((d) => String(d.id)), busca);

  // Posição de cada ditado na lista toda (os grupos por dia só dividem a tela), para a cascata de entrada.
  const ordem = useMemo(() => new Map((historico ?? []).map((d, i) => [d.id, i])), [historico]);

  const gravando = estado?.fase === "gravando";
  const semana = numeros?.palavrasNaSemana ?? 0;
  const anterior = numeros?.palavrasNaSemanaAnterior ?? 0;
  const comparavel = anterior >= 300 && semana <= anterior * 4;

  return (
    <>
      <Cabecalho
        titulo="Início"
        apoio={
          estado && (
            <>
              Segure <Kbd>{estado.atalho.simbolo}</Kbd> e fale. Ao soltar, o texto entra onde o cursor estiver. Dois
              toques rápidos deixam as mãos livres.
            </>
          )
        }
        lado={
          <Button
            variant={gravando ? "danger" : "primary"}
            size="md"
            icon={gravando ? Stop : Microphone}
            disabled={estado?.fase === "processando"}
            onClick={() => chamar("gravacao.alternar")}
          >
            {gravando ? "Terminar" : "Ditar em mãos livres"}
          </Button>
        }
      />

      {estado && <Pendencias estado={estado} irPara={irPara} />}

      {numeros && numeros.totalDitados > 0 && (
        <StatGrid
          className="max-w-none"
          sparklines
          data={[
            { id: "hoje", label: "Palavras hoje", value: numero(numeros.palavrasHoje) },
            {
              id: "semana",
              label: "Últimos 7 dias",
              value: compacto(semana),
              // Semana anterior quase vazia daria um percentual absurdo; aí o número fica sem comparação.
              delta: comparavel ? Math.round(((semana - anterior) / anterior) * 100) : undefined,
              hint: comparavel ? "contra os 7 dias anteriores" : undefined,
              trend: numeros.porDia.map((d) => d.palavras),
            },
            { id: "ppm", label: "Palavras por minuto", value: numero(numeros.ppmMedio), hint: "digitando, a média é 40" },
            { id: "economia", label: "Tempo economizado", value: duracao(numeros.minutosEconomizados), hint: "em relação a digitar" },
          ]}
        />
      )}

      <Secao
        titulo="Histórico"
        lado={
          <div className="w-64">
            <SearchField
              aria-label="Buscar no histórico"
              placeholder="Buscar no histórico"
              value={busca}
              onChange={(e) => setBusca(e.target.value)}
              onClear={() => setBusca("")}
            />
          </div>
        }
      >
        {historico === undefined ? (
          <SkeletonGroup label="Carregando o histórico" className="flex flex-col gap-2">
            {Array.from({ length: 5 }, (_, i) => (
              <Skeleton key={i} className="h-16" />
            ))}
          </SkeletonGroup>
        ) : grupos.length === 0 ? (
          busca ? (
            <EmptyState compact title={`Nada encontrado para "${busca}"`} description="Tente outra palavra." />
          ) : (
            <EmptyState
              icon={Microphone}
              title="Nenhum ditado ainda"
              description={`Clique num campo de texto em qualquer app, segure ${estado?.atalho.simbolo ?? "o atalho"} e fale.`}
            />
          )
        ) : (
          <div className="flex flex-col gap-6">
            {grupos.map((grupo) => (
              <div key={grupo.titulo}>
                <h3 className="mb-2 text-[0.75rem] font-medium text-muted-foreground">{grupo.titulo}</h3>
                <ul className="flex flex-col gap-1.5">
                  {grupo.itens.map((d) => (
                    <Item
                      key={d.id}
                      ditado={d}
                      indice={ordem.get(d.id) ?? 0}
                      entrada={novos.has(String(d.id)) ? "nova" : busca ? "nenhuma" : "lista"}
                      verBruto={() => setBruto(d)}
                    />
                  ))}
                </ul>
              </div>
            ))}
          </div>
        )}
      </Secao>

      <Dialog
        size="lg"
        title="O que o reconhecedor ouviu"
        description="Texto antes de qualquer arrumação. Serve para achar palavra que vale pôr no dicionário."
        open={bruto !== undefined}
        onOpenChange={(aberto) => !aberto && setBruto(undefined)}
      >
        <p className="selecionavel rolagem max-h-80 p-4 text-[0.8125rem] leading-6 whitespace-pre-wrap">{bruto?.textoBruto}</p>
      </Dialog>
    </>
  );
}

function Item({ ditado: d, indice, entrada, verBruto }: { ditado: Ditado; indice: number; entrada: EntradaDaLinha; verBruto: () => void }) {
  const [aberto, setAberto] = useState(false);
  const [saindo, setSaindo] = useState(false);
  const [copiado, setCopiado] = useState(false);
  useEffect(() => {
    if (!copiado) return;
    const id = setTimeout(() => setCopiado(false), 1600);
    return () => clearTimeout(id);
  }, [copiado]);
  const longo = d.texto.length > 260 || d.texto.split("\n").length > 3;
  const copiar = () =>
    chamar("copiar", { texto: d.texto }).then(() => {
      setCopiado(true);
      toast.success("Texto copiado");
    });
  // A linha recolhe primeiro; o ditado só é apagado quando ela terminou de recolher.
  const apagar = () =>
    chamar("ditado.apagar", { id: d.id })
      .then(() => toast("Ditado apagado"))
      .catch(() => setSaindo(false));
  const win = plataforma() === "windows";
  const detalhes = [
    hora(d.iniciadoEm),
    d.appNome || undefined,
    plural(d.palavras, "palavra", "palavras"),
    d.duracao > 0 ? segundos(d.duracao) : undefined,
    d.origem === "ditado" && d.latencia > 0 ? `pronto em ${espera(d.latencia)}` : undefined,
    !win && d.origem === "wispr" ? "importado do Wispr Flow" : undefined,
    d.estado === "recuperado" ? "recuperado depois de o app fechar" : undefined,
    d.estado === "parcial" ? "parte do áudio não foi transcrita" : undefined,
  ].filter(Boolean);
  const falhou = d.estado === "falhou";
  const podeRefazer = (falhou || d.estado === "parcial") && d.audio !== "";

  return (
    <LinhaAnimada entrada={entrada} indice={indice} saindo={saindo} aoSair={apagar}>
      <Inset className={`group flex gap-3 py-3 pr-2 pl-4${entrada === "nova" ? " item-destaque" : ""}`}>
        <div className="min-w-0 flex-1">
          {falhou ? (
            <p className="text-[0.875rem] leading-6 text-muted-foreground">
              O reconhecedor não conseguiu transcrever este ditado.{" "}
              {d.audio ? "O áudio está guardado: use Transcrever de novo, no menu ao lado." : "O áudio não foi guardado."}
            </p>
          ) : (
            <p className={`selecionavel text-[0.875rem] leading-6 whitespace-pre-wrap ${aberto || !longo ? "" : "line-clamp-3"}`}>
              {d.texto}
            </p>
          )}
          <p className="mt-1.5 flex flex-wrap items-center gap-x-2 text-[0.75rem] text-muted-foreground">
            <span className="tabular-nums">{detalhes.join(" · ")}</span>
            {longo && (
              <button
                type="button"
                onClick={() => setAberto(!aberto)}
                className="rounded font-medium text-foreground outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
              >
                {aberto ? "Mostrar menos" : "Mostrar tudo"}
              </button>
            )}
          </p>
        </div>
        <div className="flex shrink-0 items-start gap-0.5">
          {!falhou && (
            <IconButton
              key={copiado ? "copiado" : "copiar"}
              icon={copiado ? Check : Copy}
              iconWeight={copiado ? "bold" : "fill"}
              label={copiado ? "Texto copiado" : "Copiar o texto"}
              variant="quiet"
              tooltip
              className={copiado ? "pop-icone" : undefined}
              onClick={copiar}
            />
          )}
          <Menu
            align="end"
            trigger={<IconButton icon={DotsThree} iconWeight="bold" label="Mais ações" variant="quiet" />}
            groups={[
              {
                items: [
                  ...(podeRefazer
                    ? [{ id: "refazer", label: "Transcrever de novo", icon: ArrowClockwise, onSelect: () => void chamar("ditado.retranscrever", { id: d.id }) }]
                    : []),
                  ...(falhou ? [] : [{ id: "bruto", label: "Ver o que foi ouvido", icon: TextAa, onSelect: verBruto }]),
                ],
              },
              {
                items: [
                  {
                    id: "apagar",
                    label: "Apagar do histórico",
                    icon: Trash,
                    destructive: true,
                    onSelect: () => setSaindo(true),
                  },
                ],
              },
            ]}
          />
        </div>
      </Inset>
    </LinhaAnimada>
  );
}

/** O que falta para o ditado funcionar. Some quando está tudo certo. */
function Pendencias({ estado, irPara }: { estado: Estado; irPara: (pagina: string) => void }) {
  const win = plataforma() === "windows";
  type Acao = { rotulo: string; fazer: () => void };
  const itens: { id: string; icone: typeof Warning; titulo: string; texto: string; acao?: Acao; dispensar?: Acao; discreto?: boolean }[] = [];
  if (estado.permissoes.microfone !== "concedida") {
    itens.push({
      id: "mic",
      icone: MicrophoneSlash,
      titulo: "Permita o microfone",
      texto: win ? "Sem ele o Dita.se não ouve nada. O áudio fica neste computador." : "Sem ele o Dita.se não ouve nada. O áudio fica neste Mac.",
      acao: { rotulo: "Permitir", fazer: () => chamar("permissao.pedir", { qual: "microfone" }) },
    });
  }
  if (!win && estado.permissoes.acessibilidade !== "concedida") {
    itens.push({
      id: "ax",
      icone: Warning,
      titulo: "Permita a Acessibilidade",
      texto: "É o que deixa o atalho funcionar em qualquer app e o texto ser colado no cursor. Depois de ligar a chave do Dita.se, pode ser preciso reabrir o app.",
      acao: { rotulo: "Abrir os Ajustes do Sistema", fazer: () => chamar("permissao.pedir", { qual: "acessibilidade" }) },
    });
  }
  if (estado.reconhecedor.estado === "falhou") {
    itens.push({ id: "asr", icone: Warning, titulo: "O reconhecedor de fala não carregou", texto: estado.reconhecedor.detalhe || "Reabra o app com internet para baixar o modelo." });
  }
  const modelo = estado.modeloIA;
  const dispensarAviso: Acao = { rotulo: "Agora não", fazer: () => void chamar("ajustes.definir", { chave: "avisoModelo", valor: "0" }) };
  if (estado.limpador.ligado && modelo.aviso && (modelo.estado === "ausente" || modelo.estado === "falhou")) {
    // Aviso discreto: o ditado já funciona sem a IA; o download é uma oferta, não um bloqueio.
    itens.push({
      id: "modelo",
      icone: Sparkle,
      discreto: true,
      titulo: modelo.estado === "falhou" ? "O download do modelo de IA não terminou" : "Quer o texto mais limpo?",
      texto: modelo.estado === "falhou" ? modelo.detalhe ?? "" : `O ditado já funciona. Com o modelo de IA local (${tamanhoDeArquivo(modelo.tamanho)}), o texto sai pontuado e sem hesitações, e as reuniões ganham resumo.`,
      acao: { rotulo: modelo.estado === "falhou" ? "Tentar de novo" : "Baixar modelo", fazer: () => void chamar("modelo.baixar") },
      dispensar: dispensarAviso,
    });
  } else if (estado.limpador.ligado && (modelo.estado === "baixando" || modelo.estado === "conferindo")) {
    const feito = modelo.feito ?? 0;
    itens.push({
      id: "modelo",
      icone: Sparkle,
      discreto: true,
      titulo: modelo.estado === "conferindo" ? "Conferindo o modelo de IA" : "Baixando o modelo de IA",
      texto: modelo.estado === "conferindo" ? "Quase pronto." : `${tamanhoDeArquivo(feito)} de ${tamanhoDeArquivo(modelo.tamanho)} (${Math.floor((feito / modelo.tamanho) * 100)}%). Pode continuar ditando.`,
      acao: { rotulo: "Ver nos Ajustes", fazer: () => irPara("ajustes") },
    });
  } else if (estado.limpador.ligado && modelo.estado === "indisponivel" && modelo.aviso) {
    itens.push({
      id: "ia",
      icone: Warning,
      titulo: win ? "A limpeza por IA está indisponível neste computador" : "A limpeza por IA está indisponível neste Mac",
      texto: "O ditado segue funcionando só com as regras e o dicionário.",
      acao: { rotulo: "Ver nos Ajustes", fazer: () => irPara("ajustes") },
      dispensar: dispensarAviso,
    });
  } else if (estado.limpador.ligado && modelo.estado === "instalado" && (estado.limpador.estado === "semBinario" || estado.limpador.estado === "semModelo" || estado.limpador.estado === "falhou")) {
    itens.push({
      id: "ia",
      icone: Warning,
      titulo: "A limpeza por IA está indisponível",
      texto: "O ditado segue funcionando só com as regras e o dicionário.",
      acao: { rotulo: "Ver nos Ajustes", fazer: () => irPara("ajustes") },
    });
  }
  if (!win && estado.wisprAberto && estado.atalho.tecla === "fn") {
    itens.push({ id: "wispr", icone: Warning, titulo: "O Wispr Flow está aberto e usa a mesma tecla", texto: "Os dois vão gravar ao mesmo tempo. Feche o Wispr Flow ou troque o atalho nos Ajustes." });
  }
  if (itens.length === 0) return null;
  return (
    <ul className="mb-7 flex flex-col gap-1.5">
      {itens.map(({ id, icone: Icone, titulo, texto, acao, dispensar, discreto }) => (
        <li key={id}>
          <Inset className="flex items-center gap-3 py-3 pr-3 pl-4">
            <Icone weight="fill" className={`size-4 shrink-0 ${discreto ? "text-muted-foreground" : "text-warning"}`} aria-hidden="true" />
            <div className="min-w-0 flex-1">
              <p className="text-[0.8125rem] font-medium">{titulo}</p>
              <p className="text-[0.8125rem] text-muted-foreground">{texto}</p>
            </div>
            {dispensar && (
              <Button variant="quiet" onClick={dispensar.fazer}>
                {dispensar.rotulo}
              </Button>
            )}
            {acao && <Button onClick={acao.fazer}>{acao.rotulo}</Button>}
          </Inset>
        </li>
      ))}
    </ul>
  );
}
