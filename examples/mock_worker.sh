#!/bin/bash
# Simulates a background worker with JSON logging

rand_int() {
    local min=$1
    local max=$2
    echo $((RANDOM % (max - min + 1) + min))
}

log() {
    level=$1
    msg=$2
    job=$3
    ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
    if [ -z "$job" ]; then
        echo "{\"level\": \"$level\", \"timestamp\": \"$ts\", \"service\": \"worker\", \"msg\": \"$msg\"}"
    else
        echo "{\"level\": \"$level\", \"timestamp\": \"$ts\", \"service\": \"worker\", \"msg\": \"$msg\", \"job_id\": \"$job\"}"
    fi
}

shutting_down=0

shutdown() {
    if [ "$shutting_down" -eq 1 ]; then
        return
    fi
    shutting_down=1
    log "info" "shutdown signal received, draining for 6s"
    sleep 6
    log "info" "shutdown complete"
    exit 0
}

trap shutdown INT TERM

log "info" "starting worker pool"
log "info" "polling queue=default"
sleep 2

jobs=("SendEmail" "ResizeImage" "ProcessPayment" "GenerateReport" "SyncCRM")

while true; do
    job_name=${jobs[$(rand_int 0 4)]}
    job_id=$(printf "job_%04x%04x" "$RANDOM" "$RANDOM")

    log "info" "picked job type=$job_name queue=default" "$job_id"
    sleep $(rand_int 1 4)

    if [ $((RANDOM % 10)) -gt 8 ]; then
        log "error" "job failed: timeout while calling upstream" "$job_id" >&2
    else
        log "info" "job finished status=ok" "$job_id"
    fi
done
