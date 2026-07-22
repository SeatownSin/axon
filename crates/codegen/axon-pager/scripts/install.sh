#!/bin/bash
#
# Axon CLI installer
# https://github.com/SeatownSin/axon
#
# Downloads a released `axon` binary from this fork's GitHub Releases and
# installs it to ~/.axon/bin. Makes no calls to xAI infrastructure.
#
# Env: AXON_CHANNEL (stable|alpha, default: stable), AXON_BIN_DIR
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/SeatownSin/axon/main/crates/codegen/axon-pager/scripts/install.sh | bash            # latest stable
#   ... | bash -s 0.2.0   # specific version
#
# Windows: run under Git for Windows / MSYS2 Bash (same curl | bash flow); WSL
# uses the Linux binary.

set -e

TARGET="$1"

if [[ -n "$TARGET" ]] && [[ ! "$TARGET" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9._]+)?$ ]]; then
    echo "Invalid version format: $TARGET (expected X.Y.Z or X.Y.Z-suffix)" >&2
    exit 1
fi

REPO="SeatownSin/axon"

DOWNLOADER=""
if command -v curl >/dev/null 2>&1; then
    DOWNLOADER="curl"
elif command -v wget >/dev/null 2>&1; then
    DOWNLOADER="wget"
else
    echo "Either curl or wget is required but neither is installed" >&2
    exit 1
fi

download_file() {
    local url="$1" output="$2"
    if [ "$DOWNLOADER" = "curl" ]; then
        if [ -n "$output" ]; then
            curl -fsSL -o "$output" "$url"
        else
            curl -fsSL "$url"
        fi
    else
        if [ -n "$output" ]; then
            wget -q -O "$output" "$url"
        else
            wget -q -O - "$url"
        fi
    fi
}

# Parallel byte-range download. Falls back to single-connection download_file
# whenever HEAD lacks Content-Length, the file is small (<16 MiB), curl is
# unavailable, or any chunk fetch / concat fails.
download_file_parallel() {
    local url="$1" output="$2"
    if [ "$DOWNLOADER" != "curl" ]; then
        download_file "$url" "$output"
        return
    fi
    local size
    size=$(curl -fsSL --head "$url" 2>/dev/null | awk -F'[: \r\n]+' 'tolower($1)=="content-length"{print $2; exit}')
    if [ -z "$size" ] || ! [ "$size" -ge 16777216 ] 2>/dev/null; then
        download_file "$url" "$output"
        return
    fi
    local n=8
    local chunk_size=$(( (size + n - 1) / n ))
    local tmpdir
    tmpdir=$(mktemp -d 2>/dev/null) || { download_file "$url" "$output"; return; }
    local pids=() i start end
    for i in $(seq 0 $((n - 1))); do
        start=$((i * chunk_size))
        end=$((start + chunk_size - 1))
        [ $end -ge $size ] && end=$((size - 1))
        curl -fsSL -r "${start}-${end}" -o "${tmpdir}/$(printf 'chunk.%03d' "$i")" "$url" &
        pids+=($!)
    done
    local all_ok=true pid
    for pid in "${pids[@]}"; do
        wait "$pid" || all_ok=false
    done
    if [ "$all_ok" = true ] && cat "${tmpdir}"/chunk.* > "$output" 2>/dev/null; then
        rm -rf "$tmpdir"
        return 0
    fi
    rm -rf "$tmpdir"
    download_file "$url" "$output"
}

# Return 0 if a HEAD request for the URL gets HTTP 404.
is_not_found() {
    local url="$1" code
    if [ "$DOWNLOADER" = "curl" ]; then
        code=$(curl -o /dev/null -sSL -w '%{http_code}' --head "$url" 2>/dev/null) || true
    else
        code=$(wget --server-response --spider "$url" 2>&1 | awk '/HTTP\//{print $2}' | tail -1) || true
    fi
    [ "$code" = "404" ]
}

# JSON field extractor — extract a top-level string value using sed.
json_get() {
    local json="$1" field="$2"
    printf '%s' "$json" | sed -n -E 's/.*"'"$field"'"[[:space:]]*:[[:space:]]*"(([^"\\]|\\.)*)".*/\1/p' | head -1 \
        | sed -e 's/\\"/"/g' -e 's/\\n/\'$'\n''/g' -e 's/\\t/\'$'\t''/g' -e 's/\\\\/\\/g'
}

case "$(uname -s)" in
    Darwin) os="macos" ;;
    Linux)  os="linux" ;;
    # Git for Windows / MSYS2 / Cygwin host — native Windows builds
    MINGW* | MSYS* | CYGWIN*) os="windows" ;;
    *)      echo "Unsupported OS: $(uname -s)" >&2; exit 1 ;;
esac

case "$(uname -m)" in
    x86_64|amd64|AMD64) arch="x86_64" ;;
    arm64|aarch64|ARM64) arch="aarch64" ;;
    *)                    echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

DOWNLOAD_DIR="$HOME/.axon/downloads"
BIN_DIR="${AXON_BIN_DIR:-$HOME/.axon/bin}"
mkdir -p "$DOWNLOAD_DIR" "$BIN_DIR"

platform="${os}-${arch}"
CHANNEL="${AXON_CHANNEL:-stable}"

# Resolve the version to install. When no explicit TARGET is passed, ask the
# GitHub API for the newest release tag (newest prerelease on the alpha
# channel, newest full release otherwise).
if [ -n "$TARGET" ]; then
    version="$TARGET"
else
    echo "Fetching latest ${CHANNEL} version..." >&2
    if [ "$CHANNEL" = "alpha" ]; then
        releases_json=$(download_file "https://api.github.com/repos/${REPO}/releases?per_page=10" 2>/dev/null) || true
        tag=$(printf '%s' "$releases_json" | grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | sed -E 's/.*"([^"]*)"$/\1/')
    else
        latest_json=$(download_file "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null) || true
        tag=$(json_get "$latest_json" "tag_name")
    fi
    version="${tag#v}"
    if [ -z "$version" ]; then
        echo "Error: failed to fetch latest ${CHANNEL} version from GitHub Releases for ${REPO}" >&2
        exit 1
    fi
fi

if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9._]+)?$ ]]; then
    echo "Invalid version format: $version (expected X.Y.Z or X.Y.Z-suffix)" >&2
    exit 1
fi

echo "Installing Axon $version ($platform)..." >&2

BASE_URL="https://github.com/${REPO}/releases/download/v${version}"
binary_path="$DOWNLOAD_DIR/axon-$platform"
artifact_base="${BASE_URL}/axon-${version}-${platform}"

if [ "$os" = "windows" ]; then
    binary_path="${binary_path}.exe"
fi

binary_tmp="${binary_path}.tmp.$$"
rm -f "$binary_tmp" 2>/dev/null || true

echo "  Downloading axon ${version}..." >&2
if [ "$os" = "windows" ]; then
    if ! download_file_parallel "${artifact_base}.exe" "$binary_tmp"; then
        if ! download_file_parallel "$artifact_base" "$binary_tmp"; then
            rm -f "$binary_tmp"
            if is_not_found "${artifact_base}.exe"; then
                echo "Error: Axon is not yet available for your system ($platform)." >&2
            else
                echo "Error: binary download failed (${artifact_base}.exe and ${artifact_base})" >&2
            fi
            exit 1
        fi
    fi
elif ! download_file_parallel "$artifact_base" "$binary_tmp"; then
    rm -f "$binary_tmp"
    if is_not_found "$artifact_base"; then
        echo "Error: Axon is not yet available for your system ($platform)." >&2
    else
        echo "Error: binary download failed from ${artifact_base}" >&2
    fi
    exit 1
fi

if [ "$os" = "windows" ]; then
    mv -f "$binary_tmp" "$binary_path"
    # Symlinks require Developer Mode on Windows; copy instead.
    # If the exe is locked by a running process, rename it aside then retry.
    bin_name="axon.exe"
    rm -f "$BIN_DIR/$bin_name.old" 2>/dev/null || true  # stale backup from prior update
    if ! cp -f "$binary_path" "$BIN_DIR/$bin_name" 2>/dev/null; then
        mv -f "$BIN_DIR/$bin_name" "$BIN_DIR/$bin_name.old" 2>/dev/null || true
        if ! cp -f "$binary_path" "$BIN_DIR/$bin_name" 2>/dev/null; then
            # Rollback: restore the old binary so the install isn't broken.
            mv -f "$BIN_DIR/$bin_name.old" "$BIN_DIR/$bin_name" 2>/dev/null || true
            echo "Error: failed to install $bin_name" >&2
            exit 1
        fi
    fi
    echo "  Binary installed to $BIN_DIR/axon.exe." >&2
else
    chmod +x "$binary_tmp"
    if ! "$binary_tmp" --version </dev/null >/dev/null 2>&1; then
        echo "Error: downloaded axon failed to run; keeping the existing install." >&2
        rm -f "$binary_tmp"
        exit 1
    fi
    mv -f "$binary_tmp" "$binary_path"
    # Use relative symlinks when BIN_DIR and DOWNLOAD_DIR share a parent
    # (default layout: ~/.axon/bin and ~/.axon/downloads are siblings).
    # Relative symlinks survive Docker bind-mounts with a different $HOME.
    if [ "$(dirname "$BIN_DIR")" = "$(dirname "$DOWNLOAD_DIR")" ]; then
        link_target="../$(basename "$DOWNLOAD_DIR")/$(basename "$binary_path")"
    else
        link_target="$binary_path"
    fi
    ln -sf "$link_target" "$BIN_DIR/axon"
    echo "  Binary linked to $BIN_DIR/axon." >&2
fi

# Generate shell completions (best-effort)
mkdir -p "$HOME/.axon/completions/bash" "$HOME/.axon/completions/zsh"
"$BIN_DIR/axon" completions bash > "$HOME/.axon/completions/bash/axon.bash" 2>/dev/null || true
"$BIN_DIR/axon" completions zsh  > "$HOME/.axon/completions/zsh/_axon"      2>/dev/null || true
# Fish: write to the auto-loaded completions dir so it works immediately
if mkdir -p "$HOME/.config/fish/completions" 2>/dev/null; then
    "$BIN_DIR/axon" completions fish > "$HOME/.config/fish/completions/axon.fish" 2>/dev/null || true
fi

# Persist installer source and channel to config
CONFIG_FILE="$HOME/.axon/config.toml"
CLI_BLOCK="installer = \"internal\""
if [ "$CHANNEL" != "stable" ]; then
    CLI_BLOCK="${CLI_BLOCK}\nchannel = \"${CHANNEL}\""
fi
if [ ! -f "$CONFIG_FILE" ]; then
    printf '[cli]\n%b\n' "$CLI_BLOCK" > "$CONFIG_FILE"
elif grep -q '^\[cli\]' "$CONFIG_FILE"; then
    tmp="$CONFIG_FILE.tmp.$$"
    awk -v block="$CLI_BLOCK" '
        /^\[cli\][[:space:]]*(#.*)?$/ { print; printf "%s\n", block; in_cli=1; next }
        /^\[.*\][[:space:]]*(#.*)?$/  { in_cli=0 }
        in_cli && /^[[:space:]]*(installer|channel)[[:space:]]*=/ { next }
        { print }
    ' "$CONFIG_FILE" > "$tmp" && mv "$tmp" "$CONFIG_FILE"
else
    printf '\n[cli]\n%b\n' "$CLI_BLOCK" >> "$CONFIG_FILE"
fi

if [ "$os" = "windows" ]; then
    echo "Axon $version installed to $BIN_DIR/axon.exe" >&2
else
    echo "Axon $version installed to $BIN_DIR/axon" >&2
fi

# --- Ensure axon is on PATH ---

path_has_dir() {
    case ":$PATH:" in *":$1:"*) return 0 ;; *) return 1 ;; esac
}

# Try to symlink into a directory already on PATH so axon works immediately
# without restarting the shell. Candidate dirs in preference order.
SYMLINK_CREATED=""
if [ "$os" != "windows" ] && ! path_has_dir "$BIN_DIR"; then
    for candidate in "$HOME/.local/bin" "/usr/local/bin"; do
        if path_has_dir "$candidate" && [ -d "$candidate" ] && [ -w "$candidate" ]; then
            ln -sf "$BIN_DIR/axon" "$candidate/axon"
            SYMLINK_CREATED="$candidate"
            echo "  Symlinked $candidate/axon -> $BIN_DIR/axon" >&2
            break
        fi
    done
fi

# Also update shell config so ~/.axon/bin is on PATH for future sessions
user_shell="$(basename "${SHELL:-}")"
config_file=""

case "$user_shell" in
    bash) config_file="$HOME/.bashrc" ;;
    zsh)  config_file="$HOME/.zshrc" ;;
    fish) config_file="$HOME/.config/fish/config.fish" ;;
esac

if [ -n "$config_file" ]; then
    mkdir -p "$(dirname "$config_file")"

    # Resolve symlinks so tmp+mv rewrites the stow/dotfiles target, not the link.
    if [ -e "$config_file" ] || [ -L "$config_file" ]; then
        _cf="$config_file"
        _depth=0
        while [ -L "$_cf" ] && [ "$_depth" -lt 40 ]; do
            _link="$(readlink "$_cf")" || break
            case "$_link" in
                /*) _cf="$_link" ;;
                *)  _cf="$(cd "$(dirname "$_cf")" && pwd -P)/$_link" ;;
            esac
            _depth=$((_depth + 1))
        done
        # Still a symlink (cycle/cap): leave original path so we never rewrite the link.
        if [ ! -L "$_cf" ]; then
            config_file="$(cd "$(dirname "$_cf")" && pwd -P)/$(basename "$_cf")"
        fi
        unset _cf _link _depth
    fi

    # Build the new installer block
    if [ "$user_shell" = "fish" ]; then
        new_block='# >>> axon installer >>>
fish_add_path $HOME/.axon/bin
# <<< axon installer <<<'
    elif [ "$user_shell" = "zsh" ]; then
        new_block='# >>> axon installer >>>
export PATH="$HOME/.axon/bin:$PATH"
fpath=(~/.axon/completions/zsh $fpath)
autoload -Uz compinit && compinit -C
# <<< axon installer <<<'
    else
        new_block='# >>> axon installer >>>
export PATH="$HOME/.axon/bin:$PATH"
[[ -r "$HOME/.axon/completions/bash/axon.bash" ]] && source "$HOME/.axon/completions/bash/axon.bash"
# <<< axon installer <<<'
    fi

    if grep -qs "axon installer" "$config_file" 2>/dev/null; then
        # Replace existing block in-place (strip old >>> to <<< lines, insert new)
        tmp="$config_file.tmp.$$"
        awk '
            /# >>> axon installer >>>/ { skip=1; next }
            /# <<< axon installer <<</ { skip=0; next }
            !skip { print }
        ' "$config_file" > "$tmp" && mv "$tmp" "$config_file"
    else
        [ -f "$config_file" ] && cp "$config_file" "$config_file.bak.$(date +%s)"

        # macOS bash: ensure bash_profile sources bashrc
        if [ "$user_shell" = "bash" ] && [ "$(uname -s)" = "Darwin" ]; then
            if [ -f "$HOME/.bash_profile" ] && ! grep -qs "source ~/.bashrc" "$HOME/.bash_profile"; then
                printf '\n[[ -r ~/.bashrc ]] && source ~/.bashrc\n' >> "$HOME/.bash_profile"
            fi
        fi
    fi

    printf '\n%s\n' "$new_block" >> "$config_file"
    echo "  Updated $BIN_DIR in PATH in $config_file." >&2
fi

echo "" >&2
if path_has_dir "$BIN_DIR" || [ -n "$SYMLINK_CREATED" ]; then
    echo "Run 'axon' to get started!" >&2
elif [ -n "$config_file" ]; then
    echo "Restart your terminal, then run 'axon' to get started!" >&2
else
    echo "Add $BIN_DIR to your PATH, then run 'axon' to get started:" >&2
    echo '  export PATH="$HOME/.axon/bin:$PATH"' >&2
fi

if [ "$os" = "windows" ]; then
    echo "To use axon from cmd.exe or PowerShell, add %USERPROFILE%\\.axon\\bin to your PATH." >&2
fi
