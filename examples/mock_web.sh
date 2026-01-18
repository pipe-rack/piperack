#!/bin/bash
# Simulates a frontend dev server (Vite-style logs)

set -euo pipefail

log() {
  echo "$*"
}

log "VITE v5.0.0  ready in 412 ms"
log "➜  Local:   http://localhost:5173/"
log "➜  Network: use --host to expose"
log "➜  press h to show help"

sleep 4
log "[vite] optimizing dependencies..."
sleep 2
log "[vite] dependencies optimized"

while true; do
  sleep 6
  case $((RANDOM % 5)) in
    0) log "[vite] hmr update /src/components/Nav.tsx" ;;
    1) log "[vite] hmr update /src/pages/Dashboard.tsx" ;;
    2) log "[vite] hmr update /src/styles/app.css" ;;
    3) log "[vite] server restarted" ;;
    4) log "[vite] hmr update /src/lib/api.ts" ;;
  esac
  sleep 4
  if [ $((RANDOM % 12)) -eq 0 ]; then
    log "[vite] warn: large chunk size, consider code-splitting"
  fi
done
