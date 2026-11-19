import { motion, useReducedMotion } from "motion/react";
import { type CSSProperties, type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { FOLGA_DA_LISTA, instant, linha } from "@/lib/motion-tokens";

/** Como a linha aparece: em cascata (lista que carrega), crescendo (item novo) ou sem efeito (resultado de busca). */
export type EntradaDaLinha = "lista" | "nova" | "nenhuma";

/**
 * Linha de uma lista (`<li>`). Entra em cascata na carga da lista, cresce quando é um item novo e
 * recolhe antes de sumir quando o usuário apaga. A ação de apagar só roda em `aoSair`, depois
 * do recolhimento, para a linha não desaparecer no meio do movimento.
 */
export function LinhaAnimada({
  entrada = "lista",
  indice = 0,
  saindo = false,
  aoSair,
  children,
}: {
  entrada?: EntradaDaLinha;
  indice?: number;
  saindo?: boolean;
  aoSair?: () => void;
  children: ReactNode;
}) {
  const reduzido = useReducedMotion();
  const [nova] = useState(entrada === "nova");
  const [entrando, setEntrando] = useState(nova);
  const recolhida = { opacity: 0, height: 0, marginTop: FOLGA_DA_LISTA };

  return (
    <motion.li
      className={entrada === "lista" ? "item-entra" : undefined}
      style={{ "--i": Math.min(indice, 8), overflow: saindo || entrando ? "hidden" : undefined } as CSSProperties}
      initial={nova ? recolhida : false}
      animate={saindo ? recolhida : { opacity: 1, height: "auto", marginTop: 0 }}
      transition={reduzido ? instant : linha}
      onAnimationComplete={() => {
        setEntrando(false);
        if (saindo) aoSair?.();
      }}
    >
      {children}
    </motion.li>
  );
}

/**
 * Ids que apareceram depois da primeira carga da lista. Troca de busca recomeça a contagem, para o
 * resultado de uma busca não contar como item novo. Em StrictMode o cálculo é repetível: os ids
 * só entram em `vistos` depois que a tela foi montada.
 */
export function useNovos(ids: string[] | undefined, chave: string): Set<string> {
  const estado = useRef<{ chave: string; vistos: Set<string> } | null>(null);
  const novos = useMemo(() => {
    if (!ids) return new Set<string>();
    if (!estado.current || estado.current.chave !== chave) {
      estado.current = { chave, vistos: new Set(ids) };
      return new Set<string>();
    }
    const vistos = estado.current.vistos;
    return new Set(ids.filter((id) => !vistos.has(id)));
  }, [ids, chave]);
  useEffect(() => {
    ids?.forEach((id) => estado.current?.vistos.add(id));
  }, [ids]);
  return novos;
}
