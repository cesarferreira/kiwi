use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use kiwi_keymapper::{
    DEFAULT_CONFIG,
    config::{Action, Config},
    macos::{
        LABEL, accessibility_is_trusted, launch_agent_plist, listen_event_tap, remove_caps_remap,
        run_event_tap,
    },
};

#[derive(Parser, Debug)]
#[command(name = "kiwi", version, about = "Run portable macOS key mappings")]
struct Cli {
    /// Use a config file other than ~/.config/kiwi/config.toml
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Create an example config
    Init {
        /// Replace an existing config
        #[arg(long)]
        force: bool,
    },
    /// Validate the config without starting the daemon
    Validate,
    /// Print the configured shortcuts
    List,
    /// Run the keyboard event daemon in the foreground
    Run,
    /// Show shortcuts as they are pressed without running actions
    Listen,
    /// Install and start the per-user LaunchAgent
    Install,
    /// Start an installed LaunchAgent
    Start,
    /// Stop the LaunchAgent without uninstalling it
    Stop,
    /// Stop and remove the per-user LaunchAgent
    Uninstall,
    /// Restart an installed LaunchAgent
    Restart,
    /// Print the installed LaunchAgent status
    Status,
    /// Check config, permission, and installation state
    Doctor,
    /// Open macOS Accessibility privacy settings
    Permissions,
    /// Print the active config path
    ConfigPath,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or(default_config_path()?);

    match cli.command {
        Command::Init { force } => init_config(&config_path, force),
        Command::Validate => {
            load_config(&config_path)?;
            println!("config is valid: {}", config_path.display());
            Ok(())
        }
        Command::List => list_shortcuts(&config_path),
        Command::Run => run_event_tap(&config_path, load_config(&config_path)?),
        Command::Listen => listen_event_tap(
            &config_path,
            load_config(&config_path)?,
            io::stdout().is_terminal(),
        ),
        Command::Install => install(&config_path),
        Command::Start => start(),
        Command::Stop => stop(),
        Command::Uninstall => uninstall(),
        Command::Restart => launchctl(&["kickstart", "-k", &service_target()]),
        Command::Status => status(),
        Command::Doctor => doctor(&config_path),
        Command::Permissions => {
            let status = ProcessCommand::new("/usr/bin/open")
                .arg(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
                )
                .status()
                .context("could not open System Settings")?;
            if !status.success() {
                bail!("open exited with {status}");
            }
            Ok(())
        }
        Command::ConfigPath => {
            println!("{}", config_path.display());
            Ok(())
        }
    }
}

fn load_config(path: &Path) -> Result<kiwi_keymapper::config::CompiledConfig> {
    Config::from_path(path)?.compile()
}

fn list_shortcuts(config_path: &Path) -> Result<()> {
    let config = load_config(config_path)?;
    print!(
        "{}",
        shortcuts_table(&config.bindings, io::stdout().is_terminal())
    );
    Ok(())
}

fn shortcuts_table(bindings: &BTreeMap<String, Action>, color: bool) -> String {
    let rows: Vec<_> = bindings
        .iter()
        .map(|(shortcut, action)| {
            let (kind, value) = match action {
                Action::LaunchApp(value) => ("app", value.clone()),
                Action::OpenUrl(value) => ("url", value.clone()),
                Action::RunCommand(value) => ("command", value.clone()),
                Action::SendKeys(value) => ("keys", value.to_string()),
            };
            (shortcut.as_str(), kind, value)
        })
        .collect();
    let shortcut_width = rows
        .iter()
        .map(|(shortcut, _, _)| shortcut.len())
        .max()
        .unwrap_or(0)
        .max("SHORTCUT".len());
    let kind_width = rows
        .iter()
        .map(|(_, kind, _)| kind.len())
        .max()
        .unwrap_or(0)
        .max("TYPE".len());
    let noun = if rows.len() == 1 {
        "shortcut"
    } else {
        "shortcuts"
    };
    let title = paint(&format!("{} {noun}", rows.len()), "1;32", color);
    let header = paint(
        &format!(
            "{:<shortcut_width$}  {:<kind_width$}  ACTION",
            "SHORTCUT", "TYPE"
        ),
        "1",
        color,
    );
    let mut output = format!("{title}\n\n{header}\n");
    for (shortcut, kind, action) in rows {
        let shortcut = paint(&format!("{shortcut:<shortcut_width$}"), "36", color);
        let kind = paint(&format!("{kind:<kind_width$}"), "33", color);
        let action = paint(&action, "32", color);
        output.push_str(&format!("{shortcut}  {kind}  {action}\n"));
    }
    output
}

fn paint(value: &str, code: &str, color: bool) -> String {
    if color {
        format!("\u{1b}[{code}m{value}\u{1b}[0m")
    } else {
        value.into()
    }
}

fn init_config(path: &Path, force: bool) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    let mut options = OpenOptions::new();
    options.write(true).truncate(force);
    if force {
        options.create(true);
    } else {
        options.create_new(true);
    }
    let mut file = options.open(path).with_context(|| {
        if path.exists() && !force {
            format!(
                "config already exists at {}; use --force to replace it",
                path.display()
            )
        } else {
            format!("could not create config at {}", path.display())
        }
    })?;
    file.write_all(DEFAULT_CONFIG.as_bytes())?;
    println!("created {}", path.display());
    Ok(())
}

fn install(config_path: &Path) -> Result<()> {
    if !config_path.exists() {
        init_config(config_path, false)?;
    }
    load_config(config_path)?;

    let plist_path = launch_agent_path()?;
    let log_path = log_path()?;
    if let Some(parent) = plist_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let binary = std::env::current_exe().context("could not locate the kiwi binary")?;
    let signature_changed = ensure_stable_signature(&binary)?;
    fs::write(
        &plist_path,
        launch_agent_plist(&binary, config_path, &log_path),
    )
    .with_context(|| format!("could not write {}", plist_path.display()))?;

    unload_if_loaded(&LaunchctlManager, &service_target())?;
    launchctl(&[
        "bootstrap",
        &service_domain(),
        &plist_path.to_string_lossy(),
    ])?;
    println!("installed {LABEL}");
    if signature_changed || !accessibility_is_trusted() {
        println!(
            "next: run `kiwi permissions`, remove any old Kiwi entry, add {}, then run `kiwi restart`",
            binary.display()
        );
    }
    Ok(())
}

fn start() -> Result<()> {
    let plist_path = launch_agent_path()?;
    start_service(
        &LaunchctlManager,
        &service_target(),
        &service_domain(),
        &plist_path,
    )?;
    println!("started {LABEL}");
    Ok(())
}

fn stop() -> Result<()> {
    stop_service(&LaunchctlManager, &service_target(), remove_caps_remap)?;
    println!("stopped {LABEL}");
    Ok(())
}

fn uninstall() -> Result<()> {
    let plist_path = launch_agent_path()?;
    unload_if_loaded(&LaunchctlManager, &service_target())?;
    remove_caps_remap()?;
    if plist_path.exists() {
        fs::remove_file(&plist_path)
            .with_context(|| format!("could not remove {}", plist_path.display()))?;
    }
    println!("uninstalled {LABEL}; config was preserved");
    Ok(())
}

fn doctor(config_path: &Path) -> Result<()> {
    let mut healthy = true;
    match load_config(config_path) {
        Ok(config) => println!(
            "[ok] config: {} ({} enabled bindings)",
            config_path.display(),
            config.bindings.len()
        ),
        Err(error) => {
            healthy = false;
            println!("[fail] config: {error:#}");
        }
    }
    let binary = std::env::current_exe().context("could not locate the kiwi binary")?;
    if is_stable_requirement(&code_requirement(&binary)?) {
        println!("[ok] stable code-signing identity");
    } else {
        healthy = false;
        println!("[fail] binary is ad-hoc signed; run `kiwi install`");
    }
    match query_launch_agent_state()? {
        LaunchAgentState::Running { .. } => println!("[ok] LaunchAgent is running"),
        LaunchAgentState::NotRunning => {
            healthy = false;
            println!(
                "[fail] LaunchAgent is installed but not running; check ~/Library/Logs/kiwi.log"
            );
        }
        LaunchAgentState::Missing => {
            println!("[info] LaunchAgent is not installed (foreground mode is available)");
            if accessibility_is_trusted() {
                println!("[ok] foreground Accessibility context");
            } else {
                healthy = false;
                println!("[fail] foreground Accessibility permission is not granted");
            }
        }
    }
    if healthy {
        Ok(())
    } else {
        bail!("doctor found problems")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LaunchAgentState {
    Missing,
    Running { pid: Option<u32> },
    NotRunning,
}

fn status() -> Result<()> {
    let state = query_launch_agent_state()?;
    let elapsed = match state {
        LaunchAgentState::Running { pid: Some(pid) } => process_elapsed_time(pid),
        _ => None,
    };
    println!(
        "{}",
        status_line(state, elapsed.as_deref(), io::stdout().is_terminal())
    );
    Ok(())
}

fn query_launch_agent_state() -> Result<LaunchAgentState> {
    let output = ProcessCommand::new("/bin/launchctl")
        .args(["print", &service_target()])
        .output()
        .context("could not query LaunchAgent state")?;
    if output.status.code() == Some(113) {
        return Ok(unloaded_launch_agent_state(launch_agent_path()?.exists()));
    }
    if !output.status.success() {
        bail!(
            "launchctl print failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(launch_agent_state(&String::from_utf8_lossy(&output.stdout)))
}

fn launch_agent_state(output: &str) -> LaunchAgentState {
    if output.lines().any(|line| line.trim() == "state = running") {
        let pid = output
            .lines()
            .find_map(|line| line.trim().strip_prefix("pid = ")?.parse::<u32>().ok());
        LaunchAgentState::Running { pid }
    } else {
        LaunchAgentState::NotRunning
    }
}

fn unloaded_launch_agent_state(plist_exists: bool) -> LaunchAgentState {
    if plist_exists {
        LaunchAgentState::NotRunning
    } else {
        LaunchAgentState::Missing
    }
}

fn process_elapsed_time(pid: u32) -> Option<String> {
    let output = ProcessCommand::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "etime="])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| format_elapsed_time(&String::from_utf8_lossy(&output.stdout)))
        .flatten()
}

fn format_elapsed_time(elapsed: &str) -> Option<String> {
    let elapsed = elapsed.trim();
    let (days, clock) = match elapsed.split_once('-') {
        Some((days, clock)) => (days.parse::<u64>().ok()?, clock),
        None => (0, elapsed),
    };
    let parts = clock
        .split(':')
        .map(|part| part.parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()?;
    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => (0, *minutes, *seconds),
        [hours, minutes, seconds] => (*hours, *minutes, *seconds),
        _ => return None,
    };
    Some(if days > 0 {
        if hours > 0 {
            format!("{days}d {hours}h")
        } else {
            format!("{days}d")
        }
    } else if hours > 0 {
        if minutes > 0 {
            format!("{hours}h {minutes}m")
        } else {
            format!("{hours}h")
        }
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    })
}

fn status_line(state: LaunchAgentState, elapsed: Option<&str>, color: bool) -> String {
    match state {
        LaunchAgentState::Running { .. } => {
            let running = if color {
                "\u{1b}[32mrunning\u{1b}[0m"
            } else {
                "running"
            };
            elapsed.map_or_else(
                || running.into(),
                |elapsed| format!("{running} for {elapsed}"),
            )
        }
        LaunchAgentState::NotRunning => "stopped — check ~/Library/Logs/kiwi.log".into(),
        LaunchAgentState::Missing => "not installed — run `kiwi install`".into(),
    }
}

fn launchctl(arguments: &[&str]) -> Result<()> {
    let status = ProcessCommand::new("/bin/launchctl")
        .args(arguments)
        .status()
        .context("could not run launchctl")?;
    if !status.success() {
        bail!("launchctl exited with {status}");
    }
    Ok(())
}

fn ensure_stable_signature(binary: &Path) -> Result<bool> {
    if is_stable_requirement(&code_requirement(binary)?) {
        return Ok(false);
    }
    let output = ProcessCommand::new("/usr/bin/security")
        .args(["find-identity", "-p", "codesigning", "-v"])
        .output()
        .context("could not list code-signing identities")?;
    if !output.status.success() {
        bail!(
            "security find-identity failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let identity = select_signing_identity(&String::from_utf8_lossy(&output.stdout)).context(
        "no Apple code-signing identity is available; install an Apple Development or Developer ID certificate",
    )?;
    let output = ProcessCommand::new("/usr/bin/codesign")
        .args([
            "--force",
            "--sign",
            &identity,
            "--identifier",
            LABEL,
            &binary.to_string_lossy(),
        ])
        .output()
        .context("could not sign kiwi")?;
    if !output.status.success() {
        bail!(
            "codesign failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    if !is_stable_requirement(&code_requirement(binary)?) {
        bail!("codesign did not produce a stable designated requirement");
    }
    println!("signed kiwi as `{identity}`");
    Ok(true)
}

fn code_requirement(binary: &Path) -> Result<String> {
    let output = ProcessCommand::new("/usr/bin/codesign")
        .args(["-d", "-r-", &binary.to_string_lossy()])
        .output()
        .context("could not inspect the kiwi signature")?;
    if !output.status.success() {
        bail!(
            "codesign inspection failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(combined_codesign_output(&output.stdout, &output.stderr))
}

fn combined_codesign_output(stdout: &[u8], stderr: &[u8]) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    )
}

fn is_stable_requirement(requirement: &str) -> bool {
    requirement.contains(&format!("identifier \"{LABEL}\""))
        && !requirement.contains("designated => cdhash")
}

fn select_signing_identity(output: &str) -> Option<String> {
    let identities: Vec<_> = output
        .lines()
        .filter_map(|line| {
            let start = line.find('"')? + 1;
            let end = line.rfind('"')?;
            (start < end).then(|| line[start..end].to_owned())
        })
        .collect();
    identities
        .iter()
        .find(|identity| identity.starts_with("Developer ID Application:"))
        .or_else(|| {
            identities
                .iter()
                .find(|identity| identity.starts_with("Apple Development:"))
        })
        .cloned()
}

trait ServiceManager {
    fn is_loaded(&self, target: &str) -> Result<bool>;
    fn bootstrap(&self, domain: &str, plist: &Path) -> Result<()>;
    fn bootout(&self, target: &str) -> Result<()>;
    fn kickstart(&self, target: &str) -> Result<()>;
}

struct LaunchctlManager;

impl ServiceManager for LaunchctlManager {
    fn is_loaded(&self, target: &str) -> Result<bool> {
        let output = ProcessCommand::new("/bin/launchctl")
            .args(["print", target])
            .output()
            .context("could not query launchctl")?;
        if output.status.success() {
            return Ok(true);
        }
        if output.status.code() == Some(113) {
            return Ok(false);
        }
        bail!(
            "launchctl print failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }

    fn bootstrap(&self, domain: &str, plist: &Path) -> Result<()> {
        launchctl(&["bootstrap", domain, &plist.to_string_lossy()])
    }

    fn bootout(&self, target: &str) -> Result<()> {
        launchctl(&["bootout", target])
    }

    fn kickstart(&self, target: &str) -> Result<()> {
        launchctl(&["kickstart", target])
    }
}

fn start_service(
    manager: &impl ServiceManager,
    target: &str,
    domain: &str,
    plist: &Path,
) -> Result<()> {
    if !plist.exists() {
        bail!("Kiwi is not installed; run `kiwi install`");
    }
    if manager.is_loaded(target)? {
        manager.kickstart(target)
    } else {
        manager.bootstrap(domain, plist)
    }
}

fn stop_service(
    manager: &impl ServiceManager,
    target: &str,
    remove_mapping: impl FnOnce() -> Result<()>,
) -> Result<()> {
    unload_if_loaded(manager, target)?;
    remove_mapping()
}

fn unload_if_loaded(manager: &impl ServiceManager, target: &str) -> Result<()> {
    if manager.is_loaded(target)? {
        manager.bootout(target)?;
    }
    Ok(())
}

fn service_target() -> String {
    format!("{}/{LABEL}", service_domain())
}

fn service_domain() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}

fn default_config_path() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("could not find the home directory")?
        .join(".config/kiwi/config.toml"))
}

fn launch_agent_path() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("could not find the home directory")?
        .join(format!("Library/LaunchAgents/{LABEL}.plist")))
}

fn log_path() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("could not find the home directory")?
        .join("Library/Logs/kiwi.log"))
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, path::Path};

    use anyhow::Result;
    use kiwi_keymapper::config::Config;

    use super::{ServiceManager, unload_if_loaded};

    struct FakeServiceManager {
        loaded: bool,
        bootstrap_calls: Cell<usize>,
        bootout_calls: Cell<usize>,
        kickstart_calls: Cell<usize>,
    }

    impl ServiceManager for FakeServiceManager {
        fn is_loaded(&self, _target: &str) -> Result<bool> {
            Ok(self.loaded)
        }

        fn bootstrap(&self, _domain: &str, _plist: &Path) -> Result<()> {
            self.bootstrap_calls.set(self.bootstrap_calls.get() + 1);
            Ok(())
        }

        fn bootout(&self, _target: &str) -> Result<()> {
            self.bootout_calls.set(self.bootout_calls.get() + 1);
            Ok(())
        }

        fn kickstart(&self, _target: &str) -> Result<()> {
            self.kickstart_calls.set(self.kickstart_calls.get() + 1);
            Ok(())
        }
    }

    #[test]
    fn first_install_does_not_bootout_a_missing_service() {
        let manager = FakeServiceManager {
            loaded: false,
            bootstrap_calls: Cell::new(0),
            bootout_calls: Cell::new(0),
            kickstart_calls: Cell::new(0),
        };

        unload_if_loaded(&manager, "gui/501/example").unwrap();

        assert_eq!(manager.bootout_calls.get(), 0);
    }

    #[test]
    fn reinstall_boots_out_the_loaded_service_once() {
        let manager = FakeServiceManager {
            loaded: true,
            bootstrap_calls: Cell::new(0),
            bootout_calls: Cell::new(0),
            kickstart_calls: Cell::new(0),
        };

        unload_if_loaded(&manager, "gui/501/example").unwrap();

        assert_eq!(manager.bootout_calls.get(), 1);
    }

    #[test]
    fn start_bootstraps_an_installed_unloaded_service() {
        let manager = FakeServiceManager {
            loaded: false,
            bootstrap_calls: Cell::new(0),
            bootout_calls: Cell::new(0),
            kickstart_calls: Cell::new(0),
        };
        let plist =
            std::env::temp_dir().join(format!("kiwi-start-test-{}.plist", std::process::id()));
        std::fs::write(&plist, "test").unwrap();

        super::start_service(&manager, "gui/501/example", "gui/501", &plist).unwrap();
        std::fs::remove_file(plist).unwrap();

        assert_eq!(manager.bootstrap_calls.get(), 1);
        assert_eq!(manager.kickstart_calls.get(), 0);
    }

    #[test]
    fn start_kickstarts_an_already_loaded_service() {
        let manager = FakeServiceManager {
            loaded: true,
            bootstrap_calls: Cell::new(0),
            bootout_calls: Cell::new(0),
            kickstart_calls: Cell::new(0),
        };
        let plist = std::env::temp_dir().join(format!(
            "kiwi-start-loaded-test-{}.plist",
            std::process::id()
        ));
        std::fs::write(&plist, "test").unwrap();

        super::start_service(&manager, "gui/501/example", "gui/501", &plist).unwrap();
        std::fs::remove_file(plist).unwrap();

        assert_eq!(manager.bootstrap_calls.get(), 0);
        assert_eq!(manager.kickstart_calls.get(), 1);
    }

    #[test]
    fn stop_unloads_the_service_and_restores_caps_lock() {
        let manager = FakeServiceManager {
            loaded: true,
            bootstrap_calls: Cell::new(0),
            bootout_calls: Cell::new(0),
            kickstart_calls: Cell::new(0),
        };
        let caps_restored = Cell::new(false);

        super::stop_service(&manager, "gui/501/example", || {
            caps_restored.set(true);
            Ok(())
        })
        .unwrap();

        assert_eq!(manager.bootout_calls.get(), 1);
        assert!(caps_restored.get());
    }

    #[test]
    fn an_unloaded_service_is_stopped_when_its_plist_remains() {
        assert_eq!(
            super::unloaded_launch_agent_state(true),
            super::LaunchAgentState::NotRunning
        );
        assert_eq!(
            super::unloaded_launch_agent_state(false),
            super::LaunchAgentState::Missing
        );
    }

    #[test]
    fn selects_a_stable_apple_code_signing_identity() {
        let identities = r#"
          1) ABCDEF "Apple Development: person@example.com (TEAMID)"
          2) FEDCBA "Developer ID Application: Example Person (TEAMID)"
             2 valid identities found
        "#;

        assert_eq!(
            super::select_signing_identity(identities).as_deref(),
            Some("Developer ID Application: Example Person (TEAMID)")
        );
    }

    #[test]
    fn rejects_an_adhoc_designated_requirement_as_unstable() {
        assert!(!super::is_stable_requirement(
            r#"# designated => cdhash H"04c3928ca06f59fba04ebde63db1318ccc11e45c""#
        ));
        assert!(super::is_stable_requirement(
            r#"designated => identifier "io.github.cesarferreira.kiwi" and anchor apple generic"#
        ));
    }

    #[test]
    fn summarizes_a_running_daemon_with_its_pid() {
        let state = super::launch_agent_state("state = running\npid = 85265");

        assert_eq!(state, super::LaunchAgentState::Running { pid: Some(85265) });
        assert_eq!(
            super::status_line(state, Some("5m 57s"), false),
            "running for 5m 57s"
        );
        assert_eq!(
            super::status_line(state, Some("5m 57s"), true),
            "\u{1b}[32mrunning\u{1b}[0m for 5m 57s"
        );
    }

    #[test]
    fn summarizes_an_installed_but_stopped_daemon() {
        let state = super::launch_agent_state("state = spawn scheduled\nlast exit code = 1");

        assert_eq!(
            super::status_line(state, None, false),
            "stopped — check ~/Library/Logs/kiwi.log"
        );
    }

    #[test]
    fn summarizes_a_missing_installation() {
        assert_eq!(
            super::status_line(super::LaunchAgentState::Missing, None, false),
            "not installed — run `kiwi install`"
        );
    }

    #[test]
    fn formats_process_elapsed_time_compactly() {
        assert_eq!(
            super::format_elapsed_time("05:57").as_deref(),
            Some("5m 57s")
        );
        assert_eq!(
            super::format_elapsed_time("02:05:07").as_deref(),
            Some("2h 5m")
        );
        assert_eq!(
            super::format_elapsed_time("3-06:02:01").as_deref(),
            Some("3d 6h")
        );
    }

    #[test]
    fn reads_designated_requirement_from_codesign_stdout() {
        let output = super::combined_codesign_output(
            b"designated => identifier \"io.github.cesarferreira.kiwi\" and anchor apple generic",
            b"Executable=/tmp/kiwi",
        );

        assert!(super::is_stable_requirement(&output));
    }

    #[test]
    fn shortcut_table_colors_each_part_for_a_terminal() {
        let config = Config::from_toml(
            r#"
[bindings]
"hyper+a" = { app = "Arc" }
"#,
        )
        .unwrap()
        .compile()
        .unwrap();

        let output = super::shortcuts_table(&config.bindings, true);

        assert!(output.contains("\u{1b}[1;32m1 shortcut\u{1b}[0m"));
        assert!(output.contains("\u{1b}[36mhyper+a \u{1b}[0m"));
        assert!(output.contains("\u{1b}[33mapp \u{1b}[0m"));
        assert!(output.contains("\u{1b}[32mArc\u{1b}[0m"));
    }
}
