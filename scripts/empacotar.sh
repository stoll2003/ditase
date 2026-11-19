#!/bin/bash
# Gera o pacote público do Dita.se para macOS (Apple Silicon, arm64).
#
# Etapas: compila a interface web (npm ci + npm run build), gera o projeto (xcodegen),
# compila o app em Release com o bundle público se.dita.app (sem override local),
# confere o llama-server embutido em Contents/Resources/llama (sem Homebrew),
# assina ad hoc o app inteiro, monta Dita.se-mac-arm64.dmg com atalho para /Applications
# e grava SHA256SUMS.txt.
#
# Uso: scripts/empacotar.sh
# Variáveis opcionais:
#   DITADO_RELEASE_DIR      pasta de saída (padrão /tmp/ditado-release)
#   DITADO_DERIVED_DATA     DerivedData do xcodebuild (padrão /tmp/ditado-empacotar-dd)
#   DITADO_SOURCE_PACKAGES  pasta SourcePackages já baixada (com checkouts/FluidAudio),
#                           para compilar sem rede; sem ela, o SwiftPM baixa os pacotes.
set -euo pipefail

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
# O compilador grava caminhos de fonte no executável. Compilando de dentro da pasta pessoal,
# o nome da conta iria para o app público: copia a árvore para fora dela e roda de lá.
case "$RAIZ/" in
  "$HOME"/*)
    COPIA="$(mktemp -d /tmp/ditado-empacotar-src.XXXXXX)"
    rsync -a --exclude node_modules --exclude 'ui/dist' --exclude build.noindex --exclude .build \
      --exclude local.local.xcconfig "$RAIZ/" "$COPIA/"
    echo "==> compilando a partir de $COPIA"
    exec bash "$COPIA/scripts/empacotar.sh" "$@"
    ;;
esac
cd "$RAIZ"

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
  echo "erro: o empacotamento do Dita.se requer macOS em Apple Silicon (arm64)." >&2
  exit 1
fi
for cmd in npm xcodegen xcodebuild codesign hdiutil shasum otool; do
  command -v "$cmd" >/dev/null || { echo "erro: comando '$cmd' não encontrado." >&2; exit 1; }
done

RELEASE_DIR="${DITADO_RELEASE_DIR:-/tmp/ditado-release}"
DERIVED="${DITADO_DERIVED_DATA:-/tmp/ditado-empacotar-dd}"
BUILD_TMP="$(mktemp -d "${TMPDIR:-/tmp}/ditado-empacotar.XXXXXXXX")"
BUILD_LOG="$BUILD_TMP/xcodebuild.log"
STAGE_DIR="$BUILD_TMP/dmg-stage"
ASSET="Dita.se-mac-arm64.dmg"
SUMS="SHA256SUMS.txt"

limpar() { rm -rf "$BUILD_TMP"; }
trap limpar EXIT

# Falha se algum Mach-O depender de biblioteca fora do sistema ou do próprio bundle.
checar_dependencias() {
  local bin="$1"
  if otool -L "$bin" | tail -n +2 | grep -E "/opt/homebrew|/usr/local|openssl" >/dev/null; then
    echo "erro: dependência externa não permitida em $bin:" >&2
    otool -L "$bin" >&2
    exit 1
  fi
}

LLAMA_SRC="$RAIZ/Recursos/llama"
if [[ ! -x "$LLAMA_SRC/llama-server" ]]; then
  echo "erro: Recursos/llama/llama-server ausente ou sem permissão de execução." >&2
  exit 1
fi
for bin in "$LLAMA_SRC/llama-server" "$LLAMA_SRC"/*.dylib; do
  checar_dependencias "$bin"
done

echo "==> 1/6 Interface web (npm ci + npm run build)..."
# Compila numa cópia limpa (sem node_modules nem dist) para não mexer na pasta de trabalho.
UI_TMP="$BUILD_TMP/ui"
mkdir -p "$UI_TMP"
rsync -a --exclude node_modules --exclude dist "$RAIZ/ui/" "$UI_TMP/"
(
  cd "$UI_TMP"
  npm ci --no-audit --no-fund
  cat > vite.publicacao.config.ts <<'EOF'
import { defineConfig, mergeConfig } from 'vite'
import config from './vite.config.ts'
export default defineConfig(mergeConfig(config, { build: { license: true } }))
EOF
  npm run build -- --config vite.publicacao.config.ts
  test -s dist/.vite/license.md
)
[[ -f "$UI_TMP/dist/index.html" ]] || { echo "erro: ui/dist/index.html não foi gerado." >&2; exit 1; }
mkdir -p "$RAIZ/ui/dist"
rsync -a --delete "$UI_TMP/dist/" "$RAIZ/ui/dist/"

echo "==> 2/6 Projeto Xcode (xcodegen)..."
xcodegen generate --quiet

PKG_ARGS=()
if [[ -n "${DITADO_SOURCE_PACKAGES:-}" ]]; then
  if [[ ! -d "$DITADO_SOURCE_PACKAGES/checkouts/FluidAudio" ]]; then
    echo "erro: DITADO_SOURCE_PACKAGES não contém checkouts/FluidAudio." >&2
    exit 1
  fi
  mkdir -p "$DERIVED/SourcePackages"
  rsync -a "$DITADO_SOURCE_PACKAGES/" "$DERIVED/SourcePackages/"
  PKG_ARGS=(-clonedSourcePackagesDirPath "$DERIVED/SourcePackages" -disableAutomaticPackageResolution -skipPackageUpdates)
fi

echo "==> 3/6 Dita.se.app em Release (arm64, se.dita.app)..."
# Nenhum -xcconfig: o override local (local.local.xcconfig / DITADO_XCCONFIG) fica de fora.
unset DITADO_XCCONFIG XCODE_XCCONFIG_FILE
rm -rf "$DERIVED/Build/Products/Release/Dita.se.app"
if ! xcodebuild \
  -project Ditado.xcodeproj \
  -scheme Ditado \
  -configuration Release \
  -destination 'platform=macOS,arch=arm64' \
  -derivedDataPath "$DERIVED" \
  ${PKG_ARGS[@]+"${PKG_ARGS[@]}"} \
  ARCHS=arm64 \
  ONLY_ACTIVE_ARCH=NO \
  PRODUCT_BUNDLE_IDENTIFIER=se.dita.app \
  build > "$BUILD_LOG" 2>&1; then
  grep -E "^/.*error: |^error: |BUILD FAILED" "$BUILD_LOG" | tail -n 30 >&2 || tail -n 30 "$BUILD_LOG" >&2
  cp "$BUILD_LOG" "/tmp/ditado-empacotar-xcodebuild.log" 2>/dev/null || true
  echo "log completo: /tmp/ditado-empacotar-xcodebuild.log" >&2
  exit 1
fi
grep -q "BUILD SUCCEEDED" "$BUILD_LOG"

APP_BUILD="$DERIVED/Build/Products/Release/Dita.se.app"
[[ -d "$APP_BUILD" ]] || { echo "erro: $APP_BUILD não foi gerado." >&2; exit 1; }

# Trabalha numa cópia para não alterar o produto do xcodebuild.
APP="$BUILD_TMP/Dita.se.app"
ditto "$APP_BUILD" "$APP"

BUNDLE_ID="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APP/Contents/Info.plist")"
if [[ "$BUNDLE_ID" != "se.dita.app" ]]; then
  echo "erro: CFBundleIdentifier inesperado ($BUNDLE_ID, esperado se.dita.app)." >&2
  exit 1
fi

echo "==> 4/6 llama-server embutido, avisos de terceiros e assinatura ad hoc..."
LLAMA_DEST="$APP/Contents/Resources/llama"
if [[ ! -x "$LLAMA_DEST/llama-server" ]]; then
  echo "erro: o build não copiou Recursos/llama para Contents/Resources/llama (ver project.yml)." >&2
  exit 1
fi
# Garante conteúdo idêntico ao da árvore (bits de execução preservados).
rsync -a --delete "$LLAMA_SRC/" "$LLAMA_DEST/"
rm -rf "$APP/Contents/Resources/Recursos/llama"

# Recursos de G2P do LuxTTS (dados derivados do espeak-ng, GPL-3.0) não são usados pelo Dita.se.
# O próprio FluidAudio orienta deixá-los fora do bundle de apps que não usam LuxTTS.
find "$APP" -maxdepth 3 -type d -name "*LuxTtsG2pResources*.bundle" -prune -exec rm -rf {} +

cp -p "$RAIZ/THIRD-PARTY-NOTICES.md" "$APP/Contents/Resources/THIRD-PARTY-NOTICES.md"
cp -p "$RAIZ/LICENSE" "$APP/Contents/Resources/LICENSE"
cp -p "$UI_TMP/dist/.vite/license.md" "$APP/Contents/Resources/THIRD-PARTY-UI.md"
cp -p "$UI_TMP/node_modules/@fontsource-variable/inter/LICENSE" "$APP/Contents/Resources/INTER-LICENSE.txt"

xattr -cr "$APP" 2>/dev/null || true
for bin in "$LLAMA_DEST/llama-server" "$LLAMA_DEST"/*.dylib; do
  checar_dependencias "$bin"
  codesign --force --sign - --timestamp=none "$bin"
done
checar_dependencias "$APP/Contents/MacOS/Dita.se"
codesign --force --deep --sign - --timestamp=none "$APP"
codesign --verify --deep --strict "$APP"
env -i PATH=/usr/bin:/bin "$LLAMA_DEST/llama-server" --version >/dev/null 2>&1 \
  || { echo "erro: llama-server embutido não executa com ambiente limpo." >&2; exit 1; }

# Nenhum arquivo distribuído pode citar a pasta pessoal de quem compilou (caminhos de fonte que o
# compilador grava no executável). Compilar a partir de uma cópia fora da pasta pessoal resolve.
if PESSOAL=$(python3 - "$APP" <<'PY'
import os, pathlib, sys
h = os.fsencode(str(pathlib.Path.home()) + "/")
achados = [str(p) for p in pathlib.Path(sys.argv[1]).rglob("*") if p.is_file() and not p.is_symlink() and h in p.read_bytes()]
print("\n".join(achados)); sys.exit(0 if achados else 1)
PY
); then
  echo "erro: o app contém o caminho da pasta pessoal de quem compilou em:" >&2
  echo "$PESSOAL" >&2
  exit 1
fi

echo "==> 5/6 DMG em $RELEASE_DIR/$ASSET..."
mkdir -p "$RELEASE_DIR" "$STAGE_DIR"
ditto "$APP" "$STAGE_DIR/Dita.se.app"
ln -s /Applications "$STAGE_DIR/Applications"
rm -f "$RELEASE_DIR/$ASSET" "$RELEASE_DIR/$SUMS"
hdiutil create -volname "Dita.se" -srcfolder "$STAGE_DIR" -ov -format UDZO "$RELEASE_DIR/$ASSET" >/dev/null
hdiutil verify "$RELEASE_DIR/$ASSET" >/dev/null

echo "==> 6/6 $RELEASE_DIR/$SUMS..."
(
  cd "$RELEASE_DIR"
  shasum -a 256 "$ASSET" > "$SUMS"
  shasum -a 256 -c "$SUMS"
)

echo "Concluído: $RELEASE_DIR/$ASSET ($(du -h "$RELEASE_DIR/$ASSET" | cut -f1)) e $RELEASE_DIR/$SUMS"
