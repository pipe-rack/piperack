#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

REPO="${REPO:-pipe-rack/piperack}"
REMOTE="${REMOTE:-origin}"
ASSETS_DIR=""
NOTES=""
NOTES_FILE=""
SKIP_TAG=0
SKIP_PUSH=0
BUILD_LOCAL=0

usage() {
  cat <<'USAGE'
Usage: scripts/release_local.sh [options]

Options:
  --assets-dir <dir>   Directory containing .tar.gz and .sha256 assets
  --build-local        Build and package the local host target
  --notes <text>       Release notes text
  --notes-file <path>  Release notes file
  --skip-tag           Do not create a git tag
  --skip-push          Do not push the git tag
  --repo <owner/repo>  GitHub repo (default: pipe-rack/piperack)
  --remote <name>      Git remote for tag push (default: origin)
  -h, --help           Show this help
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --assets-dir)
      ASSETS_DIR="$2"
      shift 2
      ;;
    --build-local)
      BUILD_LOCAL=1
      shift
      ;;
    --notes)
      NOTES="$2"
      shift 2
      ;;
    --notes-file)
      NOTES_FILE="$2"
      shift 2
      ;;
    --skip-tag)
      SKIP_TAG=1
      shift
      ;;
    --skip-push)
      SKIP_PUSH=1
      shift
      ;;
    --repo)
      REPO="$2"
      shift 2
      ;;
    --remote)
      REMOTE="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown arg: $1" >&2
      usage
      exit 1
      ;;
  esac
done

version="$(
CARGO_TOML="$ROOT/Cargo.toml" python3 - <<'PY'
import os
import re
from pathlib import Path

path = Path(os.environ["CARGO_TOML"])
text = path.read_text()

in_package = False
version = None
for line in text.splitlines():
    if line.strip().startswith("["):
        in_package = line.strip() == "[package]"
        continue
    if in_package:
        m = re.match(r"\s*version\s*=\s*\"([^\"]+)\"", line)
        if m:
            version = m.group(1)
            break

if not version:
    raise SystemExit("Could not find [package] version in Cargo.toml")

print(version)
PY
)"

tag="v${version}"

if [[ -z "$ASSETS_DIR" ]]; then
  ASSETS_DIR="$ROOT/dist"
fi

if [[ "$BUILD_LOCAL" -eq 1 ]]; then
  uname_s="$(uname -s)"
  uname_m="$(uname -m)"
  case "$uname_s" in
    Darwin)
      if [[ "$uname_m" == "arm64" ]]; then
        target="aarch64-apple-darwin"
      else
        target="x86_64-apple-darwin"
      fi
      ;;
    Linux)
      if [[ "$uname_m" == "aarch64" || "$uname_m" == "arm64" ]]; then
        target="aarch64-unknown-linux-gnu"
      else
        target="x86_64-unknown-linux-gnu"
      fi
      ;;
    *)
      echo "Unsupported OS for local build: $uname_s" >&2
      exit 1
      ;;
  esac

  cargo build --release
  mkdir -p "$ASSETS_DIR"
  pkg="piperack-${tag}-${target}"
  tar -czf "$ASSETS_DIR/${pkg}.tar.gz" -C "$ROOT/target/release" piperack
  shasum -a 256 "$ASSETS_DIR/${pkg}.tar.gz" | awk '{print $1}' > "$ASSETS_DIR/${pkg}.tar.gz.sha256"
fi

if [[ ! -d "$ASSETS_DIR" ]]; then
  echo "Assets dir not found: $ASSETS_DIR" >&2
  exit 1
fi

if [[ "$SKIP_TAG" -eq 0 ]]; then
  head_sha="$(git rev-parse HEAD)"
  if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    tag_sha="$(git rev-list -n 1 "$tag")"
    if [[ "$tag_sha" != "$head_sha" ]]; then
      echo "Tag $tag exists and points to $tag_sha, not HEAD ($head_sha)." >&2
      exit 1
    fi
  else
    git tag "$tag"
  fi

  if [[ "$SKIP_PUSH" -eq 0 ]]; then
    git push "$REMOTE" "$tag"
  fi
fi

shopt -s nullglob
assets=("$ASSETS_DIR"/*.tar.gz "$ASSETS_DIR"/*.sha256)
shopt -u nullglob

if [[ ${#assets[@]} -eq 0 ]]; then
  echo "No assets found in $ASSETS_DIR" >&2
  exit 1
fi

if [[ -n "$NOTES_FILE" ]]; then
  notes_args=(--notes-file "$NOTES_FILE")
elif [[ -n "$NOTES" ]]; then
  notes_args=(--notes "$NOTES")
else
  notes_args=(--notes "Release $tag")
fi

if gh release view "$tag" --repo "$REPO" >/dev/null 2>&1; then
  gh release upload "$tag" "${assets[@]}" --repo "$REPO" --clobber
else
  gh release create "$tag" "${assets[@]}" --repo "$REPO" --title "$tag" "${notes_args[@]}"
fi

echo "Release $tag published to $REPO"
