// Automação pela UI real do Dita.se instalado via WebView2 CDP (Node 24+, sem dependências npm).
// Exercita: diagnóstico de disponibilidade (v1.0.1), fluxo Arquivos (seletor nativo + leitura do texto)
// e fluxo Reunião (iniciar, reproduzir áudio conhecido, encerrar e abrir transcrição pelos controles).
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
const WAV = arg("wav");
const REF = arg("ref");
const SAIDA = arg("saida");
const SCRIPT_DIALOGO = arg("dialogo");
const TEM_AUDIO = arg("tem-audio", "0") === "1";

const esperar = (ms) => new Promise((r) => setTimeout(r, ms));

function normalizar(s) {
  return String(s || "")
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
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

async function descobrirAlvoPrincipal(timeoutMs = 30_000) {
  const fim = Date.now() + timeoutMs;
  while (Date.now() < fim) {
    try {
      const resp = await fetch(`http://127.0.0.1:${PORTA}/json`);
      if (resp.ok) {
        const alvos = await resp.json();
        const principal = alvos.find(
          (a) =>
            a.type === "page" &&
            a.webSocketDebuggerUrl &&
            !String(a.url || "").includes("pilula.html")
        );
        if (principal) return principal;
      }
    } catch {
      // WebView2 ainda inicializando.
    }
    await esperar(300);
  }
  throw new Error(`Alvo WebView2 principal não respondeu na porta CDP ${PORTA} em ${timeoutMs} ms.`);
}

class ClienteCdp {
  constructor(ws) {
    this.ws = ws;
    this.seq = 0;
    this.pendentes = new Map();
    ws.addEventListener("message", (ev) => {
      const msg = JSON.parse(String(ev.data));
      if (msg.id && this.pendentes.has(msg.id)) {
        const { resolve, reject } = this.pendentes.get(msg.id);
        this.pendentes.delete(msg.id);
        if (msg.error) reject(new Error(msg.error.message || JSON.stringify(msg.error)));
        else resolve(msg.result);
      }
    });
  }

  static async conectar(wsUrl) {
    const ws = new WebSocket(wsUrl);
    await new Promise((resolve, reject) => {
      ws.addEventListener("open", resolve, { once: true });
      ws.addEventListener("error", (e) => reject(new Error(`Falha WebSocket CDP: ${e.message || e}`)), { once: true });
    });
    const cli = new ClienteCdp(ws);
    await cli.chamar("Page.enable");
    await cli.chamar("Runtime.enable");
    return cli;
  }

  chamar(method, params = {}) {
    const id = ++this.seq;
    return new Promise((resolve, reject) => {
      this.pendentes.set(id, { resolve, reject });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }

  async avaliar(expression) {
    const r = await this.chamar("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
    });
    if (r.exceptionDetails) {
      throw new Error(`Exceção JS: ${r.exceptionDetails.text || JSON.stringify(r.exceptionDetails)}`);
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
    this.ws.close();
  }
}

async function esperarCondicao(prazoMs, descricao, fn) {
  const fim = Date.now() + prazoMs;
  let ultimo;
  while (Date.now() < fim) {
    ultimo = await fn();
    if (ultimo) return ultimo;
    await esperar(400);
  }
  throw new Error(`Tempo esgotado (${prazoMs} ms) esperando: ${descricao}. Último valor: ${JSON.stringify(ultimo)}`);
}

async function clicarBotaoPorTexto(cdp, texto) {
  const ok = await cdp.avaliar(`(() => {
    const btns = Array.from(document.querySelectorAll("button"));
    const alvo = btns.find(b => (b.textContent || "").trim().includes(${JSON.stringify(texto)}));
    if (!alvo || alvo.disabled) return { ok: false, encontrado: !!alvo, disabled: alvo?.disabled ?? null };
    alvo.click();
    return { ok: true };
  })()`);
  return ok;
}

function rodarProcesso(cmd, args) {
  return new Promise((resolve, reject) => {
    const p = spawn(cmd, args, { stdio: ["ignore", "pipe", "pipe"] });
    let out = "";
    let err = "";
    p.stdout.on("data", (d) => { out += d; });
    p.stderr.on("data", (d) => { err += d; });
    p.on("error", reject);
    p.on("close", (code) => {
      if (code === 0) resolve({ out, err });
      else reject(new Error(`${cmd} saiu com ${code}: ${err || out}`));
    });
  });
}

function conferirPalavras(textoObtido, chaves, minimoFracao = 0.6) {
  const norm = normalizar(textoObtido);
  const encontradas = chaves.filter((k) => norm.includes(k));
  const fracao = chaves.length ? encontradas.length / chaves.length : 0;
  return {
    totalChaves: chaves.length,
    encontradas: encontradas.length,
    fracao: Number(fracao.toFixed(3)),
    passou: fracao >= minimoFracao,
  };
}

async function main() {
  fs.mkdirSync(SAIDA, { recursive: true });
  const { chaves } = REF ? palavrasChaveDaReferencia(REF) : { chaves: [] };
  const relatorio = {
    modo: MODO,
    executadoEm: new Date().toISOString(),
    screenshots: [],
    etapas: {},
    limites: [],
    veredito: "em_andamento",
  };

  const alvo = await descobrirAlvoPrincipal();
  const cdp = await ClienteCdp.conectar(alvo.webSocketDebuggerUrl);

  try {
    // 1. Aguardar a UI montar e o reconhecedor ficar pronto (via ponte do app instalado).
    const estadoPronto = await esperarCondicao(180_000, "reconhecedor pronto", () =>
      cdp.avaliar(`(async () => {
        const inv = window.__TAURI__?.core?.invoke ?? window.__TAURI_INTERNALS__?.invoke;
        if (!inv) return null;
        const est = JSON.parse(await inv("ponte", { metodo: "estado", params: {} }));
        return est?.reconhecedor?.estado === "pronto" ? est : null;
      })()`)
    );
    relatorio.etapas.reconhecedor = {
      estado: estadoPronto.reconhecedor.estado,
      nome: estadoPronto.reconhecedor.nome,
      versao: estadoPronto.versao,
    };
    relatorio.screenshots.push(await cdp.screenshot("01-inicio-pronto.png"));

    // 2. Navegar para Arquivos e checar se o bug da v1.0.1 (disponivel preso em false) ocorre.
    await clicarBotaoPorTexto(cdp, "Arquivos");
    await esperar(500);
    const uiArquivos = await cdp.avaliar(`(() => {
      const btn = Array.from(document.querySelectorAll("button")).find(b => (b.textContent || "").includes("Escolher arquivo"));
      const texto = document.body.innerText || "";
      return {
        botaoEncontrado: !!btn,
        botaoDesabilitado: btn ? !!btn.disabled : null,
        avisoIndisponivel: texto.includes("indisponível nesta versão do motor"),
      };
    })()`);
    relatorio.etapas.arquivos_disponibilidade = uiArquivos;
    relatorio.screenshots.push(await cdp.screenshot("02-arquivos-disponibilidade.png"));

    // 3. Navegar para Reunião e checar disponibilidade igualmente.
    await clicarBotaoPorTexto(cdp, "Reunião");
    await esperar(500);
    const uiReuniao = await cdp.avaliar(`(() => {
      const btn = Array.from(document.querySelectorAll("button")).find(b => (b.textContent || "").includes("Começar reunião"));
      const texto = document.body.innerText || "";
      return {
        botaoEncontrado: !!btn,
        botaoDesabilitado: btn ? !!btn.disabled : null,
        avisoIndisponivel: texto.includes("indisponível nesta versão do motor"),
      };
    })()`);
    relatorio.etapas.reuniao_disponibilidade = uiReuniao;
    relatorio.screenshots.push(await cdp.screenshot("03-reuniao-disponibilidade.png"));

    if (uiArquivos.botaoDesabilitado || uiArquivos.avisoIndisponivel || uiReuniao.avisoIndisponivel) {
      relatorio.diagnostico =
        "REPRODUZIDO BUG v1.0.1: reconhecedor está pronto, mas a UI permanece com lancamento.disponivel=false (Arquivos/Reunião indisponíveis).";
      if (MODO === "diagnostico") {
        relatorio.veredito = "diagnostico_reproduzido";
        fs.writeFileSync(path.join(SAIDA, "relatorio-qa.json"), JSON.stringify(relatorio, null, 2));
        return;
      }
      throw new Error(relatorio.diagnostico);
    }

    // 4. Fluxo Arquivos pela UI: clicar "Escolher arquivo" e selecionar o WAV no diálogo nativo.
    await clicarBotaoPorTexto(cdp, "Arquivos");
    await esperar(400);
    const dialogoPromessa = rodarProcesso("powershell", [
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      SCRIPT_DIALOGO,
      "-Arquivo",
      WAV,
    ]);
    const cliqueArq = await clicarBotaoPorTexto(cdp, "Escolher arquivo");
    if (!cliqueArq.ok) throw new Error(`Não foi possível clicar em Escolher arquivo: ${JSON.stringify(cliqueArq)}`);
    await dialogoPromessa;

    const textoArquivo = await esperarCondicao(180_000, "arquivo pronto com transcrição visível", () =>
      cdp.avaliar(`(() => {
        const main = document.querySelector("main")?.innerText || "";
        if (!main.includes("conversa-2-vozes.wav")) return null;
        if (main.includes("Falhou") || main.includes("Sem fala detectada")) {
          throw new Error("Arquivo falhou na UI: " + main.slice(0, 400));
        }
        const pronto = main.includes("pronto em ") && main.includes("palavra");
        return pronto ? main : null;
      })()`)
    );
    const confArq = conferirPalavras(textoArquivo, chaves, 0.6);
    relatorio.etapas.arquivo_ui = { ...confArq, amostra: textoArquivo.slice(0, 400) };
    relatorio.screenshots.push(await cdp.screenshot("04-arquivos-transcricao.png"));
    if (!confArq.passou) {
      throw new Error(`Texto do arquivo na UI abaixo do limite de acerto: ${JSON.stringify(confArq)}`);
    }

    // 5. Fluxo Reunião pela UI (requer dispositivo de saída de áudio para loopback WASAPI).
    if (!TEM_AUDIO) {
      relatorio.limites.push(
        "Reunião pela UI com áudio real não executada: máquina sem dispositivo de saída de áudio (Win32_SoundDevice vazio)."
      );
      relatorio.veredito = "aprovado_parcial_sem_dispositivo_audio";
    } else {
      await clicarBotaoPorTexto(cdp, "Reunião");
      await esperar(400);
      // Seleciona "Só o áudio do computador" (mac) para capturar a reprodução via loopback sem microfone físico.
      await cdp.avaliar(`(() => {
        const sel = document.querySelector("select");
        if (sel) {
          sel.value = "mac";
          sel.dispatchEvent(new Event("change", { bubbles: true }));
        }
      })()`);
      await esperar(300);
      const cliqueIniciar = await clicarBotaoPorTexto(cdp, "Começar reunião");
      if (!cliqueIniciar.ok) {
        throw new Error(`Não foi possível clicar em Começar reunião: ${JSON.stringify(cliqueIniciar)}`);
      }
      await esperar(1000);
      relatorio.screenshots.push(await cdp.screenshot("05-reuniao-gravando.png"));

      // Reproduz o WAV conhecido no dispositivo padrão do Windows para o WASAPI loopback.
      await rodarProcesso("powershell", [
        "-NoProfile",
        "-Command",
        `(New-Object Media.SoundPlayer '${WAV.replace(/'/g, "''")}').PlaySync()`,
      ]);
      await esperar(2500);

      const cliqueEncerrar = await clicarBotaoPorTexto(cdp, "Encerrar");
      if (!cliqueEncerrar.ok) {
        throw new Error(`Não foi possível clicar em Encerrar: ${JSON.stringify(cliqueEncerrar)}`);
      }

      const textoReuniao = await esperarCondicao(180_000, "reunião arquivada com transcrição aberta", () =>
        cdp.avaliar(`(() => {
          const main = document.querySelector("main")?.innerText || "";
          if (!main.includes("Reuniões anteriores")) return null;
          if (!main.includes("Transcrição")) return null;
          if (main.includes("Carregando transcrição")) return null;
          return main;
        })()`)
      );
      const confReu = conferirPalavras(textoReuniao, chaves, 0.5);
      relatorio.etapas.reuniao_ui = { ...confReu, amostra: textoReuniao.slice(0, 400) };
      relatorio.screenshots.push(await cdp.screenshot("06-reuniao-transcricao.png"));
      if (!confReu.passou) {
        throw new Error(`Texto da reunião na UI abaixo do limite de acerto: ${JSON.stringify(confReu)}`);
      }
      relatorio.veredito = "aprovado_completo";
    }

    fs.writeFileSync(path.join(SAIDA, "relatorio-qa.json"), JSON.stringify(relatorio, null, 2));
  } catch (err) {
    relatorio.veredito = "falhou";
    relatorio.erro = String(err?.message || err);
    try {
      relatorio.screenshots.push(await cdp.screenshot("99-erro.png"));
    } catch {}
    fs.writeFileSync(path.join(SAIDA, "relatorio-qa.json"), JSON.stringify(relatorio, null, 2));
    throw err;
  } finally {
    cdp.fechar();
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
