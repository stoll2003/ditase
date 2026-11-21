import { toast } from "@/components/ui/toast-stack";
import { chamar } from "./ponte";

export async function executar(acao: () => Promise<unknown>, sucesso?: string) {
  try { await acao(); if (sucesso) toast.success(sucesso); }
  catch (erro) {
    const description = erro instanceof Error ? erro.message : typeof erro === "string" ? erro : undefined;
    toast.error("Não foi possível concluir esta ação.", { description });
  }
}
export const acaoLocal = (acao: string, params: Record<string, unknown> = {}, sucesso?: string) =>
  executar(() => chamar("lancamento." + acao, params), sucesso);
