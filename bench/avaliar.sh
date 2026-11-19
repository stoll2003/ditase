#!/bin/bash
# Passa todos os áudios de bench/privado/wav pelo fluxo real do app (modo --transcrever)
# em três configurações e guarda texto e tempos em bench/privado/out/ditado-<modo>/.
# Uso: bench/avaliar.sh [regras|2b|4b ...]
cd "$(dirname "$0")/.." || exit 1
APP=build.noindex/DerivedData/Build/Products/Debug/Dita.se.app/Contents/MacOS/Dita.se
MODOS=("$@"); [ ${#MODOS[@]} -eq 0 ] && MODOS=(regras 2b 4b)
for modo in "${MODOS[@]}"; do
  case "$modo" in
    regras) ARGS=(--sem-ia) ;;
    2b) ARGS=(--modelo Qwen3.5-2B-Q4_K_M.gguf) ;;
    4b) ARGS=(--modelo Qwen3.5-4B-Q4_K_M.gguf) ;;
    *) echo "modo desconhecido: $modo"; exit 1 ;;
  esac
  SAIDA=bench/privado/out/ditado-$modo; mkdir -p "$SAIDA"
  for wav in bench/privado/wav/*.wav; do
    id=$(basename "$wav" .wav)
    "$APP" --transcrever "$wav" "${ARGS[@]}" > "$SAIDA/$id.txt" 2> "$SAIDA/$id.err"
    tail -1 "$SAIDA/$id.err" > "$SAIDA/$id.json"
  done
  echo "$modo pronto"
done
