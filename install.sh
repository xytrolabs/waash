#!/usr/bin/env bash
# WAASH — What An Amazing SHell
# Universal installer for Linux (and macOS).
#
# Works on Debian/Ubuntu, Fedora/RHEL, Arch, openSUSE, Alpine, and any distro
# with a C toolchain + curl. Installs to ~/.local/bin (no root needed).
#
# Usage:
#   bash install.sh                # detect, build, install
#   bash install.sh --no-build     # just copy an existing target/release/waash
#   bash install.sh --prefix ~/.local
#   bash install.sh --version      # print version and exit
#
# One-liner:
#   curl -fsSL https://xytro.site/waash/install.sh | bash

set -euo pipefail

# ── Config ──────────────────────────────────────────────────────────────
PREFIX="${WAASH_PREFIX:-$HOME/.local}"
BIN_DIR="$PREFIX/bin"
LIB_DIR="$PREFIX/share/waash/lib"
DOCS_DIR="$PREFIX/share/waash/docs"
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$PWD}")" 2>/dev/null && pwd || echo "$PWD")"
WAASH_REPO_URL="${WAASH_REPO_URL:-https://github.com/xytrolabs/waash}"
NO_BUILD=0
SRC_DIR=""
TMP_SRC=""

cleanup() { [ -n "$TMP_SRC" ] && [ -d "$TMP_SRC" ] && rm -rf "$TMP_SRC"; }
trap cleanup EXIT

# ── Pretty printing ─────────────────────────────────────────────────────
c_green=$'\033[32m'; c_yellow=$'\033[33m'; c_cyan=$'\033[36m'; c_reset=$'\033[0m'
info()  { printf "%s•%s %s\n" "$c_cyan" "$c_reset" "$*"; }
ok()    { printf "%s✓%s %s\n" "$c_green" "$c_reset" "$*"; }
warn()  { printf "%s!%s %s\n" "$c_yellow" "$c_reset" "$*"; }
die()   { printf "%s✗%s %s\n" "\033[31m" "$c_reset" "$*" >&2; exit 1; }

# ── Detect OS / arch ────────────────────────────────────────────────────
detect_os() {
  case "$(uname -s)" in
    Linux)  OS="linux" ;;
    Darwin) OS="macos" ;;
    *)      die "Unsupported OS: $(uname -s)" ;;
  esac
  ARCH="$(uname -m)"
  case "$ARCH" in
    x86_64|amd64)   ARCH="x86_64" ;;
    aarch64|arm64)  ARCH="aarch64" ;;
    armv7l)         ARCH="armv7" ;;
  esac
  info "Detected: ${OS}-${ARCH}"
}

# ── Ensure a Rust toolchain exists ──────────────────────────────────────
ensure_cargo() {
  if command -v cargo >/dev/null 2>&1; then
    ok "cargo found: $(cargo --version 2>/dev/null | head -n1)"
    return
  fi
  warn "cargo not found — installing Rust via rustup..."
  if command -v curl >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable \
      || die "Failed to install Rust via rustup"
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env" || true
  elif command -v wget >/dev/null 2>&1; then
    wget -qO- https://sh.rustup.rs | sh -s -- -y --default-toolchain stable \
      || die "Failed to install Rust via rustup"
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env" || true
  else
    die "Need a Rust toolchain. Install from https://rustup.rs first."
  fi
  if command -v cargo >/dev/null 2>&1; then
    ok "Rust installed: $(cargo --version 2>/dev/null | head -n1)"
  else
    die "cargo still not on PATH. Restart your shell and re-run install.sh"
  fi
}

# ── Build ───────────────────────────────────────────────────────────────
# Determine where the source lives: if the script runs from inside a WAASH
# source tree (Cargo.toml present) build there, otherwise (piped `curl | bash`
# or a non-source dir) clone the repo into a temp dir.
resolve_source() {
  if [ -f "$REPO_DIR/Cargo.toml" ]; then
    SRC_DIR="$REPO_DIR"
    info "Using source at $SRC_DIR"
    return
  fi
  TMP_SRC="$(mktemp -d -t waash-src-XXXXXX)"
  SRC_DIR="$TMP_SRC"
  info "Fetching source from $WAASH_REPO_URL ..."
  if command -v git >/dev/null 2>&1; then
    git clone --depth 1 "$WAASH_REPO_URL" "$SRC_DIR" || die "git clone failed"
  elif command -v curl >/dev/null 2>&1; then
    curl -fsSL "$WAASH_REPO_URL/archive/refs/heads/main.tar.gz" \
      | tar -xz -C "$SRC_DIR" --strip-components=1 || die "source download failed"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO- "$WAASH_REPO_URL/archive/refs/heads/main.tar.gz" \
      | tar -xz -C "$SRC_DIR" --strip-components=1 || die "source download failed"
  else
    die "Need git, curl, or wget to fetch WAASH source"
  fi
  [ -f "$SRC_DIR/Cargo.toml" ] || die "Fetched source is missing Cargo.toml"
  ok "Source ready"
}

build() {
  info "Building WAASH (release)..."
  if ! command -v cargo >/dev/null 2>&1; then
    warn "cargo not found in this shell; trying ~/.cargo/bin"
    export PATH="$HOME/.cargo/bin:$PATH"
  fi
  (cd "$SRC_DIR" && cargo build --release) \
    || die "Build failed. See errors above."
  [ -f "$SRC_DIR/target/release/waash" ] || die "Build produced no binary"
  ok "Build complete"
}

# ── Install ─────────────────────────────────────────────────────────────
install() {
  mkdir -p "$BIN_DIR" "$LIB_DIR" "$DOCS_DIR"

  # Binary — copy to a temp name then `mv` over, so a RUNNING waash isn't
  # "text file busy" (mv replaces the directory entry atomically; the old
  # process keeps its inode).
  local tmp_bin="$BIN_DIR/waash.new.$$"
  cp "$SRC_DIR/target/release/waash" "$tmp_bin"
  chmod +x "$tmp_bin"
  mv -f "$tmp_bin" "$BIN_DIR/waash"
  ok "Installed waash -> $BIN_DIR/waash"

  # Helper library for scripts
  if [ -f "$SRC_DIR/share/waash/waash.ind" ]; then
    cp "$SRC_DIR/share/waash/waash.ind" "$LIB_DIR/waash.ind"
    ok "Installed helper library -> $LIB_DIR/waash.ind"
  fi

  # Examples
  if [ -d "$SRC_DIR/share/waash/examples" ]; then
    mkdir -p "$PREFIX/share/waash/examples"
    cp "$SRC_DIR"/share/waash/examples/*.waash "$PREFIX/share/waash/examples/" 2>/dev/null || true
    ok "Installed example scripts"
  fi

  # Docs
  if [ -d "$SRC_DIR/docs" ]; then
    cp -r "$SRC_DIR"/docs/. "$DOCS_DIR/"
    ok "Installed docs -> $DOCS_DIR"
  fi

  # PATH
  add_to_path
}

add_to_path() {
  case ":$PATH:" in
    *":$BIN_DIR:"*) : ;;
    *)
      warn "Adding $BIN_DIR to your PATH..."
      shell="${SHELL:-$(basename "$(command -v sh)")}"
      case "$shell" in
        *fish)
          fish -c "fish_add_path $BIN_DIR" 2>/dev/null || echo "fish_add_path $BIN_DIR" >> "$HOME/.config/fish/config.fish"
          ;;
        *zsh)
          grep -q "$BIN_DIR" "$HOME/.zshrc" 2>/dev/null || echo "export PATH=\"$BIN_DIR:\$PATH\"" >> "$HOME/.zshrc"
          ;;
        *)
          grep -q "$BIN_DIR" "$HOME/.bashrc" 2>/dev/null || echo "export PATH=\"$BIN_DIR:\$PATH\"" >> "$HOME/.bashrc"
          ;;
      esac
      ;;
  esac
}

# ── Check for Indent runtime (optional) ─────────────────────────────────
check_indent() {
  if command -v indent >/dev/null 2>&1 || [ -x "$HOME/.local/bin/indent" ]; then
    ok "Indent runtime found (scripts will work)"
  else
    warn "Indent runtime not found. WAASH scripts need it."
    warn "Install from https://indent.xytro.site or set WAASH_INDENT_BINARY."
  fi
}

# ── Version ─────────────────────────────────────────────────────────────
print_version() {
  local ver=""
  if [ -f "$REPO_DIR/Cargo.toml" ]; then
    ver=$(grep -m1 '^version' "$REPO_DIR/Cargo.toml")
  elif [ -n "$SRC_DIR" ] && [ -f "$SRC_DIR/Cargo.toml" ]; then
    ver=$(grep -m1 '^version' "$SRC_DIR/Cargo.toml")
  fi
  if [ -n "$ver" ]; then
    # version = "0.2.0"  ->  0.2.0
    printf 'waash %s\n' "${ver#* =}" | tr -d '"'
  elif command -v curl >/dev/null 2>&1; then
    # Piped install: fetch the version straight from the repo (no clone).
    ver=$(curl -fsSL "$WAASH_REPO_URL/raw/main/Cargo.toml" 2>/dev/null | grep -m1 '^version' || true)
    if [ -n "$ver" ]; then
      printf 'waash %s\n' "${ver#* =}" | tr -d '"'
    else
      echo "waash (from $WAASH_REPO_URL)"
    fi
  else
    echo "waash (from $WAASH_REPO_URL)"
  fi
}

# ── Help ─────────────────────────────────────────────────────────────────
print_help() {
  cat <<'EOF'
WAASH installer — What An Amazing SHell

Usage:
  bash install.sh                 # detect, build, install
  bash install.sh --no-build      # just copy an existing target/release/waash
  bash install.sh --prefix DIR    # install under DIR (default ~/.local)
  bash install.sh --version       # print version and exit

Options:
  --no-build      Use an existing target/release/waash (no cargo build)
  --prefix DIR    Install under DIR (default ~/.local)
  --version       Print the WAASH version and exit
  -h, --help      Show this help
EOF
}

# ── Main ────────────────────────────────────────────────────────────────
main() {
  local i=0
  while [ "$i" -lt "$#" ]; do
    local arg="${*:i+1:1}"
    case "$arg" in
      --no-build) NO_BUILD=1 ;;
      --prefix)
        # Consume the NEXT argument as the value: `--prefix ~/.local`.
        i=$((i+1))
        if [ "$i" -lt "$#" ]; then
          PREFIX="${*:i+1:1}"
        else
          die "--prefix requires a value"
        fi
        ;;
      --prefix=*) PREFIX="${arg#--prefix=}" ;;
      --version) print_version; exit 0 ;;
      -h|--help) print_help; exit 0 ;;
      *) die "Unknown option: $arg (see --help)" ;;
    esac
    i=$((i+1))
  done

  # Recompute the install paths from the (possibly overridden) PREFIX — the
  # top-of-script defaults were computed before `--prefix` was parsed.
  BIN_DIR="$PREFIX/bin"
  LIB_DIR="$PREFIX/share/waash/lib"
  DOCS_DIR="$PREFIX/share/waash/docs"

  echo
  echo "  ${c_yellow}WAASH — What An Amazing SHell${c_reset}"
  echo "  $(print_version) | ${OS:-linux}-${ARCH:-$(uname -m)}"
  echo

  detect_os
  if [ "$NO_BUILD" -eq 0 ]; then
    ensure_cargo
    resolve_source
    build
  else
    [ -f "$REPO_DIR/target/release/waash" ] || die "--no-build needs target/release/waash (build first)"
    SRC_DIR="$REPO_DIR"
    ok "Using existing build"
  fi
  install
  check_indent

  echo
  ok "WAASH installed! Start it with:  waash"
  info "Docs: $DOCS_DIR"
  info "If 'waash' isn't found, restart your shell or run: export PATH=\"$BIN_DIR:\$PATH\""
  echo
}

main "$@"
