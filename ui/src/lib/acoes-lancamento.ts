import { toast } from "@/components/ui/toast-stack";
import { chamar } from "./ponte";

export async function executar(acao: () => Promise<unknown>, sucesso?: string) {
  try { await acao(); if (sucesso) toast.success(sucesso); }
  catch { toast.error("Não foi possível concluir esta ação. Tente novamente."); }
}
export const acaoLocal = (acao: string, params: Record<string, unknown> = {}, sucesso?: string) =>
  executar(() => chamar("lancamento." + acao, params), sucesso);
