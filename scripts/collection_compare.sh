#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG="${CONFIG_DIR:-$ROOT/config}"
DATA="${DATA_DIR:-$ROOT/var/compare}"
BRANCH="${BRANCH:-cursor/leaf-backend-refactor-6669}"
echo "config=$CONFIG data=$DATA branch=$BRANCH"
mkdir -p "$DATA"
for ref in main "$BRANCH"; do
  git -C "$ROOT" checkout "$ref"
  cargo build -p modalprism-collector --release
  echo "built @ $ref"
done
echo "请在各分支启动采集后对启用信源 trigger(confirm:true)，再统计 strata 入库条数与健康快照 healthyCount。"
