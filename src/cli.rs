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
    macos::{LABEL, accessibility_is_trusted, launch_agent_plist, run_event_tap},
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
    fs::write(
        &plist_path,
        launch_agent_plist(&binary, config_path, &log_path),
    )
    .with_context(|| format!("could not write {}", plist_path.display()))?;

    let _ = ProcessCommand::new("/bin/launchctl")
        .args(["bootout", &service_target()])
        .status();
    launchctl(&[
        "bootstrap",
        &format!("gui/{}", unsafe { libc::getuid() }),
        &plist_path.to_string_lossy(),
    ])?;
    println!("installed {LABEL}");
    if !accessibility_is_trusted() {
        println!(
            "next: run `keyweave permissions`, then allow {} and run `keyweave restart`",
            binary.display()
        );
    }
    Ok(())
}

fn uninstall() -> Result<()> {
    let plist_path = launch_agent_path()?;
    let _ = ProcessCommand::new("/bin/launchctl")
        .args(["bootout", &service_target()])
        .status();
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
    if accessibility_is_trusted() {
        println!("[ok] Accessibility permission");
    } else {
        healthy = false;
        println!("[fail] Accessibility permission is not granted");
    }
    let plist = launch_agent_path()?;
    if plist.exists() {
        println!("[ok] LaunchAgent: {}", plist.display());
    } else {
        println!("[info] LaunchAgent is not installed (foreground mode is available)");
    }
    if healthy {
        Ok(())
    } else {
        bail!("doctor found problems")
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
