#!/usr/bin/env python3
"""Taxa de erro de palavra (WER) de hipóteses contra uma referência.

Uso: wer.py <dir_ref> <sufixo_ref> <dir_hip> [--mostrar N]
Compara <dir_ref>/<id><sufixo_ref> com <dir_hip>/<id>.txt.
A referência usada no projeto é o texto do Wispr Flow, que também erra:
o número serve para comparar motores entre si, não como verdade absoluta.
"""
import re
import sys
import unicodedata
from pathlib import Path


def normalizar(texto: str) -> list[str]:
    texto = unicodedata.normalize("NFC", texto.lower())
    texto = re.sub(r"[^\w\s]", " ", texto, flags=re.UNICODE)
    return texto.split()


def alinhar(ref: list[str], hip: list[str]):
    n, m = len(ref), len(hip)
    d = [[0] * (m + 1) for _ in range(n + 1)]
    for i in range(n + 1):
        d[i][0] = i
    for j in range(m + 1):
        d[0][j] = j
    for i in range(1, n + 1):
        ri = ref[i - 1]
        linha, acima = d[i], d[i - 1]
        for j in range(1, m + 1):
            custo = 0 if ri == hip[j - 1] else 1
            linha[j] = min(acima[j] + 1, linha[j - 1] + 1, acima[j - 1] + custo)
    i, j = n, m
    pares = []
    while i > 0 or j > 0:
        if i > 0 and j > 0 and d[i][j] == d[i - 1][j - 1] + (ref[i - 1] != hip[j - 1]):
            if ref[i - 1] != hip[j - 1]:
                pares.append((ref[i - 1], hip[j - 1]))
            i, j = i - 1, j - 1
        elif i > 0 and d[i][j] == d[i - 1][j] + 1:
            pares.append((ref[i - 1], ""))
            i -= 1
        else:
            pares.append(("", hip[j - 1]))
            j -= 1
    return d[n][m], list(reversed(pares))


def main() -> None:
    dir_ref, sufixo, dir_hip = Path(sys.argv[1]), sys.argv[2], Path(sys.argv[3])
    mostrar = int(sys.argv[sys.argv.index("--mostrar") + 1]) if "--mostrar" in sys.argv else 0
    total_erros = total_palavras = 0
    for ref_arq in sorted(dir_ref.glob(f"*{sufixo}")):
        ident = ref_arq.name[: -len(sufixo)]
        hip_arq = dir_hip / f"{ident}.txt"
        if not hip_arq.exists():
            continue
        ref = normalizar(ref_arq.read_text())
        hip = normalizar(hip_arq.read_text())
        erros, pares = alinhar(ref, hip)
        total_erros += erros
        total_palavras += len(ref)
        print(f"{ident}  palavras={len(ref):5d}  erros={erros:4d}  wer={100 * erros / max(1, len(ref)):5.1f}%")
        for r, h in pares[:mostrar]:
            print(f"      {r or '∅':>22} -> {h or '∅'}")
    print(f"TOTAL palavras={total_palavras} erros={total_erros} wer={100 * total_erros / max(1, total_palavras):.2f}%")


if __name__ == "__main__":
    main()
