use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use keyweave::{
    DEFAULT_CONFIG,
    config::Config,
    macos::{
        LABEL, accessibility_is_trusted, launch_agent_plist, remove_caps_remap, run_event_tap,
    },
};

#[derive(Parser, Debug)]
#[command(name = "keyweave", version, about = "Run portable macOS key mappings")]
struct Cli {
    /// Use a config file other than ~/.config/keyweave/config.toml
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
    /// Run the keyboard event daemon in the foreground
    Run,
    /// Install and start the per-user LaunchAgent
    Install,
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
        Command::Run => run_event_tap(load_config(&config_path)?),
        Command::Install => install(&config_path),
        Command::Uninstall => uninstall(),
        Command::Restart => launchctl(&["kickstart", "-k", &service_target()]),
        Command::Status => launchctl(&["print", &service_target()]),
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

fn load_config(path: &Path) -> Result<keyweave::config::CompiledConfig> {
    Config::from_path(path)?.compile()
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
    let binary = std::env::current_exe().context("could not locate the keyweave binary")?;
    let signature_changed = ensure_stable_signature(&binary)?;
    fs::write(
        &plist_path,
        launch_agent_plist(&binary, config_path, &log_path),
    )
    .with_context(|| format!("could not write {}", plist_path.display()))?;

    unload_if_loaded(&LaunchctlManager, &service_target())?;
    launchctl(&[
        "bootstrap",
        &format!("gui/{}", unsafe { libc::getuid() }),
        &plist_path.to_string_lossy(),
    ])?;
    println!("installed {LABEL}");
    if signature_changed || !accessibility_is_trusted() {
        println!(
            "next: run `keyweave permissions`, remove any old Keyweave entry, add {}, then run `keyweave restart`",
            binary.display()
        );
    }
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
    let binary = std::env::current_exe().context("could not locate the keyweave binary")?;
    if is_stable_requirement(&code_requirement(&binary)?) {
        println!("[ok] stable code-signing identity");
    } else {
        healthy = false;
        println!("[fail] binary is ad-hoc signed; run `keyweave install`");
    }
    match query_launch_agent_state()? {
        LaunchAgentState::Running => println!("[ok] LaunchAgent is running"),
        LaunchAgentState::NotRunning => {
            healthy = false;
            println!(
                "[fail] LaunchAgent is installed but not running; check ~/Library/Logs/keyweave.log"
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
    Running,
    NotRunning,
}

fn query_launch_agent_state() -> Result<LaunchAgentState> {
    let output = ProcessCommand::new("/bin/launchctl")
        .args(["print", &service_target()])
        .output()
        .context("could not query LaunchAgent state")?;
    if output.status.code() == Some(113) {
        return Ok(LaunchAgentState::Missing);
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
        LaunchAgentState::Running
    } else {
        LaunchAgentState::NotRunning
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
        .context("could not sign keyweave")?;
    if !output.status.success() {
        bail!(
            "codesign failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    if !is_stable_requirement(&code_requirement(binary)?) {
        bail!("codesign did not produce a stable designated requirement");
    }
    println!("signed keyweave as `{identity}`");
    Ok(true)
}

fn code_requirement(binary: &Path) -> Result<String> {
    let output = ProcessCommand::new("/usr/bin/codesign")
        .args(["-d", "-r-", &binary.to_string_lossy()])
        .output()
        .context("could not inspect the keyweave signature")?;
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
    fn bootout(&self, target: &str) -> Result<()>;
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

    fn bootout(&self, target: &str) -> Result<()> {
        launchctl(&["bootout", target])
    }
}

fn unload_if_loaded(manager: &impl ServiceManager, target: &str) -> Result<()> {
    if manager.is_loaded(target)? {
        manager.bootout(target)?;
    }
    Ok(())
}

fn service_target() -> String {
    format!("gui/{}/{LABEL}", unsafe { libc::getuid() })
}

fn default_config_path() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("could not find the home directory")?
        .join(".config/keyweave/config.toml"))
}

fn launch_agent_path() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("could not find the home directory")?
        .join(format!("Library/LaunchAgents/{LABEL}.plist")))
}

fn log_path() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("could not find the home directory")?
        .join("Library/Logs/keyweave.log"))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use anyhow::Result;

    use super::{ServiceManager, unload_if_loaded};

    struct FakeServiceManager {
        loaded: bool,
        bootout_calls: Cell<usize>,
    }

    impl ServiceManager for FakeServiceManager {
        fn is_loaded(&self, _target: &str) -> Result<bool> {
            Ok(self.loaded)
        }

        fn bootout(&self, _target: &str) -> Result<()> {
            self.bootout_calls.set(self.bootout_calls.get() + 1);
            Ok(())
        }
    }

    #[test]
    fn first_install_does_not_bootout_a_missing_service() {
        let manager = FakeServiceManager {
            loaded: false,
            bootout_calls: Cell::new(0),
        };

        unload_if_loaded(&manager, "gui/501/example").unwrap();

        assert_eq!(manager.bootout_calls.get(), 0);
    }

    #[test]
    fn reinstall_boots_out_the_loaded_service_once() {
        let manager = FakeServiceManager {
            loaded: true,
            bootout_calls: Cell::new(0),
        };

        unload_if_loaded(&manager, "gui/501/example").unwrap();

        assert_eq!(manager.bootout_calls.get(), 1);
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
            r#"designated => identifier "io.github.cesarferreira.keyweave" and anchor apple generic"#
        ));
    }

    #[test]
    fn distinguishes_a_running_daemon_from_a_scheduled_restart() {
        assert_eq!(
            super::launch_agent_state("state = running"),
            super::LaunchAgentState::Running
        );
        assert_eq!(
            super::launch_agent_state("state = spawn scheduled\nlast exit code = 1"),
            super::LaunchAgentState::NotRunning
        );
    }

    #[test]
    fn reads_designated_requirement_from_codesign_stdout() {
        let output = super::combined_codesign_output(
            b"designated => identifier \"io.github.cesarferreira.keyweave\" and anchor apple generic",
            b"Executable=/tmp/keyweave",
        );

        assert!(super::is_stable_requirement(&output));
    }
}
