# Contribution Guide

Thanks for contributing to Piperack! This repo is early-stage, so tight feedback loops and small, focused PRs help the most.

## Getting Started

- Install Rust (stable) and build:
  ```bash
  cargo build
  ```
- Run the example:
  ```bash
  cargo run -- --config examples/piperack.toml
  ```

## Development Workflow

- Keep changes scoped and easy to review.
- Run tests when possible:
  ```bash
  cargo test
  ```
- Format and lint:
  ```bash
  cargo fmt
  cargo clippy
  ```

## Coding Standards

- Rust 2021 edition.
- Use `rustfmt` defaults.
- Keep modules small and single‑purpose.
- Prefer explicit errors (`anyhow` / `thiserror`) over panics.

## Pull Requests

Include:
- Clear description of the change and why.
- Screenshots or short clips for TUI changes.
- Any relevant config or CLI examples.

If you’re unsure about direction, open a small PR or issue first.
