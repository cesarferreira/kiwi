use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Error, Result};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::config::{CompiledConfig, Config};

const DEBOUNCE: Duration = Duration::from_millis(100);

pub fn watch_config(path: &Path) -> Result<Receiver<CompiledConfig>> {
    let logical_path = absolute_path(path)?;
    let resolved_path = fs::canonicalize(&logical_path)
        .with_context(|| format!("could not resolve config {}", logical_path.display()))?;
    let logical_parent = logical_path
        .parent()
        .context("config path has no parent directory")?
        .to_path_buf();
    let resolved_parent = resolved_path
        .parent()
        .context("resolved config path has no parent directory")?
        .to_path_buf();

    let (event_sender, event_receiver) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = event_sender.send(event);
    })
    .context("could not create config watcher")?;
    watcher
        .watch(&logical_parent, RecursiveMode::NonRecursive)
        .with_context(|| format!("could not watch {}", logical_parent.display()))?;
    if resolved_parent != logical_parent {
        watcher
            .watch(&resolved_parent, RecursiveMode::NonRecursive)
            .with_context(|| format!("could not watch {}", resolved_parent.display()))?;
    }

    let (config_sender, config_receiver) = mpsc::channel();
    thread::Builder::new()
        .name("kiwi-config-watcher".into())
        .spawn(move || {
            watch_loop(
                watcher,
                event_receiver,
                config_sender,
                logical_path,
                resolved_path,
            )
        })
        .context("could not start config watcher")?;

    Ok(config_receiver)
}

fn watch_loop(
    _watcher: RecommendedWatcher,
    event_receiver: Receiver<notify::Result<Event>>,
    config_sender: mpsc::Sender<CompiledConfig>,
    logical_path: PathBuf,
    resolved_path: PathBuf,
) {
    loop {
        let first_event = match event_receiver.recv() {
            Ok(event) => event,
            Err(_) => return,
        };
        if !relevant_event(first_event, &logical_path, &resolved_path) {
            continue;
        }

        let mut deadline = Instant::now() + DEBOUNCE;
        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match event_receiver.recv_timeout(deadline - now) {
                Ok(event) => {
                    if relevant_event(event, &logical_path, &resolved_path) {
                        deadline = Instant::now() + DEBOUNCE;
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }

        match Config::from_path(&logical_path).and_then(Config::compile) {
            Ok(config) => {
                if config_sender.send(config).is_err() {
                    return;
                }
            }
            Err(error) => {
                eprintln!(
                    "reload failed: {}; keeping previous config",
                    reload_error(&error)
                );
            }
        }
    }
}

fn reload_error(error: &Error) -> String {
    let summary = error.to_string();
    let root = error.root_cause().to_string();
    let mut lines = root.lines().map(str::trim).filter(|line| !line.is_empty());
    let first = lines.next().unwrap_or("unknown config error");
    let last = lines.next_back().unwrap_or(first);
    let detail = if first == last {
        first.to_owned()
    } else {
        format!("{first}: {last}")
    };

    if summary.contains('\n') || summary == first || summary == detail {
        detail
    } else {
        format!("{summary}: {detail}")
    }
}

fn relevant_event(event: notify::Result<Event>, logical_path: &Path, resolved_path: &Path) -> bool {
    match event {
        Ok(event) => event.paths.iter().any(|path| {
            path == logical_path
                || path == resolved_path
                || fs::canonicalize(path).is_ok_and(|path| path == resolved_path)
        }),
        Err(error) => {
            eprintln!("config watch error: {error}");
            false
        }
    }
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .context("could not resolve current directory")?
            .join(path))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::symlink,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };

    use super::{reload_error, watch_config};

    const INITIAL: &str = r#"
        [bindings]
        "hyper+a" = { app = "Initial" }
    "#;
    const REPLACEMENT: &str = r#"
        [bindings]
        "hyper+b" = { app = "Replacement" }
    "#;

    static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let suffix = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("kiwi-reload-test-{}-{suffix}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn write_initial(path: &Path) {
        fs::write(path, INITIAL).unwrap();
    }

    #[test]
    fn reloads_after_an_atomic_config_replacement() {
        let temp = TempDir::new();
        let config_path = temp.path().join("config.toml");
        let replacement_path = temp.path().join("replacement.toml");
        write_initial(&config_path);
        let receiver = watch_config(&config_path).unwrap();

        fs::write(&replacement_path, REPLACEMENT).unwrap();
        fs::rename(&replacement_path, &config_path).unwrap();

        let config = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(config.bindings.contains_key("hyper+b"));
    }

    #[test]
    fn invalid_edit_keeps_previous_config_and_a_later_valid_edit_recovers() {
        let temp = TempDir::new();
        let config_path = temp.path().join("config.toml");
        write_initial(&config_path);
        let receiver = watch_config(&config_path).unwrap();

        fs::write(&config_path, "not valid toml = [").unwrap();
        assert!(receiver.recv_timeout(Duration::from_millis(300)).is_err());

        fs::write(&config_path, REPLACEMENT).unwrap();
        let config = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(config.bindings.contains_key("hyper+b"));
    }

    #[test]
    fn coalesces_a_burst_of_edits_into_one_reload() {
        let temp = TempDir::new();
        let config_path = temp.path().join("config.toml");
        write_initial(&config_path);
        let receiver = watch_config(&config_path).unwrap();

        for contents in [INITIAL, REPLACEMENT, REPLACEMENT] {
            fs::write(&config_path, contents).unwrap();
        }

        let config = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(config.bindings.contains_key("hyper+b"));
        assert!(receiver.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn reloads_when_a_symlink_target_changes() {
        let temp = TempDir::new();
        let logical_dir = temp.path().join("logical");
        let target_dir = temp.path().join("dotfiles");
        fs::create_dir(&logical_dir).unwrap();
        fs::create_dir(&target_dir).unwrap();
        let target_path = target_dir.join("kiwi.toml");
        let logical_path = logical_dir.join("config.toml");
        write_initial(&target_path);
        symlink(&target_path, &logical_path).unwrap();
        let receiver = watch_config(&logical_path).unwrap();

        fs::write(&target_path, REPLACEMENT).unwrap();

        let config = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(config.bindings.contains_key("hyper+b"));
    }

    #[test]
    fn reload_error_is_condensed_to_one_informative_line() {
        let error = anyhow::anyhow!("parse failed\n| source excerpt\nspecific reason")
            .context("invalid config");

        assert_eq!(
            reload_error(&error),
            "invalid config: parse failed: specific reason"
        );
    }
}
