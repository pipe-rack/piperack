//! Piperack: A concurrent process runner with a TUI.
//!
//! This is the entry point of the application. It parses command-line arguments,
//! loads configuration, and sets up the main event loop to manage processes
//! and user interaction.

mod ansi;
mod clipboard;
mod app;
mod config;
mod events;
mod output;
mod process;
mod runner;
mod tui;
mod watch;

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use clap::builder::styling::{AnsiColor, Effects, Style};
use clap::builder::Styles;
use clap::{CommandFactory, Parser, Subcommand};
use tokio::sync::mpsc;

use crate::app::{App, AppAction};
use crate::config::ProcessConfig;
use crate::events::{Event, ProcessSignal};
use crate::output::StreamKind;
use crate::process::{ProcessSpec, ProcessState};
use crate::runner::{ProcessManager, ShutdownConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum OutputMode {
    Combined,
    Grouped,
    Raw,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum SuccessPolicy {
    First,
    Last,
    All,
}

/// Command-line interface definition.
#[derive(Debug, Parser)]
#[command(
    name = "piperack",
    version,
    about = "Concurrent command runner with TUI",
    styles = help_styles(),
    color = clap::ColorChoice::Always,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
    /// Path to piperack.toml configuration file.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Ignore any piperack.toml in the current directory.
    #[arg(long)]
    no_config: bool,
    /// Max log lines per process.
    #[arg(long)]
    max_lines: Option<usize>,
    /// Disable the TUI and print to stdout.
    #[arg(long)]
    no_ui: bool,
    /// Disable prefixed output in non-TUI mode.
    #[arg(long)]
    raw: bool,
    /// Prefix template (e.g. "[{name}]").
    #[arg(long)]
    prefix: Option<String>,
    /// Pad or truncate prefix to length.
    #[arg(long)]
    prefix_length: Option<usize>,
    /// Colorize prefixes in non-TUI output.
    #[arg(long)]
    prefix_colors: bool,
    /// Prepend timestamp to each line.
    #[arg(long)]
    timestamp: bool,
    /// Output mode in non-TUI mode ("combined", "grouped", "raw").
    #[arg(long, value_enum)]
    output: Option<OutputMode>,
    /// Success policy when processes exit ("first", "last", "all").
    #[arg(long, value_enum)]
    success: Option<SuccessPolicy>,
    /// Stop other processes when any exits.
    #[arg(long)]
    kill_others: bool,
    /// Stop other processes when any exits with failure.
    #[arg(long)]
    kill_others_on_fail: bool,
    /// Max restart attempts for restart_on_fail.
    #[arg(long)]
    restart_tries: Option<u32>,
    /// Delay before restarting (ms).
    #[arg(long)]
    restart_delay_ms: Option<u64>,
    /// Time to wait after sending SIGINT before escalating (ms).
    #[arg(long)]
    shutdown_sigint_ms: Option<u64>,
    /// Time to wait after sending SIGTERM before force-killing (ms).
    #[arg(long)]
    shutdown_sigterm_ms: Option<u64>,
    /// Disable input forwarding.
    #[arg(long)]
    no_input: bool,
    /// Log file template (e.g. "logs/{name}.log").
    #[arg(long)]
    log_file: Option<String>,
    /// Comma-separated process names (shorthand for commands list).
    #[arg(long)]
    names: Option<String>,
    /// Working directories aligned with shorthand command list.
    #[arg(long)]
    cwd: Vec<String>,
    /// Env entries (KEY=VAL, or name:KEY=VAL for per-process).
    #[arg(long)]
    env: Vec<String>,
    /// Colors aligned with shorthand command list.
    #[arg(long)]
    color: Vec<String>,
    /// Pre-commands aligned with shorthand command list.
    #[arg(long)]
    pre: Vec<String>,
    /// Restart CLI-defined processes on failure.
    #[arg(long)]
    restart_on_fail: bool,
    /// Process definitions: --name <name> -- <cmd> [args...]
    #[arg(trailing_var_arg = true)]
    args: Vec<String>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Show help information.
    Help,
    /// Show version information.
    Version,
    /// Print the ANSI banner.
    Banner,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(command) = &cli.command {
        match command {
            Commands::Help => {
                Cli::command().print_help()?;
                println!();
                return Ok(());
            }
            Commands::Version => {
                println!("piperack {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            Commands::Banner => {
                print_ansi_banner();
                return Ok(());
            }
        }
    }
    let (specs, settings) = load_specs(&cli)?;
    if specs.is_empty() {
        bail!("no processes defined (use piperack.toml or --name ... -- cmd)");
    }

    let (event_tx, mut event_rx) = mpsc::channel(256);
    let shutdown = ShutdownConfig::new(settings.shutdown_sigint_ms, settings.shutdown_sigterm_ms);
    let mut manager = ProcessManager::new(specs.clone(), event_tx.clone(), shutdown);
    let mut app = App::new(
        specs,
        settings.max_lines,
        settings.use_symbols,
        settings.input_enabled,
    );
    let mut restart_attempts: HashMap<usize, u32> = HashMap::new();

    manager.start_all().await?;

    let mut terminal = if settings.no_ui {
        None
    } else {
        Some(tui::init_terminal()?)
    };
    let tick_rate = Duration::from_millis(150);

    if !settings.no_ui {
        spawn_input_listener(event_tx.clone());
    } else if settings.input_enabled {
        spawn_stdin_listener(event_tx.clone());
    }
    watch::spawn_watchers(&app.processes, event_tx.clone());
    spawn_signal_listener(event_tx.clone());

    let mut ticker = tokio::time::interval(tick_rate);
    let mut result = Ok(());
    let mut output_state = OutputState::new(&app.processes, &settings);
    let mut shutdown_in_progress = false;
    let mut shutdown_started_at: Option<Instant> = None;
    const MIN_SHUTDOWN_DISPLAY: Duration = Duration::from_millis(1500);
    const MIN_SIGNAL_DISPLAY: Duration = Duration::from_millis(1500);
    let mut shutdown_pending: Option<ProcessSignal> = None;
    let mut shutdown_dispatch_at: Option<Instant> = None;
    let mut last_signal_at: Option<Instant> = None;

    loop {
        tokio::select! {
            Some(event) = event_rx.recv() => {
                match event {
                    Event::ProcessStarting { id } => {
                        app.on_process_starting(id);
                        let name = app
                            .processes
                            .get(id)
                            .map(|p| p.spec.name.as_str())
                            .unwrap_or("process");
                        app.set_status_message(format!("Starting {}", name));
                        if let Some(spec) = app.processes.get(id).map(|p| &p.spec) {
                            let cmd = format_command(spec);
                            emit_tool_message(
                                id,
                                format!("starting: {}", cmd),
                                &mut app,
                                &settings,
                                &mut output_state,
                            );
                        }
                    }
                    Event::ProcessStarted { id, pid } => app.on_process_started(id, pid),
                    Event::ProcessReady { id } => {
                        app.on_process_ready(id);
                        let name = app
                            .processes
                            .get(id)
                            .map(|p| p.spec.name.as_str())
                            .unwrap_or("process");
                        app.set_status_message(format!("{} ready", name));
                        emit_tool_message(
                            id,
                            "ready".to_string(),
                            &mut app,
                            &settings,
                            &mut output_state,
                        );
                        if let Err(e) = manager.mark_ready(id).await {
                             app.on_process_failed(id, e.to_string());
                        }
                    }
                    Event::ProcessWaiting { id, deps } => {
                        let name = app
                            .processes
                            .get(id)
                            .map(|p| p.spec.name.as_str())
                            .unwrap_or("process");
                        let waiting_on = if deps.is_empty() {
                            "dependencies".to_string()
                        } else {
                            deps.join(", ")
                        };
                        app.set_status_message(format!("{} waiting for {}", name, waiting_on));
                        emit_tool_message(
                            id,
                            format!("waiting for {}", waiting_on),
                            &mut app,
                            &settings,
                            &mut output_state,
                        );
                    }
                    Event::ProcessOutput { id, line, stream } => {
                        let line_for_output = line.clone();
                        app.on_process_output(id, line, stream);
                        if settings.no_ui {
                            output_state.handle_event(
                                &Event::ProcessOutput { id, line: line_for_output, stream },
                                &app,
                                &settings,
                            );
                        } else {
                            output_state.log_event(id, &line_for_output, &app, &settings);
                        }
                    }
                    Event::ProcessExited { id, code } => {
                        app.on_process_exited(id, code);
                        let name = app
                            .processes
                            .get(id)
                            .map(|p| p.spec.name.as_str())
                            .unwrap_or("process");
                        if !shutdown_in_progress {
                            let signal_recent = last_signal_at
                                .map(|at| at.elapsed() < MIN_SIGNAL_DISPLAY)
                                .unwrap_or(false);
                            if !signal_recent {
                                let message = match code {
                                    Some(0) => format!("{} exited successfully", name),
                                    Some(code) => format!("{} exited with code {}", name, code),
                                    None => format!("{} exited", name),
                                };
                                app.set_status_message(message);
                            }
                        }
                        let line = match code {
                            Some(0) => "process ended successfully".to_string(),
                            Some(code) => format!("process ended with code {}", code),
                            None => "process ended".to_string(),
                        };
                        emit_tool_message(id, line, &mut app, &settings, &mut output_state);
                        let restart_info = if shutdown_in_progress {
                            None
                        } else {
                            handle_restart(
                                id,
                                code,
                                &app,
                                &settings,
                                &mut restart_attempts,
                                &event_tx,
                            )
                        };
                        if let Some(info) = restart_info {
                            emit_tool_message(
                                id,
                                format_restart_message(&info),
                                &mut app,
                                &settings,
                                &mut output_state,
                            );
                        }
                        if shutdown_in_progress {
                            output_state.handle_exit(id, code);
                            let ready_to_exit = output_state.all_exited()
                                && shutdown_started_at
                                    .map(|start| start.elapsed() >= MIN_SHUTDOWN_DISPLAY)
                                    .unwrap_or(false);
                            if ready_to_exit {
                                app.should_quit = true;
                            }
                        } else {
                            handle_exit_policy(
                                id,
                                code,
                                &mut app,
                                &settings,
                                &mut output_state,
                                &mut manager,
                                &mut result,
                            )
                            .await;
                        }
                    }
                    Event::ProcessFailed { id, error } => {
                        let error_message = error.clone();
                        app.on_process_failed(id, error);
                        let name = app
                            .processes
                            .get(id)
                            .map(|p| p.spec.name.as_str())
                            .unwrap_or("process");
                        if !shutdown_in_progress {
                            let signal_recent = last_signal_at
                                .map(|at| at.elapsed() < MIN_SIGNAL_DISPLAY)
                                .unwrap_or(false);
                            if !signal_recent {
                                let message = format!("{} failed: {}", name, error_message);
                                app.set_status_message(message);
                            }
                        }
                        emit_tool_message(
                            id,
                            format!("process failed: {}", error_message),
                            &mut app,
                            &settings,
                            &mut output_state,
                        );
                        let restart_info = if shutdown_in_progress {
                            None
                        } else {
                            handle_restart(
                                id,
                                Some(1),
                                &app,
                                &settings,
                                &mut restart_attempts,
                                &event_tx,
                            )
                        };
                        if let Some(info) = restart_info {
                            emit_tool_message(
                                id,
                                format_restart_message(&info),
                                &mut app,
                                &settings,
                                &mut output_state,
                            );
                        }
                        if shutdown_in_progress {
                            output_state.handle_exit(id, Some(1));
                            let ready_to_exit = output_state.all_exited()
                                && shutdown_started_at
                                    .map(|start| start.elapsed() >= MIN_SHUTDOWN_DISPLAY)
                                    .unwrap_or(false);
                            if ready_to_exit {
                                app.should_quit = true;
                            }
                        } else {
                            handle_exit_policy(
                                id,
                                Some(1),
                                &mut app,
                                &settings,
                                &mut output_state,
                                &mut manager,
                                &mut result,
                            )
                            .await;
                        }
                    }
                    Event::ProcessSignal { id, signal } => {
                        let name = app
                            .processes
                            .get(id)
                            .map(|p| p.spec.name.as_str())
                            .unwrap_or("process");
                        let label = signal.label();
                        if shutdown_in_progress || shutdown_pending.is_some() {
                            app.set_status_warning_persistent(format!(
                                "shutting down — sent {} to {}",
                                label, name
                            ));
                        } else {
                            last_signal_at = Some(Instant::now());
                            app.set_status_warning_for(
                                format!("sent {} to {}", label, name),
                                MIN_SIGNAL_DISPLAY,
                            );
                        }
                        emit_tool_message(
                            id,
                            format!("sent {}", label),
                            &mut app,
                            &settings,
                            &mut output_state,
                        );
                    }
                    Event::Restart { id } => {
                        if let Err(err) = manager.restart_process(id).await {
                            app.on_process_failed(id, err.to_string());
                        }
                    }
                    Event::Shutdown { signal } => {
                        if !shutdown_in_progress {
                            let label = signal.label();
                            app.should_quit = false;
                            app.set_status_warning_persistent(format!(
                                "received {}, shutting down",
                                label
                            ));
                            shutdown_pending = Some(signal);
                            shutdown_dispatch_at = if settings.no_ui {
                                Some(Instant::now())
                            } else {
                                None
                            };
                        }
                    }
                    Event::Stdin(bytes) => {
                        if let Err(err) = manager.send_input_bytes_to_all(&bytes).await {
                            app.set_status_message(format!("Input failed: {}", err));
                        }
                    }
                    Event::Key(key) => {
                        let action = app.handle_key(key);
                        handle_app_action(
                            action,
                            &mut app,
                            &mut manager,
                            &mut restart_attempts,
                            &event_tx,
                        )
                        .await;
                    }
                    Event::Mouse(mouse) => {
                        let action = app.handle_mouse(mouse);
                        handle_app_action(
                            action,
                            &mut app,
                            &mut manager,
                            &mut restart_attempts,
                            &event_tx,
                        )
                        .await;
                    }
                    Event::Resize { width, height } => {
                        let _ = (width, height);
                        if let Some(term) = terminal.as_mut() {
                            let _ = term.autoresize();
                        }
                    }
                }

            }
            _ = ticker.tick() => {
                manager.poll_exits().await;
                if let Some(signal) = shutdown_pending {
                    if shutdown_dispatch_at
                        .map(|when| Instant::now() >= when)
                        .unwrap_or(false)
                    {
                        shutdown_in_progress = true;
                        shutdown_started_at = Some(Instant::now());
                        shutdown_pending = None;
                        shutdown_dispatch_at = None;
                        manager.begin_shutdown_all(signal).await;
                    }
                }
                if shutdown_in_progress
                    && output_state.all_exited()
                    && shutdown_started_at
                        .map(|start| start.elapsed() >= MIN_SHUTDOWN_DISPLAY)
                        .unwrap_or(false)
                {
                    app.should_quit = true;
                }
            }
        }

        if let Some(term) = terminal.as_mut() {
            if let Err(err) = tui::draw(&mut app, term) {
                result = Err(err.into());
                break;
            }
        }
        if shutdown_pending.is_some() && shutdown_dispatch_at.is_none() && !settings.no_ui {
            shutdown_dispatch_at = Some(Instant::now() + Duration::from_millis(100));
        }

        if app.should_quit {
            break;
        }
    }

    manager.shutdown_all().await;
    if let Some(term) = terminal {
        tui::restore_terminal(term)?;
    }
    result
}

fn spawn_input_listener(tx: mpsc::Sender<Event>) {
    std::thread::spawn(move || loop {
        if crossterm::event::poll(Duration::from_millis(100)).unwrap_or(false) {
            match crossterm::event::read() {
                Ok(crossterm::event::Event::Key(key)) => {
                    let _ = tx.blocking_send(Event::Key(key));
                }
                Ok(crossterm::event::Event::Mouse(mouse)) => {
                    let _ = tx.blocking_send(Event::Mouse(mouse));
                }
                Ok(crossterm::event::Event::Resize(width, height)) => {
                    let _ = tx.blocking_send(Event::Resize { width, height });
                }
                _ => {}
            }
        }
    });
}

fn spawn_signal_listener(tx: mpsc::Sender<Event>) {
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm = match signal(SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(_) => return,
            };
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    let _ = tx.send(Event::Shutdown { signal: ProcessSignal::SigInt }).await;
                }
                _ = sigterm.recv() => {
                    let _ = tx.send(Event::Shutdown { signal: ProcessSignal::SigTerm }).await;
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
            let _ = tx
                .send(Event::Shutdown {
                    signal: ProcessSignal::SigInt,
                })
                .await;
        }
    });
}

fn spawn_stdin_listener(tx: mpsc::Sender<Event>) {
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buffer = [0u8; 1024];
        loop {
            match stdin.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = tx.blocking_send(Event::Stdin(buffer[..n].to_vec()));
                }
                Err(_) => break,
            }
        }
    });
}

fn backoff_delay(attempt: u32, settings: &RunSettings) -> Duration {
    if let Some(delay_ms) = settings.restart_delay_ms {
        return Duration::from_millis(delay_ms);
    }
    let capped = attempt.saturating_sub(1).min(5);
    let delay = 1_u64 << capped;
    Duration::from_secs(delay.min(30))
}

fn load_specs(cli: &Cli) -> Result<(Vec<ProcessSpec>, RunSettings)> {
    let mut specs = Vec::new();
    let mut config_max_lines = None;
    let mut config_meta = ConfigMeta::default();
    if !cli.no_config {
        let config_path = cli
            .config
            .clone()
            .or_else(|| default_config_path().filter(|path| path.exists()));
        if let Some(path) = config_path {
            let config = config::load_config(&path)?;
            config_max_lines = config.max_lines;
            config_meta = ConfigMeta::from_config(&config);
            for process in config.processes {
                specs.push(spec_from_config(process)?);
            }
        }
    }

    if !cli.args.is_empty() {
        if cli.names.is_some() && !cli.args.iter().any(|arg| arg == "--name") {
            let cli_specs = parse_named_commands(cli)?;
            specs.extend(cli_specs);
        } else {
            let cli_specs = parse_cli_processes(&cli.args, cli.restart_on_fail)?;
            specs.extend(cli_specs);
        }
    }

    // Sort specs by Group (first tag) then Name
    specs.sort_by(|a, b| {
        let tag_a = a.tags.first().map(|s| s.as_str()).unwrap_or("");
        let tag_b = b.tags.first().map(|s| s.as_str()).unwrap_or("");
        match tag_a.cmp(tag_b) {
            std::cmp::Ordering::Equal => a.name.cmp(&b.name),
            other => other,
        }
    });

    ensure_unique_names(&specs)?;
    let settings = RunSettings::from_cli(cli, config_meta, config_max_lines);
    Ok((specs, settings))
}

fn default_config_path() -> Option<PathBuf> {
    let path = Path::new("piperack.toml");
    if path.exists() {
        Some(path.to_path_buf())
    } else {
        None
    }
}

fn spec_from_config(config: ProcessConfig) -> Result<ProcessSpec> {
    let mut parts = shell_words::split(&config.cmd)
        .with_context(|| format!("failed to parse cmd for {}", config.name))?;
    if parts.is_empty() {
        return Err(anyhow!("empty cmd for {}", config.name));
    }
    let cmd = parts.remove(0);
    Ok(ProcessSpec {
        name: config.name,
        cmd,
        args: parts,
        cwd: config.cwd,
        color: config.color,
        env: config.env.unwrap_or_default(),
        restart_on_fail: config.restart_on_fail.unwrap_or(false),
        follow: config.follow.unwrap_or(true),
        pre_cmd: config.pre_cmd,
        watch_paths: config.watch.unwrap_or_default(),
        watch_ignore: config.watch_ignore.unwrap_or_default(),
        watch_ignore_gitignore: config.watch_ignore_gitignore.unwrap_or(false),
        watch_debounce_ms: config.watch_debounce_ms.unwrap_or(200),
        depends_on: config.depends_on.unwrap_or_default(),
        ready_check: config.ready_check,
        tags: config.tags.unwrap_or_default(),
    })
}

fn parse_cli_processes(args: &[String], restart_on_fail: bool) -> Result<Vec<ProcessSpec>> {
    let mut specs = Vec::new();
    let mut idx = 0;
    while idx < args.len() {
        if args[idx] != "--name" {
            bail!("expected --name, got {}", args[idx]);
        }
        idx += 1;
        let name = args
            .get(idx)
            .ok_or_else(|| anyhow!("missing name after --name"))?
            .clone();
        idx += 1;
        let mut cwd = None;
        let mut env = HashMap::new();
        let mut color = None;
        let mut follow = true;
        let mut watch_paths = Vec::new();
        let mut watch_ignore = Vec::new();
        let mut watch_ignore_gitignore = false;
        let mut watch_debounce_ms = 200;
        let mut restart_on_fail_local = restart_on_fail;
        let mut pre_cmd = None;
        while idx < args.len() && args[idx] != "--" {
            match args[idx].as_str() {
                "--cwd" => {
                    idx += 1;
                    cwd = Some(
                        args.get(idx)
                            .ok_or_else(|| anyhow!("missing value for --cwd"))?
                            .clone(),
                    );
                }
                "--env" => {
                    idx += 1;
                    let kv = args
                        .get(idx)
                        .ok_or_else(|| anyhow!("missing value for --env"))?;
                    let (key, value) = split_env(kv)?;
                    env.insert(key, value);
                }
                "--color" => {
                    idx += 1;
                    color = Some(
                        args.get(idx)
                            .ok_or_else(|| anyhow!("missing value for --color"))?
                            .clone(),
                    );
                }
                "--follow" => {
                    follow = true;
                }
                "--no-follow" => {
                    follow = false;
                }
                "--restart-on-fail" => {
                    restart_on_fail_local = true;
                }
                "--no-restart-on-fail" => {
                    restart_on_fail_local = false;
                }
                "--pre" => {
                    idx += 1;
                    pre_cmd = Some(
                        args.get(idx)
                            .ok_or_else(|| anyhow!("missing value for --pre"))?
                            .clone(),
                    );
                }
                "--watch" => {
                    idx += 1;
                    watch_paths.push(
                        args.get(idx)
                            .ok_or_else(|| anyhow!("missing value for --watch"))?
                            .clone(),
                    );
                }
                "--watch-ignore" => {
                    idx += 1;
                    watch_ignore.push(
                        args.get(idx)
                            .ok_or_else(|| anyhow!("missing value for --watch-ignore"))?
                            .clone(),
                    );
                }
                "--watch-ignore-gitignore" => {
                    watch_ignore_gitignore = true;
                }
                "--watch-debounce-ms" => {
                    idx += 1;
                    let value = args
                        .get(idx)
                        .ok_or_else(|| anyhow!("missing value for --watch-debounce-ms"))?;
                    watch_debounce_ms = value
                        .parse::<u64>()
                        .map_err(|_| anyhow!("invalid --watch-debounce-ms"))?;
                }
                other => bail!("unknown option {} for --name {}", other, name),
            }
            idx += 1;
        }

        if args.get(idx).map(|s| s.as_str()) != Some("--") {
            bail!("expected -- after --name {}", name);
        }
        idx += 1;
        let mut cmd_parts = Vec::new();
        while idx < args.len() {
            if args[idx] == "--name" {
                break;
            }
            cmd_parts.push(args[idx].clone());
            idx += 1;
        }
        if cmd_parts.is_empty() {
            bail!("missing command for --name {}", name);
        }
        let cmd = cmd_parts.remove(0);
        specs.push(ProcessSpec {
            name,
            cmd,
            args: cmd_parts,
            cwd,
            color,
            env,
            restart_on_fail: restart_on_fail_local,
            follow,
            pre_cmd,
            watch_paths,
            watch_ignore,
            watch_ignore_gitignore,
            watch_debounce_ms,
            depends_on: Vec::new(),
            ready_check: None,
            tags: Vec::new(),
        });
    }
    Ok(specs)
}

fn ensure_unique_names(specs: &[ProcessSpec]) -> Result<()> {
    let mut seen = HashSet::new();
    for spec in specs {
        if !seen.insert(spec.name.clone()) {
            bail!("duplicate process name: {}", spec.name);
        }
    }
    Ok(())
}

fn help_styles() -> Styles {
    Styles::styled()
        .header(
            Style::new()
                .fg_color(Some(AnsiColor::Cyan.into()))
                .effects(Effects::BOLD),
        )
        .usage(
            Style::new()
                .fg_color(Some(AnsiColor::Green.into()))
                .effects(Effects::BOLD),
        )
        .literal(Style::new().fg_color(Some(AnsiColor::Yellow.into())))
        .placeholder(Style::new().fg_color(Some(AnsiColor::Magenta.into())))
        .valid(Style::new().fg_color(Some(AnsiColor::Green.into())))
        .invalid(
            Style::new()
                .fg_color(Some(AnsiColor::Red.into()))
                .effects(Effects::BOLD),
        )
}

#[derive(Debug, Default, Clone)]
struct ConfigMeta {
    symbols: Option<bool>,
    raw: Option<bool>,
    prefix: Option<String>,
    prefix_length: Option<usize>,
    prefix_colors: Option<bool>,
    timestamp: Option<bool>,
    output: Option<OutputMode>,
    success: Option<SuccessPolicy>,
    kill_others: Option<bool>,
    kill_others_on_fail: Option<bool>,
    restart_tries: Option<u32>,
    restart_delay_ms: Option<u64>,
    shutdown_sigint_ms: Option<u64>,
    shutdown_sigterm_ms: Option<u64>,
    handle_input: Option<bool>,
    log_file: Option<String>,
}

impl ConfigMeta {
    fn from_config(config: &config::Config) -> Self {
        Self {
            symbols: config.symbols,
            raw: config.raw,
            prefix: config.prefix.clone(),
            prefix_length: config.prefix_length,
            prefix_colors: config.prefix_colors,
            timestamp: config.timestamp,
            output: config
                .output
                .as_deref()
                .and_then(|v| parse_output_mode(v).ok()),
            success: config
                .success
                .as_deref()
                .and_then(|v| parse_success_policy(v).ok()),
            kill_others: config.kill_others,
            kill_others_on_fail: config.kill_others_on_fail,
            restart_tries: config.restart_tries,
            restart_delay_ms: config.restart_delay_ms,
            shutdown_sigint_ms: config.shutdown_sigint_ms,
            shutdown_sigterm_ms: config.shutdown_sigterm_ms,
            handle_input: config.handle_input,
            log_file: config.log_file.clone(),
        }
    }
}

/// Runtime configuration derived from CLI arguments and the config file.
#[derive(Debug, Clone)]
struct RunSettings {
    // Runtime behavior toggles collected from CLI + config.
    max_lines: usize,
    use_symbols: bool,
    no_ui: bool,
    raw: bool,
    prefix: Option<String>,
    prefix_length: Option<usize>,
    prefix_colors: bool,
    timestamp: bool,
    output_mode: OutputMode,
    success: SuccessPolicy,
    kill_others: bool,
    kill_others_on_fail: bool,
    restart_tries: Option<u32>,
    restart_delay_ms: Option<u64>,
    shutdown_sigint_ms: u64,
    shutdown_sigterm_ms: u64,
    input_enabled: bool,
    log_file: Option<String>,
}

impl RunSettings {
    fn from_cli(cli: &Cli, meta: ConfigMeta, config_max_lines: Option<usize>) -> Self {
        const DEFAULT_SHUTDOWN_SIGINT_MS: u64 = 800;
        const DEFAULT_SHUTDOWN_SIGTERM_MS: u64 = 800;
        let max_lines = cli.max_lines.or(config_max_lines).unwrap_or(10_000);
        let use_symbols = meta.symbols.unwrap_or(true);
        let raw = if cli.raw {
            true
        } else {
            meta.raw.unwrap_or(false)
        };
        let prefix = cli.prefix.clone().or(meta.prefix);
        let prefix_length = cli.prefix_length.or(meta.prefix_length);
        let prefix_colors = if cli.prefix_colors {
            true
        } else {
            meta.prefix_colors.unwrap_or(false)
        };
        let timestamp = if cli.timestamp {
            true
        } else {
            meta.timestamp.unwrap_or(false)
        };
        let output_mode = cli.output.or(meta.output).unwrap_or(OutputMode::Combined);
        let success = cli.success.or(meta.success).unwrap_or(SuccessPolicy::Last);
        let kill_others = cli.kill_others || meta.kill_others.unwrap_or(false);
        let kill_others_on_fail =
            cli.kill_others_on_fail || meta.kill_others_on_fail.unwrap_or(false);
        let restart_tries = cli.restart_tries.or(meta.restart_tries);
        let restart_delay_ms = cli.restart_delay_ms.or(meta.restart_delay_ms);
        let shutdown_sigint_ms = cli
            .shutdown_sigint_ms
            .or(meta.shutdown_sigint_ms)
            .unwrap_or(DEFAULT_SHUTDOWN_SIGINT_MS);
        let shutdown_sigterm_ms = cli
            .shutdown_sigterm_ms
            .or(meta.shutdown_sigterm_ms)
            .unwrap_or(DEFAULT_SHUTDOWN_SIGTERM_MS);
        let input_enabled = if cli.no_input {
            false
        } else {
            meta.handle_input.unwrap_or(true)
        };
        let log_file = cli.log_file.clone().or(meta.log_file);
        Self {
            max_lines,
            use_symbols,
            no_ui: cli.no_ui,
            raw,
            prefix,
            prefix_length,
            prefix_colors,
            timestamp,
            output_mode,
            success,
            kill_others,
            kill_others_on_fail,
            restart_tries,
            restart_delay_ms,
            shutdown_sigint_ms,
            shutdown_sigterm_ms,
            input_enabled,
            log_file,
        }
    }
}

fn parse_output_mode(value: &str) -> Result<OutputMode> {
    match value.to_lowercase().as_str() {
        "combined" => Ok(OutputMode::Combined),
        "grouped" => Ok(OutputMode::Grouped),
        "raw" => Ok(OutputMode::Raw),
        _ => Err(anyhow!("invalid output mode: {}", value)),
    }
}

fn parse_success_policy(value: &str) -> Result<SuccessPolicy> {
    match value.to_lowercase().as_str() {
        "first" => Ok(SuccessPolicy::First),
        "last" => Ok(SuccessPolicy::Last),
        "all" => Ok(SuccessPolicy::All),
        _ => Err(anyhow!("invalid success policy: {}", value)),
    }
}

fn split_env(value: &str) -> Result<(String, String)> {
    let (key, val) = value
        .split_once('=')
        .ok_or_else(|| anyhow!("invalid env {}, expected KEY=VALUE", value))?;
    Ok((key.to_string(), val.to_string()))
}

fn parse_named_commands(cli: &Cli) -> Result<Vec<ProcessSpec>> {
    // Shorthand mode: `--names a,b "cmd1" "cmd2"` with aligned arrays for cwd/env/color/pre.
    let names_raw = cli
        .names
        .as_ref()
        .ok_or_else(|| anyhow!("--names requires command list"))?;
    let names = names_raw
        .split(',')
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    if names.is_empty() {
        bail!("--names provided but no names parsed");
    }
    if cli.args.len() != names.len() {
        bail!(
            "expected {} commands for --names, got {}",
            names.len(),
            cli.args.len()
        );
    }
    let mut env_maps = vec![HashMap::new(); names.len()];
    let mut global_env = HashMap::new();
    for entry in &cli.env {
        if let Some((prefix, rest)) = entry.split_once(':') {
            if let Ok(index) = prefix.parse::<usize>() {
                if let Some(map) = env_maps.get_mut(index) {
                    let (k, v) = split_env(rest)?;
                    map.insert(k, v);
                    continue;
                }
            }
            if let Some(pos) = names.iter().position(|name| name == prefix) {
                let (k, v) = split_env(rest)?;
                env_maps[pos].insert(k, v);
                continue;
            }
        }
        let (k, v) = split_env(entry)?;
        global_env.insert(k, v);
    }
    for map in &mut env_maps {
        for (k, v) in &global_env {
            map.insert(k.clone(), v.clone());
        }
    }
    let pre_cmds = parse_aligned_list(&cli.pre, names.len(), "pre")?;
    let mut specs = Vec::new();
    for (idx, command) in cli.args.iter().enumerate() {
        let mut parts = shell_words::split(command)
            .with_context(|| format!("failed to parse command {}", command))?;
        if parts.is_empty() {
            bail!("empty command for {}", names[idx]);
        }
        let cmd = parts.remove(0);
        let cwd = cli.cwd.get(idx).cloned();
        let color = cli.color.get(idx).cloned();
        specs.push(ProcessSpec {
            name: names[idx].clone(),
            cmd,
            args: parts,
            cwd,
            color,
            env: env_maps[idx].clone(),
            restart_on_fail: cli.restart_on_fail,
            follow: true,
            pre_cmd: pre_cmds.get(idx).cloned().unwrap_or(None),
            watch_paths: Vec::new(),
            watch_ignore: Vec::new(),
            watch_ignore_gitignore: false,
            watch_debounce_ms: 200,
            depends_on: Vec::new(),
            ready_check: None,
            tags: Vec::new(),
        });
    }
    Ok(specs)
}

fn parse_aligned_list(values: &[String], len: usize, label: &str) -> Result<Vec<Option<String>>> {
    // Allow a single shared value or a fully aligned list.
    if values.is_empty() {
        return Ok(vec![None; len]);
    }
    if values.len() == 1 && len > 1 {
        return Ok(vec![Some(values[0].clone()); len]);
    }
    if values.len() != len {
        bail!(
            "expected {} values for --{}, got {}",
            len,
            label,
            values.len()
        );
    }
    Ok(values.iter().cloned().map(Some).collect())
}

struct OutputState {
    // Formatting/output state for non-TUI mode.
    output_mode: OutputMode,
    raw: bool,
    prefix: Option<String>,
    prefix_length: Option<usize>,
    prefix_colors: bool,
    timestamp: bool,
    start: std::time::Instant,
    grouped: Vec<Vec<String>>,
    logs: Vec<Option<std::io::BufWriter<std::fs::File>>>,
    names: Vec<String>,
    exit_codes: Vec<Option<i32>>,
    exited: Vec<bool>,
    last_exit: Option<(usize, Option<i32>)>,
}

impl OutputState {
    fn new(processes: &[ProcessState], settings: &RunSettings) -> Self {
        let grouped = vec![Vec::new(); processes.len()];
        let logs = init_log_writers(processes, settings.log_file.as_deref());
        let names = processes
            .iter()
            .map(|process| process.spec.name.clone())
            .collect();
        Self {
            output_mode: settings.output_mode,
            raw: settings.raw || settings.output_mode == OutputMode::Raw,
            prefix: settings.prefix.clone(),
            prefix_length: settings.prefix_length,
            prefix_colors: settings.prefix_colors,
            timestamp: settings.timestamp,
            start: std::time::Instant::now(),
            grouped,
            logs,
            names,
            exit_codes: vec![None; processes.len()],
            exited: vec![false; processes.len()],
            last_exit: None,
        }
    }

    fn handle_event(&mut self, event: &Event, app: &App, settings: &RunSettings) {
        if let Event::ProcessOutput { id, line, .. } = event {
            // Non-TUI output path: format + log each line as it arrives.
            let output = self.format_line(*id, line, app, settings);
            self.write_line(*id, &output);
            if self.output_mode == OutputMode::Grouped {
                self.grouped[*id].push(output);
            } else if self.output_mode != OutputMode::Raw {
                println!("{}", output);
            } else {
                println!("{}", line);
            }
        }
    }

    fn log_event(&mut self, id: usize, line: &str, app: &App, settings: &RunSettings) {
        let output = self.format_line(id, line, app, settings);
        self.write_line(id, &output);
    }

    fn handle_exit(&mut self, id: usize, code: Option<i32>) {
        if id >= self.exit_codes.len() {
            return;
        }
        self.exit_codes[id] = code;
        self.exited[id] = true;
        self.last_exit = Some((id, code));
        if self.output_mode == OutputMode::Grouped {
            if let Some(process) = self.grouped.get(id) {
                if !process.is_empty() {
                    let name = self.names.get(id).map(String::as_str).unwrap_or("process");
                    let header = format!("== {} ==", name);
                    println!("{}", header);
                    for line in process {
                        println!("{}", line);
                    }
                }
            }
        }
    }

    fn all_exited(&self) -> bool {
        self.exited.iter().all(|v| *v)
    }

    fn any_failed(&self) -> bool {
        self.exit_codes.iter().any(|code| code.unwrap_or(1) != 0)
    }

    fn format_line(&self, id: usize, line: &str, app: &App, settings: &RunSettings) -> String {
        if self.raw {
            return line.to_string();
        }
        let name = app
            .processes
            .get(id)
            .map(|p| p.spec.name.as_str())
            .unwrap_or("process");
        let color = app.processes.get(id).and_then(|p| p.spec.color.as_deref());
        let cleaned = strip_existing_prefix(name, line);
        let mut prefix = self.format_prefix(name, id, settings);
        if self.prefix_colors {
            prefix = apply_color(&prefix, color);
        }
        format!("{}{}", prefix, cleaned)
    }

    fn format_prefix(&self, name: &str, index: usize, _settings: &RunSettings) -> String {
        let mut prefix = if let Some(template) = self.prefix.as_deref() {
            let has_time = template.contains("{time}");
            let rendered = render_template(template, name, index, &self.elapsed());
            if self.timestamp && !has_time {
                format!("{} {}", self.elapsed(), rendered)
            } else {
                rendered
            }
        } else {
            format!("[{}]", name)
        };
        prefix = apply_prefix_length(prefix, self.prefix_length);
        if !prefix.is_empty() {
            prefix.push(' ');
        }
        prefix
    }

    fn elapsed(&self) -> String {
        let elapsed = self.start.elapsed();
        let secs = elapsed.as_secs();
        let minutes = secs / 60;
        let seconds = secs % 60;
        format!("{:02}:{:02}", minutes, seconds)
    }

    fn write_line(&mut self, id: usize, line: &str) {
        if let Some(Some(writer)) = self.logs.get_mut(id) {
            let _ = writeln!(writer, "{}", line);
        }
    }
}

fn init_log_writers(
    processes: &[ProcessState],
    template: Option<&str>,
) -> Vec<Option<std::io::BufWriter<std::fs::File>>> {
    // Create per-process log writers from a template, if provided.
    let mut writers = Vec::new();
    for (idx, process) in processes.iter().enumerate() {
        let writer = template.and_then(|tpl| {
            let time = log_timestamp();
            let path = render_template(tpl, &process.spec.name, idx, &time);
            if let Some(parent) = std::path::Path::new(&path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::File::create(path)
                .ok()
                .map(std::io::BufWriter::new)
        });
        writers.push(writer);
    }
    writers
}

fn log_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_secs().to_string()
}

fn render_template(template: &str, name: &str, index: usize, time: &str) -> String {
    // Simple token replacement for log/prefix templates.
    template
        .replace("{name}", name)
        .replace("{index}", &index.to_string())
        .replace("{time}", time)
}

fn apply_prefix_length(prefix: String, length: Option<usize>) -> String {
    let Some(length) = length else { return prefix };
    let mut out = prefix;
    if out.len() > length {
        out.truncate(length);
    } else if out.len() < length {
        out.push_str(&" ".repeat(length - out.len()));
    }
    out
}

fn apply_color(prefix: &str, color: Option<&str>) -> String {
    let code = match color.unwrap_or("").to_lowercase().as_str() {
        "black" => "30",
        "red" => "31",
        "green" => "32",
        "yellow" => "33",
        "blue" => "34",
        "magenta" => "35",
        "cyan" => "36",
        "gray" | "grey" => "90",
        _ => "0",
    };
    if code == "0" {
        prefix.to_string()
    } else {
        format!("\u{1b}[{}m{}\u{1b}[0m", code, prefix)
    }
}

fn strip_existing_prefix(name: &str, text: &str) -> String {
    let candidates = [
        format!("[{}] ", name),
        format!("[{}]", name),
        format!("{} \u{203a} ", name),
        format!("{}: ", name),
        format!("{} - ", name),
    ];
    for candidate in candidates {
        if let Some(rest) = text.strip_prefix(&candidate) {
            return rest.trim_start().to_string();
        }
    }
    text.to_string()
}

fn format_command(spec: &ProcessSpec) -> String {
    let mut parts = Vec::with_capacity(1 + spec.args.len());
    parts.push(spec.cmd.clone());
    parts.extend(spec.args.clone());
    shell_words::join(parts)
}

fn emit_tool_message(
    id: usize,
    text: String,
    app: &mut App,
    settings: &RunSettings,
    output_state: &mut OutputState,
) {
    if settings.no_ui && settings.raw {
        return;
    }
    let message = format_tool_message(&text, settings.use_symbols);
    app.on_process_output(id, message.clone(), StreamKind::Stdout);
    if settings.no_ui {
        output_state.handle_event(
            &Event::ProcessOutput {
                id,
                line: message,
                stream: StreamKind::Stdout,
            },
            app,
            settings,
        );
    } else {
        output_state.log_event(id, &message, app, settings);
    }
}

struct RestartInfo {
    attempt: u32,
    max: Option<u32>,
    delay: Duration,
}

fn format_restart_message(info: &RestartInfo) -> String {
    let delay_ms = info.delay.as_millis();
    let attempt = match info.max {
        Some(max) => format!("attempt {}/{}", info.attempt, max),
        None => format!("attempt {}", info.attempt),
    };
    format!("retrying in {}ms ({})", delay_ms, attempt)
}

fn format_tool_message(text: &str, use_symbols: bool) -> String {
    if use_symbols {
        format!("◆ piperack: {}", text)
    } else {
        format!("[piperack] {}", text)
    }
}

fn print_ansi_banner() {
    let c1 = "\x1b[38;5;39m";
    let c2 = "\x1b[38;5;45m";
    let c3 = "\x1b[38;5;51m";
    let reset = "\x1b[0m";
    let lines = [
        "██████╗ ██╗██████╗ ███████╗██████╗  █████╗  ██████╗██╗  ██╗",
        "██╔══██╗██║██╔══██╗██╔════╝██╔══██╗██╔══██╗██╔════╝██║ ██╔╝",
        "██████╔╝██║██████╔╝█████╗  ██████╔╝███████║██║     █████╔╝ ",
        "██╔═══╝ ██║██╔═══╝ ██╔══╝  ██╔══██╗██╔══██║██║     ██╔═██╗ ",
        "██║     ██║██║     ███████╗██║  ██║██║  ██║╚██████╗██║  ██╗",
        "╚═╝     ╚═╝╚═╝     ╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝╚═╝  ╚═╝",
    ];
    for (idx, line) in lines.iter().enumerate() {
        let color = match idx % 3 {
            0 => c1,
            1 => c2,
            _ => c3,
        };
        println!("{}{}{}", color, line, reset);
    }
}

fn handle_restart(
    id: usize,
    code: Option<i32>,
    app: &App,
    settings: &RunSettings,
    restart_attempts: &mut HashMap<usize, u32>,
    event_tx: &mpsc::Sender<Event>,
) -> Option<RestartInfo> {
    // Restart only on failure when enabled, with optional retry cap + delay.
    let should_restart = app
        .processes
        .get(id)
        .map(|process| process.spec.restart_on_fail)
        .unwrap_or(false);
    if should_restart && code.unwrap_or(1) != 0 {
        let attempt = restart_attempts
            .entry(id)
            .and_modify(|a| *a += 1)
            .or_insert(1);
        if settings
            .restart_tries
            .map(|max| *attempt <= max)
            .unwrap_or(true)
        {
            let backoff = backoff_delay(*attempt, settings);
            let tx = event_tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(backoff).await;
                let _ = tx.send(Event::Restart { id }).await;
            });
            return Some(RestartInfo {
                attempt: *attempt,
                max: settings.restart_tries,
                delay: backoff,
            });
        }
    } else if code.unwrap_or(1) == 0 {
        restart_attempts.remove(&id);
    }
    None
}

async fn handle_exit_policy(
    id: usize,
    code: Option<i32>,
    app: &mut App,
    settings: &RunSettings,
    output_state: &mut OutputState,
    manager: &mut ProcessManager,
    result: &mut Result<()>,
) {
    // Apply success/kill policies after a process exits.
    output_state.handle_exit(id, code);

    if settings.kill_others || (settings.kill_others_on_fail && code.unwrap_or(1) != 0) {
        manager.shutdown_all().await;
        app.should_quit = true;
        return;
    }

    match settings.success {
        SuccessPolicy::First => {
            if code.unwrap_or(1) == 0 {
                manager.shutdown_all().await;
                app.should_quit = true;
            }
        }
        SuccessPolicy::Last => {
            if output_state.all_exited() {
                if let Some((_, last)) = output_state.last_exit {
                    if last.unwrap_or(1) != 0 {
                        *result = Err(anyhow!("last process failed"));
                    }
                }
                app.should_quit = true;
            }
        }
        SuccessPolicy::All => {
            if output_state.all_exited() {
                if output_state.any_failed() {
                    *result = Err(anyhow!("one or more processes failed"));
                }
                app.should_quit = true;
            }
        }
    }
}

async fn handle_app_action(
    action: AppAction,
    app: &mut App,
    manager: &mut ProcessManager,
    restart_attempts: &mut HashMap<usize, u32>,
    event_tx: &mpsc::Sender<Event>,
) {
    match action {
        AppAction::Quit => {
            app.should_quit = false;
            let _ = event_tx
                .send(Event::Shutdown {
                    signal: ProcessSignal::SigInt,
                })
                .await;
        }
        AppAction::Kill(id) => {
            manager
                .begin_shutdown_process(id, ProcessSignal::SigInt)
                .await;
        }
        AppAction::Restart(id) => {
            restart_attempts.remove(&id);
            if let Err(err) = manager.restart_process(id).await {
                app.on_process_failed(id, err.to_string());
            }
        }
        AppAction::RestartGroup(tag) => {
            let ids: Vec<usize> = app
                .processes
                .iter()
                .enumerate()
                .filter(|(_, p)| tag == "all" || p.spec.tags.contains(&tag))
                .map(|(id, _)| id)
                .collect();

            for id in ids {
                restart_attempts.remove(&id);
                if let Err(err) = manager.restart_process(id).await {
                    app.on_process_failed(id, err.to_string());
                }
            }
        }
        AppAction::Export(id) => {
            if app.processes.get(id).is_some() {
                if let Err(err) = app.export_selected_logs() {
                    app.set_status_message(format!("Export failed: {}", err));
                }
            }
        }
        AppAction::SendInputText(id, text) => {
            if let Err(err) = manager.send_input_text(id, text).await {
                app.set_status_message(format!("Input failed: {}", err));
            }
        }
        AppAction::SendInputBytes(id, bytes) => {
            if let Err(err) = manager.send_input_bytes(id, &bytes).await {
                app.set_status_message(format!("Input failed: {}", err));
            }
        }
        AppAction::CopySelection => {
            let selection = app.selection_text();
            let payload = selection.or_else(|| app.selected_process_raw_text());
            if let Some(text) = payload {
                match clipboard::copy_text(&text) {
                    Ok(()) => app.set_status_warning_for("copied to clipboard", Duration::from_secs(2)),
                    Err(err) => app.set_status_warning_for(
                        format!("clipboard failed: {}", err),
                        Duration::from_secs(3),
                    ),
                }
            } else {
                app.set_status_warning_for("nothing to copy", Duration::from_secs(2));
            }
        }
        AppAction::None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cli_processes_splits_commands() {
        let args = vec![
            "--name".to_string(),
            "api".to_string(),
            "--".to_string(),
            "cargo".to_string(),
            "run".to_string(),
            "--name".to_string(),
            "web".to_string(),
            "--".to_string(),
            "pnpm".to_string(),
            "dev".to_string(),
        ];
        let specs = parse_cli_processes(&args, false).unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].name, "api");
        assert_eq!(specs[0].cmd, "cargo");
        assert_eq!(specs[0].args, vec!["run"]);
    }
}
