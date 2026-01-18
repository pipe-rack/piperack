#!/bin/bash
# Simulates a background worker with JSON logging

log() {
    level=$1
    msg=$2
    job=$3
    ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
    if [ -z "$job" ]; then
        echo "{"level": "$level", "timestamp": "$ts", "service": "worker", "msg": "$msg"}"
    else
        echo "{"level": "$level", "timestamp": "$ts", "service": "worker", "msg": "$msg", "job_id": "$job"}"
    fi
}

log "info" "Starting background worker..."
log "info" "Waiting for job queue..."
sleep 2

jobs=("SendEmail" "ResizeImage" "ProcessPayment" "GenerateReport")

while true; do
    job_name=${jobs[$(jot -r 1 0 3)]}
    job_id="job_$(jot -r 1 1000 9999)"
    
    log "info" "Processing job: $job_name" "$job_id"
    sleep $(jot -r 1 1 4)
    
    if [ $(( $RANDOM % 10 )) -gt 8 ]; then
        log "error" "Job failed due to timeout" "$job_id" >&2
    else
        log "info" "Job completed successfully" "$job_id"
    fi
done