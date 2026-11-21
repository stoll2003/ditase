import { CheckCircle, Copy, DownloadSimple, FolderOpen, ShieldCheck, Warning, WarningCircle } from "@phosphor-icons/react";
import type { ReactNode } from "react";
import { useState } from "react";
import { Cabecalho } from "@/componentes/Cabecalho";
import { Button } from "@/components/ui/action-button";
import { Switch } from "@/components/ui/choice-controls";
import { Kbd } from "@/components/ui/key-hint";
import { ProgressBar } from "@/components/ui/progress-bar";
import { Slider } from "@/components/ui/range-slider";
import { Select } from "@/components/ui/select-field";
import { StatusPill } from "@/components/ui/status-pill";
import { Panel, Shell, ShellHeader } from "@/components/ui/surface";
import { toast } from "@/components/ui/toast-stack";
import { useDados } from "@/lib/dados";
import { tamanhoDeArquivo } from "@/lib/formato";
import { plataforma } from "@/lib/plataforma";
import { chamar, type Ajustes as DadosDosAjustes, type Estado } from "@/lib/ponte";

const SISTEMA = "__sistema__";

export function Ajustes({ estado }: { estado?: Estado }) {
  const { dados, recarregar } = useDados<DadosDosAjustes>("ajustes.ler", {}, ["estado"]);
  const [importando, setImportando] = useState(false);
  const [copiandoRelatorio, setCopiandoRelatorio] = useState(false);
  if (!dados) return <Cabecalho titulo="Ajustes" />;

  const win = plataforma() === "windows";
  const v = dados.valores;
  const definir = (chave: string, valor: string) => chamar("ajustes.definir", { chave, valor }).then(recarregar);
  const limpador = estado?.limpador;
  const modeloIA = estado?.modeloIA;
  const semIA = modeloIA?.estado === "indisponivel";

  const importar = async () => {
    setImportando(true);
    try {
      const r = await chamar<{ ditados: number; termos: number; encontrado: boolean }>("wispr.importar");
      if (!r.encontrado) toast.error(win ? "Não achei o Wispr Flow neste computador" : "Não achei o Wispr Flow neste Mac");
      else if (r.ditados === 0 && r.termos === 0) toast("Nada novo para importar");
      else toast.success(`Importados ${r.ditados} ditados e ${r.termos} termos`);
    } finally {
      setImportando(false);
    }
  };

  const copiarRelatorio = async () => {
    setCopiandoRelatorio(true);
    try {
      await chamar<string>("relatorio.copiar");
      toast.success("Relatório copiado");
    } catch {
      toast.error("Não foi possível copiar o relatório");
    } finally {
      setCopiandoRelatorio(false);
    }
  };

  return (
    <>
      <Cabecalho titulo="Ajustes" />
      <div className="flex flex-col gap-3">
        <Grupo titulo="Atalho" apoio="A tecla que você segura para ditar">
          <Linha>
            <Select
              label="Tecla de ditado"
              value={v.atalho}
              onValueChange={(valor) => definir("atalho", valor)}
              options={dados.teclas.map((t) => ({ value: t.id, label: t.nome }))}
            />
            {win && v.atalho === "altDireito" && (
              <p role="status" className="mt-2.5 flex items-center gap-2 px-1 text-[0.8125rem] text-warning">
                <Warning className="size-4 shrink-0" weight="fill" aria-hidden="true" />
                <span>No teclado brasileiro esta tecla é o AltGr, usada para digitar / e ?. Prefira o Ctrl direito.</span>
              </p>
            )}
          </Linha>
          <Linha>
            <dl className="grid grid-cols-[auto_1fr] items-center gap-x-4 gap-y-2 text-[0.8125rem]">
              <Gesto teclas={["segurar"]} texto="Grava enquanto a tecla está apertada e cola o texto ao soltar." />
              <Gesto teclas={["2 toques"]} texto="Mãos livres: grava até você tocar na tecla de novo. É o modo para ditados longos." />
              <Gesto teclas={["tecla", "Espaço"]} texto="Também entra em mãos livres, sem soltar a tecla." />
              <Gesto teclas={["Esc"]} texto="Cancela a gravação sem colar nada." />
              <Gesto teclas={win ? ["Ctrl", "Alt", "V"] : ["⌘", "⌃", "V"]} texto="Cola de novo o último ditado." />
            </dl>
          </Linha>
        </Grupo>

        <Grupo titulo="Microfone" apoio="De onde vem a sua voz">
          <Linha>
            <Select
              label="Entrada de áudio"
              value={v.microfone || SISTEMA}
              onValueChange={(valor) => definir("microfone", valor === SISTEMA ? "" : valor)}
              options={[{ value: SISTEMA, label: "Padrão do sistema" }, ...dados.microfones.map((m) => ({ value: m.uid, label: m.nome }))]}
            />
          </Linha>
        </Grupo>

        <Grupo
          titulo="Limpeza por IA"
          apoio="Um modelo local arruma pontuação, hesitações e autocorreções"
          lado={limpador && modeloIA && <SituacaoDaIA limpador={limpador} modelo={modeloIA} />}
        >
          {modeloIA && modeloIA.estado !== "instalado" && (
            <Linha>
              <DownloadDoModelo modelo={modeloIA} />
            </Linha>
          )}
          <Linha>
            <Switch
              label="Arrumar o texto com o modelo local"
              description="Desligado, o ditado usa só as regras e o dicionário: sai na hora, com a pontuação do reconhecedor."
              checked={v.limpezaIA === "1"}
              disabled={semIA}
              onCheckedChange={(ligado) => definir("limpezaIA", ligado ? "1" : "0")}
            />
          </Linha>
          <Linha>
            <Select
              label="Modelo"
              description="Arquivos .gguf na pasta de modelos. O de 2B deixa o texto pronto em cerca de um segundo; o de 4B pontua um pouco melhor, mas faz esperar vários segundos."
              value={v.modelo}
              disabled={v.limpezaIA !== "1" || semIA}
              onValueChange={(valor) => definir("modelo", valor)}
              options={(dados.modelos.includes(v.modelo) ? dados.modelos : [v.modelo, ...dados.modelos]).map((m) => ({
                value: m,
                label: m.replace(".gguf", ""),
                description: dados.modelos.includes(m) ? undefined : "arquivo não encontrado",
              }))}
            />
            <div className="mt-3">
              <Button icon={FolderOpen} onClick={() => chamar("abrir.pastaDeModelos")}>
                Abrir a pasta de modelos
              </Button>
            </div>
          </Linha>
        </Grupo>

        <Grupo titulo="Gravação" apoio="Limites para o modo mãos livres">
          <Linha>
            <Slider
              label="Duração máxima de um ditado"
              defaultValue={Number(v.limiteMinutos)}
              min={30}
              max={240}
              step={30}
              format={(n) => (n % 60 === 0 ? `${n / 60} h` : `${Math.floor(n / 60)} h ${n % 60} min`)}
              onValueCommit={(n) => definir("limiteMinutos", String(n))}
            />
          </Linha>
          <Linha>
            <Slider
              label="Encerrar depois de silêncio contínuo"
              defaultValue={Number(v.pararEmSilencioMinutos)}
              min={0}
              max={15}
              step={1}
              format={(n) => (n === 0 ? "nunca" : `${n} min`)}
              onValueCommit={(n) => definir("pararEmSilencioMinutos", String(n))}
            />
          </Linha>
          <Linha>
            <Select
              label="Guardar o áudio dos ditados"
              description={win ? "O áudio fica só neste computador e serve de reserva se a transcrição falhar." : "O áudio fica só neste Mac e serve de reserva se a transcrição falhar."}
              value={v.guardarAudioDias}
              onValueChange={(valor) => definir("guardarAudioDias", valor)}
              options={[
                { value: "0", label: "Não guardar" },
                { value: "1", label: "Por 1 dia" },
                { value: "7", label: "Por 7 dias" },
                { value: "30", label: "Por 30 dias" },
              ]}
            />
          </Linha>
        </Grupo>

        <Grupo titulo="Comportamento">
          <Linha>
            <Switch label="Sons ao começar e ao terminar" checked={v.sons === "1"} onCheckedChange={(l) => definir("sons", l ? "1" : "0")} />
          </Linha>
          <Linha>
            <Switch
              label={win ? "Mostrar a aba na borda da tela quando parado" : "Mostrar o botão flutuante quando parado"}
              description={win
                ? "Uma faixa fina na borda direita. Passe o mouse ou clique nela para ditar em mãos livres. Desligada, a aba só aparece durante o ditado."
                : "A marca pequena na base da tela. Passe o mouse e clique nela para ditar em mãos livres. Desligada, só aparece durante o ditado."}
              checked={v.pilulaSempreVisivel === "1"}
              onCheckedChange={(l) => definir("pilulaSempreVisivel", l ? "1" : "0")}
            />
          </Linha>
          <Linha>
            <Switch
              label="Espaço depois do texto colado"
              description="Evita que dois ditados seguidos fiquem grudados."
              checked={v.espacoNoFim === "1"}
              onCheckedChange={(l) => definir("espacoNoFim", l ? "1" : "0")}
            />
          </Linha>
        </Grupo>

        {estado && (
          <Grupo titulo="Permissões" apoio={win ? "O que o Windows precisa liberar" : "O que o macOS precisa liberar"}>
            <Permissao
              nome="Microfone"
              texto={win ? "Para ouvir a sua voz. Configurações > Privacidade e segurança > Microfone." : "Para ouvir a sua voz."}
              concedida={estado.permissoes.microfone === "concedida"}
              pedir={() => chamar("permissao.pedir", { qual: "microfone" })}
            />
            {!win && (
              <Permissao
                nome="Acessibilidade"
                texto="Para o atalho valer em qualquer app e o texto ser colado no cursor."
                concedida={estado.permissoes.acessibilidade === "concedida"}
                pedir={() => chamar("permissao.pedir", { qual: "acessibilidade" })}
              />
            )}
          </Grupo>
        )}

        {!win && estado?.wisprDisponivel && (
          <Grupo titulo="Wispr Flow" apoio="Trazer o que já existe neste Mac">
            <Linha className="flex items-center gap-4">
              <p className="min-w-0 flex-1 text-[0.8125rem] text-muted-foreground">
                Copia o histórico e o dicionário do Wispr Flow para cá. Nada é alterado no Wispr, e importar de novo não duplica.
              </p>
              <Button icon={DownloadSimple} loading={importando} onClick={importar}>
                Importar
              </Button>
            </Linha>
          </Grupo>
        )}

        {win && (
          <Grupo titulo="Teste">
            <Linha className="flex items-center gap-4">
              <p className="min-w-0 flex-1 text-[0.8125rem] text-muted-foreground">
                Copia a versão, o sistema e os últimos registros do app, sem o texto dos seus ditados, para colar na conversa com quem está testando.
              </p>
              <Button icon={Copy} loading={copiandoRelatorio} onClick={copiarRelatorio}>
                Copiar relatório de teste
              </Button>
            </Linha>
          </Grupo>
        )}

        {estado && (
          <p className="px-2 pt-2 text-[0.75rem] text-muted-foreground">
            Dita.se {estado.versao} · reconhecedor {estado.reconhecedor.nome} · tudo roda neste {win ? "computador" : "Mac"}, sem conta e sem nuvem.
          </p>
        )}
      </div>
    </>
  );
}

function Grupo({ titulo, apoio, lado, children }: { titulo: string; apoio?: string; lado?: ReactNode; children: ReactNode }) {
  return (
    <Shell>
      <ShellHeader as="h2" title={titulo} subtitle={apoio} aside={lado} />
      <div className="flex flex-col gap-1.5">{children}</div>
    </Shell>
  );
}

function Linha({ children, className }: { children: ReactNode; className?: string }) {
  return <Panel className={`p-4 ${className ?? ""}`}>{children}</Panel>;
}

function Gesto({ teclas, texto }: { teclas: string[]; texto: string }) {
  return (
    <>
      <dt className="flex gap-1">
        {teclas.map((t) => (
          <Kbd key={t}>{t}</Kbd>
        ))}
      </dt>
      <dd className="text-muted-foreground">{texto}</dd>
    </>
  );
}

function Permissao({ nome, texto, concedida, pedir }: { nome: string; texto: string; concedida: boolean; pedir: () => void }) {
  return (
    <Linha className="flex items-center gap-4">
      <div className="min-w-0 flex-1">
        <p className="text-[0.8125rem] font-medium">{nome}</p>
        <p className="text-[0.8125rem] text-muted-foreground">{texto}</p>
      </div>
      {concedida ? (
        <StatusPill tone="positive" icon={CheckCircle}>
          Concedida
        </StatusPill>
      ) : (
        <Button onClick={pedir}>Permitir</Button>
      )}
    </Linha>
  );
}

/** Download, progresso e falha do modelo padrão. Só aparece enquanto ele não está instalado. */
function DownloadDoModelo({ modelo, compacto = false }: { modelo: Estado["modeloIA"]; compacto?: boolean }) {
  const win = plataforma() === "windows";
  const feito = modelo.feito ?? 0;
  const total = tamanhoDeArquivo(modelo.tamanho);
  const baixar = () => void chamar("modelo.baixar");
  if (modelo.estado === "indisponivel") {
    return (
      <div className="text-[0.8125rem]">
        <p className="font-medium">{win ? "Indisponível neste computador" : "Indisponível neste Mac"}</p>
        <p className="mt-1 text-muted-foreground">
          {win
            ? "A IA local precisa do llama-server, que não veio com esta instalação. O ditado funciona normalmente, só com as regras e o dicionário."
            : "A IA local precisa do llama-server, que não veio com esta instalação nem está no Homebrew. O ditado funciona normalmente, só com as regras e o dicionário."}
        </p>
        {!win && (
          <p className="mt-2 text-muted-foreground">
            Para instalar pelo Terminal: <span className="selecionavel font-mono">brew install llama.cpp</span>
          </p>
        )}
      </div>
    );
  }
  if (modelo.estado === "baixando" || modelo.estado === "conferindo") {
    const conferindo = modelo.estado === "conferindo";
    const pct = Math.floor((feito / modelo.tamanho) * 100);
    return (
      <div className="text-[0.8125rem]">
        <div className="mb-2 flex items-baseline gap-2">
          <p className="font-medium">{conferindo ? "Conferindo o arquivo" : "Baixando o modelo"}</p>
          <p className="ml-auto text-muted-foreground tabular-nums" aria-hidden="true">
            {conferindo ? "quase pronto" : `${tamanhoDeArquivo(feito)} de ${total} · ${pct}%`}
          </p>
        </div>
        <ProgressBar
          size="sm"
          label={conferindo ? "Conferindo o modelo baixado" : "Download do modelo"}
          value={conferindo ? 100 : pct}
          striped={conferindo}
          valueText={conferindo ? "Conferindo" : `${tamanhoDeArquivo(feito)} de ${total}`}
        />
        <div className="mt-3 flex items-center gap-3">
          <p className="min-w-0 flex-1 text-muted-foreground">
            {conferindo ? "Comparando com a assinatura do original antes de usar." : "Pode continuar ditando enquanto baixa."}
          </p>
          {!conferindo && (
            <Button variant="quiet" onClick={() => void chamar("modelo.cancelar")}>
              Cancelar
            </Button>
          )}
        </div>
      </div>
    );
  }
  const retomar = feito > 0;
  const rotulo = retomar ? `Continuar download (faltam ${tamanhoDeArquivo(modelo.tamanho - feito)})` : `Baixar modelo (${total})`;
  return (
    <div className={`flex gap-4 text-[0.8125rem] ${compacto ? "items-center" : "flex-col sm:flex-row sm:items-center"}`}>
      <div className="min-w-0 flex-1">
        <p className="font-medium">Modelo não instalado</p>
        {modelo.estado === "falhou" ? (
          <p role="alert" className="mt-1 text-negative">
            {modelo.detalhe}
          </p>
        ) : (
          <p className="mt-1 text-muted-foreground">
            O {modelo.arquivo.replace(/-Q4_K_M\.gguf$/, "").replace("-", " ")} roda neste {win ? "computador" : "Mac"}, sem nuvem: arruma o texto do ditado e resume reuniões. Baixa uma vez só.
          </p>
        )}
      </div>
      <Button variant="primary" icon={DownloadSimple} onClick={baixar}>
        {modelo.estado === "falhou" ? "Tentar de novo" : rotulo}
      </Button>
    </div>
  );
}

function SituacaoDaIA({ limpador, modelo }: { limpador: Estado["limpador"]; modelo: Estado["modeloIA"] }) {
  switch (modelo.estado) {
    case "indisponivel":
      return (
        <StatusPill tone="warning" icon={WarningCircle}>
          {plataforma() === "windows" ? "Indisponível neste computador" : "Indisponível neste Mac"}
        </StatusPill>
      );
    case "ausente":
    case "falhou":
      return (
        <StatusPill tone="warning" icon={WarningCircle}>
          Modelo não instalado
        </StatusPill>
      );
    case "baixando":
      return <StatusPill tone="brand">Baixando {Math.floor(((modelo.feito ?? 0) / modelo.tamanho) * 100)}%</StatusPill>;
    case "conferindo":
      return (
        <StatusPill tone="brand" icon={ShieldCheck}>
          Conferindo
        </StatusPill>
      );
    default:
      return <SituacaoDoLimpador limpador={limpador} />;
  }
}

function SituacaoDoLimpador({ limpador }: { limpador: Estado["limpador"] }) {
  if (!limpador.ligado) return <StatusPill tone="neutral">Desligada</StatusPill>;
  switch (limpador.estado) {
    case "pronto":
      return (
        <StatusPill tone="positive" icon={CheckCircle}>
          Pronta
        </StatusPill>
      );
    case "iniciando":
      return <StatusPill tone="brand">Carregando o modelo</StatusPill>;
    case "desligado":
      return <StatusPill tone="neutral">Em repouso</StatusPill>;
    case "semBinario":
      return (
        <StatusPill tone="warning" icon={WarningCircle}>
          Falta o llama-server
        </StatusPill>
      );
    case "semModelo":
      return (
        <StatusPill tone="warning" icon={WarningCircle}>
          Modelo não encontrado
        </StatusPill>
      );
    default:
      return (
        <StatusPill tone="negative" icon={WarningCircle}>
          Com erro
        </StatusPill>
      );
  }
}
