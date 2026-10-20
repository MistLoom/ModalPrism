#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
if ! command -v npm >/dev/null; then
  test -f dist/index.html || { echo "missing dist/index.html"; exit 1; }
  exit 0
fi
npm ci --offline 2>/dev/null || npm ci
npm run build
if git diff --quiet -- dist; then
  echo "dist matches build output"
else
  echo "dist is stale — commit apps/console-web/dist after npm run build" >&2
  exit 1
fi
