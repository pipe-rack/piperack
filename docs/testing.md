# Testing

This project uses Rust's built-in test harness for unit tests and a small set of manual smoke checks for the TUI.

## Quick commands

- `cargo check` — fast compile/typecheck
- `cargo test` — run unit tests
- `cargo fmt` — format
- `cargo clippy` — lint

## Coverage (optional)

We use `cargo-llvm-cov` for local coverage reports.

Install:

```bash
cargo install cargo-llvm-cov
```

Run:

```bash
cargo llvm-cov
```

On macOS, you may need to point to Xcode/CommandLineTools LLVM binaries:

```bash
LLVM_COV=/Library/Developer/CommandLineTools/usr/bin/llvm-cov \
LLVM_PROFDATA=/Library/Developer/CommandLineTools/usr/bin/llvm-profdata \
cargo llvm-cov
```

## Manual TUI smoke checks

- Start the full stack example: `cargo run -- --config examples/full_stack.toml`
- Verify shutdown UX:
  - `q` shows persistent "shutting down" status and exits cleanly.
  - `k` shows "sent SIGINT" immediately for a process.
- Verify clipboard selection:
  - Mouse drag selects log lines.
  - Ctrl+C copies selection; if none, copies full selected process buffer.
- Verify selection stability:
  - Selection remains while logs stream.
  - Selection persists when toggling follow.
