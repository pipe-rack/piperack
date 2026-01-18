#!/usr/bin/env bash
set -euo pipefail

lines=(
  "Lorem ipsum dolor sit amet, consectetur adipiscing elit."
  "Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua."
  "Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris."
  "Duis aute irure dolor in reprehenderit in voluptate velit esse cillum."
  "Excepteur sint occaecat cupidatat non proident, sunt in culpa."
)

i=0
while true; do
  printf "[alpha] %s\n" "${lines[$((i % ${#lines[@]}))]}"
  i=$((i + 1))
  sleep 0.7
done
