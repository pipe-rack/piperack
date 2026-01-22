//! File watching and auto-restart functionality.
//!
//! This module handles spawning threads to watch for file changes in specified directories
//! and triggering process restarts when relevant changes occur. It supports debouncing
//! and ignoring files based on glob patterns and `.gitignore`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::{Event as NotifyEvent, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::events::Event;
use crate::process::ProcessState;

/// Spawns watcher threads for all processes that have `watch` configurations.
///
/// For each process with configured watch paths, a background thread is started
/// that monitors the file system. When a change is detected (and confirmed relevant),
/// an `Event::Restart` is sent to the main event loop.
pub fn spawn_watchers(processes: &[ProcessState], tx: mpsc::Sender<Event>) {
    for (id, process) in processes.iter().enumerate() {
        if process.spec.watch_paths.is_empty() {
            continue;
        }
        let spec = process.spec.clone();
        let tx = tx.clone();
        std::thread::spawn(move || {
            if let Err(err) = watch_process(id, &spec, tx) {
                eprintln!("watcher for {} failed: {}", spec.name, err);
            }
        });
    }
}

fn watch_process(
    id: usize,
    spec: &crate::process::ProcessSpec,
    tx: mpsc::Sender<Event>,
) -> Result<()> {
    let base = spec
        .cwd
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().context("failed to resolve current dir")?);
    let watch_paths = resolve_watch_paths(&base, &spec.watch_paths);
    let matcher = IgnoreMatcher::new(&base, &spec.watch_ignore, spec.watch_ignore_gitignore)?;

    let (raw_tx, raw_rx) = std::sync::mpsc::channel();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = raw_tx.send(res);
        },
        notify::Config::default(),
    )
    .context("failed to create watcher")?;

    for path in &watch_paths {
        watcher
            .watch(path, RecursiveMode::Recursive)
            .with_context(|| format!("failed to watch {}", path.display()))?;
    }

    let debounce = Duration::from_millis(spec.watch_debounce_ms.max(50));
    loop {
        let event = match raw_rx.recv() {
            Ok(res) => res,
            Err(_) => break,
        };
        if !is_relevant(&event, &matcher) {
            continue;
        }

        let mut last = Instant::now();
        loop {
            let elapsed = last.elapsed();
            if elapsed >= debounce {
                break;
            }
            match raw_rx.recv_timeout(debounce - elapsed) {
                Ok(res) => {
                    if is_relevant(&res, &matcher) {
                        last = Instant::now();
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }

        let _ = tx.blocking_send(Event::Restart { id });
    }

    Ok(())
}

fn resolve_watch_paths(base: &Path, paths: &[String]) -> Vec<PathBuf> {
    paths
        .iter()
        .map(|path| {
            let path_buf = PathBuf::from(path);
            if path_buf.is_absolute() {
                path_buf
            } else {
                base.join(path_buf)
            }
        })
        .collect()
}

fn is_relevant(event: &notify::Result<NotifyEvent>, matcher: &IgnoreMatcher) -> bool {
    let Ok(event) = event else {
        return true;
    };
    if event.paths.is_empty() {
        return true;
    }
    for path in &event.paths {
        if !matcher.is_ignored(path) {
            return true;
        }
    }
    false
}

struct IgnoreMatcher {
    // Combines explicit ignore globs with optional gitignore rules.
    base: PathBuf,
    globset: Option<GlobSet>,
    gitignore: Option<Gitignore>,
}

impl IgnoreMatcher {
    fn new(base: &Path, patterns: &[String], ignore_gitignore: bool) -> Result<Self> {
        let globset = if patterns.is_empty() {
            None
        } else {
            let mut builder = GlobSetBuilder::new();
            for pattern in patterns {
                for expanded in expand_pattern(pattern) {
                    builder.add(Glob::new(&expanded)?);
                }
            }
            Some(builder.build()?)
        };

        let gitignore = if ignore_gitignore {
            None
        } else {
            Some(build_gitignore(base)?)
        };

        Ok(Self {
            base: base.to_path_buf(),
            globset,
            gitignore,
        })
    }

    fn is_ignored(&self, path: &Path) -> bool {
        if let Some(globset) = &self.globset {
            if globset.is_match(path) {
                return true;
            }
            if let Ok(relative) = path.strip_prefix(&self.base) {
                if globset.is_match(relative) {
                    return true;
                }
            }
        }
        if let Some(gitignore) = &self.gitignore {
            let is_dir = path.is_dir();
            if gitignore.matched(path, is_dir).is_ignore() {
                return true;
            }
        }
        false
    }
}

fn expand_pattern(pattern: &str) -> Vec<String> {
    let trimmed = pattern.trim_end_matches('/');
    let has_glob = pattern.contains('*') || pattern.contains('?') || pattern.contains('[');
    if has_glob {
        vec![pattern.to_string()]
    } else {
        vec![trimmed.to_string(), format!("{}/**", trimmed)]
    }
}

fn build_gitignore(base: &Path) -> Result<Gitignore> {
    let mut builder = GitignoreBuilder::new(base);
    for ancestor in base.ancestors() {
        let path = ancestor.join(".gitignore");
        if path.exists() {
            builder.add(path);
        }
        let exclude = ancestor.join(".git").join("info").join("exclude");
        if exclude.exists() {
            builder.add(exclude);
        }
    }
    Ok(builder.build()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_watch_paths_handles_absolute_and_relative() {
        let base = Path::new("/tmp/piperack-tests");
        let paths = vec!["src".to_string(), "/var/log".to_string()];
        let resolved = resolve_watch_paths(base, &paths);
        assert_eq!(resolved[0], base.join("src"));
        assert_eq!(resolved[1], PathBuf::from("/var/log"));
    }

    #[test]
    fn expand_pattern_adds_recursive_glob_for_dirs() {
        let patterns = expand_pattern("src");
        assert_eq!(patterns, vec!["src".to_string(), "src/**".to_string()]);

        let trimmed = expand_pattern("src/");
        assert_eq!(trimmed, vec!["src".to_string(), "src/**".to_string()]);

        let globbed = expand_pattern("*.rs");
        assert_eq!(globbed, vec!["*.rs".to_string()]);
    }

    #[test]
    fn ignore_matcher_respects_globs() {
        let base = Path::new("/tmp/piperack-tests");
        let matcher = IgnoreMatcher::new(base, &vec!["target".to_string()], true).unwrap();
        assert!(matcher.is_ignored(&base.join("target")));
        assert!(matcher.is_ignored(&PathBuf::from("target")));
        assert!(!matcher.is_ignored(&base.join("src")));
    }
}
