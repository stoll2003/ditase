import { readFile, writeFile } from 'node:fs/promises'
import { join } from 'node:path'

const [pasta, commit, repositorio, caminhoTauri] = process.argv.slice(2)
if (!pasta || !/^[\da-f]{40}$/.test(commit ?? '') || !repositorio || !caminhoTauri) {
  throw new Error('Uso: node montar-canal-teste.mjs <artefato> <commit> <dono/repo> <tauri.conf.json>')
}

const ler = async (nome) => (await readFile(join(pasta, nome), 'utf8')).trim()
const versao = await ler('versao.txt')
if (!/^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)-teste\.[1-9]\d*$/.test(versao)) {
  throw new Error(`Versão de teste inválida: ${versao}`)
}
if (await ler('commit.txt') !== commit) {
  throw new Error('O commit do artefato difere do commit solicitado.')
}

const base = `https://github.com/${repositorio}/releases/download/windows-teste`
const config = JSON.parse(await readFile(caminhoTauri, 'utf8'))
if (!config.plugins?.updater?.endpoints?.includes(`${base}/latest.json`)) {
  throw new Error('O Tauri não consulta o manifesto do repositório informado.')
}

const instalador = `Dita.se-windows-x64-${versao}-setup.exe`
const assinatura = await ler(`${instalador}.sig`)
const linhas = Buffer.from(assinatura, 'base64').toString('utf8').trimEnd().split('\n')
const versoes = linhas[2]?.split('\t').filter((campo) => campo.startsWith('version:'))
if (linhas.length !== 4 || !linhas[0].startsWith('untrusted comment:') ||
    !linhas[2].startsWith('trusted comment:') || versoes?.length !== 1 ||
    versoes[0] !== `version:${versao}`) {
  throw new Error('A assinatura do Tauri precisa conter a versão exata do instalador.')
}

const manifesto = {
  version: versao,
  notes: `Versão de teste ${versao} (commit ${commit.slice(0, 7)})`,
  pub_date: new Date().toISOString().replace(/\.\d{3}Z$/, 'Z'),
  platforms: {
    'windows-x86_64': { signature: assinatura, url: `${base}/${instalador}` },
  },
}

await writeFile(join(pasta, 'latest.json'), JSON.stringify(manifesto, null, 2) + '\n')
