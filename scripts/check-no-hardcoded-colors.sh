#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT_DIR"

SCAN_DIR="frontend"
TOKENS_FILE="frontend/tokens.css"

# Patterns to forbid outside token sources
PATTERN_HEX='#[0-9a-fA-F]{3,8}'
PATTERN_RGB='\brgba?\('
PATTERN_HSL='\bhsla?\('

# Search scope: frontend/ (exclude tokens.css)
# Note: grep -P is not always available; use ripgrep if present, else fallback.
if command -v rg >/dev/null 2>&1; then
  hits=$(rg -n --pcre2 "$PATTERN_HEX|$PATTERN_RGB|$PATTERN_HSL" "$SCAN_DIR" \
    --glob "!$(basename "$TOKENS_FILE")" \
    --glob "!**/*.snap" \
    --glob "!**/*.min.*" || true)
else
  hits=$(grep -RInE "$PATTERN_HEX|$PATTERN_RGB|$PATTERN_HSL" "$SCAN_DIR" \
    --exclude="$(basename "$TOKENS_FILE")" \
    --exclude-dir="__tests__" 2>/dev/null || true)
fi

if [ -n "$hits" ]; then
  echo "ERROR: Hardcoded colors found outside $TOKENS_FILE"
  echo
  echo "$hits"
  echo
  echo "Fix: move the color into $TOKENS_FILE and reference it via a CSS variable."
  exit 1
fi

echo "OK: no hardcoded colors outside $TOKENS_FILE"
