#!/bin/bash
# Compila a interface e o app em Release, assina com a identidade local e instala em /Applications.
set -euo pipefail
cd "$(dirname "$0")/.."
(cd ui && npm run build >/dev/null)
if [ -z "${DITADO_XCCONFIG:-}" ] && [ -f "$PWD/local.local.xcconfig" ]; then
  export DITADO_XCCONFIG="$PWD/local.local.xcconfig"
fi
scripts/compilar.sh Release | tee /tmp/ditado-compilar.log
grep -q "BUILD SUCCEEDED" /tmp/ditado-compilar.log
Derived="${DITADO_DERIVED_DATA:-build.noindex/DerivedData}"
APP="$Derived/Build/Products/Release/Dita.se.app"
IDENTIDADE=$(scripts/assinatura.sh | tail -1)
codesign --force --deep --sign "$IDENTIDADE" --keychain "$HOME/Library/Keychains/ditado-assinatura.keychain-db" "$APP"
codesign --verify --deep --strict "$APP"
pkill -x Dita.se 2>/dev/null && sleep 1 || true
pkill -x Ditado 2>/dev/null && sleep 1 || true
mover_para_lixeira() {
  local alvo="$1"
  [ -e "$alvo" ] || return 0
  if [ -n "${DITADO_LIXEIRA:-}" ] && [ -x "$DITADO_LIXEIRA" ]; then
    "$DITADO_LIXEIRA" "$alvo" >/dev/null
  elif /usr/bin/osascript -e 'on run argv' -e 'tell application "Finder" to delete POSIX file (item 1 of argv)' -e 'end run' "$alvo" >/dev/null 2>&1; then
    :
  else
    mkdir -p "$HOME/.Trash"
    mv "$alvo" "$HOME/.Trash/$(basename "$alvo")-$(date +%Y%m%d-%H%M%S)"
  fi
}
# Nome antigo ou versão anterior do app: sai para a Lixeira.
mover_para_lixeira /Applications/Ditado.app
mover_para_lixeira /Applications/Dita.se.app
ditto "$APP" /Applications/Dita.se.app
codesign -dv /Applications/Dita.se.app 2>&1 | grep -E "Authority|Identifier|TeamIdentifier" || true
# O sistema registra cada cópia compilada do app; com várias cópias o Spotlight e a Dock podem
# mostrar o ícone de uma antiga. Fica registrada só a instalada, e o Spotlight é reindexado.
LS=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
for copia in "$Derived"/Build/Products/*/Dita.se.app "$HOME"/.Trash/Dita.se*.app "$HOME"/.Trash/Ditado*.app; do
  [ -d "$copia" ] && "$LS" -u "$copia" >/dev/null 2>&1 || true
done
touch /Applications/Dita.se.app
"$LS" -f /Applications/Dita.se.app
mdimport /Applications/Dita.se.app
echo "instalado em /Applications/Dita.se.app"
