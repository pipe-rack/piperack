# Configuration

Piperack is configured via a `piperack.toml` file in your project root. If no configuration file is found, you can pass commands via CLI arguments.

## Automated Configuration (AI Skill)

You can use the official Piperack skill to automatically generate a configuration file based on your project structure. This works with **Gemini, Claude, Cursor, and other agents** that support the `skills` protocol.

1.  **Install the skill**:
    ```bash
    npx skills add pipe-rack/skills
    ```

2.  **Ask your Agent**:
    ```text
    Help me configure piperack for this repo
    ```

The skill will analyze your project (detecting `package.json`, `Cargo.toml`, `docker-compose.yml`, etc.) and propose a complete `piperack.toml` with appropriate ready checks and dependencies.

## Global Options

These options control the overall behavior of Piperack.

| Option | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `max_lines` | `integer` | `10000` | Maximum number of log lines to keep in memory per process. |
| `symbols` | `boolean` | `true` | Use Unicode symbols in the TUI. |
| `raw` | `boolean` | `false` | Disable TUI and print raw output (useful for CI). |
| `prefix` | `string` | `[{name}]` | Template for log prefixes in raw mode. |
| `prefix_colors` | `boolean` | `false` | Colorize prefixes in raw mode. |
| `timestamp` | `boolean` | `false` | Add timestamps to logs in raw mode. |
| `output` | `string` | `"combined"` | Output mode for raw execution: `combined`, `grouped`, `raw`. |
| `success` | `string` | `"last"` | Exit policy: `first` (exit on first success), `last` (wait for all, fail if last fails), `all` (wait for all). |
| `kill_others` | `boolean` | `false` | If one process exits, kill all others. |
| `kill_others_on_fail` | `boolean` | `false` | If one process fails (non-zero exit), kill all others. |
| `restart_tries` | `integer` | - | Maximum number of restart attempts (default: infinite). |
| `restart_delay_ms` | `integer` | - | Delay in milliseconds before restarting a process. |
| `shutdown_sigint_ms` | `integer` | `800` | Time to wait after sending SIGINT before escalating. |
| `shutdown_sigterm_ms` | `integer` | `800` | Time to wait after sending SIGTERM before force-killing. |
| `handle_input` | `boolean` | `true` | Enable stdin forwarding. |
| `log_file` | `string` | - | Template for writing logs to files (e.g., `logs/{name}.log`). |

## Process Configuration

Define processes in the `[[process]]` array.

```toml
[[process]]
name = "api"
cmd = "cargo run"
```

| Option | Type | Description |
| :--- | :--- | :--- |
| `name` | `string` | **Required.** Unique identifier for the process. |
| `cmd` | `string` | **Required.** Command to execute. |
| `cwd` | `string` | Working directory for the process. |
| `env` | `map` | Environment variables (e.g., `{ PORT = "3000" }`). |
| `color` | `string` | Color for the process name (e.g., "blue", "red"). |
| `restart_on_fail` | `boolean` | Restart the process if it exits with a non-zero code. |
| `follow` | `boolean` | Automatically follow logs when selected (default: `true`). |
| `pre_cmd` | `string` | Command to run *before* starting the main command. |
| `depends_on` | `list` | List of process names that must be ready before this one starts. |
| `tags` | `list` | List of string tags for grouping processes in the UI. |

### Watch Mode

Piperack can restart processes when files change.

```toml
[[process]]
name = "server"
cmd = "go run main.go"
watch = ["*.go", "templates/"]
watch_ignore = ["*_test.go"]
watch_ignore_gitignore = true
watch_debounce_ms = 500
```

### Readiness Checks

Define how Piperack knows a process is "ready" (for `depends_on`).

**TCP Port:**
```toml
ready_check = { tcp = 8080 }
```

**Log Message (Regex):**
```toml
ready_check = { log = "Listening on port .*" }
```

**Fixed Delay:**
```toml
ready_check = { delay = 5000 } # milliseconds
```

## Example Configuration

```toml
# piperack.toml
prefix_colors = true
kill_others_on_fail = true

[[process]]
name = "db"
cmd = "docker-compose up db"
ready_check = { tcp = 5432 }
tags = ["backend"]

[[process]]
name = "api"
cmd = "npm start"
depends_on = ["db"]
tags = ["backend"]
env = { NODE_ENV = "development" }

[[process]]
name = "web"
cmd = "npm run dev"
tags = ["frontend"]
```
