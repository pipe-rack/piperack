# Usage

## CLI Arguments

You can run Piperack with a configuration file or by defining processes directly on the command line.

### Using a Config File

```bash
piperack
# or
piperack --config my-config.toml
```

### Inline Commands

You can define processes using the `--name` flag followed by the command after `--`.

```bash
piperack --name api -- cargo run --name web -- npm start
```

**Shorthand Mode:**
If you have a list of commands, you can use `--names`.

```bash
piperack --names "api,web" "cargo run" "npm start"
```

**Additional Flags:**

### Global Configuration

| Flag | Description |
| :--- | :--- |
| `--config <path>` | Path to `piperack.toml` configuration file. |
| `--no-config` | Ignore any `piperack.toml` in the current directory. |
| `--max-lines <n>` | Max log lines per process (default: 10,000). |
| `--no-ui` | Run without the TUI (streams output to stdout). |
| `--raw` | In `--no-ui` mode, output raw lines without prefixes. |
| `--prefix <tpl>` | Prefix template (e.g. `[{name}]`). |
| `--prefix-length <n>` | Pad or truncate prefix to length. |
| `--prefix-colors` | Colorize prefixes in non-TUI output. |
| `--timestamp` | Prepend timestamp to each line. |
| `--output <mode>` | Output mode for `--no-ui`: `combined`, `grouped`, `raw`. |
| `--success <policy>` | Exit policy: `first`, `last`, `all`. |
| `--kill-others` | Kill all processes if one exits. |
| `--kill-others-on-fail` | Kill all processes if one fails. |
| `--restart-tries <n>` | Max restart attempts for `restart_on_fail`. |
| `--restart-delay-ms <ms>` | Delay before restarting (ms). |
| `--no-input` | Disable input forwarding. |
| `--log-file <tpl>` | Log file template (e.g. `logs/{name}.log`). |

### Inline Process Definition

When defining processes inline with `--name <name> ... -- <cmd>`, you can use these flags **before** the `--` separator:

| Flag | Description |
| :--- | :--- |
| `--cwd <path>` | Working directory for the process. |
| `--env <KEY=VAL>` | Set environment variable. |
| `--color <color>` | Override process color. |
| `--follow` / `--no-follow` | Enable/disable auto-follow. |
| `--restart-on-fail` | Restart if the process fails. |
| `--pre <cmd>` | Command to run before the main process. |
| `--watch <path>` | Watch path for changes. |
| `--watch-ignore <path>` | Ignore path when watching. |
| `--watch-debounce-ms <ms>` | Debounce time for watch events. |

**Example:**
```bash
piperack --name api --cwd ./backend --watch src -- cargo run
```

### Shorthand Mode

For quickly running multiple commands, you can use aligned lists.

| Flag | Description |
| :--- | :--- |
| `--names <a,b>` | Comma-separated names. |
| `--cwd <path>` | Working directory (one per name, or one shared). |
| `--env <KEY=VAL>` | Env var (global, or `name:KEY=VAL`, or `index:KEY=VAL`). |
| `--color <color>` | Color (one per name). |
| `--pre <cmd>` | Pre-command (one per name). |

**Example:**
```bash
piperack --names "api,web" --cwd "./api" "./web" "cargo run" "npm start"
```

## TUI Controls

Once Piperack is running, use these keys to interact with the interface.

### Navigation

| Key | Action |
| :--- | :--- |
| `↑` / `↓` | Select previous/next process. |
| `Tab` | Cycle through processes. |
| `PgUp` / `PgDown` | Scroll logs up/down. |
| `Home` / `End` | Scroll to top/bottom (and follow). |
| `f` | Toggle **Follow** mode (auto-scroll). |
| `t` | Toggle **Timeline** view (merged logs from all processes). |
| `Mouse Click` | Select process. |
| `Mouse Wheel` | Scroll logs. |

### Actions

| Key | Action |
| :--- | :--- |
| `r` | **Restart** the selected process. |
| `R` | **Restart All** processes. |
| `k` | **Kill** the selected process. |
| `e` | **Export** logs of the selected process to a file. |
| `q` or `Ctrl+c` | **Quit** Piperack. |

### View & Search

| Key | Action |
| :--- | :--- |
| `/` | **Search** mode. Type a query to highlight matches. |
| `n` / `N` | Jump to next/previous search match. |
| `F` | **Filter** mode (Show only lines matching query - *Coming Soon*). |
| `j` | Toggle **JSON** pretty-printing. |
| `a` | Toggle **ANSI** stripping (show/hide colors). |
| `?` | Toggle **Help** overlay. |

### Group Actions

| Key | Action |
| :--- | :--- |
| `g` | **Group Restart**. Type a tag name to restart all processes with that tag. |

### Input

| Key | Action |
| :--- | :--- |
| `Enter` | Enter **Input Mode**. Type text and press Enter again to send to the process's stdin. |
