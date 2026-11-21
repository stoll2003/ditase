import { Button } from "@/components/ui/action-button";

export function ErroDeLeitura({ erro, recarregar }: { erro: string; recarregar: () => void }) {
  return <div className="mb-4">
    <p role="alert" className="mb-2 text-sm text-warning">Não foi possível carregar os dados. {erro}</p>
    <Button size="sm" variant="quiet" onClick={recarregar}>Tentar carregar de novo</Button>
  </div>;
}
