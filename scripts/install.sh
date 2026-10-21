#!/bin/sh
set -eu

REPO="${DEVZYMER_REPO:-devadeiltonlima/devzymer}"
VERSION="${DEVZYMER_VERSION:-latest}"
INSTALL_DIR="${DEVZYMER_INSTALL_DIR:-$HOME/.local/bin}"

fail() {
    printf 'devzymer: %s\n' "$1" >&2
    exit 1
}

case "$(uname -s)" in
    Linux) triple_os="unknown-linux-gnu" ;;
    Darwin) triple_os="apple-darwin" ;;
    *) fail "sistema não suportado: $(uname -s). No Windows use install.ps1" ;;
esac

case "$(uname -m)" in
    x86_64 | amd64) cpu="x64" ;;
    arm64 | aarch64) cpu="arm64" ;;
    *) fail "arquitetura não suportada: $(uname -m)" ;;
esac

archive="devzymer-${cpu}-${triple_os}.tar.gz"

if [ -n "${DEVZYMER_BASE_URL:-}" ]; then
    base="$DEVZYMER_BASE_URL"
elif [ "$VERSION" = "latest" ]; then
    base="https://github.com/$REPO/releases/latest/download"
else
    base="https://github.com/$REPO/releases/download/$VERSION"
fi

download() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$1" -O "$2"
    else
        fail "é preciso ter curl ou wget"
    fi
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        fail "é preciso ter sha256sum ou shasum para verificar o download"
    fi
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

printf 'devzymer: baixando %s\n' "$archive"
download "$base/$archive" "$tmp/$archive" || fail "não foi possível baixar $base/$archive"
download "$base/$archive.sha256" "$tmp/$archive.sha256" || fail "não foi possível baixar $base/$archive.sha256"

expected="$(awk 'NR==1 {print tolower($1)}' "$tmp/$archive.sha256")"
actual="$(sha256_of "$tmp/$archive")"
[ -n "$expected" ] && [ "$expected" = "$actual" ] || fail "o checksum do download não confere; nada foi instalado"

tar -xzf "$tmp/$archive" -C "$tmp" devzymer
mkdir -p "$INSTALL_DIR"
install -m 755 "$tmp/devzymer" "$INSTALL_DIR/.devzymer.new"
mv -f "$INSTALL_DIR/.devzymer.new" "$INSTALL_DIR/devzymer"

printf 'devzymer: instalado em %s/devzymer\n' "$INSTALL_DIR"
"$INSTALL_DIR/devzymer" --version || true

case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        printf '\ndevzymer: %s não está no PATH. Adicione ao seu shell:\n' "$INSTALL_DIR"
        printf '  export PATH="%s:$PATH"\n' "$INSTALL_DIR"
        ;;
esac

printf '\nPróximo passo, dentro do seu projeto:\n  devzymer init\n'
