// Ilha no centro da borda superior do Windows (SPEC-dynamic-island.md rev. 2 e MOTION-windows.md).
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
let janelaAtual = { modo: "repouso", w: 92, h: 7 };
let encolherTimer = null;

// Barras da voz: 5 (compacta) e 9 (aberta / mãos livres)
const barras5 = Array.from($("barras-5").children);
const barras9a = Array.from($("barras-9a").children);
const barras9m = Array.from($("barras-9m").children);
const PESOS9 = [0.48, 0.62, 0.78, 0.92, 1, 0.92, 0.78, 0.62, 0.48];
const PESOS5 = [0.62, 0.85, 1, 0.85, 0.62];
const historico = Array(5).fill(0);
const atual9 = Array(9).fill(0.18);
const atual5 = Array(5).fill(0.22);
const ultimoEscrito9a = Array(9).fill(-1);
const ultimoEscrito9m = Array(9).fill(-1);
const ultimoEscrito5 = Array(5).fill(-1);
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

// Contorno único (meia elipse, topo reto): centro (W/2, 0), semieixos W/2 e H.
// `recuo` desloca o traço para dentro (0,5 px para a borda de 1 px, 1 px para o arco de 2 px).
function caminhoMeiaElipse(w, h, recuo = 0, fechar = false) {
  const rx = w / 2 - recuo;
  const ry = h - recuo;
  const meio = `${w / 2} ${h - recuo}`;
  const d = `M ${recuo} 0 A ${rx} ${ry} 0 0 0 ${meio} A ${rx} ${ry} 0 0 0 ${w - recuo} 0`;
  return fechar ? `${d} Z` : d;
}

function desenharForma(svg, fundo, borda, w, h) {
  svg.setAttribute("width", w);
  svg.setAttribute("height", h);
  svg.setAttribute("viewBox", `0 0 ${w} ${h}`);
  fundo.setAttribute("d", caminhoMeiaElipse(w, h, 0, true));
  borda.setAttribute("d", caminhoMeiaElipse(w, h, 0.5));
}

// Mesma regra de quebra da renderização: 2 linhas só com frase dupla acima de 26 caracteres.
function linhasDoAviso(texto) {
  const partes = texto.split(/\.\s+/);
  return partes.length >= 2 && texto.length > 26 ? [partes[0], partes.slice(1).join(". ")] : [texto];
}

function maiorLinha(texto) {
  return Math.max(...linhasDoAviso(texto).map((l) => l.length));
}

const limitar = (min, v, max) => Math.max(min, Math.min(max, v));

// Medidas da spec rev. 2 (px lógicos): W x H da meia elipse, centro vertical da linha de
// conteúdo e largura máxima de texto (miolo com 52 px de cada lado).
function calcularEstado() {
  if (aviso) {
    const erro = aviso.nivel === "erro";
    // Duas linhas descem mais na curva: +16 px de largura para manter a folga de 6 px.
    const duas = linhasDoAviso(aviso.texto).length === 2;
    const w = limitar(erro ? 232 : 220, Math.round(maiorLinha(aviso.texto) * 7.0 + 116 + (duas ? 16 : 0)), 336);
    const centro = duas ? 22 : 24;
    // Largura do texto limitada pela curva na base da linha (10 px na horizontal ≈ 6 px até a curva).
    const fator = Math.sqrt(1 - ((centro + (duas ? 16 : 8)) / 56) ** 2);
    const textoMax = Math.min(w - 96 - 24, Math.floor(2 * ((w / 2) * fator - 10)) - 24);
    return { id: erro ? "erro" : "aviso", w, h: 56, centro, vista: "v-aviso", textoMax };
  }
  if (fase === "maosLivres") {
    return { id: "maosLivres", w: 284, h: 56, centro: 23, vista: "v-maos" };
  }
  if (fase === "gravando") {
    return hoverAberto
      ? { id: "gravandoAberta", w: 248, h: 54, centro: 22, vista: "v-grav-aberta" }
      : { id: "gravandoCompacta", w: 116, h: 40, centro: 17, vista: "v-grav-comp" };
  }
  if (fase === "processando" || (fase === "ociosa" && progressoAtivo)) {
    if (hoverAberto) {
      const textoEtapa = typeof etapa === "string" && etapa ? etapa : "Transcrevendo";
      const w = limitar(248, Math.round(textoEtapa.length * 7.5 + 128), 296);
      return { id: "processandoAberta", w, h: 54, centro: 22, vista: "v-proc-aberta", textoMax: w - 104 - 28 };
    }
    return { id: "processandoCompacta", w: 116, h: 40, centro: 17, vista: "v-proc-comp" };
  }
  if (baixando) {
    return hoverAberto
      ? { id: "baixandoAberta", w: 296, h: 56, centro: 23, vista: "v-baix-aberta", textoMax: 296 - 104 - 20 }
      : { id: "baixandoCompacta", w: 132, h: 42, centro: 18, vista: "v-baix-comp" };
  }
  if (hoverAberto) {
    return { id: "ociosaAberta", w: 248, h: 54, centro: 22, vista: "v-ociosa" };
  }
  if (acenoAtivo) {
    return { id: "aceno", w: 112, h: 11, vista: null };
  }
  return { id: "repouso", w: 92, h: 7, vista: null };
}

function modoDesejado(id) {
  if (id === "repouso" || id === "aceno") return "repouso";
  if (id === "gravandoCompacta" || id === "processandoCompacta" || id === "baixandoCompacta") {
    return "compacta";
  }
  return "aberta";
}

// Pede ao Rust tamanho, posição e recorte da janela numa mensagem só (`modo:{nome}:{w}:{h}`).
// Cresce antes de animar; ao recolher para o repouso, só encolhe depois da animação de saída.
function sincronizarJanela(est) {
  if (encolherTimer) { clearTimeout(encolherTimer); encolherTimer = null; }
  const alvo = { modo: modoDesejado(est.id), w: est.w, h: est.h };
  const igual = (x, y) => x.modo === y.modo && x.w === y.w && x.h === y.h;
  if (igual(alvo, janelaAtual)) return;
  const enviar = () => {
    janelaAtual = alvo;
    invocar(`modo:${alvo.modo}:${alvo.w}:${alvo.h}`);
  };
  const recolheParaRepouso = alvo.modo === "repouso" && janelaAtual.modo !== "repouso";
  if (!recolheParaRepouso) {
    enviar();
    return;
  }
  const aplicarEncolher = () => {
    const agora = calcularEstado();
    if (modoDesejado(agora.id) === "repouso" && agora.w === alvo.w && agora.h === alvo.h) enviar();
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
  // Mesma meia elipse da casca, 1 px para dentro: M 1 0 A (W/2-1) (H-1) ... (W-1) 0.
  contorno.setAttribute("width", est.w);
  contorno.setAttribute("height", est.h);
  contorno.setAttribute("viewBox", `0 0 ${est.w} ${est.h}`);
  contornoPath.setAttribute("d", caminhoMeiaElipse(est.w, est.h, 1));
  contornoPath.style.strokeDashoffset = String(1 - v);
  contorno.classList.add("ativo");
}

let estadoAnterior = "repouso";

function renderizar() {
  const est = calcularEstado();
  sincronizarJanela(est);

  const expandido = est.id !== "repouso" && est.id !== "aceno";
  const eraExpandido = estadoAnterior !== "repouso" && estadoAnterior !== "aceno";
  palco.dataset.estado = est.id;
  palco.dataset.recolhendo = eraExpandido && !expandido ? "1" : "0";
  estadoAnterior = est.id;

  // A bordinha sempre segue repouso/aceno.
  const pw = acenoAtivo ? 112 : 92;
  const ph = acenoAtivo ? 11 : 7;
  const pontaSvg = $("ponta-svg");
  desenharForma(pontaSvg, $("ponta-fundo"), $("ponta-borda"), pw, ph);
  pontaSvg.style.marginLeft = `${-pw / 2}px`;
  ponta.style.width = `${pw}px`;
  ponta.style.height = `${ph}px`;

  // A casca só muda de forma quando expandida: ao recolher, sobe com a última forma.
  if (expandido) {
    casca.style.width = `${est.w}px`;
    casca.style.height = `${est.h}px`;
    desenharForma($("casca-svg"), $("casca-fundo"), $("casca-borda"), est.w, est.h);
    $("arco-erro").setAttribute("d", caminhoMeiaElipse(est.w, est.h, 1.25));
    palco.style.setProperty("--centro", `${est.centro}px`);
    palco.style.setProperty("--texto-max", `${est.textoMax ?? 200}px`);
    for (const v of document.querySelectorAll(".vista")) {
      v.classList.toggle("ativa", v.id === est.vista);
    }
  }

  $("chip-atalho").textContent = atalho;
  ponta.title = atalho
    ? `Dita.se: clique para ditar. Segure ${atalho} para ditar sem clicar`
    : "Dita.se: clique para ditar";
  $("etapa-texto").textContent = etapa;

  if (baixando) {
    $("baixando-texto").textContent = `Baixando o modelo · ${baixando.pct}%`;
    $("baixando-pct").textContent = `${baixando.pct}%`;
  }

  if (aviso) {
    const ehErro = aviso.nivel === "erro";
    $("icone-info").style.display = ehErro ? "none" : "block";
    $("icone-erro").style.display = ehErro ? "block" : "none";
    const prefixo = ehErro && mqAltoContraste.matches ? "Erro: " : "";
    const linhas = linhasDoAviso(aviso.texto);
    const el = $("texto-aviso");
    el.title = aviso.texto;
    el.setAttribute("aria-label", aviso.texto);
    if (linhas.length === 2) {
      el.textContent = "";
      const l1 = document.createElement("span");
      l1.textContent = prefixo + linhas[0];
      const l2 = document.createElement("span");
      l2.className = "linha2";
      l2.textContent = linhas[1];
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
  for (let i = 0; i < 5; i++) {
    const dist = Math.abs(i - 2);
    const amostra = dist === 0 ? levantado : historico[dist] ?? 0;
    const alvo = Math.max(0.22, Math.min(1, amostra * PESOS5[i] + respiracao));
    const tau = alvo > atual5[i] ? 0.045 : 0.16 + dist * 0.025;
    atual5[i] += (alvo - atual5[i]) * (1 - Math.exp(-dt / tau));
  }

  aplicarEscalaBarras(barras5, atual5, ultimoEscrito5);
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
      // Com o ponteiro sobre a ilha, nada recolhe sozinho; recolhe ao sair.
      agendarFimAviso(0.5);
      return;
    }
    aviso = null;
    renderizar();
  }, Math.max(500, segundos * 1000));
}

// Protocolo da ilha via Channel IPC (SPEC-windows.md §3.6)
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
        for (let i = 0; i < 5; i++) {
          const d = Math.abs(i - 2);
          atual5[i] = Math.max(0.22, Math.min(1, (historico[d] ?? 0) * PESOS5[i]));
        }
        aplicarEscalaBarras(barras5, atual5, ultimoEscrito5);
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
