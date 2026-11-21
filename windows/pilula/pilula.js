// Aba na borda direita do Windows (SPEC-windows.md §3 e MOTION-windows.md).
// Repouso com 0% de CPU: nenhum rAF, setInterval ou animação CSS ativa fora de gravação/processamento.
const $ = (id) => document.getElementById(id);

const temaParam = new URLSearchParams(location.search).get("tema");
if (temaParam === "claro" || temaParam === "escuro") {
  document.documentElement.dataset.tema = temaParam;
}

const palco = $("palco");
const ponta = $("ponta");
const casca = $("casca");
const contorno = $("contorno");
const contornoPath = $("contorno-path");
const leitor = $("leitor");

const mqReduzido = window.matchMedia("(prefers-reduced-motion: reduce)");
const mqAltoContraste = window.matchMedia("(forced-colors: active)");

const invocar = (acao) => {
  const t = window.__TAURI__?.core ?? window.__TAURI_INTERNALS__;
  t?.invoke("pilula_acao", { acao });
};

let fase = "ociosa"; // ociosa | gravando | maosLivres | processando
let atalho = "Ctrl dir.";
let borda = "direita"; // direita | esquerda
let ponteiroSobre = false;
let acenoAtivo = false;
let hoverAberto = false;
let etapa = "Transcrevendo";
let progressoAtivo = false;
let progressoReal = null; // 0..1 ou null
let baixando = null; // { feito, total, pct } ou null
let aviso = null; // { texto, nivel } ou null
let avisoTimer = null;
let abrirTimer = null;
let fecharTimer = null;
let modoAtual = "repouso";
let encolherTimer = null;

// Barras da voz: 3 (compacta) e 9 (aberta / mãos livres)
const barras3 = Array.from($("barras-3").children);
const barras9a = Array.from($("barras-9a").children);
const barras9m = Array.from($("barras-9m").children);
const PESOS9 = [0.48, 0.62, 0.78, 0.92, 1, 0.92, 0.78, 0.62, 0.48];
const PESOS3 = [0.78, 1, 0.78];
const historico = Array(5).fill(0);
const atual9 = Array(9).fill(0.18);
const atual3 = Array(3).fill(0.18);
const ultimoEscrito9a = Array(9).fill(-1);
const ultimoEscrito9m = Array(9).fill(-1);
const ultimoEscrito3 = Array(3).fill(-1);
let nivelAlvo = 0;
let rafId = 0;
let ultimoQuadro = 0;
let tempoInicio = 0;

function anunciar(texto) {
  if (!texto) return;
  leitor.textContent = "";
  requestAnimationFrame(() => { leitor.textContent = texto; });
}

function limparTimersHover() {
  if (abrirTimer) { clearTimeout(abrirTimer); abrirTimer = null; }
  if (fecharTimer) { clearTimeout(fecharTimer); fecharTimer = null; }
}

function modoDesejado(estadoVisual) {
  if (estadoVisual === "repouso" || estadoVisual === "aceno") return "repouso";
  if (estadoVisual === "gravandoCompacta" || estadoVisual === "processandoCompacta" || estadoVisual === "baixandoCompacta") {
    return "compacta";
  }
  return "aberta";
}

function ordemModo(m) {
  return m === "repouso" ? 0 : m === "compacta" ? 1 : 2;
}

function sincronizarJanela(proximoModo) {
  if (encolherTimer) { clearTimeout(encolherTimer); encolherTimer = null; }
  if (proximoModo === modoAtual) return;
  if (ordemModo(proximoModo) > ordemModo(modoAtual)) {
    // Crescer a janela ANTES de animar (MOTION-windows.md §6).
    modoAtual = proximoModo;
    invocar(`modo:${proximoModo}`);
  } else {
    // Encolher só no fim da transição (ou logo em movimento reduzido).
    const alvo = proximoModo;
    const aplicarEncolher = () => {
      if (modoDesejado(calcularEstado().id) === alvo && modoAtual !== alvo) {
        modoAtual = alvo;
        invocar(`modo:${alvo}`);
      }
    };
    if (mqReduzido.matches) {
      encolherTimer = setTimeout(aplicarEncolher, 140);
    } else {
      const onEnd = (e) => {
        if (e.propertyName === "transform") {
          casca.removeEventListener("transitionend", onEnd);
          aplicarEncolher();
        }
      };
      casca.addEventListener("transitionend", onEnd);
      encolherTimer = setTimeout(() => {
        casca.removeEventListener("transitionend", onEnd);
        aplicarEncolher();
      }, 240);
    }
  }
}

function larguraAviso(texto) {
  const linhas = texto.split(/\n/);
  const maior = Math.max(...linhas.map((l) => l.length));
  return Math.max(120, Math.min(240, Math.round(maior * 7.2 + 54)));
}

function calcularEstado() {
  if (aviso) {
    return {
      id: aviso.nivel === "erro" ? "erro" : "aviso",
      visivel: larguraAviso(aviso.texto),
      vista: "v-aviso",
    };
  }
  if (fase === "maosLivres") {
    return { id: "maosLivres", visivel: 232, vista: "v-maos" };
  }
  if (fase === "gravando") {
    return hoverAberto
      ? { id: "gravandoAberta", visivel: 200, vista: "v-grav-aberta" }
      : { id: "gravandoCompacta", visivel: 32, vista: "v-grav-comp" };
  }
  if (fase === "processando" || (fase === "ociosa" && progressoAtivo)) {
    if (hoverAberto) {
      const textoEtapa = typeof etapa === "string" && etapa ? etapa : "Transcrevendo";
      const w = Math.max(200, Math.min(240, Math.round(textoEtapa.length * 7.5 + 64)));
      return { id: "processandoAberta", visivel: w, vista: "v-proc-aberta" };
    }
    return { id: "processandoCompacta", visivel: 32, vista: "v-proc-comp" };
  }
  if (baixando) {
    return hoverAberto
      ? { id: "baixandoAberta", visivel: 240, vista: "v-baix-aberta" }
      : { id: "baixandoCompacta", visivel: 32, vista: "v-baix-comp" };
  }
  if (hoverAberto) {
    return { id: "ociosaAberta", visivel: 176, vista: "v-ociosa" };
  }
  if (acenoAtivo) {
    return { id: "aceno", visivel: 6, vista: null };
  }
  return { id: "repouso", visivel: 4, vista: null };
}

function atualizarContorno(est) {
  let pct = null;
  if (est.id === "baixandoCompacta" || est.id === "baixandoAberta") {
    pct = baixando ? baixando.pct / 100 : null;
  } else if (est.id === "processandoCompacta" || est.id === "processandoAberta") {
    pct = progressoReal;
  }
  if (pct == null || Number.isNaN(pct)) {
    contorno.classList.remove("ativo", "brilho", "erro");
    return;
  }
  const v = Math.max(0, Math.min(1, pct));
  // Ajusta o path em C para a largura visível atual.
  const xEsq = borda === "direita" ? 1 : 240 - est.visivel + 1;
  const xDir = borda === "direita" ? est.visivel : 239;
  const d = borda === "direita"
    ? `M ${xDir} 1 H 14 A 13 13 0 0 0 1 14 V 42 A 13 13 0 0 0 14 55 H ${xDir}`
    : `M ${xEsq} 1 H ${xDir - 13} A 13 13 0 0 1 ${xDir} 14 V 42 A 13 13 0 0 1 ${xDir - 13} 55 H ${xEsq}`;
  contornoPath.setAttribute("d", d);
  contornoPath.style.strokeDashoffset = String(1 - v);
  contorno.classList.add("ativo");
}

function renderizar() {
  const anteriorVisivel = parseFloat(getComputedStyle(palco).getPropertyValue("--visivel")) || 4;
  const est = calcularEstado();
  const proximoModo = modoDesejado(est.id);
  sincronizarJanela(proximoModo);

  palco.dataset.estado = est.id;
  palco.dataset.borda = borda;
  palco.dataset.recolhendo = est.visivel < anteriorVisivel ? "1" : "0";
  palco.style.setProperty("--visivel", `${est.visivel}px`);

  for (const v of document.querySelectorAll(".vista")) {
    v.classList.toggle("ativa", v.id === est.vista);
  }

  $("chip-atalho").textContent = atalho;
  ponta.title = atalho
    ? `Dita.se: clique para ditar. Segure ${atalho} para ditar sem clicar`
    : "Dita.se: clique para ditar";
  $("etapa-texto").textContent = etapa;

  if (baixando) {
    $("baixando-texto").textContent = `Baixando o modelo · ${baixando.pct}%`;
  }

  if (aviso) {
    const ehErro = aviso.nivel === "erro";
    $("icone-info").style.display = ehErro ? "none" : "block";
    $("icone-erro").style.display = ehErro ? "block" : "none";
    const prefixo = ehErro && mqAltoContraste.matches ? "Erro: " : "";
    const partes = aviso.texto.split(/\.\s+/);
    const el = $("texto-aviso");
    el.title = aviso.texto;
    el.setAttribute("aria-label", aviso.texto);
    if (partes.length >= 2 && aviso.texto.length > 26) {
      el.textContent = "";
      const l1 = document.createElement("span");
      l1.textContent = prefixo + partes[0];
      const l2 = document.createElement("span");
      l2.className = "linha2";
      l2.textContent = partes.slice(1).join(". ");
      el.append(l1, l2);
    } else {
      el.textContent = prefixo + aviso.texto;
    }
  }

  atualizarContorno(est);
  atualizarLoopBarras();
}

// Laço de barras da voz: nasce só em gravando/maosLivres, cancela ao sair (0% CPU em repouso).
function aplicarEscalaBarras(els, valores, cache) {
  for (let i = 0; i < els.length; i++) {
    const px = valores[i] * 22;
    if (Math.abs(px - cache[i]) >= 0.5) {
      cache[i] = px;
      els[i].style.transform = `scaleY(${valores[i].toFixed(3)})`;
    }
  }
}

function passoBarras(ts) {
  rafId = 0;
  if (fase !== "gravando" && fase !== "maosLivres") return;
  rafId = requestAnimationFrame(passoBarras);
  if (ultimoQuadro && ts - ultimoQuadro < 14) return; // Teto ~60 fps em monitores 120/144 Hz
  const dt = ultimoQuadro ? Math.min(0.05, (ts - ultimoQuadro) / 1000) : 0.016;
  ultimoQuadro = ts;

  const limpo = Math.max(0, nivelAlvo - 0.03);
  const levantado = limpo > 0 ? Math.pow(limpo, 0.65) : 0;
  const respiracao = levantado < 0.02
    ? 0.18 + 0.045 * (0.5 + 0.5 * Math.sin(((ts - tempoInicio) / 1000) * 2.4))
    : 0;

  for (let i = 0; i < 9; i++) {
    const dist = Math.abs(i - 4);
    const amostra = dist === 0 ? levantado : historico[dist] ?? 0;
    const alvo = Math.max(0.18, Math.min(1, amostra * PESOS9[i] + respiracao));
    const tau = alvo > atual9[i] ? 0.045 : 0.16 + dist * 0.025;
    atual9[i] += (alvo - atual9[i]) * (1 - Math.exp(-dt / tau));
  }
  for (let i = 0; i < 3; i++) {
    const dist = Math.abs(i - 1);
    const amostra = dist === 0 ? levantado : historico[1] ?? 0;
    const alvo = Math.max(0.18, Math.min(1, amostra * PESOS3[i] + respiracao));
    const tau = alvo > atual3[i] ? 0.045 : 0.16 + dist * 0.025;
    atual3[i] += (alvo - atual3[i]) * (1 - Math.exp(-dt / tau));
  }

  aplicarEscalaBarras(barras3, atual3, ultimoEscrito3);
  aplicarEscalaBarras(barras9a, atual9, ultimoEscrito9a);
  aplicarEscalaBarras(barras9m, atual9, ultimoEscrito9m);
}

function atualizarLoopBarras() {
  const gravando = fase === "gravando" || fase === "maosLivres";
  if (gravando && !mqReduzido.matches) {
    if (!rafId) {
      ultimoQuadro = 0;
      tempoInicio = performance.now();
      rafId = requestAnimationFrame(passoBarras);
    }
  } else if (rafId) {
    cancelAnimationFrame(rafId);
    rafId = 0;
  }
}

// Interações de ponteiro, clique e toque (SPEC-windows.md §3.3)
function agendarFechar() {
  if (fecharTimer) clearTimeout(fecharTimer);
  fecharTimer = setTimeout(() => {
    fecharTimer = null;
    if (!ponteiroSobre) {
      hoverAberto = false;
      acenoAtivo = false;
      renderizar();
    }
  }, 450);
}

ponta.addEventListener("pointerenter", (e) => {
  ponteiroSobre = true;
  if (fecharTimer) { clearTimeout(fecharTimer); fecharTimer = null; }
  if (e.pointerType === "touch" || e.pointerType === "pen") return;
  if (fase === "ociosa" && !aviso && !baixando && !hoverAberto) {
    acenoAtivo = true;
    renderizar();
    if (abrirTimer) clearTimeout(abrirTimer);
    abrirTimer = setTimeout(() => {
      abrirTimer = null;
      if (ponteiroSobre) {
        acenoAtivo = false;
        hoverAberto = true;
        renderizar();
      }
    }, 300);
  }
});

ponta.addEventListener("pointerdown", () => {
  limparTimersHover();
  acenoAtivo = false;
  hoverAberto = true;
  renderizar();
});

casca.addEventListener("pointerenter", (e) => {
  ponteiroSobre = true;
  if (fecharTimer) { clearTimeout(fecharTimer); fecharTimer = null; }
  if (e.pointerType === "touch" || e.pointerType === "pen") return;
  if ((fase === "gravando" || fase === "processando" || progressoAtivo || baixando) && !hoverAberto) {
    if (abrirTimer) clearTimeout(abrirTimer);
    abrirTimer = setTimeout(() => {
      abrirTimer = null;
      if (ponteiroSobre) {
        hoverAberto = true;
        renderizar();
      }
    }, 300);
  }
});

palco.addEventListener("pointerleave", () => {
  ponteiroSobre = false;
  if (abrirTimer) { clearTimeout(abrirTimer); abrirTimer = null; }
  if (acenoAtivo && !hoverAberto) {
    acenoAtivo = false;
    renderizar();
    return;
  }
  if (hoverAberto) agendarFechar();
});

document.addEventListener("contextmenu", (e) => e.preventDefault());

$("btn-ditar").onclick = () => invocar("alternar");
$("btn-cancelar-a").onclick = () => invocar("cancelar");
$("btn-cancelar-m").onclick = () => invocar("cancelar");
$("btn-parar").onclick = () => invocar("parar");

function agendarFimAviso(segundos) {
  if (avisoTimer) clearTimeout(avisoTimer);
  avisoTimer = setTimeout(() => {
    avisoTimer = null;
    if (ponteiroSobre) {
      // Com o ponteiro sobre a aba, nada recolhe sozinho; recolhe ao sair.
      agendarFimAviso(0.5);
      return;
    }
    aviso = null;
    renderizar();
  }, Math.max(500, segundos * 1000));
}

// Protocolo da aba via Channel IPC (SPEC-windows.md §3.6)
window.pilula = (m) => {
  switch (m.tipo) {
    case "fase": {
      const anterior = fase;
      if (m.fase !== "ociosa") {
        aviso = null;
        if (avisoTimer) { clearTimeout(avisoTimer); avisoTimer = null; }
      }
      if (aviso && m.fase === "ociosa") return;
      fase = m.fase;
      if (fase === "gravando" || fase === "maosLivres") {
        historico.fill(0);
        nivelAlvo = 0;
        $("tempo-a").textContent = "0:00";
        $("tempo-m").textContent = "0:00";
        if (anterior !== fase) {
          anunciar(
            fase === "maosLivres"
              ? `Ouvindo em mãos livres. ${atalho} ou Parar e colar para terminar`
              : "Ouvindo"
          );
        }
      } else if (fase === "processando") {
        etapa = "Transcrevendo";
        progressoReal = null;
        anunciar(etapa);
      } else if (fase === "ociosa") {
        progressoReal = null;
        if (!ponteiroSobre) hoverAberto = false;
      }
      renderizar();
      break;
    }
    case "aviso": {
      const nivel = m.nivel === "erro" ? "erro" : "info";
      const segundos = nivel === "erro" ? Math.max(3.5, m.segundos || 3.5) : (m.segundos || 3);
      aviso = { texto: m.texto || "", nivel };
      fase = "ociosa";
      anunciar(nivel === "erro" ? `Erro: ${aviso.texto}` : aviso.texto);
      renderizar();
      agendarFimAviso(segundos);
      break;
    }
    case "nivel": {
      nivelAlvo = Math.max(0, Math.min(1, Number(m.nivel) || 0));
      historico.pop();
      historico.unshift(nivelAlvo > 0.03 ? Math.pow(nivelAlvo - 0.03, 0.65) : 0);
      if (mqReduzido.matches && (fase === "gravando" || fase === "maosLivres")) {
        for (let i = 0; i < 9; i++) {
          const d = Math.abs(i - 4);
          atual9[i] = Math.max(0.18, Math.min(1, (historico[d] ?? 0) * PESOS9[i]));
        }
        for (let i = 0; i < 3; i++) {
          const d = Math.abs(i - 1);
          atual3[i] = Math.max(0.18, Math.min(1, (historico[d] ?? 0) * PESOS3[i]));
        }
        aplicarEscalaBarras(barras3, atual3, ultimoEscrito3);
        aplicarEscalaBarras(barras9a, atual9, ultimoEscrito9a);
        aplicarEscalaBarras(barras9m, atual9, ultimoEscrito9m);
      }
      break;
    }
    case "segundos": {
      const s = Math.max(0, Math.floor(m.segundos || 0));
      const texto = `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
      $("tempo-a").textContent = texto;
      $("tempo-m").textContent = texto;
      break;
    }
    case "atalho": {
      atalho = m.simbolo || "Ctrl dir.";
      renderizar();
      break;
    }
    case "progresso": {
      if (!m.progresso) {
        progressoAtivo = false;
        progressoReal = null;
        etapa = "Transcrevendo";
      } else {
        progressoAtivo = true;
        const e = m.progresso.etapa;
        const rotulo = typeof e === "string" ? e : (e && typeof e.rotulo === "string" ? e.rotulo : null);
        if (rotulo) etapa = rotulo;
        if (typeof m.progresso.progresso === "number") {
          progressoReal = Math.max(progressoReal ?? 0, Math.min(1, m.progresso.progresso));
        }
      }
      renderizar();
      break;
    }
    case "modelo": {
      if (m.feito == null) {
        baixando = null;
      } else {
        const eraNulo = baixando == null;
        const pct = m.total > 0 ? Math.min(100, Math.round((m.feito * 100) / m.total)) : 0;
        baixando = { feito: m.feito, total: m.total, pct };
        if (eraNulo) anunciar("Baixando o modelo de voz");
      }
      renderizar();
      break;
    }
    case "borda": {
      borda = m.lado === "esquerda" ? "esquerda" : "direita";
      renderizar();
      break;
    }
    case "ponteiroFora": {
      if (ponteiroSobre) {
        ponteiroSobre = false;
        if (hoverAberto) agendarFechar();
      }
      break;
    }
    // Auxiliar para pré-visualização/testes de estados (não muda o protocolo de produção).
    case "simular": {
      if (typeof m.hover === "boolean") hoverAberto = m.hover;
      if (typeof m.aceno === "boolean") acenoAtivo = m.aceno;
      renderizar();
      break;
    }
  }
};

renderizar();

// Canal IPC exclusivo da pílula (M-3): sem core:event:allow-listen.
const core = window.__TAURI__?.core;
if (core?.Channel) {
  const canal = new core.Channel();
  canal.onmessage = (m) => window.pilula(m);
  core.invoke("pilula_conectar", { canal });
}
