# Release Process

This project ships binaries via GitHub Releases and updates the Homebrew tap automatically.

## Option A: Preferred (GitHub Actions)

1) Ensure `HOMEBREW_TAP_TOKEN` and `RELEASE_TOKEN` are set in `pipe-rack/piperack` → Settings → Secrets and variables → Actions.
2) Trigger the **bump-and-tag** workflow (Actions → bump-and-tag) and choose `patch|minor|major`.
3) After the tag is created, run the **release** workflow for that tag (Actions → release → Run workflow, ref: `vX.Y.Z`).
4) Verify:
   - GitHub Release has 4 tarballs + 4 `.sha256` files.
   - `pipe-rack/homebrew-tap` `Formula/piperack.rb` is updated to the new version + SHA256s.

## Option B: Local (manual release)

Use the local script to create the tag and GitHub Release from your machine:

```bash
# from repo root
./scripts/release_local.sh --build-local
```

Notes:
- This builds and uploads the **current host target only**.
- To include all targets, prefer Option A or provide prebuilt assets:

```bash
./scripts/release_local.sh --assets-dir /path/to/dist
```

## Brew install check

```bash
brew tap pipe-rack/homebrew-tap
brew install pipe-rack/homebrew-tap/piperack
```
