#!/bin/sh
set -eu

REPO="${DEVZYMER_REPO:-devadeiltonlima/devzymer}"
VERSION="${DEVZYMER_VERSION:-latest}"
INSTALL_DIR="${DEVZYMER_INSTALL_DIR:-$HOME/.local/bin}"

fancy=0
animate=0
truecolor=0
unicode=0
if { [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-}" != "dumb" ]; } || [ -n "${DEVZYMER_FORCE_FANCY:-}" ]; then
    fancy=1
fi
if [ "$fancy" = 1 ] && [ -t 1 ] && [ -z "${DEVZYMER_NO_ANIM:-}" ] && [ -z "${CI:-}" ]; then
    animate=1
fi
case "${COLORTERM:-}" in
    *truecolor* | *24bit*) truecolor=1 ;;
esac
case "${LC_ALL:-${LC_CTYPE:-${LANG:-}}}" in
    *UTF-8* | *utf-8* | *UTF8* | *utf8*) unicode=1 ;;
esac

ESC="$(printf '\033')"
if [ "$truecolor" = 1 ]; then
    GOOD="${ESC}[38;2;63;185;80m"
    BAD="${ESC}[38;2;248;81;73m"
    MUTED="${ESC}[38;2;139;148;158m"
    ACCENT="${ESC}[38;2;0;217;245m"
else
    GOOD="${ESC}[32m"
    BAD="${ESC}[31m"
    MUTED="${ESC}[90m"
    ACCENT="${ESC}[36m"
fi
RESET="${ESC}[0m"

ROW_P1=' ██████████   ██████████ █████   █████ ███████████ █████ █████ ██████   ██████ ██████████ ███████████'
ROW_P2='▒▒███▒▒▒▒███ ▒▒███▒▒▒▒▒█▒▒███   ▒▒███ ▒█▒▒▒▒▒▒███ ▒▒███ ▒▒███ ▒▒██████ ██████ ▒▒███▒▒▒▒▒█▒▒███▒▒▒▒▒███'
ROW_P3=' ▒███   ▒▒███ ▒███  █ ▒  ▒███    ▒███ ▒     ███▒   ▒▒███ ███   ▒███▒█████▒███  ▒███  █ ▒  ▒███    ▒███'
ROW_P4=' ▒███    ▒███ ▒██████    ▒███    ▒███      ███      ▒▒█████    ▒███▒▒███ ▒███  ▒██████    ▒██████████'
ROW_P5=' ▒███    ▒███ ▒███▒▒█    ▒▒███   ███      ███        ▒▒███     ▒███ ▒▒▒  ▒███  ▒███▒▒█    ▒███▒▒▒▒▒███'
ROW_P6=' ▒███    ███  ▒███ ▒   █  ▒▒▒█████▒     ████     █    ▒███     ▒███      ▒███  ▒███ ▒   █ ▒███    ▒███'
ROW_P7=' ██████████   ██████████    ▒▒███      ███████████    █████    █████     █████ ██████████ █████   █████'
ROW_P8='▒▒▒▒▒▒▒▒▒▒   ▒▒▒▒▒▒▒▒▒▒      ▒▒▒      ▒▒▒▒▒▒▒▒▒▒▒    ▒▒▒▒▒    ▒▒▒▒▒     ▒▒▒▒▒ ▒▒▒▒▒▒▒▒▒▒ ▒▒▒▒▒   ▒▒▒▒▒'
ROW_C1=' \033[38;2;0;244;162m██\033[38;2;0;243;167m███\033[38;2;0;241;172m███\033[38;2;0;240;177m██   \033[38;2;0;238;182m█\033[38;2;0;236;187m███\033[38;2;0;235;192m███\033[38;2;0;233;197m███ \033[38;2;0;231;202m██\033[38;2;0;230;207m███   \033[38;2;0;226;217m███\033[38;2;0;225;222m██ \033[38;2;0;223;227m███\033[38;2;0;221;232m███\033[38;2;0;220;237m███\033[38;2;0;218;242m██ \033[38;2;2;214;245m███\033[38;2;10;206;246m██ \033[38;2;17;198;246m███\033[38;2;24;190;247m██ \033[38;2;32;181;248m███\033[38;2;39;173;248m███   \033[38;2;53;157;249m███\033[38;2;61;148;250m███ \033[38;2;68;140;250m██\033[38;2;75;132;251m███\033[38;2;83;124;252m███\033[38;2;90;115;252m██ \033[38;2;97;107;253m███\033[38;2;105;99;253m███\033[38;2;112;91;254m███\033[38;2;119;82;255m██\033[0m'
ROW_C2='\033[38;2;8;112;81m▒▒\033[38;2;0;244;162m█\033[38;2;0;243;167m██\033[38;2;8;112;83m▒\033[38;2;8;111;86m▒▒▒\033[38;2;0;240;177m███ \033[38;2;8;110;90m▒▒\033[38;2;0;236;187m███\033[38;2;8;109;94m▒▒▒\033[38;2;8;108;96m▒▒\033[38;2;0;233;197m█\033[38;2;8;107;98m▒▒\033[38;2;0;231;202m█\033[38;2;0;230;207m██   \033[38;2;8;106;102m▒\033[38;2;8;105;104m▒\033[38;2;0;226;217m██\033[38;2;0;225;222m█ \033[38;2;8;104;107m▒\033[38;2;0;223;227m█\033[38;2;8;104;109m▒▒\033[38;2;8;103;111m▒▒▒\033[38;2;8;102;113m▒\033[38;2;0;220;237m██\033[38;2;0;218;242m█ \033[38;2;8;101;115m▒\033[38;2;8;100;116m▒\033[38;2;2;214;245m██\033[38;2;10;206;246m█ \033[38;2;12;96;117m▒\033[38;2;15;93;117m▒\033[38;2;17;198;246m██\033[38;2;24;190;247m█ \033[38;2;18;90;117m▒\033[38;2;21;86;118m▒\033[38;2;32;181;248m██\033[38;2;39;173;248m███\033[38;2;46;165;249m█ █\033[38;2;53;157;249m███\033[38;2;61;148;250m██ \033[38;2;36;69;118m▒▒\033[38;2;68;140;250m█\033[38;2;75;132;251m██\033[38;2;39;65;119m▒\033[38;2;42;62;119m▒▒▒\033[38;2;45;58;119m▒\033[38;2;90;115;252m█\033[38;2;45;58;119m▒\033[38;2;48;55;120m▒\033[38;2;97;107;253m██\033[38;2;105;99;253m█\033[38;2;52;51;120m▒▒\033[38;2;55;48;120m▒▒▒\033[38;2;119;82;255m███\033[0m'
ROW_C3=' \033[38;2;8;112;81m▒\033[38;2;0;244;162m█\033[38;2;0;243;167m██   \033[38;2;8;111;86m▒\033[38;2;8;111;88m▒\033[38;2;0;240;177m██\033[38;2;0;238;182m█ \033[38;2;8;110;90m▒\033[38;2;0;236;187m███  \033[38;2;0;235;192m█ \033[38;2;8;108;96m▒  \033[38;2;8;107;98m▒\033[38;2;0;231;202m█\033[38;2;0;230;207m██    \033[38;2;8;105;104m▒\033[38;2;0;226;217m██\033[38;2;0;225;222m█ \033[38;2;8;104;107m▒     \033[38;2;0;221;232m█\033[38;2;0;220;237m██\033[38;2;8;102;113m▒   \033[38;2;8;100;116m▒▒\033[38;2;2;214;245m█\033[38;2;10;206;246m██ \033[38;2;17;198;246m███   \033[38;2;21;86;118m▒\033[38;2;32;181;248m██\033[38;2;39;173;248m█\033[38;2;24;83;118m▒\033[38;2;39;173;248m█\033[38;2;46;165;249m███\033[38;2;53;157;249m█\033[38;2;30;76;118m▒\033[38;2;53;157;249m█\033[38;2;61;148;250m██  \033[38;2;36;69;118m▒\033[38;2;68;140;250m█\033[38;2;75;132;251m██  \033[38;2;83;124;252m█ \033[38;2;45;58;119m▒  \033[38;2;48;55;120m▒\033[38;2;97;107;253m██\033[38;2;105;99;253m█    \033[38;2;55;48;120m▒\033[38;2;119;82;255m███\033[0m'
ROW_C4=' \033[38;2;8;112;81m▒\033[38;2;0;244;162m█\033[38;2;0;243;167m██    \033[38;2;8;111;88m▒\033[38;2;0;240;177m██\033[38;2;0;238;182m█ \033[38;2;8;110;90m▒\033[38;2;0;236;187m███\033[38;2;0;235;192m███    \033[38;2;8;107;98m▒\033[38;2;0;231;202m█\033[38;2;0;230;207m██    \033[38;2;8;105;104m▒\033[38;2;0;226;217m██\033[38;2;0;225;222m█      \033[38;2;0;221;232m██\033[38;2;0;220;237m█      \033[38;2;8;100;116m▒▒\033[38;2;10;206;246m███\033[38;2;17;198;246m██    \033[38;2;21;86;118m▒\033[38;2;32;181;248m██\033[38;2;39;173;248m█\033[38;2;24;83;118m▒▒\033[38;2;46;165;249m███ \033[38;2;30;76;118m▒\033[38;2;53;157;249m█\033[38;2;61;148;250m██  \033[38;2;36;69;118m▒\033[38;2;68;140;250m█\033[38;2;75;132;251m███\033[38;2;83;124;252m██    \033[38;2;48;55;120m▒\033[38;2;97;107;253m██\033[38;2;105;99;253m███\033[38;2;112;91;254m███\033[38;2;119;82;255m██\033[0m'
ROW_C5=' \033[38;2;8;112;81m▒\033[38;2;0;244;162m█\033[38;2;0;243;167m██    \033[38;2;8;111;88m▒\033[38;2;0;240;177m██\033[38;2;0;238;182m█ \033[38;2;8;110;90m▒\033[38;2;0;236;187m███\033[38;2;8;109;94m▒▒\033[38;2;0;235;192m█    \033[38;2;8;107;98m▒▒\033[38;2;0;230;207m███   \033[38;2;0;226;217m███      \033[38;2;0;221;232m███        \033[38;2;8;100;116m▒\033[38;2;12;96;117m▒\033[38;2;10;206;246m██\033[38;2;17;198;246m█     \033[38;2;21;86;118m▒\033[38;2;32;181;248m██\033[38;2;39;173;248m█ \033[38;2;24;83;118m▒\033[38;2;27;79;118m▒▒  \033[38;2;30;76;118m▒\033[38;2;53;157;249m█\033[38;2;61;148;250m██  \033[38;2;36;69;118m▒\033[38;2;68;140;250m█\033[38;2;75;132;251m██\033[38;2;39;65;119m▒\033[38;2;42;62;119m▒\033[38;2;83;124;252m█    \033[38;2;48;55;120m▒\033[38;2;97;107;253m██\033[38;2;105;99;253m█\033[38;2;52;51;120m▒▒\033[38;2;55;48;120m▒▒▒\033[38;2;119;82;255m███\033[0m'
ROW_C6=' \033[38;2;8;112;81m▒\033[38;2;0;244;162m█\033[38;2;0;243;167m██    \033[38;2;0;240;177m███  \033[38;2;8;110;90m▒\033[38;2;0;236;187m███ \033[38;2;8;109;94m▒   \033[38;2;0;233;197m█  \033[38;2;8;107;98m▒\033[38;2;8;106;100m▒▒\033[38;2;0;230;207m█\033[38;2;0;228;212m███\033[38;2;0;226;217m█\033[38;2;8;105;104m▒     \033[38;2;0;223;227m██\033[38;2;0;221;232m██     \033[38;2;0;218;242m█    \033[38;2;12;96;117m▒\033[38;2;10;206;246m██\033[38;2;17;198;246m█     \033[38;2;21;86;118m▒\033[38;2;32;181;248m██\033[38;2;39;173;248m█      \033[38;2;30;76;118m▒\033[38;2;53;157;249m█\033[38;2;61;148;250m██  \033[38;2;36;69;118m▒\033[38;2;68;140;250m█\033[38;2;75;132;251m██ \033[38;2;42;62;119m▒   \033[38;2;90;115;252m█ \033[38;2;48;55;120m▒\033[38;2;97;107;253m██\033[38;2;105;99;253m█    \033[38;2;55;48;120m▒\033[38;2;119;82;255m███\033[0m'
ROW_C7=' \033[38;2;0;244;162m██\033[38;2;0;243;167m███\033[38;2;0;241;172m███\033[38;2;0;240;177m██   \033[38;2;0;238;182m█\033[38;2;0;236;187m███\033[38;2;0;235;192m███\033[38;2;0;233;197m███    \033[38;2;8;106;100m▒▒\033[38;2;0;228;212m███      \033[38;2;0;223;227m███\033[38;2;0;221;232m███\033[38;2;0;220;237m███\033[38;2;0;218;242m██    \033[38;2;10;206;246m███\033[38;2;17;198;246m██    \033[38;2;32;181;248m███\033[38;2;39;173;248m██     \033[38;2;53;157;249m██\033[38;2;61;148;250m███ \033[38;2;68;140;250m██\033[38;2;75;132;251m███\033[38;2;83;124;252m███\033[38;2;90;115;252m██ \033[38;2;97;107;253m███\033[38;2;105;99;253m██   \033[38;2;112;91;254m█\033[38;2;119;82;255m███\033[38;2;124;77;255m█\033[0m'
ROW_C8='\033[38;2;8;112;81m▒▒▒\033[38;2;8;112;83m▒▒▒\033[38;2;8;111;86m▒▒▒\033[38;2;8;111;88m▒   \033[38;2;8;110;90m▒▒\033[38;2;8;109;92m▒▒▒\033[38;2;8;109;94m▒▒▒\033[38;2;8;108;96m▒▒      \033[38;2;8;106;100m▒\033[38;2;8;106;102m▒▒      \033[38;2;8;104;107m▒\033[38;2;8;104;109m▒▒▒\033[38;2;8;103;111m▒▒▒\033[38;2;8;102;113m▒▒▒\033[38;2;8;101;115m▒    \033[38;2;8;100;116m▒\033[38;2;12;96;117m▒▒▒\033[38;2;15;93;117m▒    \033[38;2;18;90;117m▒\033[38;2;21;86;118m▒▒▒\033[38;2;24;83;118m▒     \033[38;2;30;76;118m▒▒▒\033[38;2;33;72;118m▒▒ \033[38;2;36;69;118m▒▒▒\033[38;2;39;65;119m▒▒▒\033[38;2;42;62;119m▒▒▒\033[38;2;45;58;119m▒ ▒\033[38;2;48;55;120m▒▒▒\033[38;2;52;51;120m▒   \033[38;2;55;48;120m▒▒\033[38;2;58;44;120m▒▒▒\033[0m'

fail() {
    if [ "$fancy" = 1 ]; then
        printf '\r%s[2K' "$ESC"
    fi
    printf 'devzymer: %s\n' "$1" >&2
    exit 1
}

step() {
    if [ "$1" = ok ]; then
        color="$GOOD"
        if [ "$unicode" = 1 ]; then symbol="✔"; else symbol="+"; fi
    else
        color="$BAD"
        if [ "$unicode" = 1 ]; then symbol="✖"; else symbol="x"; fi
    fi
    printf '\r%s[2K  %s%s%s %-34s%s%s%s\n' "$ESC" "$color" "$symbol" "$RESET" "$2" "$MUTED" "$3" "$RESET"
}

human() {
    awk -v bytes="$1" 'BEGIN { printf "%.1f MB", bytes / 1048576 }'
}

file_size() {
    if [ -f "$1" ]; then wc -c < "$1" | tr -d ' '; else echo 0; fi
}

show_banner() {
    columns="$(tput cols 2>/dev/null || echo 120)"
    printf '\n'
    if [ "$columns" -lt 105 ]; then
        printf '  %s%sDEVZYMER%s\n  Less Tokens. More Engineering.\n\n' "${ESC}[1m" "$ACCENT" "$RESET"
        return
    fi
    for n in 1 2 3 4 5 6 7 8; do
        if [ "$truecolor" = 1 ]; then
            eval "row=\$ROW_C$n"
            printf '%b\n' "$row"
        else
            eval "row=\$ROW_P$n"
            printf '%s%s%s\n' "$ACCENT" "$row" "$RESET"
        fi
        if [ "$animate" = 1 ]; then sleep 0.06 2>/dev/null || true; fi
    done
    printf ' %s%sLess Tokens. More Engineering.%s\n\n' "${ESC}[3m" "$MUTED" "$RESET"
}

if [ "$fancy" = 0 ]; then
    GOOD=""
    BAD=""
    MUTED=""
    ACCENT=""
    RESET=""
fi

case "$(uname -s)" in
    Linux) triple_os="unknown-linux-gnu"; platform_os="Linux" ;;
    Darwin) triple_os="apple-darwin"; platform_os="macOS" ;;
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

fetch() {
    if [ "$animate" = 1 ]; then
        download "$1" "$2" &
        pid=$!
        i=0
        while kill -0 "$pid" 2>/dev/null; do
            if [ "$unicode" = 1 ]; then
                case $((i % 10)) in
                    0) frame="⠋" ;; 1) frame="⠙" ;; 2) frame="⠹" ;; 3) frame="⠸" ;; 4) frame="⠼" ;;
                    5) frame="⠴" ;; 6) frame="⠦" ;; 7) frame="⠧" ;; 8) frame="⠇" ;; *) frame="⠏" ;;
                esac
            else
                case $((i % 4)) in 0) frame="|" ;; 1) frame="/" ;; 2) frame="-" ;; *) frame="\\" ;; esac
            fi
            printf '\r%s[2K  %s%s%s %s  %s%s%s' "$ESC" "$ACCENT" "$frame" "$RESET" "$3" "$MUTED" "$(human "$(file_size "$2")")" "$RESET"
            i=$((i + 1))
            sleep 0.1 2>/dev/null || true
        done
        wait "$pid"
    else
        download "$1" "$2"
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
trap 'rm -rf "$tmp"; if [ "$animate" = 1 ]; then printf "%s[?25h" "$ESC"; fi' EXIT

if [ "$fancy" = 1 ]; then
    if [ "$animate" = 1 ]; then printf '%s[?25l' "$ESC"; fi
    show_banner
    printf '  %s%sInstalando o devzymer%s\n\n' "${ESC}[1m" "$ACCENT" "$RESET"
    step ok "Plataforma detectada" "$platform_os $cpu"
else
    printf 'devzymer: baixando %s\n' "$archive"
fi

fetch "$base/$archive" "$tmp/$archive" "Baixando o devzymer" || {
    if [ "$fancy" = 1 ]; then step fail "Baixando o devzymer" "falhou"; fi
    fail "não foi possível baixar $base/$archive"
}
fetch "$base/$archive.sha256" "$tmp/$archive.sha256" "Baixando o checksum" || {
    if [ "$fancy" = 1 ]; then step fail "Baixando o checksum" "falhou"; fi
    fail "não foi possível baixar $base/$archive.sha256"
}
if [ "$fancy" = 1 ]; then
    step ok "Baixando o devzymer" "$(human "$(file_size "$tmp/$archive")")"
fi

expected="$(awk 'NR==1 {print tolower($1)}' "$tmp/$archive.sha256")"
actual="$(sha256_of "$tmp/$archive")"
if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
    if [ "$fancy" = 1 ]; then step fail "Conferindo o SHA-256" "não confere"; fi
    fail "o checksum do download não confere; nada foi instalado"
fi
if [ "$fancy" = 1 ]; then
    step ok "Conferindo o SHA-256" "$(printf '%s' "$actual" | cut -c1-16)"
fi

tar -xzf "$tmp/$archive" -C "$tmp" devzymer
mkdir -p "$INSTALL_DIR"
install -m 755 "$tmp/devzymer" "$INSTALL_DIR/.devzymer.new"
mv -f "$INSTALL_DIR/.devzymer.new" "$INSTALL_DIR/devzymer"

if [ "$fancy" = 1 ]; then
    step ok "Instalando" "$INSTALL_DIR/devzymer"
    reported="$("$INSTALL_DIR/devzymer" --version 2>/dev/null || echo "instalado")"
    step ok "Verificando o executavel" "$reported"
else
    printf 'devzymer: instalado em %s/devzymer\n' "$INSTALL_DIR"
    "$INSTALL_DIR/devzymer" --version || true
fi

case ":$PATH:" in
    *":$INSTALL_DIR:"*)
        if [ "$fancy" = 1 ]; then step ok "PATH" "já configurado"; fi
        ;;
    *)
        if [ "$fancy" = 1 ]; then
            step ok "PATH" "adicione ao seu shell: export PATH=\"$INSTALL_DIR:\$PATH\""
        else
            printf '\ndevzymer: %s não está no PATH. Adicione ao seu shell:\n' "$INSTALL_DIR"
            printf '  export PATH="%s:$PATH"\n' "$INSTALL_DIR"
        fi
        ;;
esac

if [ "$fancy" = 1 ]; then
    printf '\n  %s%sPronto. Próximo passo, dentro do seu projeto:%s\n\n' "${ESC}[1m" "$ACCENT" "$RESET"
    printf '    devzymer init   %s# escolha os agentes e indexe o projeto%s\n\n' "$MUTED" "$RESET"
else
    printf '\nPróximo passo, dentro do seu projeto:\n  devzymer init\n'
fi
