#!/bin/bash
# Publica no canal de teste do Windows o build feito pelo GitHub Actions.
# Roda neste Mac, que é o único lugar com a chave do atualizador:
#   chave privada: ~/.config/ditase/updater.key (0600)
#   senha:         Chaveiro do macOS, serviço ditase-updater, conta ditase
#
# Passos: espera a execução do workflow "canal-teste.yml" do commit atual da branch
# windows (ou do commit passado como argumento), baixa o artefato, assina o instalador
# com tauri signer sign, gera latest.json e SHA256SUMS.txt e cria ou atualiza o
# pre-release windows-teste (sem marcar latest, tag movida para o commit), subindo
# exe, .sig, SHA256SUMS.txt e por último latest.json.
#
# Uso: scripts/windows/publicar-teste.sh [commit]
# Variáveis: DITASE_REPO (padrão stoll2003/ditase), DITASE_CHAVE (caminho da chave).
# Nada da chave nem da senha é impresso.
set -euo pipefail

REPO="${DITASE_REPO:-stoll2003/ditase}"
BRANCH="windows"
WORKFLOW="canal-teste.yml"
ARTEFATO="canal-teste-windows"
TAG="windows-teste"
INSTALADOR="Dita.se-windows-x64-setup.exe"
CHAVE="${DITASE_CHAVE:-$HOME/.config/ditase/updater.key}"

for cmd in gh npx node shasum security; do
  command -v "$cmd" >/dev/null || { echo "Falta o comando $cmd." >&2; exit 1; }
done
[ -r "$CHAVE" ] || { echo "Chave do atualizador não encontrada em $CHAVE." >&2; exit 1; }
security find-generic-password -s ditase-updater -a ditase >/dev/null 2>&1 \
  || { echo "Senha da chave não encontrada no Chaveiro (serviço ditase-updater)." >&2; exit 1; }

SHA="${1:-$(gh api "repos/$REPO/branches/$BRANCH" --jq .commit.sha)}"
echo "Commit: $SHA"

# Espera a execução do canal aparecer (o push pode ter acabado de acontecer).
RUN_ID=""
for _ in $(seq 1 30); do
  RUN_ID="$(gh run list --repo "$REPO" --workflow "$WORKFLOW" --commit "$SHA" \
    --json databaseId --jq '.[0].databaseId // empty' --limit 1)"
  [ -n "$RUN_ID" ] && break
  sleep 10
done
[ -n "$RUN_ID" ] || { echo "Nenhuma execução de $WORKFLOW para $SHA após 5 min." >&2; exit 1; }
echo "Execução: https://github.com/$REPO/actions/runs/$RUN_ID"
gh run watch "$RUN_ID" --repo "$REPO" --exit-status --interval 20 >/dev/null \
  || { echo "A execução $RUN_ID falhou; nada publicado." >&2; exit 1; }

TMP="$(mktemp -d "${TMPDIR:-/tmp}/ditase-publicar.XXXXXXXX")"
trap 'rm -rf "$TMP"' EXIT
gh run download "$RUN_ID" --repo "$REPO" --name "$ARTEFATO" --dir "$TMP/art" \
  || { echo "Artefato $ARTEFATO ausente (a casca já tem tauri.conf.json?)." >&2; exit 1; }

EXE="$TMP/art/$INSTALADOR"
[ -f "$EXE" ] || { echo "Instalador ausente no artefato." >&2; exit 1; }
VERSAO="$(tr -d '[:space:]' < "$TMP/art/versao.txt")"
COMMIT_ART="$(tr -d '[:space:]' < "$TMP/art/commit.txt")"
[ "$COMMIT_ART" = "$SHA" ] || { echo "Artefato é do commit $COMMIT_ART, esperado $SHA." >&2; exit 1; }
echo "Versão: $VERSAO"

echo "Assinando para o atualizador..."
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(security find-generic-password -s ditase-updater -a ditase -w)" \
  npx -y @tauri-apps/cli@2 signer sign -f "$CHAVE" "$EXE" >/dev/null 2>&1 \
  || { echo "Falha ao assinar o instalador." >&2; exit 1; }
[ -s "$EXE.sig" ] || { echo "Assinatura não gerada." >&2; exit 1; }

SAIDA="$TMP/saida"
mkdir -p "$SAIDA"
cp "$EXE" "$SAIDA/$INSTALADOR"
cp "$EXE.sig" "$SAIDA/$INSTALADOR.sig"
URL="https://github.com/$REPO/releases/download/$TAG/$INSTALADOR"
VERSAO="$VERSAO" URL="$URL" SHA="$SHA" SIG_FILE="$SAIDA/$INSTALADOR.sig" OUT="$SAIDA/latest.json" node -e '
  const fs = require("fs");
  const e = process.env;
  const manifesto = {
    version: e.VERSAO,
    notes: "Versão de teste " + e.VERSAO + " (commit " + e.SHA.slice(0, 7) + ")",
    pub_date: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
    platforms: {
      "windows-x86_64": { signature: fs.readFileSync(e.SIG_FILE, "utf8").trim(), url: e.URL }
    }
  };
  fs.writeFileSync(e.OUT, JSON.stringify(manifesto, null, 2) + "\n");
'
(cd "$SAIDA" && shasum -a 256 "$INSTALADOR" latest.json > SHA256SUMS.txt)

NOTAS="Canal de teste do Dita.se para Windows. Não é versão estável.
Versão $VERSAO, commit $SHA.
Primeira instalação (PowerShell): irm https://raw.githubusercontent.com/$REPO/$BRANCH/scripts/install.ps1 | iex"

if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  gh api -X PATCH "repos/$REPO/git/refs/tags/$TAG" -f sha="$SHA" -F force=true >/dev/null
  gh release edit "$TAG" --repo "$REPO" --prerelease --latest=false \
    --title "Windows teste $VERSAO" --notes "$NOTAS" >/dev/null
else
  gh release create "$TAG" --repo "$REPO" --target "$SHA" --prerelease --latest=false \
    --title "Windows teste $VERSAO" --notes "$NOTAS" >/dev/null
fi

# latest.json por último: o app só vê a versão nova quando o instalador já está lá.
gh release upload "$TAG" --repo "$REPO" --clobber \
  "$SAIDA/$INSTALADOR" "$SAIDA/$INSTALADOR.sig" "$SAIDA/SHA256SUMS.txt"
gh release upload "$TAG" --repo "$REPO" --clobber "$SAIDA/latest.json"

echo "Publicado $VERSAO: https://github.com/$REPO/releases/tag/$TAG"
