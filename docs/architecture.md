# Architecture

Piperack is built in Rust and designed for performance and reliability. It follows an asynchronous, event-driven architecture.

## Core Components

### 1. Main Entry Point (`main.rs`)
The entry point parses CLI arguments and loads the configuration. It initializes the `ProcessManager` and the TUI `App`. It sets up the main event loop, which ticks at a fixed rate (to handle UI updates) and listens for asynchronous events.

### 2. Process Manager (`runner.rs`, `process.rs`)
The `ProcessManager` is responsible for spawning and managing OS processes.
- **Tokio:** Uses `tokio::process` for non-blocking process management.
- **Output Streaming:** Captures `stdout` and `stderr` asynchronously and sends them via an `mpsc` channel to the main loop.
- **Dependencies:** Handles dependency resolution (`depends_on`) and readiness checks (`ready_check`) before starting processes.

### 3. Application State (`app.rs`)
The `App` struct holds the state of the UI.
- **Logs:** Stores in-memory log buffers for each process and a global "Timeline" buffer.
- **Selection:** Tracks the currently selected process, scroll position, and active view (Process vs. Timeline).
- **Search:** Manages search queries and highlights matches.

### 4. TUI (`tui.rs`)
Piperack uses the `ratatui` library for rendering the terminal user interface.
- **Stateless Rendering:** The UI is redrawn completely on every frame based on the current `App` state.
- **Layout:** The screen is split into a sidebar (process list) and a main area (logs/timeline).

## Event Loop

The core loop in `main.rs` listens for:
1.  **Process Events:** Output lines, exit codes, readiness signals.
2.  **User Input:** Key presses and mouse events (via `crossterm`).
3.  **File Changes:** Watcher events (via `notify`) to trigger restarts.

When an event is received, the `App` state is updated, and the UI is redrawn.

## File Structure

- `src/main.rs`: Entry point and event loop.
- `src/config.rs`: Configuration parsing (TOML).
- `src/process.rs`: Process state and specification definitions.
- `src/runner.rs`: Logic for spawning and managing processes.
- `src/app.rs`: UI state and logic (search, scroll, etc.).
- `src/tui.rs`: Rendering logic using Ratatui.
- `src/output.rs`: Log buffering and storage.
- `src/watch.rs`: File watching logic.
