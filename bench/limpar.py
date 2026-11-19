#!/usr/bin/env python3
"""Manda uma transcrição ao llama-server com o prompt de limpeza e mede o tempo.

Uso: limpar.py <arquivo.txt> [--porta 18434] [--vocab "A, B"]
"""
import json
import sys
import time
import urllib.request
from pathlib import Path

RAIZ = Path(__file__).resolve().parent.parent
porta = sys.argv[sys.argv.index("--porta") + 1] if "--porta" in sys.argv else "18434"
vocab = sys.argv[sys.argv.index("--vocab") + 1] if "--vocab" in sys.argv else ""
texto = Path(sys.argv[1]).read_text().strip()
sistema = (RAIZ / "Recursos" / "prompt-limpeza.md").read_text()
usuario = ""
if vocab:
    usuario += f"<vocabulario>{vocab}</vocabulario>\n"
usuario += f"<transcricao>{texto}</transcricao>"
corpo = {
    "messages": [{"role": "system", "content": sistema}, {"role": "user", "content": usuario}],
    "temperature": 0,
    "max_tokens": 4096,
    "cache_prompt": True,
}
t = time.time()
req = urllib.request.Request(
    f"http://127.0.0.1:{porta}/v1/chat/completions",
    data=json.dumps(corpo).encode(),
    headers={"Content-Type": "application/json"},
)
r = json.load(urllib.request.urlopen(req, timeout=600))
dt = time.time() - t
print(r["choices"][0]["message"]["content"])
tm = r.get("timings", {})
print(
    f"\n[{dt:.2f}s  prompt={tm.get('prompt_n')}tok {tm.get('prompt_per_second', 0):.0f}/s  "
    f"saida={tm.get('predicted_n')}tok {tm.get('predicted_per_second', 0):.0f}/s]",
    file=sys.stderr,
)
