// Ilha do Windows 1.0.2 (SPEC-windows-identidade.md §2, rev. 1.1; MOTION-ilha-v102.md §3).
// Uma forma só: a barrinha, a recolhida e a aberta são a mesma .casca com outro tamanho e topo.
// A forma anima só durante a transição (rAF) e termina sozinha. Repouso com 0% de CPU.
const $ = (id) => document.getElementById(id);

const temaParam = new URLSearchParams(location.search).get("tema");
if (temaParam === "claro" || temaParam === "escuro") {
  document.documentElement.dataset.tema = temaParam;
}

const palco = $("palco");
const casca = $("casca");
const tracos = $("tracos");
const ponta = $("ponta");
const conteudo = $("conteudo");
const contornoPath = $("contorno-path");
const inteiroPath = $("inteiro-path");
const segmentoPath = $("segmento-path");
const leitor = $("leitor");

const mqReduzido = window.matchMedia("(prefers-reduced-motion: reduce)");
const mqAltoContraste = window.matchMedia("(forced-colors: active)");
const noApp = Boolean(window.__TAURI__?.core ?? window.__TAURI_INTERNALS__);

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

function anunciar(texto) {
  if (!texto) return;
  leitor.textContent = "";
  requestAnimationFrame(() => { leitor.textContent = texto; });
}

function limparTimersHover() {
  if (abrirTimer) { clearTimeout(abrirTimer); abrirTimer = null; }
  if (fecharTimer) { clearTimeout(fecharTimer); fecharTimer = null; }
}

const limitar = (min, v, max) => Math.max(min, Math.min(max, v));

// Largura real de texto na fonte da ilha (medida, não estimada por caractere).
const medidor = document.createElement("canvas").getContext("2d");
function larguraTexto(texto, peso = 600) {
  medidor.font = `${peso} 12px "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif`;
  return Math.ceil(medidor.measureText(texto).width);
}

// Mesma regra de quebra da renderização: 2 linhas só com frase dupla acima de 26 caracteres.
function linhasDoAviso(texto) {
  const partes = texto.split(/\.\s+/);
  return partes.length >= 2 && texto.length > 26 ? [partes[0], partes.slice(1).join(". ")] : [texto];
}

// Avisos de alerta (âmbar) e de confirmação (azul) pelo texto, enquanto o protocolo só traz nível.
const CONFIRMACAO = /^(copiado|transcrito|pronto|texto inserido|colado)/i;

// Geometria da §2.3 (DIP): cápsula W x H com topo em Y; raio sempre H/2.
function calcularEstado() {
  if (aviso) {
    const erro = aviso.nivel === "erro";
    const linhas = linhasDoAviso(aviso.texto);
    const maior = Math.max(larguraTexto(linhas[0], 600), linhas[1] ? larguraTexto(linhas[1], 400) : 0);
    const duas = linhas.length === 2;
    const w = limitar(200, 7 + 8 + maior + 32, 360);
    return { id: erro ? "erro" : "aviso", w, h: duas ? 52 : 36, y: 6, vista: "v-aviso", textoMax: w - 32 - 15 };
  }
  if (fase === "maosLivres") return { id: "maosLivres", w: 252, h: 36, y: 6, vista: "v-maos" };
  if (fase === "gravando") {
    return hoverAberto
      ? { id: "gravandoAberta", w: 216, h: 36, y: 6, vista: "v-grav-aberta" }
      : { id: "gravandoCompacta", w: 112, h: 30, y: 6, vista: "v-grav-comp", recolhida: true };
  }
  if (fase === "processando" || (fase === "ociosa" && progressoAtivo)) {
    if (hoverAberto) {
      const texto = typeof etapa === "string" && etapa ? etapa : "Transcrevendo";
      const w = limitar(196, 23 + 8 + larguraTexto(texto) + 28, 320);
      return { id: "processandoAberta", w, h: 36, y: 6, vista: "v-proc-aberta", textoMax: w - 59 };
    }
    return { id: "processandoCompacta", w: 112, h: 30, y: 6, vista: "v-proc-comp", recolhida: true };
  }
  if (baixando) {
    return hoverAberto
      ? { id: "baixandoAberta", w: 264, h: 36, y: 6, vista: "v-baix-aberta", textoMax: 264 - 48 }
      : { id: "baixandoCompacta", w: 120, h: 30, y: 6, vista: "v-baix-comp", recolhida: true };
  }
  if (hoverAberto) return { id: "ociosaAberta", w: 184, h: 36, y: 6, vista: "v-ociosa" };
  if (acenoAtivo) return { id: "aceno", w: 56, h: 6, y: 3, vista: null };
  return { id: "repouso", w: 40, h: 5, y: 3, vista: null };
}

function modoDesejado(id) {
  if (id === "repouso" || id === "aceno") return "repouso";
  if (id === "gravandoCompacta" || id === "processandoCompacta" || id === "baixandoCompacta") {
    return "compacta";
  }
  return "aberta";
}

const ehBarrinha = (e) => e.id === "repouso" || e.id === "aceno";

// Janela (§2.7): barrinha em 72x14 sem recorte; cápsula + 2 DIP de cada lado e até a borda de
// cima (o vão de cima fica fora da região de clique). Sempre presa em y = work_y.
function janelaDe(e) {
  if (ehBarrinha(e)) return { w: 72, h: 14 };
  return { w: Math.ceil(e.w) + 4, h: Math.ceil(e.y + e.h) + 2 };
}

// Caminho fechado da cápsula w x h, recuado `r0`, começando no centro do topo, sentido horário.
function caminhoCapsula(w, h, r0 = 1) {
  const r = Math.max(0, h / 2 - r0);
  return `M ${w / 2} ${r0} H ${w - h / 2} A ${r} ${r} 0 0 1 ${w - h / 2} ${h - r0} ` +
    `H ${h / 2} A ${r} ${r} 0 0 1 ${h / 2} ${r0} Z`;
}

// Curvas da §2.8: crescer 240 ms (0.23, 1, 0.32, 1); encolher 180 ms (0.32, 0, 0.67, 1).
function bezier(x1, y1, x2, y2) {
  const c = (a, b, t) => 3 * a * t * (1 - t) ** 2 + 3 * b * t * t * (1 - t) + t ** 3;
  return (x) => {
    let lo = 0, hi = 1, t = x;
    for (let i = 0; i < 20; i++) {
      t = (lo + hi) / 2;
      if (c(x1, x2, t) < x) lo = t; else hi = t;
    }
    return c(y1, y2, t);
  };
}
const CURVA_CRESCER = bezier(0.23, 1, 0.32, 1);
const CURVA_ENCOLHER = bezier(0.32, 0, 0.67, 1);

const forma = { w: 40, h: 5, y: 3 };
let alvo = { id: "repouso", w: 40, h: 5, y: 3, vista: null };
let tween = null; // { de, para, inicio, dur, curva }
let formaRaf = 0;
let janelaAtual = { modo: "repouso", w: 72, h: 14, chave: "" };

function desenharForma() {
  casca.style.top = `${forma.y}px`;
  casca.style.width = `${forma.w}px`;
  casca.style.height = `${forma.h}px`;
  casca.style.borderRadius = `${forma.h / 2}px`;
  tracos.style.top = `${forma.y}px`;
  tracos.setAttribute("width", forma.w);
  tracos.setAttribute("height", forma.h);
  tracos.setAttribute("viewBox", `0 0 ${forma.w} ${forma.h}`);
  const d = caminhoCapsula(forma.w, forma.h, 1);
  contornoPath.setAttribute("d", d);
  inteiroPath.setAttribute("d", d);
  segmentoPath.setAttribute("d", d);
}

// Pede ao Rust tamanho da janela, cápsula e tipo de recorte numa mensagem só:
// modo:{nome}:{janela w}:{janela h}:{cápsula w}:{cápsula h}:{cápsula y}:{c|r}
function pedirJanela(modo, j, cap, recorte) {
  const chave = `${modo}:${j.w}:${j.h}:${cap.w}:${cap.h}:${cap.y}:${recorte}`;
  if (janelaAtual.chave === chave) return;
  janelaAtual = { modo, w: j.w, h: j.h, chave };
  invocar(`modo:${chave}`);
}

function capsulaInteira(e) {
  return { w: Math.round(e.w), h: Math.round(e.h), y: Math.round(e.y) };
}

// Durante a transição: janela do tamanho da união das duas, sem recorte de cápsula (clique nas
// quinas cai na ilha por até 240 ms; o recorte animado quadro a quadro piscaria).
function janelaDaTransicao(de, para) {
  const a = janelaDe(de);
  const b = janelaDe(para);
  pedirJanela(modoDesejado(para.id), { w: Math.max(a.w, b.w), h: Math.max(a.h, b.h) }, capsulaInteira(para), "r");
}

function janelaFinal() {
  pedirJanela(modoDesejado(alvo.id), janelaDe(alvo), capsulaInteira(alvo), ehBarrinha(alvo) ? "r" : "c");
}

function passoForma(ts) {
  if (!tween) { formaRaf = 0; return; }
  if (tween.inicio == null) tween.inicio = ts;
  const p = limitar(0, (ts - tween.inicio) / tween.dur, 1);
  const k = tween.curva(p);
  for (const c of ["w", "h", "y"]) forma[c] = tween.de[c] + (tween.para[c] - tween.de[c]) * k;
  desenharForma();
  if (p >= 0.33) revelarConteudoSePronto();
  if (p >= 1) {
    tween = null;
    formaRaf = 0;
    janelaFinal();
    revelarConteudoSePronto();
    return;
  }
  formaRaf = requestAnimationFrame(passoForma);
}

function animarForma(de) {
  const igual = Math.abs(forma.w - alvo.w) < 0.25 && Math.abs(forma.h - alvo.h) < 0.25 && Math.abs(forma.y - alvo.y) < 0.25;
  if (igual) {
    Object.assign(forma, { w: alvo.w, h: alvo.h, y: alvo.y });
    desenharForma();
    janelaFinal();
    revelarConteudoSePronto();
    return;
  }
  if (mqReduzido.matches) {
    // Reduzido: troca direta de forma, com o conteúdo em fade de 120 ms; a janela cresce antes
    // e encolhe depois do fade.
    janelaDaTransicao(de, alvo);
    const encolhe = alvo.w < forma.w || alvo.h < forma.h;
    setTimeout(() => {
      Object.assign(forma, { w: alvo.w, h: alvo.h, y: alvo.y });
      desenharForma();
      janelaFinal();
      revelarConteudoSePronto();
    }, encolhe ? 120 : 0);
    return;
  }
  const cresce = alvo.h > forma.h || alvo.w > forma.w;
  janelaDaTransicao({ ...de, w: Math.max(de.w, forma.w), h: Math.max(de.h, forma.h), y: Math.min(de.y, forma.y) }, alvo);
  tween = {
    de: { w: forma.w, h: forma.h, y: forma.y },
    para: { w: alvo.w, h: alvo.h, y: alvo.y },
    inicio: null,
    dur: cresce ? 240 : 180,
    curva: cresce ? CURVA_CRESCER : CURVA_ENCOLHER,
  };
  if (!formaRaf) formaRaf = requestAnimationFrame(passoForma);
}

// Conteúdo: o antigo sai em 100 ms; o novo entra em 160 ms depois de 80 ms de forma.
let vistaMostrada = null;
let trocaTimer = null;
let conteudoPendente = false;

function aplicarVista(est) {
  for (const v of document.querySelectorAll(".vista")) v.classList.toggle("ativa", v.id === est.vista);
  palco.style.setProperty("--texto-max", `${est.textoMax ?? 300}px`);
  vistaMostrada = est.vista;
}

function revelarConteudoSePronto() {
  if (!conteudoPendente || trocaTimer || !alvo.vista) return;
  conteudoPendente = false;
  palco.dataset.conteudo = "visivel";
  // Botões só aceitam clique com o conteúdo acima de 50%.
  setTimeout(() => { if (palco.dataset.conteudo === "visivel") palco.dataset.interativo = "1"; }, 80);
}

function trocarConteudo(est) {
  if (!est.vista) {
    palco.dataset.conteudo = "";
    palco.dataset.interativo = "0";
    conteudoPendente = false;
    return;
  }
  if (est.vista === vistaMostrada && palco.dataset.conteudo === "visivel") {
    aplicarVista(est);
    return;
  }
  palco.dataset.interativo = "0";
  if (palco.dataset.conteudo === "visivel") {
    palco.dataset.conteudo = "";
    if (trocaTimer) clearTimeout(trocaTimer);
    trocaTimer = setTimeout(() => {
      trocaTimer = null;
      if (!alvo.vista) return;
      aplicarVista(alvo);
      conteudoPendente = true;
      if (!tween) revelarConteudoSePronto();
    }, mqReduzido.matches ? 120 : 100);
    return;
  }
  if (!trocaTimer) aplicarVista(est);
  conteudoPendente = true;
  if (!tween && !mqReduzido.matches) revelarConteudoSePronto();
}

// Contorno com fração real (§2.6) e segmento indeterminado do processamento (§2.5).
let segmentoSaida = null;

function atualizarContorno(est) {
  let pct = null;
  if (est.id === "baixandoCompacta" || est.id === "baixandoAberta") {
    pct = baixando ? baixando.pct / 100 : null;
  } else if (est.id === "processandoCompacta" || est.id === "processandoAberta") {
    pct = progressoReal;
  }
  const processando = est.id === "processandoCompacta" || est.id === "processandoAberta";
  const indeterminado = processando && (pct == null || Number.isNaN(pct));

  if (indeterminado) {
    if (segmentoSaida) { clearTimeout(segmentoSaida); segmentoSaida = null; }
    palco.dataset.segmento = "girando";
  } else if (palco.dataset.segmento === "girando") {
    palco.dataset.segmento = "saindo";
    segmentoSaida = setTimeout(() => {
      delete palco.dataset.segmento;
      segmentoSaida = null;
      renderizar();
    }, mqReduzido.matches ? 120 : 160);
  }

  if (pct == null || Number.isNaN(pct)) {
    delete palco.dataset.contorno;
    return;
  }
  palco.style.setProperty("--fracao", String(limitar(0, pct, 1)));
  palco.dataset.contorno = "ativo";
}

let estadoAnterior = { id: "repouso", w: 40, h: 5, y: 3, vista: null };

function renderizar() {
  const est = calcularEstado();
  atualizarContorno(est);
  // Apaga o segmento no contorno atual antes de mudar a cápsula ou o conteúdo.
  if (segmentoSaida !== null) return;
  const anterior = estadoAnterior;
  estadoAnterior = est;
  alvo = est;
  palco.dataset.estado = est.id;
  palco.dataset.recolhida = est.recolhida ? "1" : "0";
  palco.dataset.alerta = aviso && aviso.nivel !== "erro" && !CONFIRMACAO.test(aviso.texto) ? "1" : "0";

  trocarConteudo(est);
  animarForma(anterior);

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
    const prefixo = ehErro && mqAltoContraste.matches ? "Erro: " : "";
    const linhas = linhasDoAviso(aviso.texto);
    const el = $("texto-aviso");
    el.title = aviso.texto;
    el.setAttribute("aria-label", aviso.texto);
    el.textContent = "";
    const l1 = document.createElement("span");
    l1.textContent = prefixo + linhas[0];
    el.append(l1);
    if (linhas.length === 2) {
      const l2 = document.createElement("span");
      l2.className = "linha2";
      l2.textContent = linhas[1];
      el.append(l2);
    }
  }

  atualizarLoopBarras();
}

// Barras da voz: 9 nas três vistas de gravação.
const barrasC = Array.from($("barras-c").children);
const barrasA = Array.from($("barras-a").children);
const barrasM = Array.from($("barras-m").children);
const PESOS9 = [0.48, 0.62, 0.78, 0.92, 1, 0.92, 0.78, 0.62, 0.48];
const historico = Array(5).fill(0);
const atual9 = Array(9).fill(0.22);
const ultimoEscritoC = Array(9).fill(-1);
const ultimoEscritoA = Array(9).fill(-1);
const ultimoEscritoM = Array(9).fill(-1);
let nivelAlvo = 0;
let rafId = 0;
let ultimoQuadro = 0;
let tempoInicio = 0;

function aplicarBarras() {
  aplicarEscalaBarras(barrasC, atual9, ultimoEscritoC);
  aplicarEscalaBarras(barrasA, atual9, ultimoEscritoA);
  aplicarEscalaBarras(barrasM, atual9, ultimoEscritoM);
}

// Laço de barras da voz: nasce só em gravando/maosLivres, cancela ao sair (0% CPU em repouso).
function aplicarEscalaBarras(els, valores, cache) {
  for (let i = 0; i < els.length; i++) {
    // Altura de 4 a 18 px pelo nível; branco com opacidade 0,55 + 0,45 x nível (§2.3).
    const px = valores[i] * 18;
    if (Math.abs(px - cache[i]) >= 0.5) {
      cache[i] = px;
      els[i].style.transform = `scaleY(${valores[i].toFixed(3)})`;
      els[i].style.opacity = (0.55 + 0.45 * ((valores[i] - 0.22) / 0.78)).toFixed(2);
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
    ? 0.22 + 0.045 * (0.5 + 0.5 * Math.sin(((ts - tempoInicio) / 1000) * 2.4))
    : 0;

  for (let i = 0; i < 9; i++) {
    const dist = Math.abs(i - 4);
    const amostra = dist === 0 ? levantado : historico[dist] ?? 0;
    const alvo = Math.max(0.22, Math.min(1, amostra * PESOS9[i] + respiracao));
    const tau = alvo > atual9[i] ? 0.045 : 0.16 + dist * 0.025;
    atual9[i] += (alvo - atual9[i]) * (1 - Math.exp(-dt / tau));
  }
  aplicarBarras();
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

// Interações. A janela nativa só pega o ponteiro sobre a cápsula (ou a janela da barrinha).
function agendarFechar() {
  if (fecharTimer) return;
  fecharTimer = setTimeout(() => {
    fecharTimer = null;
    if (!ponteiroSobre) {
      hoverAberto = false;
      acenoAtivo = false;
      renderizar();
    }
  }, 450);
}

palco.addEventListener("pointerenter", (e) => {
  ponteiroSobre = true;
  if (fecharTimer) { clearTimeout(fecharTimer); fecharTimer = null; }
  if (e.pointerType === "touch" || e.pointerType === "pen") return;
  if (hoverAberto) return;
  const ociosaLivre = fase === "ociosa" && !aviso && !baixando && !progressoAtivo;
  const compacta = fase === "gravando" || fase === "processando" || progressoAtivo || baixando;
  if (!ociosaLivre && !compacta) return;
  if (ociosaLivre) {
    acenoAtivo = true;
    renderizar();
  }
  if (abrirTimer) clearTimeout(abrirTimer);
  abrirTimer = setTimeout(() => {
    abrirTimer = null;
    if (ponteiroSobre) {
      acenoAtivo = false;
      hoverAberto = true;
      renderizar();
    }
  }, 300);
});

ponta.addEventListener("pointerdown", () => {
  limparTimersHover();
  acenoAtivo = false;
  hoverAberto = true;
  renderizar();
});

palco.addEventListener("pointerleave", () => {
  ponteiroSobre = false;
  if (abrirTimer) { clearTimeout(abrirTimer); abrirTimer = null; }
  if (acenoAtivo && !hoverAberto) {
    acenoAtivo = false;
    renderizar();
    return;
  }
  // No app, quem decide a saída é o vigia do Rust (§2.7: cápsula ampliada em 16 DIP para os lados
  // e para baixo, e até a borda de cima). Fora do app (prévia no navegador), fecha pela saída.
  if (hoverAberto && !noApp) agendarFechar();
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
        // Reduzido (§2.8): barras paradas na altura média.
        atual9.fill(0.5);
        aplicarBarras();
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
      // Fora da zona de manter aberta (§2.7).
      ponteiroSobre = false;
      if (hoverAberto) agendarFechar();
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
