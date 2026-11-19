const $ = (id) => document.getElementById(id);
const pilula = document.querySelector(".pilula");
const barras = $("barras");
const N = 14;
for (let i = 0; i < N; i++) barras.appendChild(document.createElement("i"));
const niveis = Array(N).fill(0);
let atalho = "";
let avisoAte = 0;

const invocar = (acao) => {
  const t = window.__TAURI__?.core ?? window.__TAURI_INTERNALS__;
  t?.invoke("pilula_acao", { acao });
};
$("microfone").onclick = () => invocar("alternar");
$("cancelar").onclick = () => invocar("cancelar");
$("parar").onclick = () => invocar("parar");

function mostrar(fase, texto) {
  pilula.dataset.fase = fase;
  const gravando = fase === "gravando" || fase === "maosLivres";
  $("microfone").hidden = fase !== "ociosa" || !!texto;
  $("barras").hidden = !gravando;
  $("tempo").hidden = !gravando;
  $("giro").hidden = fase !== "processando";
  $("texto").hidden = !texto && fase !== "processando";
  $("texto").textContent = texto ?? (fase === "processando" ? "Transcrevendo…" : "");
  $("cancelar").hidden = !gravando;
  $("parar").hidden = fase !== "maosLivres";
  $("microfone").title = atalho ? `Segure ${atalho} para ditar. Clique para mãos livres.` : "";
}

window.pilula = (m) => {
  switch (m.tipo) {
    case "fase":
      if (m.fase !== "ociosa") avisoAte = 0;
      if (Date.now() < avisoAte && m.fase === "ociosa") return;
      if (m.fase === "gravando" || m.fase === "maosLivres") { niveis.fill(0); $("tempo").textContent = "0:00"; }
      mostrar(m.fase);
      break;
    case "aviso":
      avisoAte = Date.now() + m.segundos * 1000;
      mostrar("ociosa", m.texto);
      setTimeout(() => { if (Date.now() >= avisoAte) mostrar("ociosa"); }, m.segundos * 1000 + 20);
      break;
    case "nivel":
      niveis.shift(); niveis.push(m.nivel);
      barras.querySelectorAll("i").forEach((b, i) => { b.style.height = `${3 + niveis[i] * 19}px`; });
      break;
    case "segundos":
      $("tempo").textContent = `${Math.floor(m.segundos / 60)}:${String(m.segundos % 60).padStart(2, "0")}`;
      break;
    case "atalho":
      atalho = m.simbolo; mostrar(pilula.dataset.fase);
      break;
    case "progresso":
      if (pilula.dataset.fase === "processando" && m.progresso?.etapa) $("texto").textContent = m.progresso.etapa;
      break;
  }
};
mostrar("ociosa");
