use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crossterm::event::{Event, EventStream as CrosstermEventStream};
use futures::StreamExt;
use tokio::sync::mpsc;

use yrr_runtime::events::SwarmEvent;

/// Unified event type for the TUI application.
pub enum AppEvent {
    /// Terminal input event (key press, resize, etc.)
    Terminal(Event),
    /// Swarm runtime event (signal flow, agent lifecycle, etc.)
    Swarm(SwarmEvent),
    /// Periodic tick for animation and time display updates.
    Tick,
    /// One or more watched files changed on disk.
    FileChanged,
}

/// Merges terminal events, swarm events, ticks, and file-change notifications.
pub struct EventStream {
    swarm_rx: Option<mpsc::UnboundedReceiver<SwarmEvent>>,
    crossterm_stream: CrosstermEventStream,
    tick_interval: tokio::time::Interval,
    file_watcher: Option<FileWatcher>,
}

impl EventStream {
    /// Create a stream in preview mode — no swarm events yet.
    pub fn new_preview(watch_paths: Vec<PathBuf>) -> Self {
        let watcher = if watch_paths.is_empty() {
            None
        } else {
            Some(FileWatcher::new(watch_paths))
        };
        Self {
            swarm_rx: None,
            crossterm_stream: CrosstermEventStream::new(),
            tick_interval: tokio::time::interval(Duration::from_millis(100)),
            file_watcher: watcher,
        }
    }

    /// Attach a swarm event receiver (when the swarm starts running).
    pub fn attach_swarm_rx(&mut self, rx: mpsc::UnboundedReceiver<SwarmEvent>) {
        self.swarm_rx = Some(rx);
    }

    /// Wait for the next event from any source.
    pub async fn next(&mut self) -> AppEvent {
        // Build the file-check future.
        let file_check = async {
            if let Some(watcher) = &mut self.file_watcher {
                watcher.wait_for_change().await
            } else {
                // Never resolves.
                std::future::pending::<()>().await
            }
        };

        // If we have a swarm receiver, include it in the select.
        if let Some(rx) = &mut self.swarm_rx {
            tokio::select! {
                Some(Ok(evt)) = self.crossterm_stream.next() => {
                    AppEvent::Terminal(evt)
                }
                Some(evt) = rx.recv() => {
                    AppEvent::Swarm(evt)
                }
                _ = self.tick_interval.tick() => {
                    AppEvent::Tick
                }
                _ = file_check => {
                    AppEvent::FileChanged
                }
            }
        } else {
            tokio::select! {
                Some(Ok(evt)) = self.crossterm_stream.next() => {
                    AppEvent::Terminal(evt)
                }
                _ = self.tick_interval.tick() => {
                    AppEvent::Tick
                }
                _ = file_check => {
                    AppEvent::FileChanged
                }
            }
        }
    }
}

/// Poll-based file watcher — checks modification times every 2 seconds.
struct FileWatcher {
    paths: Vec<PathBuf>,
    mtimes: HashMap<PathBuf, SystemTime>,
    interval: tokio::time::Interval,
}

impl FileWatcher {
    fn new(paths: Vec<PathBuf>) -> Self {
        let mut mtimes = HashMap::new();
        for path in &paths {
            if let Ok(meta) = std::fs::metadata(path) {
                if let Ok(mtime) = meta.modified() {
                    mtimes.insert(path.clone(), mtime);
                }
            }
        }
        Self {
            paths,
            mtimes,
            interval: tokio::time::interval(Duration::from_secs(2)),
        }
    }

    /// Wait until at least one file has a newer mtime.
    async fn wait_for_change(&mut self) {
        loop {
            self.interval.tick().await;
            for path in &self.paths {
                if let Ok(meta) = std::fs::metadata(path) {
                    if let Ok(mtime) = meta.modified() {
                        let changed = match self.mtimes.get(path) {
                            Some(&prev) => mtime > prev,
                            None => true,
                        };
                        if changed {
                            // Update all mtimes and return.
                            for p in &self.paths {
                                if let Ok(m) = std::fs::metadata(p) {
                                    if let Ok(t) = m.modified() {
                                        self.mtimes.insert(p.clone(), t);
                                    }
                                }
                            }
                            return;
                        }
                    }
                }
            }
        }
    }
}
