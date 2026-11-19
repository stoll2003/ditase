import { useCallback, useEffect, useRef, useState } from "react";
import { chamar, ouvir } from "./ponte";

/**
 * Busca um dado do app e o mantém atualizado: refaz a busca quando os parâmetros mudam
 * e quando o app avisa que um dos `assuntos` mudou.
 */
export function useDados<T>(metodo: string, params: Record<string, unknown> = {}, assuntos: string[] = [], mesclar?: (novo: T, anterior?: T) => T) {
  const [dados, setDados] = useState<T | undefined>(undefined);
  const [erro, setErro] = useState<string | undefined>(undefined);
  const chave = JSON.stringify(params);
  const pedido = useRef(0);

  const recarregar = useCallback(() => {
    const meu = ++pedido.current;
    chamar<T>(metodo, JSON.parse(chave))
      .then((resposta) => {
        // Resposta de um pedido antigo não sobrescreve a do mais novo.
        if (meu === pedido.current) {
          setDados((anterior) => mesclar ? mesclar(resposta, anterior) : resposta);
          setErro(undefined);
        }
      })
      .catch((e) => meu === pedido.current && setErro(String(e)));
  }, [metodo, chave, mesclar]);

  useEffect(recarregar, [recarregar]);
  const lista = assuntos.join(",");
  useEffect(() => ouvir(lista ? lista.split(",") : [], recarregar), [lista, recarregar]);

  return { dados, erro, recarregar, setDados };
}

export function useHash(padrao: string): [string, (pagina: string) => void] {
  // Aceita "#/arquivos", "#arquivos" e "#/arquivos/": todos abrem a mesma página.
  const ler = () => location.hash.replace(/^#\/?/, "").replace(/\/+$/, "").toLowerCase() || padrao;
  const [pagina, setPagina] = useState(ler);
  useEffect(() => {
    const tratar = () => setPagina(ler());
    window.addEventListener("hashchange", tratar);
    return () => window.removeEventListener("hashchange", tratar);
  });
  return [pagina, (p) => (location.hash = `#/${p}`)];
}
