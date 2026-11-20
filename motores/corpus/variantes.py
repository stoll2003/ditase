"""Monta conversas de 2 e 4 vozes a partir dos WAVs sintéticos já versionados.

Uso: python3 motores/corpus/variantes.py DIRETORIO_NOVO
Depois: medir --corpus DIRETORIO_NOVO --falantes
"""
import argparse
import json
import shutil
import wave
from pathlib import Path

ORIGEM = Path(__file__).resolve().parent


def montar(destino):
    destino.mkdir(exist_ok=False, parents=True)
    for caminho in ORIGEM.glob("luciana-*"):
        shutil.copyfile(caminho, destino / caminho.name)
    falantes = destino / "falantes"
    shutil.copytree(ORIGEM / "falantes", falantes)
    for vozes in [("luciana", "joana"), ("eddy", "reed", "joana", "luciana")]:
        base = falantes / f"conversa-{len(vozes)}-vozes"
        turnos = []
        with wave.open(str(base.with_suffix(".wav")), "wb") as conversa:
            conversa.setparams((1, 2, 16000, 0, "NONE", "not compressed"))
            for rodada in range(2):
                for voz in vozes:
                    arquivo = sorted(ORIGEM.rglob(f"{voz}-??.wav"))[rodada]
                    with wave.open(str(arquivo), "rb") as fala:
                        assert fala.getparams()[:3] == (1, 2, 16000)
                        inicio = conversa.tell() / 16000
                        conversa.writeframes(fala.readframes(fala.getnframes()))
                    turnos.append(dict(voz=voz, inicio=inicio, fim=conversa.tell() / 16000))
                    conversa.writeframes(bytes(16000))
        base.with_suffix(".json").write_text(json.dumps(turnos), encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destino", type=Path)
    montar(parser.parse_args().destino)
