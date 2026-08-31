use std::process::Command;

#[test]
fn make_install_restores_signature_and_launch_agent_after_cargo_install() {
    for target in ["install", "install-release"] {
        let output = Command::new("make")
            .args(["--dry-run", target])
            .output()
            .unwrap();
        assert!(output.status.success());
        let commands = String::from_utf8(output.stdout).unwrap();
        let cargo_install = commands.find("cargo install").unwrap();
        let kiwi_install = commands.find("kiwi install").unwrap_or_else(|| {
            panic!("`make {target}` must restore the stable signature after Cargo replaces it")
        });
        assert!(kiwi_install > cargo_install);
    }
}
