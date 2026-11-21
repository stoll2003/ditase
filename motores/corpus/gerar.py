"""Gera o corpus sintético pt-BR com as vozes do macOS (say + afconvert, 16 kHz mono 16 bits).

Uso (no Mac, de qualquer pasta): python3 -I motores/corpus/gerar.py
Só a conversa de duas vozes naturais (prova de ponta a ponta): ... gerar.py --conversa-2-vozes
Os WAVs e as referências ficam nesta pasta; o CI Windows só lê os arquivos já gerados.
Grupos (cada um é uma pasta que o `medir --corpus` lê):
- `.` (principal): Luciana, a voz pt-BR natural do macOS, em ritmo normal e rápido.
- `pt-pt/`: Joana (pt-PT), controle, porque o Parakeet v3 foi treinado com português europeu.
- `roboticas/`: vozes Eloquence pt-BR (Eddy, Flo, Reed, Rocko, Sandy): fala sintética de baixa
  qualidade, para teste de robustez; não representa voz humana.
Textos inventados, sem dado pessoal. Sem números por extenso nem algarismos, para o WER não
depender de normalização de numerais.
"""
import json
import os
import subprocess
import sys
import tempfile
import wave

AQUI = os.path.dirname(os.path.abspath(__file__))

TEXTOS = [
    "bom dia, preciso que você revise a petição inicial antes do almoço.",
    "o prazo para apresentar a contestação termina na próxima semana.",
    "por favor, envie a minuta do contrato para a cliente conferir as cláusulas.",
    "a audiência de conciliação foi remarcada para quinta-feira à tarde.",
    "você pode verificar se o juiz já despachou o pedido de tutela de urgência?",
    "anota aí: comprar café, ligar para o cartório e organizar a pasta do processo.",
    "o recurso de apelação precisa citar a jurisprudência do tribunal sobre danos morais.",
    "hoje à noite eu vou estudar direito civil e depois assistir à aula de processo penal.",
    "a reunião com o perito ficou para depois do feriado, então temos mais tempo para preparar os quesitos.",
    "quando terminar o relatório, salve uma cópia na nuvem e avise a equipe pelo grupo.",
    "não esqueça de juntar a procuração e os documentos pessoais antes de protocolar.",
    "o sistema ficou lento de manhã, mas agora voltou a funcionar normalmente.",
]

# (subpasta, prefixo, voz, ritmo em palavras por minuto ou None)
GRUPOS = [
    (".", "luciana", "Luciana", None),
    (".", "luciana-rapida", "Luciana", 230),
    ("pt-pt", "joana", "Joana", None),
]
ROBOTICAS = ["Eddy", "Flo", "Reed", "Sandy", "Rocko"]

CONVERSA = [
    ("Luciana", "voz-a", "olá, tudo bem? vamos começar a reunião sobre o caso da empresa."),
    ("Reed", "voz-b", "tudo certo. eu li os documentos e acho que temos bons argumentos."),
    ("Luciana", "voz-a", "ótimo. qual é o ponto mais forte da nossa defesa?"),
    ("Reed", "voz-b", "a falta de provas do dano. a outra parte não juntou nenhum laudo."),
    ("Sandy", "voz-c", "concordo, mas precisamos responder à questão da prescrição também."),
    ("Luciana", "voz-a", "bem lembrado. então fechamos assim e cada um prepara a sua parte."),
]

# Duas vozes naturais (Luciana pt-BR e Joana pt-PT): o WER do arquivo e da reunião de ponta a
# ponta só vale com vozes que o Parakeet reconhece; as robóticas da conversa de três vozes servem
# para a separação de falantes, não para o texto.
CONVERSA_2 = [
    ("Luciana", "voz-a", "bom dia a todos. hoje vamos revisar a minuta do acordo com o fornecedor."),
    ("Joana", "voz-b", "bom dia. eu li a minuta ontem e encontrei duas cláusulas que preocupam."),
    ("Luciana", "voz-a", "quais são as cláusulas? começa pela mais urgente, por favor."),
    ("Joana", "voz-b", "a multa por atraso está muito alta e o prazo de entrega ficou curto."),
    ("Luciana", "voz-a", "concordo. vou pedir ao fornecedor uma proposta nova ainda esta semana."),
    ("Joana", "voz-b", "perfeito. eu preparo um resumo das mudanças e envio para a equipe."),
]


def falar(voz, texto, destino, tmp, ritmo=None):
    aiff = os.path.join(tmp, "f.aiff")
    subprocess.run(["say", "-v", voz, "-o", aiff] + (["-r", str(ritmo)] if ritmo else []) + [texto], check=True)
    subprocess.run(["afconvert", "-f", "WAVE", "-d", "LEI16@16000", "-c", "1", aiff, destino], check=True)


def ler(caminho):
    with wave.open(caminho, "rb") as w:
        assert w.getframerate() == 16000 and w.getnchannels() == 1 and w.getsampwidth() == 2
        return w.readframes(w.getnframes())


def gravar(caminho, quadros):
    with wave.open(caminho, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(16000)
        w.writeframes(quadros)


def main():
    if "--conversa-2-vozes" in sys.argv:
        with tempfile.TemporaryDirectory() as tmp:
            conversa("conversa-2-vozes", CONVERSA_2, tmp, m4a=True)
        print("conversa de duas vozes gerada em", os.path.join(AQUI, "falantes"))
        return
    vozes = subprocess.run(["say", "-v", "?"], capture_output=True, text=True, check=True).stdout
    usadas = {g[2] for g in GRUPOS} | set(ROBOTICAS) | {v for v, _, _ in CONVERSA + CONVERSA_2}
    faltam = [v for v in usadas if not any(l.startswith(v + " ") for l in vozes.splitlines())]
    if faltam:
        sys.exit(f"vozes ausentes neste Mac: {faltam}")
    with tempfile.TemporaryDirectory() as tmp:
        trabalhos = []
        for sub, prefixo, voz, ritmo in GRUPOS:
            textos = TEXTOS if sub == "." and ritmo is None else TEXTOS[::2]
            for i, texto in enumerate(textos, 1):
                trabalhos.append((sub, f"{prefixo}-{i:02d}", voz, ritmo, texto))
        for i, texto in enumerate(TEXTOS[:len(ROBOTICAS) * 2], 0):
            voz = ROBOTICAS[i % len(ROBOTICAS)]
            trabalhos.append(("roboticas", f"{voz.lower()}-{i + 1:02d}", voz, None, texto))
        for sub, base, voz, ritmo, texto in trabalhos:
            os.makedirs(os.path.join(AQUI, sub), exist_ok=True)
            nome = os.path.join(AQUI, sub, base)
            falar(voz, texto, nome + ".wav", tmp, ritmo)
            with open(nome + ".txt", "w", encoding="utf-8") as f:
                f.write(texto + "\n")
        conversa("conversa-3-vozes", CONVERSA, tmp)
        conversa("conversa-2-vozes", CONVERSA_2, tmp, m4a=True)
    print("corpus gerado em", AQUI)


def conversa(nome, falas, tmp, m4a=False):
    """Falas em sequência com 0,5 s de silêncio; JSON com voz, início, fim e texto de cada uma."""
    os.makedirs(os.path.join(AQUI, "falantes"), exist_ok=True)
    silencio = b"\x00\x00" * 8000  # 0,5 s entre falas
    audio = b""
    faixas = []
    for voz, rotulo, texto in falas:
        p = os.path.join(tmp, "c.wav")
        falar(voz, texto, p, tmp)
        q = ler(p)
        inicio = len(audio) / 2 / 16000
        audio += q
        faixas.append({"voz": rotulo, "inicio": round(inicio, 3), "fim": round(len(audio) / 2 / 16000, 3), "texto": texto})
        audio += silencio
    base = os.path.join(AQUI, "falantes", nome)
    gravar(base + ".wav", audio)
    with open(base + ".json", "w", encoding="utf-8") as f:
        json.dump(faixas, f, ensure_ascii=False, indent=1)
    if m4a:
        # AAC em MP4: no Windows passa pelo Media Foundation, como áudio de celular.
        if os.path.exists(base + ".m4a"):
            os.remove(base + ".m4a")
        subprocess.run(["afconvert", "-f", "m4af", "-d", "aac", "-b", "48000", base + ".wav", base + ".m4a"], check=True)


if __name__ == "__main__":
    main()
