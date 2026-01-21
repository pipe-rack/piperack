//! Process execution and management.
//!
//! This module contains the `ProcessManager`, which is responsible for spawning,
//! monitoring, and interacting with child processes. It handles standard I/O streams
//! and bridges system process events to the application's event channel.

use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use regex::Regex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::config::ReadinessCheck;
use crate::events::{Event, ProcessSignal};
use crate::output::StreamKind;
use crate::process::ProcessSpec;

/// Manages the lifecycle and I/O of child processes.
pub struct ProcessManager {
    processes: Vec<ManagedProcess>,
    event_tx: mpsc::Sender<Event>,
    shutdown: ShutdownConfig,
}

struct ManagedProcess {
    spec: ProcessSpec,
    child: Option<tokio::process::Child>,
    stdin: Option<tokio::process::ChildStdin>,
    started: bool,
    ready: bool,
    waiting_on: Vec<String>,
    shutdown: Option<ShutdownState>,
}

#[derive(Debug, Clone, Copy)]
pub struct ShutdownConfig {
    sigint_ms: u64,
    sigterm_ms: u64,
}

#[derive(Debug, Clone, Copy)]
struct ShutdownState {
    stage: ShutdownStage,
    deadline: tokio::time::Instant,
}

#[derive(Debug, Clone, Copy)]
enum ShutdownStage {
    SigInt,
    SigTerm,
    Kill,
}

impl ShutdownConfig {
    pub fn new(sigint_ms: u64, sigterm_ms: u64) -> Self {
        Self {
            sigint_ms,
            sigterm_ms,
        }
    }

    fn sigint_timeout(&self) -> Duration {
        Duration::from_millis(self.sigint_ms)
    }

    fn sigterm_timeout(&self) -> Duration {
        Duration::from_millis(self.sigterm_ms)
    }

    fn sigint_enabled(&self) -> bool {
        self.sigint_ms > 0
    }

    fn sigterm_enabled(&self) -> bool {
        self.sigterm_ms > 0
    }
}

impl ProcessManager {
    /// Creates a new `ProcessManager` with the given process specifications.
    pub fn new(
        specs: Vec<ProcessSpec>,
        event_tx: mpsc::Sender<Event>,
        shutdown: ShutdownConfig,
    ) -> Self {
        let processes = specs
            .into_iter()
            .map(|spec| ManagedProcess {
                spec,
                child: None,
                stdin: None,
                started: false,
                ready: false,
                waiting_on: Vec::new(),
                shutdown: None,
            })
            .collect();
        Self {
            processes,
            event_tx,
            shutdown,
        }
    }

    /// Starts all configured processes, respecting dependencies.
    pub async fn start_all(&mut self) -> Result<()> {
        self.update_scheduler().await
    }

    /// Checks dependencies and starts pending processes.
    pub async fn update_scheduler(&mut self) -> Result<()> {
        // Simple loop to resolve chains of "immediate" readiness
        let mut changed = true;
        while changed {
            changed = false;
            // Snapshot current state to avoid borrowing issues
            let states: Vec<(String, bool)> = self
                .processes
                .iter()
                .map(|p| (p.spec.name.clone(), p.ready))
                .collect();

            for idx in 0..self.processes.len() {
                if self.processes[idx].started {
                    continue;
                }

                let depends_on = self.processes[idx].spec.depends_on.clone();
                let missing: Vec<String> = depends_on
                    .iter()
                    .filter(|dep| !states.iter().any(|(name, ready)| name == *dep && *ready))
                    .cloned()
                    .collect();

                if missing.is_empty() {
                    if !self.processes[idx].waiting_on.is_empty() {
                        self.processes[idx].waiting_on.clear();
                    }
                    self.start_process(idx).await?;
                    changed = true;
                } else if self.processes[idx].waiting_on != missing {
                    self.processes[idx].waiting_on = missing.clone();
                    let _ = self
                        .event_tx
                        .send(Event::ProcessWaiting {
                            id: idx,
                            deps: missing,
                        })
                        .await;
                }
            }
        }
        Ok(())
    }

    /// Marks a process as ready and updates the scheduler.
    pub async fn mark_ready(&mut self, id: usize) -> Result<()> {
        if let Some(proc) = self.processes.get_mut(id) {
            proc.ready = true;
        }
        self.update_scheduler().await
    }

    /// Starts a specific process by ID.
    ///
    /// This handles running the pre-command (if any) and then spawning the main process.
    /// Standard output and error streams are captured and forwarded to the event channel.
    pub async fn start_process(&mut self, id: usize) -> Result<()> {
        let Some(spec) = self.processes.get(id).map(|p| p.spec.clone()) else {
            return Ok(());
        };

        self.processes[id].started = true;
        self.processes[id].waiting_on.clear();
        let _ = self.event_tx.send(Event::ProcessStarting { id }).await;

        if !self.run_pre_cmd(id, &spec).await? {
            return Ok(());
        }

        let mut command = Command::new(&spec.cmd);
        command.args(&spec.args);
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        if !spec.env.is_empty() {
            command.envs(&spec.env);
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.kill_on_drop(true);

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
            command.creation_flags(CREATE_NEW_PROCESS_GROUP);
        }

        #[cfg(unix)]
        unsafe {
            command.pre_exec(|| {
                let _ = libc::setpgid(0, 0);
                Ok(())
            });
        }

        let mut child = command
            .spawn()
            .with_context(|| format!("failed to spawn {}", spec.name))?;
        let pid = child.id().unwrap_or(0);
        let _ = self.event_tx.send(Event::ProcessStarted { id, pid }).await;

        if let Some(stdin) = child.stdin.take() {
            if let Some(process) = self.processes.get_mut(id) {
                process.stdin = Some(stdin);
            }
        }

        // Determine output capture regex for readiness
        let log_ready_regex = if let Some(ReadinessCheck::Log(pattern)) = &spec.ready_check {
            Regex::new(pattern).ok()
        } else {
            None
        };

        if let Some(stdout) = child.stdout.take() {
            let tx = self.event_tx.clone();
            let regex = log_ready_regex.clone();
            tokio::spawn(read_stream(id, StreamKind::Stdout, stdout, tx, regex));
        }
        if let Some(stderr) = child.stderr.take() {
            let tx = self.event_tx.clone();
            let regex = log_ready_regex; // move last clone
            tokio::spawn(read_stream(id, StreamKind::Stderr, stderr, tx, regex));
        }

        if let Some(process) = self.processes.get_mut(id) {
            process.child = Some(child);
        }

        // Handle readiness checks
        match &spec.ready_check {
            Some(ReadinessCheck::Tcp(port)) => {
                let tx = self.event_tx.clone();
                let port = *port;
                tokio::spawn(async move {
                    check_tcp_readiness(id, port, tx).await;
                });
            }
            Some(ReadinessCheck::Delay(ms)) => {
                let tx = self.event_tx.clone();
                let ms = *ms;
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                    let _ = tx.send(Event::ProcessReady { id }).await;
                });
            }
            Some(ReadinessCheck::Log(_)) => {
                // Handled in read_stream
            }
            None => {
                // Immediate readiness
                if let Some(proc) = self.processes.get_mut(id) {
                    proc.ready = true;
                }
                let _ = self.event_tx.send(Event::ProcessReady { id }).await;
            }
        }

        Ok(())
    }

    // Run an optional pre-command before spawning the main process.
    async fn run_pre_cmd(&self, id: usize, spec: &ProcessSpec) -> Result<bool> {
        let Some(pre_cmd) = &spec.pre_cmd else {
            return Ok(true);
        };
        let mut parts = shell_words::split(pre_cmd)
            .with_context(|| format!("failed to parse pre_cmd for {}", spec.name))?;
        if parts.is_empty() {
            return Ok(true);
        }
        let cmd = parts.remove(0);
        let mut command = Command::new(cmd);
        command.args(parts);
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        if !spec.env.is_empty() {
            command.envs(&spec.env);
        }
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(err) => {
                let _ = self
                    .event_tx
                    .send(Event::ProcessFailed {
                        id,
                        error: format!("pre_cmd failed: {}", err),
                    })
                    .await;
                return Ok(false);
            }
        };
        if let Some(stdout) = child.stdout.take() {
            let tx = self.event_tx.clone();
            tokio::spawn(read_stream_with_prefix(
                id,
                StreamKind::Stdout,
                "[pre] ",
                stdout,
                tx,
            ));
        }
        if let Some(stderr) = child.stderr.take() {
            let tx = self.event_tx.clone();
            tokio::spawn(read_stream_with_prefix(
                id,
                StreamKind::Stderr,
                "[pre] ",
                stderr,
                tx,
            ));
        }
        let status = child.wait().await?;
        if !status.success() {
            let _ = self
                .event_tx
                .send(Event::ProcessFailed {
                    id,
                    error: format!("pre_cmd exited {}", status.code().unwrap_or(1)),
                })
                .await;
            return Ok(false);
        }
        Ok(true)
    }

    pub async fn restart_process(&mut self, id: usize) -> Result<()> {
        self.stop_process(id, true).await?;
        // Reset state for restart
        if let Some(p) = self.processes.get_mut(id) {
            p.started = false;
            p.ready = false;
        }
        // Use update_scheduler to respect dependencies again?
        // Or force restart? Typically restart implies force, but if dependencies are dead?
        // For now, force start, assuming dependencies are still "conceptually" there.
        // But better to use update_scheduler if we want to be strict.
        // However, user pressed restart, they probably want it NOW.
        // Let's force start logic by calling start_process directly.
        // But we should check dependencies first?
        // If we just call start_process, it works.
        self.start_process(id).await?;
        Ok(())
    }

    pub async fn send_input_text(&mut self, id: usize, text: String) -> Result<()> {
        self.send_input_bytes(id, text.as_bytes()).await?;
        self.send_input_bytes(id, b"\n").await?;
        Ok(())
    }

    pub async fn send_input_bytes(&mut self, id: usize, bytes: &[u8]) -> Result<()> {
        let Some(process) = self.processes.get_mut(id) else {
            return Ok(());
        };
        let Some(stdin) = process.stdin.as_mut() else {
            return Ok(());
        };
        if bytes.is_empty() {
            return Ok(());
        }
        stdin.write_all(bytes).await?;
        stdin.flush().await?;
        Ok(())
    }

    pub async fn send_input_bytes_to_all(&mut self, bytes: &[u8]) -> Result<()> {
        for idx in 0..self.processes.len() {
            let _ = self.send_input_bytes(idx, bytes).await;
        }
        Ok(())
    }

    pub async fn begin_shutdown_process(&mut self, id: usize, signal: ProcessSignal) {
        self.begin_shutdown(id, signal).await;
    }

    pub async fn begin_shutdown_all(&mut self, signal: ProcessSignal) {
        for idx in 0..self.processes.len() {
            self.begin_shutdown(idx, signal).await;
        }
    }

    pub async fn shutdown_all(&mut self) {
        for idx in 0..self.processes.len() {
            let _ = self.stop_process(idx, true).await;
        }
    }

    pub async fn poll_exits(&mut self) {
        for (id, process) in self.processes.iter_mut().enumerate() {
            if let Some(child) = process.child.as_mut() {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let code = status.code();
                        let _ = self.event_tx.send(Event::ProcessExited { id, code }).await;
                        process.child = None;
                        process.ready = false; // It exited, so it's not ready
                        process.shutdown = None;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        let _ = self
                            .event_tx
                            .send(Event::ProcessFailed {
                                id,
                                error: err.to_string(),
                            })
                            .await;
                        process.child = None;
                        process.ready = false;
                        process.shutdown = None;
                    }
                }
            }
        }
        self.poll_shutdowns().await;
    }

    async fn begin_shutdown(&mut self, id: usize, initial: ProcessSignal) {
        let now = tokio::time::Instant::now();
        let shutdown = self.shutdown;
        let (stage, signal, deadline) = Self::initial_shutdown_stage(shutdown, initial, now);
        let (pid, signal) = {
            let Some(process) = self.processes.get_mut(id) else {
                return;
            };
            if process.child.is_none() || process.shutdown.is_some() {
                return;
            }
            let pid = process.child.as_ref().and_then(|c| c.id());
            process.shutdown = Some(ShutdownState { stage, deadline });
            (pid, signal)
        };

        if let (Some(pid), Some(signal)) = (pid, signal) {
            self.send_signal(id, pid, signal).await;
        }
    }

    fn initial_shutdown_stage(
        shutdown: ShutdownConfig,
        initial: ProcessSignal,
        now: tokio::time::Instant,
    ) -> (ShutdownStage, Option<ProcessSignal>, tokio::time::Instant) {
        match initial {
            ProcessSignal::SigInt => {
                if shutdown.sigint_enabled() {
                    return (
                        ShutdownStage::SigInt,
                        Some(ProcessSignal::SigInt),
                        now + shutdown.sigint_timeout(),
                    );
                }
                if shutdown.sigterm_enabled() {
                    return (
                        ShutdownStage::SigTerm,
                        Some(ProcessSignal::SigTerm),
                        now + shutdown.sigterm_timeout(),
                    );
                }
            }
            ProcessSignal::SigTerm => {
                if shutdown.sigterm_enabled() {
                    return (
                        ShutdownStage::SigTerm,
                        Some(ProcessSignal::SigTerm),
                        now + shutdown.sigterm_timeout(),
                    );
                }
                if shutdown.sigint_enabled() {
                    return (
                        ShutdownStage::SigInt,
                        Some(ProcessSignal::SigInt),
                        now + shutdown.sigint_timeout(),
                    );
                }
            }
        }
        (ShutdownStage::Kill, None, now)
    }

    async fn poll_shutdowns(&mut self) {
        let now = tokio::time::Instant::now();
        for id in 0..self.processes.len() {
            let mut send_signal = None;
            let mut kill_child = None;
            {
                let process = &mut self.processes[id];
                if process.child.is_none() {
                    process.shutdown = None;
                    continue;
                }
                let Some(state) = process.shutdown else {
                    continue;
                };
                if now < state.deadline {
                    continue;
                }

                match state.stage {
                    ShutdownStage::SigInt => {
                        if self.shutdown.sigterm_enabled() {
                            let pid = process.child.as_ref().and_then(|c| c.id());
                            let deadline = now + self.shutdown.sigterm_timeout();
                            process.shutdown = Some(ShutdownState {
                                stage: ShutdownStage::SigTerm,
                                deadline,
                            });
                            if let Some(pid) = pid {
                                send_signal = Some((pid, ProcessSignal::SigTerm));
                            }
                        } else {
                            process.ready = false;
                            kill_child = process.child.take();
                            process.shutdown = None;
                        }
                    }
                    ShutdownStage::SigTerm => {
                        process.ready = false;
                        kill_child = process.child.take();
                        process.shutdown = None;
                    }
                    ShutdownStage::Kill => {
                        process.ready = false;
                        kill_child = process.child.take();
                        process.shutdown = None;
                    }
                }
            }

            if let Some((pid, signal)) = send_signal {
                self.send_signal(id, pid, signal).await;
            }

            if let Some(mut child) = kill_child {
                let _ = child.kill().await;
                match wait_for_exit(&mut child, Duration::from_millis(500)).await {
                    Ok(Some(status)) => {
                        let _ = self
                            .event_tx
                            .send(Event::ProcessExited { id, code: status.code() })
                            .await;
                    }
                    Ok(None) => match child.wait().await {
                        Ok(status) => {
                            let _ = self
                                .event_tx
                                .send(Event::ProcessExited { id, code: status.code() })
                                .await;
                        }
                        Err(err) => {
                            let _ = self
                                .event_tx
                                .send(Event::ProcessFailed { id, error: err.to_string() })
                                .await;
                        }
                    },
                    Err(err) => {
                        let _ = self
                            .event_tx
                            .send(Event::ProcessFailed { id, error: err.to_string() })
                            .await;
                    }
                }
            }
        }
    }

    async fn stop_process(&mut self, id: usize, graceful: bool) -> Result<()> {
        if let Some(process) = self.processes.get_mut(id) {
            process.ready = false; // Mark not ready immediately
            process.shutdown = None;
            if let Some(mut child) = process.child.take() {
                process.stdin = None;
                if graceful {
                    if self.shutdown.sigint_enabled() {
                        if let Some(pid) = child.id() {
                            self.send_signal(id, pid, ProcessSignal::SigInt).await;
                        }
                        match wait_for_exit(&mut child, self.shutdown.sigint_timeout()).await {
                            Ok(Some(status)) => {
                                let _ = self
                                    .event_tx
                                    .send(Event::ProcessExited {
                                        id,
                                        code: status.code(),
                                    })
                                    .await;
                                return Ok(());
                            }
                            Ok(None) => {}
                            Err(err) => {
                                let _ = self
                                    .event_tx
                                    .send(Event::ProcessFailed {
                                        id,
                                        error: err.to_string(),
                                    })
                                    .await;
                            }
                        }
                    }

                    if self.shutdown.sigterm_enabled() {
                        if let Some(pid) = child.id() {
                            self.send_signal(id, pid, ProcessSignal::SigTerm).await;
                        }
                        match wait_for_exit(&mut child, self.shutdown.sigterm_timeout()).await {
                            Ok(Some(status)) => {
                                let _ = self
                                    .event_tx
                                    .send(Event::ProcessExited {
                                        id,
                                        code: status.code(),
                                    })
                                    .await;
                                return Ok(());
                            }
                            Ok(None) => {}
                            Err(err) => {
                                let _ = self
                                    .event_tx
                                    .send(Event::ProcessFailed {
                                        id,
                                        error: err.to_string(),
                                    })
                                    .await;
                            }
                        }
                    }
                }
                let _ = child.kill().await;
                match child.wait().await {
                    Ok(status) => {
                        let _ = self
                            .event_tx
                            .send(Event::ProcessExited {
                                id,
                                code: status.code(),
                            })
                            .await;
                    }
                    Err(err) => {
                        let _ = self
                            .event_tx
                            .send(Event::ProcessFailed {
                                id,
                                error: err.to_string(),
                            })
                            .await;
                    }
                }
            }
        }
        Ok(())
    }

    async fn send_signal(&self, id: usize, pid: u32, signal: ProcessSignal) {
        let _ = self
            .event_tx
            .send(Event::ProcessSignal { id, signal })
            .await;
        send_os_signal(pid, signal);
    }
}

#[cfg(unix)]
fn send_os_signal(pid: u32, signal: ProcessSignal) {
    unsafe {
        let sig = match signal {
            ProcessSignal::SigInt => libc::SIGINT,
            ProcessSignal::SigTerm => libc::SIGTERM,
        };
        let pid = pid as i32;
        let _ = libc::kill(-pid, sig);
        let _ = libc::kill(pid, sig);
    }
}

#[cfg(not(unix))]
fn send_os_signal(pid: u32, signal: ProcessSignal) {
    send_ctrl_break(pid, signal);
}

#[cfg(all(not(unix), windows))]
fn send_ctrl_break(pid: u32, signal: ProcessSignal) {
    use windows_sys::Win32::System::Console::GenerateConsoleCtrlEvent;
    use windows_sys::Win32::System::Console::CTRL_BREAK_EVENT;
    // Windows has no SIGTERM/SIGINT; CTRL_BREAK is the closest console signal we can emit.
    let _ = signal;
    unsafe {
        let _ = GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid);
    }
}

#[cfg(all(not(unix), not(windows)))]
fn send_ctrl_break(_pid: u32, _signal: ProcessSignal) {}

async fn wait_for_exit(
    child: &mut tokio::process::Child,
    timeout: Duration,
) -> Result<Option<std::process::ExitStatus>> {
    if timeout.is_zero() {
        return Ok(None);
    }
    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => Ok(Some(status)),
        Ok(Err(err)) => Err(err.into()),
        Err(_) => Ok(None),
    }
}

async fn read_stream<R>(
    id: usize,
    stream: StreamKind,
    reader: R,
    tx: mpsc::Sender<Event>,
    readiness_regex: Option<Regex>,
) where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    let mut matched = false;
    while let Ok(Some(line)) = lines.next_line().await {
        if !matched {
            if let Some(regex) = &readiness_regex {
                if regex.is_match(&line) {
                    let _ = tx.send(Event::ProcessReady { id }).await;
                    matched = true;
                }
            }
        }
        let _ = tx.send(Event::ProcessOutput { id, line, stream }).await;
    }
}

// Prefix pre-command output so it is visible in logs and non-TUI mode.
async fn read_stream_with_prefix<R>(
    id: usize,
    stream: StreamKind,
    prefix: &str,
    reader: R,
    tx: mpsc::Sender<Event>,
) where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let _ = tx
            .send(Event::ProcessOutput {
                id,
                line: format!("{}{}", prefix, line),
                stream,
            })
            .await;
    }
}

async fn check_tcp_readiness(id: usize, port: u16, tx: mpsc::Sender<Event>) {
    let addr = format!("127.0.0.1:{}", port);
    // Try for up to 60 seconds
    let end = tokio::time::Instant::now() + Duration::from_secs(60);
    while tokio::time::Instant::now() < end {
        if TcpStream::connect(&addr).await.is_ok() {
            let _ = tx.send(Event::ProcessReady { id }).await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    // Timeout? We could send Failed, but for now just don't send Ready.
}
