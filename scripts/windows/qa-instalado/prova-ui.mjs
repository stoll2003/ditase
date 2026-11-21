// Automação pela UI real do Dita.se instalado via WebView2 CDP (Node 22+, sem dependências npm).
//
// Fase 1 (transição): o app abre com a pasta de modelos VAZIA. `lancamento.estado.disponivel` vem
// de `fala_disponivel()`, que olha o disco (Parakeet instalado), não o carregamento. Então o
// defeito da v1.0.1 só aparece quando o modelo chega ao disco DEPOIS que a janela montou. A prova
// observa, na mesma sessão e sem recarregar a página, a ponte (consulta direta, que não mexe no
// React) e a tela, e compara as duas quando a ponte passa a dizer disponível.
// Fase 2 (completo): Arquivos pelo seletor nativo e Reunião com áudio conhecido pelos controles.
//
// Saída: relatorio-qa.json. Código 0 só quando o veredito bate com --esperado.
//   --esperado=defeito   instalador original: a tela tem de ficar presa em indisponível.
//   --esperado=corrigido candidato: a tela tem de destravar e os fluxos têm de passar.
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";

function arg(nome, padrao = "") {
  const pref = `--${nome}=`;
  const achado = process.argv.find((a) => a.startsWith(pref));
  return achado ? achado.slice(pref.length) : padrao;
}

const PORTA = Number(arg("porta", "9222"));
const MODO = arg("modo", "completo"); // diagnostico | completo
const ESPERADO = arg("esperado", "corrigido"); // defeito | corrigido
const WAV = arg("wav");
const REF = arg("ref");
const SAIDA = arg("saida");
const SCRIPT_DIALOGO = arg("dialogo");
const TEM_AUDIO = arg("tem-audio", "0") === "1";
const DITADO = arg("ditado", "0") === "1";
const IA4B = arg("ia4b", "0") === "1";
const MODELO_4B = "Qwen3.5-4B-Q4_K_M.gguf";
const SCRIPT_DESK = arg("desk");
const PRAZO_MODELO_MS = Number(arg("prazo-modelo-ms", String(20 * 60_000)));
const FOLGA_APOS_DISCO_MS = Number(arg("folga-ms", "8000"));
const PERFIL_WEBVIEW = arg("perfil-webview"); // pasta de dados do WebView2 (DevToolsActivePort)

/** Porta real escrita pelo WebView2 em DevToolsActivePort, quando existir. */
function portaDoPerfil() {
  if (!PERFIL_WEBVIEW) return null;
  for (const f of [path.join(PERFIL_WEBVIEW, "EBWebView", "DevToolsActivePort"), path.join(PERFIL_WEBVIEW, "DevToolsActivePort")]) {
    try {
      const n = Number(fs.readFileSync(f, "utf8").split(/\r?\n/)[0]);
      if (n > 0) return n;
    } catch {}
  }
  return null;
}

const esperar = (ms) => new Promise((r) => setTimeout(r, ms));
const agora = (() => { const t0 = Date.now(); return () => Date.now() - t0; })();

/** Falha de infraestrutura da prova (ponte, CDP, página). Nunca conta como defeito nem como aprovação. */
class ErroDeProva extends Error {
  constructor(tipo, mensagem, detalhe) {
    super(`${tipo}: ${mensagem}`);
    this.tipo = tipo;
    this.detalhe = detalhe;
  }
}

function normalizar(s) {
  return String(s || "")
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9\s]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function palavrasChaveDaReferencia(refPath) {
  const segs = JSON.parse(fs.readFileSync(refPath, "utf8"));
  const completo = segs.map((s) => s.texto).join(" ");
  const unicas = new Set(normalizar(completo).split(" ").filter((p) => p.length >= 5));
  return { completo, chaves: [...unicas] };
}

async function descobrirAlvoPrincipal(timeoutMs = 60_000) {
  const fim = Date.now() + timeoutMs;
  let ultimoErro = "sem resposta";
  while (Date.now() < fim) {
    const porta = portaDoPerfil() ?? PORTA;
    try {
      const resp = await fetch(`http://127.0.0.1:${porta}/json`);
      if (resp.ok) {
        const alvos = await resp.json();
        // Só a página do app (tauri.localhost / tauri://localhost), nunca a pílula nem about:blank.
        const doApp = (u) => /^(https?:\/\/tauri\.localhost|tauri:\/\/localhost)/.test(u);
        const principal = alvos.find(
          (a) => a.type === "page" && a.webSocketDebuggerUrl && doApp(String(a.url || "")) && !String(a.url).includes("pilula")
        );
        if (principal) return principal;
        ultimoErro = `alvos sem página principal: ${JSON.stringify(alvos.map((a) => a.url))}`;
      }
    } catch (e) {
      ultimoErro = String(e?.message || e);
    }
    await esperar(300);
  }
  throw new ErroDeProva("cdp_indisponivel", `porta ${PORTA} sem página principal em ${timeoutMs} ms`, ultimoErro);
}

class ClienteCdp {
  constructor(ws) {
    this.ws = ws;
    this.seq = 0;
    this.pendentes = new Map();
    this.fechado = false;
    ws.addEventListener("message", (ev) => {
      const msg = JSON.parse(String(ev.data));
      if (msg.id && this.pendentes.has(msg.id)) {
        const { resolve, reject } = this.pendentes.get(msg.id);
        this.pendentes.delete(msg.id);
        if (msg.error) reject(new ErroDeProva("cdp_erro", msg.error.message || JSON.stringify(msg.error)));
        else resolve(msg.result);
      }
    });
    ws.addEventListener("close", () => {
      this.fechado = true;
      for (const { reject } of this.pendentes.values()) reject(new ErroDeProva("cdp_fechado", "WebSocket CDP fechou (app saiu?)"));
      this.pendentes.clear();
    });
  }

  static async conectar(wsUrl) {
    const ws = new WebSocket(wsUrl);
    await new Promise((resolve, reject) => {
      ws.addEventListener("open", resolve, { once: true });
      ws.addEventListener("error", (e) => reject(new ErroDeProva("cdp_indisponivel", `WebSocket: ${e.message || e}`)), { once: true });
    });
    const cli = new ClienteCdp(ws);
    await cli.chamar("Page.enable");
    await cli.chamar("Runtime.enable");
    return cli;
  }

  chamar(method, params = {}) {
    if (this.fechado) return Promise.reject(new ErroDeProva("cdp_fechado", "WebSocket CDP já fechado"));
    const id = ++this.seq;
    return new Promise((resolve, reject) => {
      this.pendentes.set(id, { resolve, reject });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }

  /** Avalia uma expressão na página. Exceção na página vira ErroDeProva("pagina_excecao"). */
  async avaliar(expression) {
    const r = await this.chamar("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) {
      const d = r.exceptionDetails;
      throw new ErroDeProva("pagina_excecao", d.exception?.description || d.text || "exceção sem texto");
    }
    return r.result?.value;
  }

  async screenshot(nome) {
    const { data } = await this.chamar("Page.captureScreenshot", { format: "png" });
    const destino = path.join(SAIDA, nome);
    fs.writeFileSync(destino, Buffer.from(data, "base64"));
    return destino;
  }

  fechar() {
    try { this.ws.close(); } catch {}
  }
}

/**
 * Consulta a ponte nativa direto (mesmo `invoke("ponte")` da UI, sem passar pelo React).
 * Devolve o valor ou lança ErroDeProva com o tipo: ponte_ausente, ponte_rejeitou, ponte_json_invalido.
 */
async function consultarPonte(cdp, metodo, params = {}) {
  const r = await cdp.avaliar(`(async () => {
    const inv = window.__TAURI__?.core?.invoke ?? window.__TAURI_INTERNALS__?.invoke;
    if (typeof inv !== "function") return { ok: false, tipo: "ponte_ausente", mensagem: "invoke do Tauri não existe na página" };
    let bruto;
    try {
      bruto = await inv("ponte", { metodo: ${JSON.stringify(metodo)}, params: ${JSON.stringify(params)} });
    } catch (e) {
      return { ok: false, tipo: "ponte_rejeitou", mensagem: String(e?.message ?? e) };
    }
    try {
      return { ok: true, valor: JSON.parse(String(bruto)) };
    } catch (e) {
      return { ok: false, tipo: "ponte_json_invalido", mensagem: String(bruto).slice(0, 200) };
    }
  })()`);
  if (!r || typeof r !== "object") throw new ErroDeProva("pagina_resposta_invalida", `consulta ${metodo} devolveu ${JSON.stringify(r)}`);
  if (!r.ok) throw new ErroDeProva(r.tipo, `${metodo}: ${r.mensagem}`);
  return r.valor;
}

/** Estado da tela atual, só pelo DOM renderizado. */
function lerTela(cdp, rotuloBotao) {
  return cdp.avaliar(`(() => {
    const btn = Array.from(document.querySelectorAll("button")).find(b => (b.textContent || "").trim().includes(${JSON.stringify(rotuloBotao)}));
    const texto = document.querySelector("main")?.innerText || document.body.innerText || "";
    return {
      marca: window.__qaMarca ?? null,
      hash: location.hash,
      botaoEncontrado: !!btn,
      botaoDesabilitado: btn ? !!btn.disabled : null,
      avisoIndisponivel: texto.includes("indisponível nesta versão do motor"),
      // Texto do aviso de status (o candidato mostra o motivo do reconhecedor durante a carga).
      status: Array.from(document.querySelectorAll("main [role=status], main [role=alert]")).map(e => e.innerText.trim()).filter(Boolean).join(" | ").slice(0, 300),
    };
  })()`);
}

async function esperarCondicao(prazoMs, descricao, fn) {
  const fim = Date.now() + prazoMs;
  let ultimo;
  while (Date.now() < fim) {
    ultimo = await fn();
    if (ultimo) return ultimo;
    await esperar(400);
  }
  throw new ErroDeProva("tempo_esgotado", `${prazoMs} ms esperando ${descricao}`, ultimo);
}

/** Clica num botão visível pelo texto. Não lança: devolve se achou e se estava habilitado. */
function clicarBotaoPorTexto(cdp, texto) {
  return cdp.avaliar(`(() => {
    const btns = Array.from(document.querySelectorAll("button"));
    const alvo = btns.find(b => (b.textContent || "").trim().includes(${JSON.stringify(texto)}));
    if (!alvo || alvo.disabled) return { ok: false, encontrado: !!alvo, disabled: alvo?.disabled ?? null };
    alvo.click();
    return { ok: true };
  })()`);
}

async function irPara(cdp, pagina) {
  // Navega pelo menu (o mesmo controle do usuário). O lançamento fica em App.tsx e NÃO é
  // reconsultado ao trocar de página, então navegar não esconde o defeito.
  const r = await clicarBotaoPorTexto(cdp, pagina);
  if (!r.ok) throw new ErroDeProva("ui_controle_ausente", `menu ${pagina} não clicável`, r);
  await esperar(400);
}

function rodarProcesso(cmd, args) {
  return new Promise((resolve, reject) => {
    const p = spawn(cmd, args, { stdio: ["ignore", "pipe", "pipe"] });
    let out = "";
    let err = "";
    p.stdout.on("data", (d) => { out += d; });
    p.stderr.on("data", (d) => { err += d; });
    p.on("error", reject);
    p.on("close", (code) => (code === 0 ? resolve({ out, err }) : reject(new Error(`${cmd} saiu com ${code}: ${err || out}`))));
  });
}

function conferirPalavras(textoObtido, chaves, minimoFracao) {
  const norm = normalizar(textoObtido);
  const encontradas = chaves.filter((k) => norm.includes(k));
  const fracao = chaves.length ? encontradas.length / chaves.length : 0;
  return { totalChaves: chaves.length, encontradas: encontradas.length, fracao: Number(fracao.toFixed(3)), passou: fracao >= minimoFracao };
}

/**
 * Fase 1. Devolve "defeito_reproduzido" ou "transicao_ok"; lança ErroDeProva quando a observação
 * não vale (modelo já no disco, download falhou, página recarregou, ponte com erro).
 */
async function faseTransicao(cdp, relatorio) {
  const linha = [];
  relatorio.etapas.transicao = { linhaDoTempo: linha };
  const marca = `qa-${Math.random().toString(36).slice(2)}`;
  await cdp.avaliar(`window.__qaMarca = ${JSON.stringify(marca)}; true`);

  // A UI monta e pede o lançamento uma vez; espera o React desenhar o menu.
  await esperarCondicao(30_000, "menu montado", () =>
    cdp.avaliar(`Array.from(document.querySelectorAll("button")).some(b => (b.textContent||"").includes("Arquivos"))`)
  );
  await irPara(cdp, "Arquivos");

  const amostrar = async (rotulo) => {
    const est = await consultarPonte(cdp, "estado");
    const lanc = await consultarPonte(cdp, "lancamento.estado");
    const tela = await lerTela(cdp, "Escolher arquivo");
    const a = {
      t: agora(),
      rotulo,
      reconhecedor: est?.reconhecedor?.estado ?? null,
      detalhe: est?.reconhecedor?.detalhe ?? null,
      ponteDisponivel: lanc?.disponivel ?? null,
      telaBotaoDesabilitado: tela.botaoDesabilitado,
      telaAvisoIndisponivel: tela.avisoIndisponivel,
      telaStatus: tela.status,
      mesmaPagina: tela.marca === marca,
    };
    if (!tela.botaoEncontrado) throw new ErroDeProva("ui_controle_ausente", "botão Escolher arquivo não está na tela", a);
    if (a.ponteDisponivel === null) throw new ErroDeProva("ponte_contrato", "lancamento.estado sem campo disponivel", lanc);
    if (!a.mesmaPagina) throw new ErroDeProva("sessao_recarregada", "a página recarregou; a observação não vale", a);
    const ant = linha[linha.length - 1];
    const mudou = !ant || ant.ponteDisponivel !== a.ponteDisponivel || ant.reconhecedor !== a.reconhecedor
      || ant.detalhe !== a.detalhe || ant.telaBotaoDesabilitado !== a.telaBotaoDesabilitado || ant.telaStatus !== a.telaStatus;
    if (mudou && linha.length < 400) linha.push(a);
    return a;
  };

  const primeira = await amostrar("inicio");
  relatorio.screenshots.push(await cdp.screenshot("01-inicio-sem-modelo.png"));
  // A pasta de modelos estava vazia na abertura (garantido por executar-qa-instalado.ps1). Se o
  // download terminou antes da conexão CDP, a transição aconteceu mas não foi vista ao vivo: o
  // veredito sai do estado final e o relatório marca isso.
  relatorio.etapas.transicao.observadaAoVivo = !primeira.ponteDisponivel;
  if (!primeira.ponteDisponivel && !primeira.telaBotaoDesabilitado) {
    throw new ErroDeProva("precondicao_tela", "sem modelo pronto, Escolher arquivo deveria estar desabilitado", primeira);
  }

  // Espera o modelo chegar ao disco na mesma sessão (o app baixa sozinho).
  let ultima = primeira;
  const fim = Date.now() + PRAZO_MODELO_MS;
  let ultimoLog = 0;
  while (!ultima.ponteDisponivel) {
    if (Date.now() > fim) throw new ErroDeProva("tempo_esgotado", `${PRAZO_MODELO_MS} ms sem o modelo no disco`, ultima);
    await esperar(1000);
    ultima = await amostrar("aguardando");
    if (ultima.reconhecedor === "falhou") {
      throw new ErroDeProva("modelo_falhou", "download ou carga do Parakeet falhou (não é o defeito da UI)", ultima);
    }
    if (agora() - ultimoLog > 15_000) {
      ultimoLog = agora();
      console.log(`[${Math.round(agora() / 1000)} s] reconhecedor=${ultima.reconhecedor} ${ultima.detalhe || ""}`);
    }
  }
  linha.push({ ...ultima, rotulo: "ponte_disponivel" });

  // Dá tempo para o app avisar a UI (carga do modelo e evento), sem recarregar nada.
  const prazoPronto = Date.now() + 5 * 60_000;
  while (Date.now() < prazoPronto) {
    ultima = await amostrar("pos_disco");
    if (ultima.reconhecedor === "falhou") throw new ErroDeProva("modelo_falhou", "carga do Parakeet falhou", ultima);
    if (ultima.reconhecedor === "pronto") break;
    await esperar(1000);
  }
  await esperar(FOLGA_APOS_DISCO_MS);
  // Troca de página ida e volta, como o usuário faria: não reconsulta o lançamento em App.tsx.
  await irPara(cdp, "Reunião");
  const telaReuniao = await lerTela(cdp, "Começar reunião");
  await irPara(cdp, "Arquivos");
  const final = await amostrar("final");
  linha.push(final);
  relatorio.screenshots.push(await cdp.screenshot("02-arquivos-apos-modelo.png"));
  relatorio.etapas.transicao.reuniao = telaReuniao;

  const telaPresa = final.telaBotaoDesabilitado || final.telaAvisoIndisponivel || telaReuniao.avisoIndisponivel;
  // Percentual do download na tela (1.0.2): quantos valores distintos a ponte e a tela mostraram.
  const pct = (t) => (String(t || "").match(/(\d+)%/) || [])[1];
  const ponteP = [...new Set(linha.map((a) => pct(a.detalhe)).filter(Boolean))];
  const telaP = [...new Set(linha.map((a) => pct(a.telaStatus)).filter(Boolean))];
  relatorio.etapas.transicao.percentual = { ponte: ponteP, tela: telaP };
  relatorio.etapas.transicao.resumo = {
    primeiraPonteDisponivel: primeira.ponteDisponivel,
    finalPonteDisponivel: final.ponteDisponivel,
    reconhecedorFinal: final.reconhecedor,
    telaPresaEmIndisponivel: telaPresa,
    mesmaSessao: final.mesmaPagina,
  };
  if (telaPresa) return "defeito_reproduzido";
  // Só cobra quando o download foi visto com progresso na ponte (3+ valores) e a prova espera corrigido.
  if (ESPERADO === "corrigido" && ponteP.length >= 3 && telaP.length < 2) return "produto_percentual_congelado";
  return "transicao_ok";
}

/**
 * Grava a pílula/cápsula (alvo pilula.html do mesmo WebView2) por screencast CDP: cada quadro vira
 * JPEG com horário. O WebView2 só emite quadro quando a imagem muda, então buracos longos durante o
 * processamento indicam animação parada; o intervalo entre quadros mede a fluidez.
 */
async function gravarPilula(rotulo) {
  const porta = portaDoPerfil() ?? PORTA;
  let alvo;
  try {
    const alvos = await (await fetch(`http://127.0.0.1:${porta}/json`)).json();
    alvo = alvos.find((a) => a.type === "page" && String(a.url || "").includes("pilula") && a.webSocketDebuggerUrl);
  } catch {}
  if (!alvo) return { parar: async () => ({ rotulo, erro: "alvo da pílula não encontrado" }) };
  const pasta = path.join(SAIDA, `pilula-${rotulo}`);
  fs.mkdirSync(pasta, { recursive: true });
  const ws = new WebSocket(alvo.webSocketDebuggerUrl);
  await new Promise((ok, erro) => { ws.addEventListener("open", ok, { once: true }); ws.addEventListener("error", erro, { once: true }); });
  let seq = 0;
  const quadros = [];
  const enviar = (method, params = {}) => ws.send(JSON.stringify({ id: ++seq, method, params }));
  ws.addEventListener("message", (ev) => {
    const msg = JSON.parse(String(ev.data));
    if (msg.method !== "Page.screencastFrame") return;
    const { data, metadata, sessionId } = msg.params;
    const n = quadros.length;
    fs.writeFileSync(path.join(pasta, `${String(n).padStart(5, "0")}.jpg`), Buffer.from(data, "base64"));
    quadros.push({ n, t: metadata.timestamp, w: metadata.deviceWidth, h: metadata.deviceHeight });
    enviar("Page.screencastFrameAck", { sessionId });
  });
  enviar("Page.enable");
  enviar("Page.startScreencast", { format: "jpeg", quality: 85, everyNthFrame: 1 });
  const inicio = Date.now() / 1000;
  return {
    async parar() {
      try { enviar("Page.stopScreencast"); } catch {}
      await esperar(300);
      try { ws.close(); } catch {}
      const t = quadros.map((q) => q.t).sort((x, y) => x - y);
      const gaps = t.slice(1).map((v, i) => v - t[i]).sort((x, y) => x - y);
      const p = (q) => (gaps.length ? Number(gaps[Math.min(gaps.length - 1, Math.floor(q * gaps.length))].toFixed(3)) : null);
      const resumo = { rotulo, quadros: quadros.length, inicio, primeiro: t[0] ?? null, ultimo: t.at(-1) ?? null,
        duracaoS: t.length > 1 ? Number((t.at(-1) - t[0]).toFixed(2)) : 0, intervaloP50: p(0.5), intervaloP95: p(0.95), intervaloMax: p(1) };
      fs.writeFileSync(path.join(pasta, "quadros.json"), JSON.stringify({ resumo, quadros }, null, 1));
      return resumo;
    },
  };
}

function acaoDesk(args) {
  return rodarProcesso("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", SCRIPT_DESK, ...args]).then((r) => r.out.trim());
}

async function conectarPilula() {
  const porta = portaDoPerfil() ?? PORTA;
  const alvos = await (await fetch(`http://127.0.0.1:${porta}/json`)).json();
  const alvo = alvos.find((a) => a.type === "page" && String(a.url || "").includes("pilula") && a.webSocketDebuggerUrl);
  if (!alvo) throw new ErroDeProva("pilula_ausente", "alvo CDP da pílula não encontrado");
  return ClienteCdp.conectar(alvo.webSocketDebuggerUrl);
}

/** Centro físico (px de tela) do controle visível da pílula cujo rótulo casa com `padrao`. */
function controleNaPilula(cp, padrao, excluir = "cancelar") {
  return cp.avaliar(`(() => {
    const re = new RegExp(${JSON.stringify(padrao)}, "i"), ex = new RegExp(${JSON.stringify(excluir)}, "i");
    const vis = (e) => { const r = e.getBoundingClientRect(), c = getComputedStyle(e);
      return r.width > 0 && r.height > 0 && c.visibility !== "hidden" && Number(c.opacity) > 0.05 && !e.closest("[hidden]"); };
    const el = [...document.querySelectorAll("button,[role=button]")].filter(vis).find((e) => {
      const t = [e.getAttribute("aria-label"), e.title, e.textContent].join(" ");
      return re.test(t) && !ex.test(t);
    });
    if (!el) return null;
    const r = el.getBoundingClientRect(), d = devicePixelRatio;
    return { x: Math.round((screenX + r.left + r.width / 2) * d), y: Math.round((screenY + r.top + r.height / 2) * d),
      rotulo: (el.getAttribute("aria-label") || el.title || el.textContent).trim().slice(0, 40), cursor: getComputedStyle(el).cursor };
  })()`);
}

/**
 * Fluxo real do ditado (aceite do arco, decisão do Founder): Bloco de Notas em primeiro plano,
 * Ditar (mãos livres) com o mouse real, fala sintética pelo VB-CABLE (CABLE Output = microfone),
 * Parar e colar, e filmagem da faixa de cima da tela + screencast da pílula até o texto aparecer.
 */
async function faseDitado(relatorio, chaves) {
  const et = (relatorio.etapas.ditado = { entradas: JSON.parse((await acaoDesk(["entrada"])) || "[]") });
  const np = JSON.parse(await acaoDesk(["notepad"]));
  et.blocoDeNotas = np;
  const cp = await conectarPilula();
  try {
    const jan = await cp.avaliar(`({ sx: screenX, sy: screenY, w: innerWidth, h: innerHeight, d: devicePixelRatio, tela: screen.width })`);
    const d = jan.d, cx = Math.round((jan.sx + jan.w / 2) * d);
    const faixa = { x: 0, y: 0, w: Math.round(jan.tela * d), h: Math.round(300 * d) };
    const pastaFilme = path.join(SAIDA, "ditado-tela");
    const sinal = path.join(SAIDA, "ditado-parar.sinal");
    const filme = rodarProcesso("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", SCRIPT_DESK, "filmar",
      "-X", String(faixa.x), "-Y", String(faixa.y), "-W", String(faixa.w), "-H", String(faixa.h), "-Pasta", pastaFilme, "-Parar", sinal, "-Fps", "12"]);
    const screencast = await gravarPilula("ditado").catch((e) => ({ parar: async () => ({ erro: String(e) }) }));
    const t0 = Date.now() / 1000;
    const marcos = (et.marcos = []);
    const marco = (nome, extra = {}) => marcos.push({ nome, t: Number((Date.now() / 1000).toFixed(3)), ...extra });

    await acaoDesk(["mover", "-X", String(cx), "-Y", String(Math.round(jan.sy * d) + 2)]);
    const ditar = await esperarCondicao(5000, "botão Ditar visível na pílula", () => controleNaPilula(cp, "ditar"));
    marco("hover_aberta", { ditar });
    const c1 = JSON.parse(await acaoDesk(["clicar", "-X", String(ditar.x), "-Y", String(ditar.y)]));
    marco("clique_ditar", c1);
    const parar = await esperarCondicao(8000, "botão Parar e colar (gravando em mãos livres)", () => controleNaPilula(cp, "parar"));
    marco("gravando", { parar });
    await rodarProcesso("powershell", ["-NoProfile", "-Command", `(New-Object Media.SoundPlayer '${WAV.replace(/'/g, "''")}').PlaySync()`]);
    marco("fala_terminou");
    const parar2 = (await controleNaPilula(cp, "parar")) || parar;
    const c2 = JSON.parse(await acaoDesk(["clicar", "-X", String(parar2.x), "-Y", String(parar2.y)]));
    marco("clique_parar_e_colar", c2);

    let ultimo = "", estavelDesde = 0;
    const texto = await esperarCondicao(4 * 60_000, "texto colado e estável no Bloco de Notas", async () => {
      const r = JSON.parse(await acaoDesk(["texto", "-Janela", String(np.hwnd)]));
      if (!r.texto) return null;
      if (!ultimo) marco("texto_apareceu", { primeiroPlano: r.primeiroPlano });
      if (r.texto !== ultimo) { ultimo = r.texto; estavelDesde = Date.now(); return null; }
      return Date.now() - estavelDesde > 2500 ? r.texto : null;
    });
    marco("texto_estavel");
    await esperar(3000);
    fs.writeFileSync(sinal, "parar");
    await filme.catch(() => {});
    et.screencastPilula = await screencast.parar();
    try { et.filmeTela = JSON.parse(fs.readFileSync(path.join(pastaFilme, "quadros.json"), "utf8").replace(/^\uFEFF/, "")); delete et.filmeTela.tempos; } catch {}
    const conf = conferirPalavras(texto, chaves, 0.5);
    Object.assign(et, conf, { duracaoS: Number((Date.now() / 1000 - t0).toFixed(1)), amostra: texto.slice(0, 300) });
    return conf.passou;
  } finally {
    cp.fechar();
  }
}

/**
 * Modelo 4B pelo app instalado (critérios A2-A4): padrão no perfil novo, download pela tela de Ajustes
 * com cancelar no meio e continuar até instalar (o app confere o SHA-256). Depois disso a limpeza do
 * ditado e o resumo da reunião rodam com o 4B real (A5), nas fases seguintes.
 */
async function faseModelo4B(cdp, relatorio) {
  const et = (relatorio.etapas.modelo4b = { marcos: [] });
  const marco = (nome, extra = {}) => et.marcos.push({ nome, t: Number((Date.now() / 1000).toFixed(3)), ...extra });
  const est0 = await consultarPonte(cdp, "estado");
  const aj0 = await consultarPonte(cdp, "ajustes.ler");
  et.padrao = { modeloIA: est0.modeloIA, prefModelo: aj0?.valores?.modelo, limpezaIA: aj0?.valores?.limpezaIA, modeloPadrao: aj0?.modeloPadrao };
  const padraoOk = est0?.modeloIA?.arquivo === MODELO_4B && aj0?.valores?.modelo === MODELO_4B && aj0?.modeloPadrao === MODELO_4B;
  if (!padraoOk) throw new ErroDeProva("produto_modelo_padrao", "perfil novo não está com o 4B por padrão", et.padrao);

  await irPara(cdp, "Ajustes");
  relatorio.screenshots.push(await cdp.screenshot("06-ajustes-modelo-4b.png"));
  const b1 = await clicarBotaoPorTexto(cdp, "Baixar modelo");
  if (!b1.ok) throw new ErroDeProva("ui_controle_ausente", "Baixar modelo não clicável em Ajustes", b1);
  marco("baixar");
  const parcial = await esperarCondicao(5 * 60_000, "download do 4B passar de 15%", async () => {
    const m = (await consultarPonte(cdp, "estado")).modeloIA;
    if (m.estado === "falhou") throw new ErroDeProva("produto_download_falhou", m.detalhe || "download falhou", m);
    return m.estado === "baixando" && m.feito / m.tamanho > 0.15 ? m : null;
  });
  marco("parcial", { feito: parcial.feito, tamanho: parcial.tamanho });
  relatorio.screenshots.push(await cdp.screenshot("07-ajustes-baixando.png"));
  const bc = await clicarBotaoPorTexto(cdp, "Cancelar");
  if (!bc.ok) throw new ErroDeProva("ui_controle_ausente", "Cancelar download não clicável", bc);
  const cancelado = await esperarCondicao(30_000, "download cancelado com parcial guardado", async () => {
    const m = (await consultarPonte(cdp, "estado")).modeloIA;
    return m.estado !== "baixando" && m.estado !== "conferindo" && (m.feito || 0) > 0 ? m : null;
  });
  marco("cancelado", { estado: cancelado.estado, feito: cancelado.feito });
  const retomar = await esperarCondicao(10_000, "botão Continuar download", () =>
    cdp.avaliar(`Array.from(document.querySelectorAll("button")).some(b => /Continuar download/.test(b.textContent || ""))`));
  relatorio.screenshots.push(await cdp.screenshot("08-ajustes-continuar.png"));
  const bcont = await clicarBotaoPorTexto(cdp, "Continuar download");
  if (!retomar || !bcont.ok) throw new ErroDeProva("produto_sem_retomada", "sem botão para continuar o download", bcont);
  marco("continuar");
  // A retomada tem de começar do parcial, não do zero.
  const pos = await esperarCondicao(60_000, "download retomado", async () => {
    const m = (await consultarPonte(cdp, "estado")).modeloIA;
    return m.estado === "baixando" ? m : null;
  });
  et.retomouDe = { parcial: cancelado.feito, primeiroDepois: pos.feito, doZero: pos.feito < cancelado.feito * 0.5 };
  const fim = await esperarCondicao(15 * 60_000, "4B instalado (SHA conferido)", async () => {
    const m = (await consultarPonte(cdp, "estado")).modeloIA;
    if (m.estado === "falhou") throw new ErroDeProva("produto_download_falhou", m.detalhe || "download falhou", m);
    return m.estado === "instalado" ? m : null;
  });
  marco("instalado");
  const aj1 = await consultarPonte(cdp, "ajustes.ler");
  et.final = { modeloIA: fim, prefModelo: aj1?.valores?.modelo, modelos: aj1?.modelos, limpador: (await consultarPonte(cdp, "estado")).limpador };
  relatorio.screenshots.push(await cdp.screenshot("09-ajustes-instalado.png"));
  if (et.retomouDe.doZero) throw new ErroDeProva("produto_retomada_do_zero", "o download recomeçou do zero", et.retomouDe);
  return true;
}

async function faseArquivos(cdp, relatorio, chaves) {
  await irPara(cdp, "Arquivos");
  const gravacao = await gravarPilula("arquivos").catch((e) => ({ parar: async () => ({ erro: String(e) }) }));
  const dialogo = rodarProcesso("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", SCRIPT_DIALOGO, "-Arquivo", WAV, "-ArvoreSaida", path.join(SAIDA, "dialogo-arvore.txt")]);
  const clique = await clicarBotaoPorTexto(cdp, "Escolher arquivo");
  if (!clique.ok) throw new ErroDeProva("ui_controle_ausente", "Escolher arquivo não clicável", clique);
  try {
    await dialogo;
  } catch (e) {
    throw new ErroDeProva("dialogo_nativo", String(e.message || e));
  }
  const texto = await esperarCondicao(10 * 60_000, "arquivo pronto com texto", async () => {
    const main = await cdp.avaliar(`document.querySelector("main")?.innerText || ""`);
    if (!main.includes("conversa-2-vozes.wav")) return null;
    if (main.includes("Falhou") || main.includes("Sem fala detectada")) {
      throw new ErroDeProva("produto_arquivo_falhou", "o card do arquivo falhou na UI", main.slice(0, 600));
    }
    return main.includes("pronto em ") && main.includes("palavra") ? main : null;
  });
  await esperar(3000); // fim do processamento e transição de volta na pílula
  relatorio.etapas.pilula_arquivos = await gravacao.parar();
  const conf = conferirPalavras(texto, chaves, 0.6);
  relatorio.etapas.arquivo_ui = { ...conf, amostra: texto.slice(0, 400) };
  relatorio.screenshots.push(await cdp.screenshot("03-arquivos-transcricao.png"));
  return conf.passou;
}

async function faseReuniao(cdp, relatorio, chaves) {
  await irPara(cdp, "Reunião");
  // Select controlado do React: usa o setter nativo para o onChange disparar.
  const sel = await cdp.avaliar(`(() => {
    const s = Array.from(document.querySelectorAll("select")).find(x => Array.from(x.options).some(o => o.value === "mac"));
    if (!s) return false;
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set.call(s, "mac");
    s.dispatchEvent(new Event("change", { bubbles: true }));
    return s.value === "mac";
  })()`);
  if (!sel) throw new ErroDeProva("ui_controle_ausente", "select de fonte sem a opção Só o áudio do computador");
  await esperar(300);
  const iniciar = await clicarBotaoPorTexto(cdp, "Começar reunião");
  if (!iniciar.ok) throw new ErroDeProva("ui_controle_ausente", "Começar reunião não clicável", iniciar);
  await esperarCondicao(15_000, "reunião gravando (botão Encerrar)", () =>
    cdp.avaliar(`Array.from(document.querySelectorAll("button")).some(b => (b.textContent||"").trim() === "Encerrar")`)
  );
  await esperar(1000);
  const erroCaptura = await cdp.avaliar(`(document.querySelector("main")?.innerText || "").match(/nenhuma saída de áudio[^\\n]*/)?.[0] ?? null`);
  relatorio.screenshots.push(await cdp.screenshot("04-reuniao-gravando.png"));
  if (erroCaptura) throw new ErroDeProva("ambiente_sem_saida_audio", erroCaptura);

  await rodarProcesso("powershell", ["-NoProfile", "-Command", `(New-Object Media.SoundPlayer '${WAV.replace(/'/g, "''")}').PlaySync()`]);
  await esperar(2500);
  const encerrar = await clicarBotaoPorTexto(cdp, "Encerrar");
  if (!encerrar.ok) throw new ErroDeProva("ui_controle_ausente", "Encerrar não clicável", encerrar);

  // A reunião mais recente abre sozinha em "Reuniões anteriores" (anteriores[0] em Reuniao.tsx).
  const texto = await esperarCondicao(10 * 60_000, "reunião arquivada com transcrição", async () => {
    const main = await cdp.avaliar(`document.querySelector("main")?.innerText || ""`);
    if (!main.includes("Reuniões anteriores") || !main.includes("Transcrição")) return null;
    if (main.includes("Carregando transcrição") || main.includes("Encerrando")) return null;
    return main;
  });
  if (IA4B) {
    // A5: resumo real com o 4B; espera o resumo sair (ou o erro aparecer) por até 5 min.
    const t0 = Date.now();
    const r = await esperarCondicao(5 * 60_000, "resumo da reunião", async () => {
      const m = await cdp.avaliar(`document.querySelector("main")?.innerText || ""`);
      if (m.includes("Gerado localmente")) return { gerado: true, trecho: m.slice(m.indexOf("Resumo"), m.indexOf("Resumo") + 400) };
      if (/Resumo local não gerado|não consegui|falhou/i.test(m) && Date.now() - t0 > 90_000) return { gerado: false, trecho: m.slice(0, 400) };
      return null;
    }).catch((e) => ({ gerado: false, erro: String(e.message || e) }));
    relatorio.etapas.resumo_4b = { ...r, segundos: Number(((Date.now() - t0) / 1000).toFixed(1)) };
    relatorio.screenshots.push(await cdp.screenshot("05b-reuniao-resumo.png"));
  }
  const conf = conferirPalavras(texto, chaves, 0.5);
  relatorio.etapas.reuniao_ui = { ...conf, amostra: texto.slice(0, 400) };
  relatorio.screenshots.push(await cdp.screenshot("05-reuniao-transcricao.png"));
  return conf.passou;
}

async function main() {
  fs.mkdirSync(SAIDA, { recursive: true });
  const { chaves } = REF ? palavrasChaveDaReferencia(REF) : { chaves: [] };
  const relatorio = {
    modo: MODO,
    esperado: ESPERADO,
    executadoEm: new Date().toISOString(),
    screenshots: [],
    etapas: {},
    limites: [],
    veredito: "em_andamento",
    confere: false,
  };
  const gravar = () => fs.writeFileSync(path.join(SAIDA, "relatorio-qa.json"), JSON.stringify(relatorio, null, 2));

  let cdp;
  try {
    const alvo = await descobrirAlvoPrincipal();
    cdp = await ClienteCdp.conectar(alvo.webSocketDebuggerUrl);
    relatorio.alvoCdp = alvo.url;
    // Página carregada e ponte injetada. Se a página carregou e o invoke não veio, é erro de ponte.
    const pagina = await esperarCondicao(60_000, "página do app carregada", () =>
      cdp.avaliar(`document.readyState === "complete" ? { url: location.href, invoke: typeof (window.__TAURI__?.core?.invoke ?? window.__TAURI_INTERNALS__?.invoke) } : null`)
    ).catch((e) => { throw new ErroDeProva("pagina_nao_carregou", e.message, e.detalhe); });
    relatorio.pagina = pagina;
    if (pagina.invoke !== "function") {
      await esperarCondicao(10_000, "invoke do Tauri", () =>
        cdp.avaliar(`typeof (window.__TAURI__?.core?.invoke ?? window.__TAURI_INTERNALS__?.invoke) === "function"`)
      ).catch(() => { throw new ErroDeProva("ponte_ausente", `página ${pagina.url} carregada sem invoke do Tauri`); });
    }
    const versao = await consultarPonte(cdp, "estado").then((e) => e?.versao ?? null);
    relatorio.versaoApp = versao;

    const transicao = await faseTransicao(cdp, relatorio);
    relatorio.veredito = transicao;

    if (transicao === "transicao_ok" && MODO === "completo" && IA4B) {
      await faseModelo4B(cdp, relatorio);
    }
    if (transicao === "transicao_ok" && MODO === "completo") {
      const arqOk = await faseArquivos(cdp, relatorio, chaves);
      if (!arqOk) relatorio.veredito = "produto_arquivo_texto_abaixo_do_limite";
      else if (!TEM_AUDIO) {
        relatorio.limites.push("Reunião não executada: sem endpoint de saída de áudio no Windows (veja audio-virtual.json).");
        relatorio.veredito = "corrigido_sem_reuniao";
      } else {
        const reuOk = await faseReuniao(cdp, relatorio, chaves);
        relatorio.veredito = reuOk ? "corrigido_completo" : "produto_reuniao_texto_abaixo_do_limite";
        if (reuOk && DITADO) {
          const ditOk = await faseDitado(relatorio, chaves);
          if (!ditOk) relatorio.veredito = "produto_ditado_texto_abaixo_do_limite";
        }
      }
    }
  } catch (err) {
    const tipo = err instanceof ErroDeProva ? err.tipo : "erro_inesperado";
    // produto_* e modelo_falhou são falhas do app; o resto é infraestrutura da prova.
    relatorio.veredito = tipo.startsWith("produto_") ? tipo : tipo === "ambiente_sem_saida_audio" ? tipo : `inconclusivo_${tipo}`;
    relatorio.erro = { tipo, mensagem: String(err?.message || err), detalhe: err?.detalhe ?? null };
    if (tipo === "ambiente_sem_saida_audio") relatorio.limites.push("Endpoint de áudio instalado, mas a captura não achou saída padrão.");
    try { if (cdp) relatorio.screenshots.push(await cdp.screenshot("99-erro.png")); } catch {}
  } finally {
    cdp?.fechar();
  }

  const aceitos = ESPERADO === "defeito"
    ? ["defeito_reproduzido"]
    : MODO === "diagnostico" ? ["transicao_ok"] : ["corrigido_completo", "corrigido_sem_reuniao"];
  relatorio.confere = aceitos.includes(relatorio.veredito);
  gravar();
  console.log(`veredito=${relatorio.veredito} esperado=${ESPERADO} confere=${relatorio.confere}`);
  if (relatorio.erro) console.log(`erro: ${relatorio.erro.mensagem}`);
  process.exit(relatorio.confere ? 0 : 1);
}

main();
