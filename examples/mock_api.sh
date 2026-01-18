#!/bin/bash
# Simulates a REST API with JSON logging

# Helper for JSON logging
log() {
    level=$1
    msg=$2
    ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
    echo "{\"level\": \"$level\", \"timestamp\": \"$ts\", \"service\": \"api-gateway\", \"msg\": \"$msg\", \"request_id\": \"req_$(jot -r 1 1000 9999)\"}"
}

echo "{\"level\": \"info\", \"msg\": \"Starting API Gateway...\"}"
sleep 1
log "info" "Connecting to Database..."
sleep 1
log "info" "Connecting to Redis..."
sleep 0.5
log "info" "Server started on http://localhost:8080"

while true; do
    sleep $(jot -r 1 1 3)
    endpoint=$(jot -r 1 0 3)
    case $endpoint in
        0) log "info" "GET /users/123 200 OK" ;;
        1) log "info" "POST /orders 201 Created" ;;
        2) log "warn" "GET /admin 403 Forbidden" ;;
        3) log "error" "Database connection timeout" ;;
    esac
done
