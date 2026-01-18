#!/bin/bash
# Simulates a Postgres-like database with JSON logging

log() {
    level=$1
    msg=$2
    component=${3:-"db"}
    ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
    echo "{\"level\": \"$level\", \"timestamp\": \"$ts\", \"service\": \"postgres\", \"component\": \"$component\", \"pid\": $$, \"msg\": \"$msg\"}"
}

log "info" "starting PostgreSQL 16.1" "startup"
sleep 2
log "info" "reading configuration files" "startup"
sleep 1
log "info" "initializing storage engine" "startup"
sleep 2
# The readiness check looks for this string. It still exists inside the JSON.
log "info" "Database is ready to accept connections" "startup"
log "info" "listening on 127.0.0.1:5432" "listener"

while true; do
    sleep 4
    log "info" "checkpoint complete: wrote 12 buffers, 0 WAL files recycled" "checkpoint"
    sleep 2
    log "info" "autovacuum: ANALYZE public.orders" "autovacuum"
    sleep 5
done
