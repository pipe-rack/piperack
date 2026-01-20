#!/usr/bin/env bash
set -euo pipefail

colors=(31 32 33 34 35 36 91 92 93 94 95 96 97)
labels=("RED" "GREEN" "YELLOW" "BLUE" "MAGENTA" "CYAN" "BRIGHT_RED" "BRIGHT_GREEN" "BRIGHT_YELLOW" "BRIGHT_BLUE" "BRIGHT_MAGENTA" "BRIGHT_CYAN" "BRIGHT_WHITE")

i=0
tick=0
while true; do
  idx=$((i % ${#colors[@]}))
  code="${colors[$idx]}"
  label="${labels[$idx]}"
  printf "\033[%sm[ansi] %s sample line %03d\033[0m\n" "$code" "$label" "$i"

  # Simulate an ANSI progress update on the same line.
  bar=$((tick % 20))
  printf "\r\033[2K\033[90m[ansi]\033[0m progress: ["
  for j in $(seq 0 19); do
    if [ "$j" -le "$bar" ]; then
      printf "\033[92m#\033[0m"
    else
      printf "\033[90m-\033[0m"
    fi
  done
  printf "]\n"

  i=$((i + 1))
  tick=$((tick + 1))
  sleep 0.6
done
