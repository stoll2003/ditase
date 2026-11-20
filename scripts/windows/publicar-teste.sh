#!/bin/bash
# Publica no canal de teste do Windows o build feito pelo GitHub Actions.
# Roda neste Mac, que é o único lugar com a chave do atualizador:
#   chave privada: ~/.config/ditase/updater.key (0600)
#   senha:         Chaveiro do macOS, serviço ditase-updater, conta ditase
#
# Passos: espera a execução do workflow "canal-teste.yml" do commit revisado da branch
# windows, baixa o artefato, assina o instalador
# com tauri signer sign, gera latest.json e SHA256SUMS.txt e cria ou atualiza o
# pre-release windows-teste (sem marcar latest, tag movida para o commit), subindo
# exe, .sig, SHA256SUMS.txt e por último latest.json.
#
# Prepare o assinador antes, sem ler a senha do Chaveiro:
#   npm ci --prefix scripts/windows/tauri-cli --ignore-scripts --no-audit --no-fund
# Uso: scripts/windows/publicar-teste.sh <commit completo revisado>
# Variáveis: DITASE_REPO (padrão stoll2003/ditase), DITASE_CHAVE (caminho da chave).
# Nada da chave nem da senha é impresso.
set -euo pipefail

SCRIPTS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

REPO="${DITASE_REPO:-stoll2003/ditase}"
BRANCH="windows"
WORKFLOW="canal-teste.yml"
ARTEFATO="canal-teste-windows"
TAG="windows-teste"
ARQUIVO_BUILD="Dita.se-windows-x64-setup.exe"
CHAVE="${DITASE_CHAVE:-$HOME/.config/ditase/updater.key}"

for cmd in gh node shasum security base64; do
  command -v "$cmd" >/dev/null || { echo "Falta o comando $cmd." >&2; exit 1; }
done
CLI="$SCRIPTS/tauri-cli/node_modules/@tauri-apps/cli/tauri.js"
CLI_VERSAO="$(node -p 'require(process.argv[1]).dependencies["@tauri-apps/cli"]' "$SCRIPTS/tauri-cli/package.json")"
if [ ! -r "$CLI" ] || [ "$(node "$CLI" --version)" != "tauri-cli $CLI_VERSAO" ]; then
  echo "Instale o assinador fixado com npm ci --prefix scripts/windows/tauri-cli --ignore-scripts --no-audit --no-fund." >&2
  exit 1
fi

SHA="${1:-}"
[[ "$SHA" =~ ^[a-f0-9]{40}$ ]] || { echo "Informe o SHA completo do commit revisado." >&2; exit 1; }
echo "Commit: $SHA"

# Espera a execução do canal aparecer (o push pode ter acabado de acontecer).
RUN_ID=""
for _ in $(seq 1 30); do
  RUN_ID="$(gh run list --repo "$REPO" --workflow "$WORKFLOW" --branch "$BRANCH" --commit "$SHA" \
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

[ -f "$TMP/art/$ARQUIVO_BUILD" ] || { echo "Instalador ausente no artefato." >&2; exit 1; }
VERSAO="$(tr -d '\r\n' < "$TMP/art/versao.txt")"
[[ "$VERSAO" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-teste\.[1-9][0-9]*$ ]] \
  || { echo "Versão de teste inválida no artefato." >&2; exit 1; }
COMMIT_ART="$(tr -d '\r\n' < "$TMP/art/commit.txt")"
[ "$COMMIT_ART" = "$SHA" ] || { echo "Artefato é do commit $COMMIT_ART, esperado $SHA." >&2; exit 1; }
(cd "$TMP/art" && test "$(shasum -a 256 "$ARQUIVO_BUILD")" = "$(tr -d '\r' < SHA256SUMS.txt)") \
  || { echo "SHA-256 do instalador não confere; nada publicado." >&2; exit 1; }
gh api "repos/$REPO/contents/windows/src-tauri/tauri.conf.json?ref=$SHA" --jq .content \
  | base64 -D > "$TMP/tauri.conf.json"

INSTALADOR="Dita.se-windows-x64-$VERSAO-setup.exe"
SAIDA="$TMP/art"
EXE="$SAIDA/$INSTALADOR"
mv "$SAIDA/$ARQUIVO_BUILD" "$EXE"
echo "Versão: $VERSAO"
echo "Assinando para o atualizador..."
[ -r "$CHAVE" ] || { echo "Chave do atualizador não encontrada em $CHAVE." >&2; exit 1; }
senha="$(security find-generic-password -s ditase-updater -a ditase -w)" \
  || { echo "Senha da chave não encontrada no Chaveiro (serviço ditase-updater)." >&2; exit 1; }
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$senha" \
  node "$CLI" signer sign -f "$CHAVE" --app-version "$VERSAO" "$EXE" >/dev/null 2>&1 \
  || { echo "Falha ao assinar o instalador." >&2; exit 1; }
unset senha

node "$SCRIPTS/montar-canal-teste.mjs" "$SAIDA" "$SHA" "$REPO" "$TMP/tauri.conf.json"
(cd "$SAIDA" && shasum -a 256 "$INSTALADOR" latest.json > SHA256SUMS.txt)

NOTAS="Canal de teste do Dita.se para Windows.
Versão $VERSAO, commit $SHA, execução $RUN_ID.
Primeira instalação: baixe $INSTALADOR e compare o SHA-256 com a soma enviada pelo Dev Founder por um canal separado."

if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  gh api -X PATCH "repos/$REPO/git/refs/tags/$TAG" -f sha="$SHA" -F force=true >/dev/null
  gh release edit "$TAG" --repo "$REPO" --prerelease --latest=false \
    --title "Windows teste $VERSAO" --notes "$NOTAS" >/dev/null
else
  gh release create "$TAG" --repo "$REPO" --target "$SHA" --prerelease --latest=false \
    --title "Windows teste $VERSAO" --notes "$NOTAS" >/dev/null
fi

# Mantenha cada instalador versionado. Em uma retomada, aceite o asset existente
# apenas quando seus bytes forem os mesmos do build verificado.
if ! gh release upload "$TAG" --repo "$REPO" "$EXE"; then
  gh release download "$TAG" --repo "$REPO" --pattern "$INSTALADOR" --dir "$TMP/remoto"
  cmp -s "$EXE" "$TMP/remoto/$INSTALADOR" \
    || { echo "Já existe um instalador diferente para $VERSAO." >&2; exit 1; }
fi

gh release upload "$TAG" --repo "$REPO" --clobber "$EXE.sig" "$SAIDA/SHA256SUMS.txt"
gh release upload "$TAG" --repo "$REPO" --clobber "$SAIDA/latest.json"

echo "Publicado $VERSAO: https://github.com/$REPO/releases/tag/$TAG"
echo "SHA-256 para enviar separadamente ao testador:"
sed -n '1p' "$SAIDA/SHA256SUMS.txt"
