import { BookBookmark, PencilSimple, Plus, Trash } from "@phosphor-icons/react";
import { useEffect, useMemo, useState } from "react";
import { Cabecalho } from "@/componentes/Cabecalho";
import { type EntradaDaLinha, LinhaAnimada, useNovos } from "@/componentes/Movimento";
import { Button, IconButton } from "@/components/ui/action-button";
import { EmptyState } from "@/components/ui/empty-state";
import { Dialog } from "@/components/ui/modal-sheet";
import { Tag } from "@/components/ui/status-pill";
import { Inset } from "@/components/ui/surface";
import { SearchField, TextField } from "@/components/ui/text-field";
import { toast } from "@/components/ui/toast-stack";
import { useDados } from "@/lib/dados";
import { plural } from "@/lib/formato";
import { chamar, type Termo } from "@/lib/ponte";

type Rascunho = { id: number; termo: string; formas: string };

export function Dicionario() {
  const { dados: termos, recarregar } = useDados<Termo[]>("dicionario.listar", {}, ["dicionario"]);
  const [busca, setBusca] = useState("");
  const [rascunho, setRascunho] = useState<Rascunho | undefined>();
  const [erro, setErro] = useState("");

  const filtro = busca.trim().toLowerCase();
  const visiveis = (termos ?? []).filter(
    (t) => !filtro || t.termo.toLowerCase().includes(filtro) || t.ouvidoComo.some((f) => f.toLowerCase().includes(filtro)),
  );

  const novos = useNovos(termos?.map((t) => String(t.id)), busca);
  // Termo editado: o fundo acende como num termo novo.
  const [editado, setEditado] = useState<number | null>(null);
  useEffect(() => {
    if (editado === null) return;
    const id = setTimeout(() => setEditado(null), 1600);
    return () => clearTimeout(id);
  }, [editado]);
  const ordem = useMemo(() => new Map((termos ?? []).map((t, i) => [t.id, i])), [termos]);

  const abrir = (t?: Termo) => {
    setErro("");
    setRascunho(t ? { id: t.id, termo: t.termo, formas: t.ouvidoComo.join(", ") } : { id: 0, termo: "", formas: "" });
  };

  const salvar = async () => {
    if (!rascunho) return;
    const termo = rascunho.termo.trim();
    if (!termo) return setErro("Escreva o termo como ele deve sair.");
    const repetido = (termos ?? []).find((t) => t.id !== rascunho.id && t.termo.toLowerCase() === termo.toLowerCase());
    if (repetido && rascunho.id !== 0) return setErro("Já existe um termo com esse nome.");
    const ouvidoComo = rascunho.formas.split(",").map((f) => f.trim()).filter(Boolean);
    try {
      await chamar("dicionario.salvar", { ...(rascunho.id ? { id: rascunho.id } : {}), termo, ouvidoComo });
      setRascunho(undefined);
      if (rascunho.id) setEditado(rascunho.id);
      recarregar();
      toast.success(rascunho.id ? "Termo atualizado" : `"${termo}" entrou no dicionário`);
    } catch (e) {
      setErro(String(e));
    }
  };

  const apagar = async (t: Termo) => {
    await chamar("dicionario.apagar", { id: t.id });
    recarregar();
    toast(`"${t.termo}" saiu do dicionário`, {
      action: {
        label: "Desfazer",
        onClick: () => chamar("dicionario.salvar", { termo: t.termo, ouvidoComo: t.ouvidoComo }).then(recarregar),
      },
    });
  };

  return (
    <>
      <Cabecalho
        titulo="Dicionário"
        apoio="Nomes, siglas e jargões que o reconhecedor erra. O que estiver aqui sai sempre com esta grafia."
        lado={
          <Button variant="primary" size="md" icon={Plus} iconWeight="bold" onClick={() => abrir()}>
            Adicionar termo
          </Button>
        }
      />

      {termos && termos.length > 0 && (
        <div className="mb-4 w-72">
          <SearchField aria-label="Buscar no dicionário" placeholder="Buscar termo" value={busca} onChange={(e) => setBusca(e.target.value)} onClear={() => setBusca("")} />
        </div>
      )}

      {termos === undefined ? null : termos.length === 0 ? (
        <EmptyState
          icon={BookBookmark}
          title="Nenhum termo ainda"
          description="Quando uma palavra sair errada num ditado, cadastre aqui a grafia certa e, se quiser, como ela costuma sair."
          action={{ label: "Adicionar termo", onClick: () => abrir(), icon: Plus, iconWeight: "bold" }}
        />
      ) : visiveis.length === 0 ? (
        <EmptyState compact title={`Nada encontrado para "${busca}"`} />
      ) : (
        <ul className="flex flex-col gap-1.5">
          {visiveis.map((t) => (
            <Linha
              key={t.id}
              termo={t}
              indice={ordem.get(t.id) ?? 0}
              entrada={novos.has(String(t.id)) ? "nova" : busca ? "nenhuma" : "lista"}
              destaque={editado === t.id}
              editar={() => abrir(t)}
              apagar={() => apagar(t)}
            />
          ))}
        </ul>
      )}

      <Dialog
        title={rascunho?.id ? "Editar termo" : "Adicionar termo"}
        open={rascunho !== undefined}
        onOpenChange={(aberto) => !aberto && setRascunho(undefined)}
        footer={
          <>
            <Button variant="quiet" onClick={() => setRascunho(undefined)}>
              Cancelar
            </Button>
            <Button variant="primary" onClick={salvar}>
              Salvar
            </Button>
          </>
        }
      >
        <form
          className="flex flex-col gap-4 p-4"
          onSubmit={(e) => {
            e.preventDefault();
            salvar();
          }}
        >
          <TextField
            label="Como deve sair"
            placeholder="Claude"
            autoFocus
            value={rascunho?.termo ?? ""}
            error={erro || undefined}
            onChange={(e) => rascunho && setRascunho({ ...rascunho, termo: e.target.value })}
          />
          <TextField
            label="Como costuma sair errado (opcional)"
            description="Separe por vírgula. Use quando o erro é uma palavra que existe, como Cloud no lugar de Claude."
            placeholder="Cloud, Clode"
            value={rascunho?.formas ?? ""}
            onChange={(e) => rascunho && setRascunho({ ...rascunho, formas: e.target.value })}
          />
          <button type="submit" hidden />
        </form>
      </Dialog>
    </>
  );
}

function Linha({
  termo: t,
  indice,
  entrada,
  destaque,
  editar,
  apagar,
}: {
  termo: Termo;
  indice: number;
  entrada: EntradaDaLinha;
  destaque: boolean;
  editar: () => void;
  apagar: () => void;
}) {
  const [saindo, setSaindo] = useState(false);
  // O fundo acende em termo novo ou recém-editado; `key` reinicia o efeito a cada edição.
  const acende = entrada === "nova" || destaque;
  return (
    // A linha recolhe primeiro; o termo só é apagado quando ela terminou de recolher.
    <LinhaAnimada entrada={entrada} indice={indice} saindo={saindo} aoSair={apagar}>
      <Inset key={destaque ? "editado" : "normal"} className={`flex items-center gap-3 py-2.5 pr-2 pl-4${acende ? " item-destaque" : ""}`}>
        <div className="min-w-0 flex-1">
          <p className="selecionavel truncate text-[0.875rem] font-medium">{t.termo}</p>
          {t.ouvidoComo.length > 0 && (
            <p className="mt-1 flex flex-wrap items-center gap-1.5 text-[0.75rem] text-muted-foreground">
              costuma sair como
              {t.ouvidoComo.map((f) => (
                <Tag key={f}>{f}</Tag>
              ))}
            </p>
          )}
        </div>
        <span className="shrink-0 text-[0.75rem] text-muted-foreground tabular-nums">
          {t.usos > 0 ? plural(t.usos, "correção", "correções") : "ainda sem uso"}
        </span>
        <div className="flex shrink-0 gap-0.5">
          <IconButton icon={PencilSimple} label={`Editar ${t.termo}`} variant="quiet" tooltip="Editar" onClick={editar} />
          <IconButton icon={Trash} label={`Apagar ${t.termo}`} variant="quiet" tooltip="Apagar" onClick={() => setSaindo(true)} />
        </div>
      </Inset>
    </LinhaAnimada>
  );
}
