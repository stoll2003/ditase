#!/bin/bash
# Uso: scripts/testar-cauda.sh DIRETORIO_DAS_FIXTURES [CACHE_DOS_FALANTES] [BINARIO_DITASE]
# Fixtures sintéticas: conversa2.wav (PCM 16 kHz mono) e conversa2.mp4.
set -euo pipefail
raiz="$(cd "$(dirname "$0")/.." && pwd)"
fixtures="$(cd "${1:?informe o diretório de fixtures sintéticas}" && pwd)"
binario="${3:-$raiz/build.noindex/DerivedData/Build/Products/Debug/Dita.se.app/Contents/MacOS/Dita.se}"
temporario="${TMPDIR:-/tmp}"
saida="$(mktemp -d "${temporario%/}/ditase-cauda.XXXXXXXX")"
echo "Provas em $saida"
cache=()
if [[ -n "${2:-}" ]]; then cache=(--cache-falantes "$2"); fi

python3 -I - "$fixtures/conversa2.wav" "$saida" <<'PY'
from pathlib import Path
import struct
import sys
import wave

entrada, pasta = sys.argv[1:]
with wave.open(entrada, 'rb') as fonte:
    parametros = fonte.getparams()
    assert (parametros.nchannels, parametros.sampwidth, parametros.framerate) == (1, 2, 16_000)
    pcm = fonte.readframes(parametros.nframes)
ultima_amostra = max(i for i, (a,) in enumerate(struct.iter_unpack('<h', pcm)) if abs(a) >= 262)
pcm = pcm[:2 * (ultima_amostra + 1)]
for nome, conteudo in [('sem-silencio.wav', pcm), ('silencio.wav', bytes(len(pcm)))]:
    with wave.open(str(Path(pasta) / nome), 'wb') as destino:
        destino.setparams(parametros)
        destino.writeframes(conteudo)
PY

for midia in "$fixtures/conversa2.wav" "$fixtures/conversa2.mp4" "$saida/sem-silencio.wav"; do
    nome="${midia##*/}"
    "$binario" --transcrever-arquivo "$midia" --falantes --sem-ia --banco "$saida/prova.sqlite" ${cache[@]+"${cache[@]}"} > "$saida/$nome.json" 2> "$saida/$nome.log"
done
"$binario" --reuniao-simulada "$fixtures/conversa2.mp4" --sem-resumo ${cache[@]+"${cache[@]}"} > "$saida/reuniao.json" 2> "$saida/reuniao.log"
"$binario" --reuniao-simulada "$saida/silencio.wav" "$saida/sem-silencio.wav" --sem-resumo ${cache[@]+"${cache[@]}"} > "$saida/reuniao-mic.json" 2> "$saida/reuniao-mic.log"

python3 -I - "$saida" <<'PY'
from pathlib import Path
import json
import sys

pasta = Path(sys.argv[1])
for nome in ['conversa2.wav', 'conversa2.mp4', 'sem-silencio.wav', 'reuniao', 'reuniao-mic']:
    doc = json.loads((pasta / (nome + '.json')).read_text())
    trecho = doc['trechos'][-1]
    voz = next(v['nome'] for v in doc['vozes'] if v['id'] == trecho['voz'])
    esperado = 'Você' if nome == 'reuniao-mic' else 'Pessoa 2'
    print(f"{nome}: {voz} | {trecho['inicio']:.2f}–{trecho['fim']:.2f} s | {trecho['texto']}")
    assert 'combinado' in trecho['texto'].casefold() and trecho['inicio'] >= 34.2 and trecho['fim'] >= 35.8, nome
    assert voz == esperado and trecho['fim'] <= doc['duracao'], nome
    if 'bruto' in doc:
        assert doc['bruto'].casefold().count('combinado') == 1, nome
PY
