#!/bin/bash
# Cria, uma vez, uma identidade de assinatura local ("Ditado Local") num chaveiro próprio.
# Sem identidade estável o macOS trata cada build como um app novo e as permissões de
# Microfone e Acessibilidade precisam ser dadas de novo a cada instalação.
set -euo pipefail
NOME="Ditado Local"
CHAVEIRO="$HOME/Library/Keychains/ditado-assinatura.keychain-db"
SENHA_ARQ="$HOME/Library/Application Support/Ditado/assinatura.senha"
mkdir -p "$(dirname "$SENHA_ARQ")"
if [ ! -f "$CHAVEIRO" ]; then
  TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
  SENHA=$(openssl rand -hex 16); (umask 077; printf '%s' "$SENHA" > "$SENHA_ARQ")
  cat > "$TMP/cert.cnf" <<CNF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $NOME
[ext]
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
basicConstraints = critical, CA:false
CNF
  openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -keyout "$TMP/chave.pem" -out "$TMP/cert.pem" -config "$TMP/cert.cnf" 2>/dev/null
  openssl pkcs12 -export -legacy -inkey "$TMP/chave.pem" -in "$TMP/cert.pem" -out "$TMP/id.p12" -passout "pass:$SENHA" -name "$NOME" 2>/dev/null \
    || openssl pkcs12 -export -inkey "$TMP/chave.pem" -in "$TMP/cert.pem" -out "$TMP/id.p12" -passout "pass:$SENHA" -name "$NOME"
  security create-keychain -p "$SENHA" "$CHAVEIRO"
  security set-keychain-settings "$CHAVEIRO"
  security unlock-keychain -p "$SENHA" "$CHAVEIRO"
  security import "$TMP/id.p12" -k "$CHAVEIRO" -P "$SENHA" -T /usr/bin/codesign >/dev/null
  security set-key-partition-list -S apple-tool:,apple: -s -k "$SENHA" "$CHAVEIRO" >/dev/null
  # Entra na lista de busca do usuário, sem tirar os chaveiros que já estão lá.
  security list-keychains -d user -s "$CHAVEIRO" $(security list-keychains -d user | sed 's/"//g')
fi
security unlock-keychain -p "$(cat "$SENHA_ARQ")" "$CHAVEIRO"
echo "$NOME"
