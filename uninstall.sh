#!/usr/bin/env bash
# WAASH — uninstaller
# Removes the waash binary, helper library, docs, and optionally config.
#
# Usage:
#   bash uninstall.sh              # remove binary + lib + docs
#   bash uninstall.sh --all        # also remove ~/.config/waash and history

set -euo pipefail

PREFIX="${WAASH_PREFIX:-$HOME/.local}"
BIN="$PREFIX/bin/waash"
LIB="$PREFIX/share/waash"
ALL=0

for arg in "$@"; do
  case "$arg" in
    --all) ALL=1 ;;
    -h|--help) echo "Usage: bash uninstall.sh [--all]"; exit 0 ;;
    *) echo "Unknown option: $arg"; exit 1 ;;
  esac
done

echo "Removing WAASH..."

[ -f "$BIN" ] && rm -f "$BIN" && echo "  ✓ removed $BIN"
[ -d "$LIB" ] && rm -rf "$LIB" && echo "  ✓ removed $LIB"

if [ "$ALL" -eq 1 ]; then
  [ -d "$HOME/.config/waash" ] && rm -rf "$HOME/.config/waash" && echo "  ✓ removed ~/.config/waash"
  [ -d "$HOME/.local/share/waash" ] && rm -rf "$HOME/.local/share/waash" && echo "  ✓ removed ~/.local/share/waash"
fi

echo "Done. WAASH has been removed."
