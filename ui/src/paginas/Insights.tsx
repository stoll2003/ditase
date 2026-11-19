import { ChartBar } from "@phosphor-icons/react";
import { useState } from "react";
import { Cabecalho } from "@/componentes/Cabecalho";
import { ChartCard } from "@/components/ui/chart-card";
import { DotHistogram } from "@/components/ui/dot-histogram";
import { EmptyState } from "@/components/ui/empty-state";
import { Skeleton, SkeletonGroup } from "@/components/ui/loading-skeleton";
import { ProgressBar } from "@/components/ui/progress-bar";
import { RadialDots } from "@/components/ui/radial-dots";
import { Segmented } from "@/components/ui/segmented-control";
import { StatGrid } from "@/components/ui/stat-grid";
import { Panel, Shell, ShellHeader } from "@/components/ui/surface";
import { useDados } from "@/lib/dados";
import { compacto, diaCurto, duracao, espera, numero, plural } from "@/lib/formato";
import { plataforma } from "@/lib/plataforma";
import type { Estatisticas } from "@/lib/ponte";

const periodos = [
  { value: "7", label: "7 dias" },
  { value: "30", label: "30 dias" },
  { value: "90", label: "90 dias" },
];

export function Insights() {
  const [dias, setDias] = useState("30");
  const { dados: e } = useDados<Estatisticas>("estatisticas", { dias: Number(dias) }, ["historico"]);

  if (!e) {
    return (
      <>
        <Cabecalho titulo="Insights" />
        <SkeletonGroup label="Carregando as estatísticas" className="flex flex-col gap-3">
          <Skeleton className="h-40" />
          <Skeleton className="h-64" />
        </SkeletonGroup>
      </>
    );
  }
  if (e.totalDitados === 0) {
    return (
      <>
        <Cabecalho titulo="Insights" />
        <EmptyState icon={ChartBar} title="Ainda não há o que medir" description="Os números aparecem aqui depois do primeiro ditado." />
      </>
    );
  }

  const noPeriodo = e.porDia.reduce((soma, d) => soma + d.palavras, 0);
  const maiorApp = Math.max(1, ...e.porApp.map((a) => a.palavras));
  const horaDePico = e.porHora.indexOf(Math.max(...e.porHora));

  const win = plataforma() === "windows";
  return (
    <>
      <Cabecalho
        titulo="Insights"
        apoio={`${plural(e.totalDitados, "ditado", "ditados")} em ${plural(e.diasAtivos, "dia", "dias")} de uso. Tudo calculado neste ${win ? "computador" : "Mac"}.`}
      />

      <StatGrid
        className="max-w-none"
        data={[
          { id: "palavras", label: "Palavras ditadas", value: compacto(e.totalPalavras), trend: e.porDia.map((d) => d.palavras) },
          { id: "ppm", label: "Palavras por minuto", value: numero(e.ppmMedio), hint: `${numero(e.ppmMedio / Estatisticas_ppmDigitando)} vezes a digitação média` },
          { id: "tempo", label: "Tempo falando", value: duracao(e.minutosFalados) },
          { id: "economia", label: "Tempo economizado", value: duracao(e.minutosEconomizados), hint: "contra digitar a 40 palavras por minuto" },
        ]}
      />

      <div className="mt-3">
        <ChartCard
          className="max-w-none"
          type="bar"
          title="Palavras por dia"
          subtitle={`Últimos ${dias} dias`}
          total={numero(noPeriodo)}
          partialLast
          format={(v) => compacto(v)}
          data={{ labels: e.porDia.map((d) => diaCurto(d.dia)), series: [{ id: "palavras", label: "Palavras", values: e.porDia.map((d) => d.palavras) }] }}
        />
        <div className="mt-2 flex justify-end">
          <Segmented label="Período do gráfico" value={dias} onValueChange={setDias} options={periodos} />
        </div>
      </div>

      <div className="mt-3 grid grid-cols-2 gap-3">
        <RadialDots
          title="Hora do dia"
          subtitle="Quando você mais dita"
          labels={Array.from({ length: 24 }, (_, h) => `${h}h`)}
          axis={[0, 6, 12, 18]}
          series={[{ id: "palavras", label: "Palavras", values: e.porHora }]}
          center={{ value: `${horaDePico}h`, caption: "pico" }}
          format={(v) => numero(v)}
        />
        {e.ppmDeCadaDitado.length >= 5 ? (
          <DotHistogram
            title="Velocidade da fala"
            subtitle="Cada ponto é um ditado"
            values={e.ppmDeCadaDitado}
            bins={32}
            unit="ditados"
            format={(v) => `${Math.round(v)}`}
          />
        ) : (
          <Shell>
            <ShellHeader title="Velocidade da fala" subtitle="Cada ponto é um ditado" />
            <Panel>
              <EmptyState compact title="Poucos ditados" description="A distribuição aparece a partir de cinco." />
            </Panel>
          </Shell>
        )}
      </div>

      <div className="mt-3 grid grid-cols-2 gap-3">
        <Shell>
          <ShellHeader title="Onde você dita" subtitle="Palavras por aplicativo" />
          <Panel className="flex flex-col gap-3.5 p-4">
            {e.porApp.slice(0, 6).map((a) => (
              <div key={a.app}>
                <div className="mb-1.5 flex items-baseline justify-between gap-3 text-[0.8125rem]">
                  <span className="truncate font-medium">{a.nome}</span>
                  <span className="shrink-0 text-muted-foreground tabular-nums">{numero(a.palavras)}</span>
                </div>
                <ProgressBar kind="meter" size="sm" tone="neutral" label={a.nome} value={a.palavras} max={maiorApp} valueText={`${numero(a.palavras)} palavras`} />
              </div>
            ))}
          </Panel>
        </Shell>
        <Shell>
          <ShellHeader title="Marcas" subtitle="Seus números de destaque" />
          <Panel className="p-4">
            <dl className="flex flex-col gap-2.5 text-[0.8125rem]">
              <Marca rotulo="Dias seguidos" valor={plural(e.diasSeguidos, "dia", "dias")} />
              <Marca rotulo="Maior sequência" valor={plural(e.maiorSequencia, "dia", "dias")} />
              <Marca rotulo="Maior ditado" valor={`${numero(e.maiorDitadoPalavras)} palavras em ${duracao(e.maiorDitadoMinutos)}`} />
              <Marca rotulo="Correções do dicionário" valor={numero(e.trocasDicionario)} />
              <Marca rotulo="Espera ao soltar a tecla" valor={e.latenciaMediana > 0 ? `${espera(e.latenciaMediana)} (mediana)` : "sem medição ainda"} />
            </dl>
          </Panel>
        </Shell>
      </div>
    </>
  );
}

const Estatisticas_ppmDigitando = 40;

function Marca({ rotulo, valor }: { rotulo: string; valor: string }) {
  return (
    <div className="flex items-baseline justify-between gap-4">
      <dt className="text-muted-foreground">{rotulo}</dt>
      <dd className="text-right font-medium tabular-nums">{valor}</dd>
    </div>
  );
}
