#!/bin/sh
# bloat installer — builds the newest commit from GitHub with cargo.
#
#   curl -fsSL https://raw.githubusercontent.com/BananaBites/bloat/main/install.sh | sh
#
# Options (also usable when piped, via `| sh -s -- ...`):
#   --branch <name>   build a specific branch/tag instead of the default branch
#   --root <dir>      install into <dir>/bin instead of ~/.cargo/bin
#   --no-locked       do not require the committed Cargo.lock
#   --help
#
# Environment overrides: BLOAT_REPO, BLOAT_BRANCH, BLOAT_ROOT
set -eu

REPO="${BLOAT_REPO:-https://github.com/BananaBites/bloat}"
BRANCH="${BLOAT_BRANCH:-}"
ROOT="${BLOAT_ROOT:-}"
LOCKED=1

usage() {
    cat <<'USAGE'
bloat installer — builds the newest commit from GitHub with cargo.

  curl -fsSL https://raw.githubusercontent.com/BananaBites/bloat/main/install.sh | sh

Options (also usable when piped, via `| sh -s -- ...`):
  --branch <name>   build a specific branch/tag instead of the default branch
  --root <dir>      install into <dir>/bin instead of ~/.cargo/bin
  --no-locked       do not require the committed Cargo.lock
  --help

Environment overrides: BLOAT_REPO, BLOAT_BRANCH, BLOAT_ROOT
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --branch) BRANCH="${2:-}"; shift 2 ;;
        --branch=*) BRANCH="${1#*=}"; shift ;;
        --root) ROOT="${2:-}"; shift 2 ;;
        --root=*) ROOT="${1#*=}"; shift ;;
        --no-locked) LOCKED=0; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "bloat installer: unknown option '$1'" >&2; usage >&2; exit 2 ;;
    esac
done

if ! command -v cargo >/dev/null 2>&1 && [ -x "${HOME:-/root}/.cargo/bin/cargo" ]; then
    PATH="${HOME}/.cargo/bin:$PATH"
    export PATH
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "· no cargo found — installing Rust (rustup, minimal profile, user-local)"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
        sh -s -- -y --profile minimal --no-modify-path
    PATH="${HOME}/.cargo/bin:$PATH"
    export PATH
    echo "· note: add ~/.cargo/bin to your PATH to run bloat in future shells"
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "bloat installer: cargo is not on PATH; add ~/.cargo/bin and re-run" >&2
    exit 1
fi

set -- install --git "$REPO" --force
if [ -n "$BRANCH" ]; then set -- "$@" --branch "$BRANCH"; fi
if [ -n "$ROOT" ]; then set -- "$@" --root "$ROOT"; fi
if [ "$LOCKED" = 1 ]; then set -- "$@" --locked; fi

echo "· installing bloat from ${REPO}${BRANCH:+ (branch ${BRANCH})}"
cargo "$@"

BIN="bloat"
if [ -n "$ROOT" ]; then
    BIN="${ROOT}/bin/bloat"
elif command -v bloat >/dev/null 2>&1; then
    BIN="$(command -v bloat)"
elif [ -x "${HOME:-/root}/.cargo/bin/bloat" ]; then
    BIN="${HOME}/.cargo/bin/bloat"
fi

echo
echo "✓ $("$BIN" --version 2>/dev/null || echo bloat) → $BIN"
echo
cat <<'EOF'
next steps
  completions (run once per shell; they call bloat, so they never go stale):
    bash   echo 'source <(COMPLETE=bash bloat)' >> ~/.bashrc
    zsh    echo 'source <(COMPLETE=zsh bloat)'  >> ~/.zshrc
    fish   echo 'COMPLETE=fish bloat | source'  >> ~/.config/fish/completions/bloat.fish

  what do I have?   bloat doctor
  get the newest:   bloat update
EOF