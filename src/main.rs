#[cfg(target_os = "macos")]
mod cli;

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<std::process::ExitCode> {
    cli::run()
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("kiwi only supports macOS");
    std::process::exit(1);
}
