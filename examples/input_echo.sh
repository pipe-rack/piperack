#!/usr/bin/env bash
set -euo pipefail

printf "[prompt] type a line and press enter (service exits after input)\n"
if IFS= read -r line; then
  printf "[prompt] you typed: %s\n" "$line"
fi
exit 0
