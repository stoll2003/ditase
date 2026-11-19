export type Plataforma = "windows" | "mac";

/**
 * Identifica a plataforma da interface: "windows" no app Tauri ou navegador Windows
 * (ou com `?plataforma=windows` na demonstração), e "mac" como padrão.
 */
export function plataforma(): Plataforma {
  if (typeof window === "undefined") return "mac";
  const parametro = new URLSearchParams(window.location.search).get("plataforma");
  if (parametro === "windows") return "windows";
  if (parametro === "mac") return "mac";
  if ((window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ != null) return "windows";
  if (typeof navigator !== "undefined" && navigator.userAgent.includes("Windows")) return "windows";
  return "mac";
}
