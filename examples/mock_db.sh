#!/bin/bash
# Simulates a Postgres-like database with JSON logging

log() {
    level=$1
    msg=$2
    ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
    echo "{\"level\": \"$level\", \"timestamp\": \"$ts\", \"source\": \"postgres\", \"msg\": \"$msg\"}"
}

log "info" "Database initializing..."
sleep 2
log "info" "Loading configuration..."
sleep 1
log "info" "Setting up storage engine..."
sleep 2
# The readiness check looks for this string. It still exists inside the JSON.
log "info" "Database is ready to accept connections on port 5432"

while true; do
    sleep 3
    log "info" "executing checkpoint"
    sleep 0.5
    log "debug" "vacuuming table 'users'"
    sleep 4
done