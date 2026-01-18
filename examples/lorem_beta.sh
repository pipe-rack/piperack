#!/usr/bin/env bash
set -euo pipefail

lines=(
  "Integer nec odio. Praesent libero. Sed cursus ante dapibus diam."
  "Sed nisi. Nulla quis sem at nibh elementum imperdiet."
  "Duis sagittis ipsum. Praesent mauris. Fusce nec tellus sed augue."
  "Mauris ipsum. Nulla metus metus, ullamcorper vel, tincidunt sed."
  "Curabitur sodales ligula in libero. Sed dignissim lacinia nunc."
)

i=0
while true; do
  printf "[beta] %s\n" "${lines[$((i % ${#lines[@]}))]}"
  i=$((i + 1))
  sleep 1.0
done
