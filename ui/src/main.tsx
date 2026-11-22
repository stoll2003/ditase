import "@fontsource-variable/inter";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App.tsx";
import { plataforma } from "./lib/plataforma";
import "./index.css";

const raiz = document.documentElement;
raiz.dataset.plataforma = plataforma();

// Campos e menus podem receber :focus-visible mesmo no clique.
if (raiz.dataset.plataforma === "windows") {
  window.addEventListener("pointerdown", () => { raiz.dataset.entrada = "ponteiro"; }, true);
  window.addEventListener("keydown", (evento) => {
    if (evento.key === "Tab" || evento.key.startsWith("Arrow")) delete raiz.dataset.entrada;
  }, true);
}

// O tema acompanha o sistema. `?tema=claro|escuro` força um deles (usado na conferência visual).
const forcado = new URLSearchParams(location.search).get("tema");
const escuro = window.matchMedia("(prefers-color-scheme: dark)");
const aplicarTema = () =>
  document.documentElement.classList.toggle("dark", forcado ? forcado === "escuro" : escuro.matches);
aplicarTema();
escuro.addEventListener("change", aplicarTema);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
