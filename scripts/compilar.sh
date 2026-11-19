#!/bin/bash
# Compila o app e mostra só erros e o resultado. Sai com o código do xcodebuild.
# Uso: scripts/compilar.sh [Debug|Release] [opções do xcodebuild]. O log inteiro fica em build.noindex/compilar.log.
set -o pipefail
cd "$(dirname "$0")/.." || exit 1
CONFIG="${1:-Debug}"
Derived="${DITADO_DERIVED_DATA:-build.noindex/DerivedData}"
Log="${DITADO_BUILD_LOG:-build.noindex/compilar.log}"
xcodegen generate --quiet || exit 1
mkdir -p "$(dirname "$Log")" "$Derived"
XCCONFIG_ARGS=()
if [ -n "${DITADO_XCCONFIG:-}" ] && [ -f "$DITADO_XCCONFIG" ]; then
  XCCONFIG_ARGS=(-xcconfig "$DITADO_XCCONFIG")
fi
xcodebuild -scheme Ditado -configuration "$CONFIG" -destination 'platform=macOS' -derivedDataPath "$Derived" "${XCCONFIG_ARGS[@]}" "${@:2}" build > "$Log" 2>&1
CODIGO=$?
grep -E "^/.*(error|warning): |^error: |BUILD (SUCCEEDED|FAILED)" "$Log" | grep -v "/SourcePackages/" | sed "s|$PWD/||" | sort -u
if [ $CODIGO -ne 0 ] && ! grep -q "BUILD FAILED" "$Log"; then tail -5 "$Log"; fi
exit $CODIGO
