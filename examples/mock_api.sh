#!/bin/bash
# Simulates a REST API with JSON logging

rand_int() {
    local min=$1
    local max=$2
    echo $((RANDOM % (max - min + 1) + min))
}

log() {
    level=$1
    msg=$2
    req_id=$3
    ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
    if [ -n "$req_id" ]; then
        echo "{\"level\": \"$level\", \"timestamp\": \"$ts\", \"service\": \"api\", \"msg\": \"$msg\", \"request_id\": \"$req_id\"}"
    else
        echo "{\"level\": \"$level\", \"timestamp\": \"$ts\", \"service\": \"api\", \"msg\": \"$msg\"}"
    fi
}

log "info" "booting http server"
sleep 1
log "info" "connecting to postgres"
sleep 1
log "info" "connecting to redis"
sleep 0.5
log "info" "listening on http://127.0.0.1:8080"

while true; do
    sleep $(rand_int 1 3)
    req_id=$(printf "req_%04x%04x" "$RANDOM" "$RANDOM")
    case $(rand_int 0 5) in
        0) log "info" "GET /healthz 200 2ms" "$req_id" ;;
        1) log "info" "GET /users/123 200 18ms" "$req_id" ;;
        2) log "info" "POST /orders 201 54ms" "$req_id" ;;
        3) log "warn" "GET /admin 403 3ms" "$req_id" ;;
        4) log "warn" "GET /projects/77 429 1ms" "$req_id" ;;
        5) log "error" "db timeout while reading users 504 1200ms" "$req_id" ;;
    esac
done
