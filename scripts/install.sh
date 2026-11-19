#!/bin/bash
# Instala o Dita.se no Mac a partir da última release do GitHub.
# Baixa o DMG e o arquivo SHA256SUMS.txt, confere o hash SHA-256,
# copia Dita.se.app para /Applications (ou ~/Applications) e remove a marca de quarentena.
set -euo pipefail

REPO="${DITASE_REPO:-stoll2003/ditase}"
ASSET="${DITASE_ASSET:-Dita.se-mac-arm64.dmg}"
SUMS="${DITASE_SUMS:-SHA256SUMS.txt}"
BASE_URL="${DITASE_BASE_URL:-https://github.com/${REPO}/releases/latest/download}"


if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Este instalador é exclusivo para macOS." >&2
  exit 1
fi

if [[ "$(uname -m)" != "arm64" ]]; then
  echo "O Dita.se requer um Mac com chip Apple (arm64)." >&2
  exit 1
fi

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ditase-install.XXXXXXXX")"
MOUNT_DIR=""
limpar() {
  if [[ -n "$MOUNT_DIR" && -d "$MOUNT_DIR" ]]; then
    hdiutil detach "$MOUNT_DIR" -quiet >/dev/null 2>&1 || true
  fi
  rm -rf "$TMP_DIR"
}
trap limpar EXIT

echo "Baixando ${ASSET} e ${SUMS} de ${BASE_URL}..."
curl -fL --retry 3 -o "$TMP_DIR/$ASSET" "$BASE_URL/$ASSET"
curl -fL --retry 3 -o "$TMP_DIR/$SUMS" "$BASE_URL/$SUMS"

echo "Conferindo SHA-256..."
ESPERADO="$(awk -v arq="$ASSET" '$2 == arq || $2 == "*"arq { print $1; exit }' "$TMP_DIR/$SUMS")"
if [[ -z "$ESPERADO" ]]; then
  echo "Hash de ${ASSET} não encontrado em ${SUMS}." >&2
  exit 1
fi
ATUAL="$(shasum -a 256 "$TMP_DIR/$ASSET" | awk '{ print $1 }')"
if [[ "$ATUAL" != "$ESPERADO" ]]; then
  echo "Falha na verificação SHA-256: esperado ${ESPERADO}, calculado ${ATUAL}." >&2
  exit 1
fi

MOUNT_DIR="$TMP_DIR/mnt"
mkdir -p "$MOUNT_DIR"
hdiutil attach "$TMP_DIR/$ASSET" -nobrowse -readonly -mountpoint "$MOUNT_DIR" -quiet

APP_ORIGEM="$MOUNT_DIR/Dita.se.app"
if [[ ! -d "$APP_ORIGEM" ]]; then
  echo "Dita.se.app não encontrado dentro do DMG." >&2
  exit 1
fi

if [[ -n "${DITASE_DESTINO_DIR:-}" ]]; then
  # Destino alternativo (testes): não fecha o Dita.se em uso.
  DESTINO_DIR="$DITASE_DESTINO_DIR"
  mkdir -p "$DESTINO_DIR"
elif [[ -w "/Applications" ]]; then
  DESTINO_DIR="/Applications"
else
  DESTINO_DIR="$HOME/Applications"
  mkdir -p "$DESTINO_DIR"
fi
DESTINO="$DESTINO_DIR/Dita.se.app"

[[ -n "${DITASE_DESTINO_DIR:-}" ]] || { pkill -x "Dita.se" 2>/dev/null && sleep 1 || true; }

if [[ -d "$DESTINO" ]]; then
  if /usr/bin/osascript -e 'on run argv' -e 'tell application "Finder" to delete POSIX file (item 1 of argv)' -e 'end run' "$DESTINO" >/dev/null 2>&1; then
    :
  else
    mkdir -p "$HOME/.Trash"
    mv "$DESTINO" "$HOME/.Trash/Dita.se.app-$(date +%Y%m%d-%H%M%S)"
  fi
fi

echo "Instalando em ${DESTINO}..."
ditto "$APP_ORIGEM" "$DESTINO"
xattr -dr com.apple.quarantine "$DESTINO" 2>/dev/null || true

LS="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
if [[ -x "$LS" ]]; then
  "$LS" -f "$DESTINO" >/dev/null 2>&1 || true
fi
mdimport "$DESTINO" >/dev/null 2>&1 || true

echo "Dita.se instalado em ${DESTINO}. Abra pelo Spotlight, Launchpad ou na pasta Aplicativos."
