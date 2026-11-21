// Rode npm run dev e abra /tests/lancamento-janela.html.
// Exercita o app React e o adaptador Tauri reais. O host IPC e o áudio são simulados.
const ouvintes = new Map();
const arquivo = documento("a1", "prova.wav", "pronto", "Texto reconhecido no arquivo.");
const reuniao = {
  ...documento("r1", "Reunião de teste", "pronta", "Texto reconhecido na reunião."),
  fonte: "macEMicrofone",
};
let pronto = false;
let progresso = 0;
const motivo = () => `Baixando o modelo de fala: ${progresso}%`;
let microfone = "negada";
let erroEstado;
let leituras = 0;
let arquivoSalvo = false;
let reuniaoAtiva = false;
let reuniaoSalva = false;

function documento(id, nome, fase, texto) {
  return {
    id, nome, fase, criadoEm: 1700000000, duracao: 3, palavras: 4,
    vozes: [{ id: "voz", nome: "Pessoa 1" }],
    trechos: [{ id: id + "-1", inicio: 0, fim: 3, voz: "voz", texto }],
  };
}
const leve = (doc) => ({ ...doc, trechos: [] });
const emitir = (payload) => ouvintes.get("ditado")?.({ payload });

window.__TAURI_INTERNALS__ = {}; // Presente no WebView2 desde a inicialização.
window.__TAURI__ = {
  event: { listen: async (nome, receber) => {
    ouvintes.set(nome, receber);
    return () => ouvintes.delete(nome);
  } },
  core: { invoke: async (comando, { metodo, params }) => {
    if (comando !== "ponte") throw new Error("Comando inesperado: " + comando);
    switch (metodo) {
      case "estado": return JSON.stringify({
        fase: "parado", permissoes: { microfone }, atalho: { simbolo: "Ctrl" },
        reconhecedor: { estado: pronto ? "pronto" : "carregando", detalhe: pronto ? "" : motivo() },
      });
      case "lancamento.estado":
        leituras++;
        if (erroEstado) throw erroEstado;
        return JSON.stringify({
          disponivel: pronto,
          ...(pronto ? {} : { motivoIndisponibilidade: motivo() }),
          permissoes: { audioSistema: "concedida", microfone },
          arquivos: arquivoSalvo ? [leve(arquivo)] : [],
          anteriores: reuniaoSalva ? [leve(reuniao)] : [],
          reuniao: reuniaoAtiva ? { ...leve(reuniao), fase: "gravando" } : undefined,
        });
      case "lancamento.buscarArquivos": return "[]";
      case "lancamento.escolher":
        arquivoSalvo = true;
        emitir("lancamento");
        return JSON.stringify(arquivo.id);
      case "lancamento.iniciar":
        if (params.fonte !== reuniao.fonte) throw new Error("Fonte inesperada: " + params.fonte);
        reuniaoAtiva = true;
        emitir("lancamento");
        return "true";
      case "lancamento.encerrar":
        reuniaoAtiva = false;
        reuniaoSalva = true;
        emitir("lancamento");
        return "true";
      case "lancamento.documento":
        return JSON.stringify(params.id === arquivo.id ? arquivo : reuniao);
      default: throw new Error("Método inesperado: " + metodo);
    }
  } },
};

function botao(nome) {
  return [...document.querySelectorAll("button")].find((el) => el.textContent.trim() === nome);
}

async function esperar(descricao, predicado) {
  const ate = performance.now() + 4000;
  while (performance.now() < ate) {
    if (predicado()) return;
    await new Promise((resolver) => setTimeout(resolver, 20));
  }
  throw new Error(descricao);
}

const temTexto = (texto) => document.querySelector("main")?.textContent.includes(texto);
const resultado = document.querySelector("#resultado");
try {
  location.hash = "#/arquivos";
  await import("/src/main.tsx");
  await esperar("Arquivos não exibiu o estado inicial", () => leituras > 0 && botao("Escolher arquivo")?.disabled);
  await esperar("Arquivos não mostrou o download em 0%", () => temTexto(motivo()));
  progresso = 40;
  emitir("estado"); // O carregador só publica "estado"; o reconhecedor segue "carregando".
  await esperar("O percentual do download ficou congelado na tela", () => temTexto(motivo()));
  pronto = true;
  emitir("estado"); // É o único assunto publicado pelo carregador da v1.0.1.
  await esperar("Arquivos ficou desabilitado após o Parakeet pronto", () => botao("Escolher arquivo")?.disabled === false);
  botao("Escolher arquivo").click();
  await esperar("A transcrição do arquivo não apareceu", () => temTexto(arquivo.trechos[0].texto));

  botao("Reunião").click();
  await esperar("Reunião não pediu permissão", () => botao("Começar reunião")?.disabled);
  microfone = "concedida";
  emitir("estado"); // Volta das configurações de microfone.
  await esperar("Reunião ficou desabilitada após conceder o microfone", () => botao("Começar reunião")?.disabled === false);
  botao("Começar reunião").click();
  await esperar("Reunião não iniciou", () => botao("Encerrar"));
  botao("Encerrar").click();
  await esperar("A transcrição da reunião não apareceu", () => temTexto(reuniao.trechos[0].texto));

  erroEstado = "SQLite temporariamente indisponível";
  emitir("lancamento");
  await esperar("Falha da ponte não apareceu na Reunião", () => temTexto(erroEstado) && botao("Começar reunião")?.disabled);
  botao("Arquivos").click();
  await esperar("Falha da ponte não apareceu em Arquivos", () => temTexto(erroEstado) && botao("Escolher arquivo")?.disabled);
  erroEstado = undefined;
  botao("Tentar carregar de novo").click();
  await esperar("A nova tentativa não reabilitou Arquivos", () => botao("Escolher arquivo")?.disabled === false);
  resultado.textContent = "PASSOU: percentual do download, modelo, arquivo, microfone, reunião, falha da ponte e nova tentativa.";
  resultado.dataset.estado = "passou";
} catch (erro) {
  resultado.textContent = "FALHOU: " + erro.message;
  resultado.dataset.estado = "falhou";
  console.error(erro);
}
