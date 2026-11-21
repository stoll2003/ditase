// Exportação .txt, .srt e .md dos documentos da prova de ponta a ponta, com a mesma função da
// interface (`textoDoDocumento` em ui/src/lib/lancamento.ts), e conferência do formato.
// Uso: node windows/src-tauri/tests/e2e/exportar.mjs <pasta com os JSON da prova>
// Node 24 ou mais novo: remove os tipos do TypeScript sozinho; o gancho abaixo só resolve as
// importações sem extensão da interface ("./ponte" vira "./ponte.ts").
import { registerHooks } from "node:module";
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

registerHooks({
  resolve(especificador, contexto, proximo) {
    if (especificador.startsWith(".") && !/\.[cm]?[jt]s$/.test(especificador) && contexto.parentURL) {
      const alvo = fileURLToPath(new URL(especificador + ".ts", contexto.parentURL));
      if (existsSync(alvo)) return proximo(pathToFileURL(alvo).href, contexto);
    }
    return proximo(especificador, contexto);
  },
});

// Os dados de exemplo da interface leem `location` e agendam uma demonstração em `window` ao
// carregar; fora do navegador, uma página vazia e um temporizador inerte (sem app nativo nem Tauri).
globalThis.location ??= new URL("http://localhost/");
globalThis.window ??= { location: globalThis.location, navigator: {}, setInterval: () => 0, clearInterval: () => {}, addEventListener: () => {}, removeEventListener: () => {} };

const aqui = dirname(fileURLToPath(import.meta.url));
const raiz = resolve(aqui, "../../../..");
const { textoDoDocumento } = await import(pathToFileURL(join(raiz, "ui/src/lib/lancamento.ts")).href);

const pasta = process.argv[2];
if (!pasta) throw new Error("informe a pasta com os JSON da prova");
const saida = join(pasta, "exportados");
mkdirSync(saida, { recursive: true });

const falhas = [];
const conferir = (ok, mensagem) => { if (!ok) falhas.push(mensagem); };
const documentos = readdirSync(pasta).filter((n) => n.endsWith(".json") && n !== "relatorio.json");
conferir(documentos.length >= 4, `esperava 4 documentos, achei ${documentos.length}: ${documentos}`);

for (const nome of documentos) {
  const doc = JSON.parse(readFileSync(join(pasta, nome), "utf8"));
  const base = nome.replace(/\.json$/, "");
  const trechos = doc.trechos.filter((t) => t.fim > t.inicio && !t.parcial);
  const nomes = [...new Set(doc.trechos.map((t) => doc.vozes.find((v) => v.id === t.voz)?.nome))];

  const txt = textoDoDocumento(doc, "txt");
  const srt = textoDoDocumento(doc, "srt");
  const md = textoDoDocumento(doc, "md");
  for (const [ext, texto] of [["txt", txt], ["srt", srt], ["md", md]]) writeFileSync(join(saida, `${base}.${ext}`), texto);

  // .txt: com resumo, o bloco dele vem antes de "Transcrição"; depois, rótulo "[mm:ss] Nome" em
  // cada bloco e todo o texto dos trechos.
  let corpo = txt;
  if (doc.resumo?.length) {
    conferir(txt.startsWith("Resumo gerado localmente") && txt.includes("- " + doc.resumo[0]), `${base}.txt sem o resumo`);
    corpo = txt.split("\nTranscrição\n\n")[1] ?? "";
  }
  conferir(/^\[\d{2}:\d{2}\] .+\n/.test(corpo), `${base}.txt sem rótulo de tempo e voz no início`);
  for (const n of nomes) conferir(corpo.includes(`] ${n}\n`), `${base}.txt sem a voz ${n}`);
  const palavras = (s) => s.split(/\s+/).filter(Boolean).length;
  const noTxt = corpo.split("\n").filter((l) => !/^\[\d{2}:\d{2}\] /.test(l)).join(" ");
  conferir(palavras(noTxt) === palavras(doc.trechos.map((t) => t.texto).join(" ")), `${base}.txt perdeu palavras`);

  // .srt: blocos numerados em ordem, carimbos HH:MM:SS,mmm crescentes, um por trecho.
  const blocos = srt.trim().split(/\n\n/);
  conferir(blocos.length === trechos.length, `${base}.srt com ${blocos.length} blocos para ${trechos.length} trechos`);
  let anterior = -1;
  blocos.forEach((b, i) => {
    const [num, tempo, ...texto] = b.split("\n");
    conferir(num === String(i + 1), `${base}.srt bloco ${i + 1} numerado ${num}`);
    const m = /^(\d{2}):(\d{2}):(\d{2}),(\d{3}) --> (\d{2}):(\d{2}):(\d{2}),(\d{3})$/.exec(tempo ?? "");
    conferir(!!m, `${base}.srt bloco ${i + 1} com carimbo inválido: ${tempo}`);
    if (m) {
      const s = (h, mi, se, ms) => +h * 3600 + +mi * 60 + +se + +ms / 1000;
      const de = s(m[1], m[2], m[3], m[4]), ate = s(m[5], m[6], m[7], m[8]);
      conferir(ate > de && de >= anterior - 0.001, `${base}.srt bloco ${i + 1} fora de ordem`);
      anterior = de;
    }
    conferir(texto.join(" ").trim().length > 0, `${base}.srt bloco ${i + 1} sem texto`);
  });

  // .md: título, rótulos em negrito e, na reunião com resumo, o bloco do resumo.
  conferir(md.startsWith(`# ${doc.nome}\n`), `${base}.md sem o título`);
  conferir(/\*\*\[\d{2}:\d{2}\] .+:\*\* /.test(md), `${base}.md sem rótulo em negrito`);
  if (doc.resumo?.length) conferir(md.includes("Resumo gerado localmente") && md.includes("- " + doc.resumo[0]), `${base}.md sem o resumo`);
  console.log(`exportado ${base}: txt ${txt.length} bytes, srt ${blocos.length} blocos, md ${md.length} bytes`);
}

if (falhas.length) {
  console.error("EXPORTAÇÃO FALHOU:\n- " + falhas.join("\n- "));
  process.exit(1);
}
console.log(`EXPORTAÇÃO OK: ${documentos.length} documentos em ${saida}`);
