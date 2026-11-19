#!/bin/bash
# Gera o par de chaves do atualizador do Tauri (gratuito, não é certificado de código)
# e guarda a chave privada e a senha SÓ como secrets do repositório no GitHub.
# A chave pública vai para windows/updater.pub, que a casca embute no app.
#
# Uso, na raiz do repositório, com o gh logado numa conta que pode gravar secrets:
#   scripts/windows/gerar-chaves-atualizador.sh [dono/repo]
#
# Nada da chave privada é impresso. Os arquivos temporários são apagados no fim,
# com ou sem erro. Se os secrets falharem, windows/updater.pub não é alterado.
# Atenção: rodar de novo troca a chave; apps já instalados com a chave antiga
# deixam de aceitar atualizações e precisam ser reinstalados pelo install.ps1.
set -euo pipefail

REPO="${1:-stoll2003/ditase}"
RAIZ="$(git rev-parse --show-toplevel)"
DESTINO="$RAIZ/windows/updater.pub"

TMP="$(mktemp -d "${TMPDIR:-/tmp}/ditase-chave.XXXXXXXX")"
chmod 700 "$TMP"
limpar() { rm -rf "$TMP"; }
trap limpar EXIT

if ! gh api "repos/$REPO/actions/secrets/public-key" >/dev/null 2>&1; then
  echo "O gh logado não tem permissão para gravar secrets em $REPO." >&2
  echo "Rode 'gh auth login' com a sua conta (escopo repo) e tente de novo." >&2
  exit 1
fi

openssl rand -base64 32 | tr -d '\n' > "$TMP/senha"
npx -y @tauri-apps/cli@2 signer generate -w "$TMP/updater.key" -p "$(cat "$TMP/senha")" --ci >/dev/null 2>&1

gh secret set TAURI_SIGNING_PRIVATE_KEY --repo "$REPO" < "$TMP/updater.key"
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --repo "$REPO" < "$TMP/senha"

for nome in TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD; do
  if ! gh secret list --repo "$REPO" --json name --jq '.[].name' | grep -qx "$nome"; then
    echo "Secret $nome não aparece em $REPO. windows/updater.pub não foi alterado." >&2
    exit 1
  fi
done

cp "$TMP/updater.key.pub" "$DESTINO"
echo "Secrets TAURI_SIGNING_PRIVATE_KEY e TAURI_SIGNING_PRIVATE_KEY_PASSWORD gravados em $REPO."
echo "Chave pública em windows/updater.pub. Faça commit desse arquivo."
