import "@fontsource-variable/inter";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App.tsx";
import { plataforma } from "./lib/plataforma";
import "./index.css";

document.documentElement.dataset.plataforma = plataforma();

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
