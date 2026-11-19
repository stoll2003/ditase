import type { ReactNode } from "react";

/** Título da página, uma linha de apoio e, à direita, a ação principal. */
export function Cabecalho({ titulo, apoio, lado }: { titulo: string; apoio?: ReactNode; lado?: ReactNode }) {
  return (
    <header className="mb-7 flex items-end justify-between gap-6">
      <div className="min-w-0">
        <h1 className="text-[1.375rem] font-semibold tracking-tight">{titulo}</h1>
        {apoio && <p className="mt-1 text-[0.8125rem] text-muted-foreground">{apoio}</p>}
      </div>
      {lado && <div className="flex shrink-0 items-center gap-2">{lado}</div>}
    </header>
  );
}

/** Título de seção dentro da página. */
export function Secao({ titulo, lado, children }: { titulo: string; lado?: ReactNode; children: ReactNode }) {
  return (
    <section className="mt-9">
      <div className="mb-3 flex h-8 items-center justify-between gap-4">
        <h2 className="text-[0.9375rem] font-medium">{titulo}</h2>
        {lado}
      </div>
      {children}
    </section>
  );
}
