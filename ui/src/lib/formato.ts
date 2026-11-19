const inteiro = new Intl.NumberFormat("pt-BR");
const decimal = new Intl.NumberFormat("pt-BR", { maximumFractionDigits: 1 });

export function numero(n: number): string {
  return inteiro.format(Math.round(n));
}

/** 12.345 vira "12,3 mil"; abaixo de dez mil fica por extenso. */
export function compacto(n: number): string {
  if (n >= 1_000_000) return `${decimal.format(n / 1_000_000)} mi`;
  if (n >= 10_000) return `${decimal.format(n / 1000)} mil`;
  return numero(n);
}

/** Minutos em "45 min", "2 h 10 min" ou "38 s". */
export function duracao(minutos: number): string {
  if (minutos < 1) return `${Math.round(minutos * 60)} s`;
  if (minutos < 60) return `${Math.round(minutos)} min`;
  const h = Math.floor(minutos / 60);
  const m = Math.round(minutos - h * 60);
  return m === 0 ? `${h} h` : `${h} h ${m} min`;
}

export function segundos(s: number): string {
  if (s < 60) return `${Math.round(s)} s`;
  const m = Math.floor(s / 60);
  const r = Math.round(s - m * 60);
  return r === 0 ? `${m} min` : `${m} min ${r} s`;
}

export function espera(s: number): string {
  return `${decimal.format(s)} s`;
}

export function hora(epoch: number): string {
  return new Date(epoch * 1000).toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" });
}

/** "Hoje", "Ontem" ou "sexta, 13 de novembro". */
export function dia(epoch: number): string {
  const data = new Date(epoch * 1000);
  const hoje = new Date();
  const inicio = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const diferenca = Math.round((inicio(hoje) - inicio(data)) / 86_400_000);
  if (diferenca === 0) return "Hoje";
  if (diferenca === 1) return "Ontem";
  const texto = data.toLocaleDateString("pt-BR", {
    weekday: "long",
    day: "numeric",
    month: "long",
    ...(data.getFullYear() !== hoje.getFullYear() ? { year: "numeric" } : {}),
  });
  return texto.charAt(0).toUpperCase() + texto.slice(1);
}

/** "2026-11-15" vira "15/11". */
export function diaCurto(iso: string): string {
  const [, m, d] = iso.split("-");
  return `${d}/${m}`;
}

export function plural(n: number, um: string, varios: string): string {
  return `${numero(n)} ${n === 1 ? um : varios}`;
}

/** Tamanho em bytes no formato do Finder: "1,3 GB", "820 MB". */
export function tamanhoDeArquivo(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toLocaleString("pt-BR", { maximumFractionDigits: 1 })} GB`;
  return `${Math.max(1, Math.round(bytes / 1e6)).toLocaleString("pt-BR")} MB`;
}
