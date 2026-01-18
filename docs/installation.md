# Installation

Piperack is a single binary with no external runtime dependencies. You can install it via Homebrew, Cargo, or by building from source.

## Homebrew (macOS / Linux)

The easiest way to install on macOS is via our custom tap.

```bash
brew tap pipe-rack/homebrew-tap
brew install pipe-rack/homebrew-tap/piperack
```

To update:

```bash
brew upgrade piperack
```

## Build from Source

To build from the latest source code:

1.  **Clone the repository:**

    ```bash
    git clone https://github.com/pipe-rack/piperack.git
    cd piperack
    ```

2.  **Build and install:**

    ```bash
    cargo install --path .
    ```

## Pre-built Binaries

Check the [GitHub Releases](https://github.com/pipe-rack/piperack/releases) page for pre-compiled binaries for your architecture. Download the binary, make it executable (`chmod +x piperack`), and move it to a directory in your `$PATH`.
