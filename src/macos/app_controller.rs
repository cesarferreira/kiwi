use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::config::{AppAction, AppBehavior};

const APP_ACTION_SCRIPT: &str = r#"on run argv
    set appOperation to item 1 of argv
    set targetKind to item 2 of argv
    set appTarget to item 3 of argv
    tell application "System Events"
        if targetKind is "path" then
            try
                set targetFile to (POSIX file appTarget) as alias
                set targetBundleId to bundle identifier of (info for targetFile)
            on error
                error "application is not running: " & appTarget number 1
            end try
            set matchingProcesses to application processes whose bundle identifier is targetBundleId
        else if targetKind is "bundle_id" then
            set matchingProcesses to application processes whose bundle identifier is appTarget
        else
            set matchingProcesses to application processes whose name is appTarget
        end if
        if (count of matchingProcesses) is 0 then
            if appOperation is "toggle" then return "missing"
            error "application is not running: " & appTarget number 1
        end if
        set targetProcess to item 1 of matchingProcesses

        if appOperation is "toggle" then
            set frontmostProcess to missing value
            repeat with candidateProcess in matchingProcesses
                if frontmost of candidateProcess then
                    set frontmostProcess to contents of candidateProcess
                    exit repeat
                end if
            end repeat
            if frontmostProcess is missing value then
                set frontmost of targetProcess to true
                return "activated"
            else
                set visible of frontmostProcess to false
                return "hidden"
            end if
        else if appOperation is "hide" then
            set visible of targetProcess to false
        else if appOperation is "cycle" then
            set appWindows to windows of targetProcess
            set windowCount to count of appWindows
            if windowCount is 0 then
                error "application has no windows: " & appTarget number 1
            end if

            set currentIndex to 0
            repeat with windowIndex from 1 to windowCount
                set candidateWindow to item windowIndex of appWindows
                set isCurrent to false
                try
                    set isCurrent to value of attribute "AXMain" of candidateWindow
                end try
                if not isCurrent then
                    try
                        set isCurrent to value of attribute "AXFocused" of candidateWindow
                    end try
                end if
                if isCurrent then
                    set currentIndex to windowIndex
                    exit repeat
                end if
            end repeat

            set nextIndex to currentIndex + 1
            if nextIndex > windowCount then set nextIndex to 1
            set nextWindow to item nextIndex of appWindows
            try
                set value of attribute "AXMain" of nextWindow to true
            end try
            try
                set value of attribute "AXFocused" of nextWindow to true
            end try
            try
                perform action "AXRaise" of nextWindow
            end try
            set frontmost of targetProcess to true
        else
            error "unknown app operation: " & appOperation number 2
        end if
    end tell
end run"#;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    pub program: &'static str,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CommandOutput {
    success: bool,
    status: String,
    stdout: String,
    stderr: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetKind {
    Name,
    Path,
    BundleIdentifier,
}

impl TargetKind {
    fn as_arg(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Path => "path",
            Self::BundleIdentifier => "bundle_id",
        }
    }
}

pub trait AppController {
    fn execute(&self, action: &AppAction) -> Result<()>;
}

trait CommandRunner {
    fn run(&self, command: &CommandSpec) -> Result<CommandOutput>;
}

pub struct MacOsAppController;

struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, spec: &CommandSpec) -> Result<CommandOutput> {
        let output = Command::new(spec.program)
            .args(&spec.args)
            .output()
            .context("could not start app action")?;
        Ok(CommandOutput {
            success: output.status.success(),
            status: output.status.to_string(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

impl AppController for MacOsAppController {
    fn execute(&self, action: &AppAction) -> Result<()> {
        execute_with_runner(action, &SystemCommandRunner)
    }
}

fn execute_with_runner(action: &AppAction, runner: &impl CommandRunner) -> Result<()> {
    let output = runner
        .run(&command_for(action))
        .with_context(|| format!("could not start app action for `{}`", action.target))?;
    ensure_success(action, &output)?;

    if action.behavior != AppBehavior::Toggle {
        return Ok(());
    }

    match output.stdout.trim() {
        "activated" | "hidden" => Ok(()),
        "missing" => {
            let launch = AppAction {
                target: action.target.clone(),
                behavior: AppBehavior::Launch,
            };
            let output = runner
                .run(&command_for(&launch))
                .with_context(|| format!("could not launch missing app `{}`", action.target))?;
            ensure_success(action, &output)
        }
        other => bail!(
            "app action for `{}` reported an unexpected toggle result `{other}`",
            action.target
        ),
    }
}

fn ensure_success(action: &AppAction, output: &CommandOutput) -> Result<()> {
    if output.success {
        return Ok(());
    }
    let detail = output.stderr.trim();
    if detail.is_empty() {
        bail!(
            "app action for `{}` exited with {}",
            action.target,
            output.status
        );
    }
    bail!("app action for `{}` failed: {detail}", action.target);
}

pub fn command_for(action: &AppAction) -> CommandSpec {
    let target_kind = classify_target(&action.target);
    match action.behavior {
        AppBehavior::Launch => CommandSpec {
            program: "/usr/bin/open",
            args: open_args(target_kind, false, &action.target),
        },
        AppBehavior::Toggle | AppBehavior::Hide | AppBehavior::Cycle => {
            osascript(action.behavior, target_kind, &action.target)
        }
        AppBehavior::NewWindow => CommandSpec {
            program: "/usr/bin/open",
            args: open_args(target_kind, true, &action.target),
        },
    }
}

pub fn classify_target(target: &str) -> TargetKind {
    if target.starts_with('/') {
        TargetKind::Path
    } else if looks_like_bundle_identifier(target) {
        TargetKind::BundleIdentifier
    } else {
        TargetKind::Name
    }
}

fn looks_like_bundle_identifier(target: &str) -> bool {
    let components: Vec<_> = target.split('.').collect();
    let Some(prefix) = components.first() else {
        return false;
    };
    let known_reverse_dns_prefix = matches!(
        *prefix,
        "app" | "co" | "com" | "dev" | "edu" | "gov" | "io" | "me" | "net" | "org"
    );
    components.len() >= 2
        && (components.len() >= 3 || known_reverse_dns_prefix)
        && components.iter().all(|component| {
            !component.is_empty()
                && component
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character))
        })
}

fn open_args(kind: TargetKind, new_window: bool, target: &str) -> Vec<String> {
    match (kind, new_window) {
        (TargetKind::BundleIdentifier, false) => vec!["-b".into(), target.into()],
        (TargetKind::BundleIdentifier, true) => {
            vec!["-n".into(), "-b".into(), target.into()]
        }
        (_, false) => vec!["-a".into(), target.into()],
        (_, true) => vec!["-na".into(), target.into()],
    }
}

fn osascript(behavior: AppBehavior, kind: TargetKind, target: &str) -> CommandSpec {
    let operation = match behavior {
        AppBehavior::Toggle => "toggle",
        AppBehavior::Hide => "hide",
        AppBehavior::Cycle => "cycle",
        AppBehavior::Launch | AppBehavior::NewWindow => {
            unreachable!("launch and new-window actions use open")
        }
    };
    CommandSpec {
        program: "/usr/bin/osascript",
        args: vec![
            "-e".into(),
            APP_ACTION_SCRIPT.into(),
            "--".into(),
            operation.into(),
            kind.as_arg().into(),
            target.into(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque};

    use super::{
        AppAction, AppBehavior, CommandOutput, CommandRunner, CommandSpec, Result,
        execute_with_runner,
    };

    struct FakeRunner {
        outputs: RefCell<VecDeque<CommandOutput>>,
        commands: RefCell<Vec<CommandSpec>>,
    }

    impl FakeRunner {
        fn new(outputs: impl IntoIterator<Item = CommandOutput>) -> Self {
            Self {
                outputs: RefCell::new(outputs.into_iter().collect()),
                commands: RefCell::new(Vec::new()),
            }
        }

        fn programs(&self) -> Vec<&'static str> {
            self.commands
                .borrow()
                .iter()
                .map(|command| command.program)
                .collect()
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, command: &CommandSpec) -> Result<CommandOutput> {
            self.commands.borrow_mut().push(command.clone());
            Ok(self
                .outputs
                .borrow_mut()
                .pop_front()
                .expect("runner was called more times than the test expected"))
        }
    }

    fn succeeded(stdout: &str) -> CommandOutput {
        CommandOutput {
            success: true,
            status: "exit status: 0".into(),
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    fn failed(stdout: &str, stderr: &str) -> CommandOutput {
        CommandOutput {
            success: false,
            status: "exit status: 1".into(),
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    fn toggle(target: &str) -> AppAction {
        AppAction {
            target: target.into(),
            behavior: AppBehavior::Toggle,
        }
    }

    #[test]
    fn toggle_launches_only_when_the_process_operation_reports_missing() {
        for (target, expected_open_args) in [
            ("Ghostty", vec!["-a", "Ghostty"]),
            (
                "/Applications/Ghostty.app",
                vec!["-a", "/Applications/Ghostty.app"],
            ),
            ("com.mitchellh.ghostty", vec!["-b", "com.mitchellh.ghostty"]),
        ] {
            for (result, expected_programs) in [
                (" missing \n", vec!["/usr/bin/osascript", "/usr/bin/open"]),
                ("activated\n", vec!["/usr/bin/osascript"]),
                ("hidden\n", vec!["/usr/bin/osascript"]),
            ] {
                let runner = FakeRunner::new([succeeded(result), succeeded("")]);

                execute_with_runner(&toggle(target), &runner).unwrap();

                assert_eq!(runner.programs(), expected_programs);
                let commands = runner.commands.borrow();
                assert_eq!(commands[0].args[3], "toggle");
                if expected_programs.len() == 2 {
                    assert_eq!(commands[1].args, expected_open_args);
                }
            }
        }
    }

    #[test]
    fn a_failed_toggle_reporting_missing_never_launches_and_surfaces_stderr() {
        let runner = FakeRunner::new([failed("missing\n", "execution error: not authorized")]);

        let error = execute_with_runner(&toggle("Ghostty"), &runner)
            .unwrap_err()
            .to_string();

        assert_eq!(runner.programs(), vec!["/usr/bin/osascript"]);
        assert!(error.contains("Ghostty"), "{error}");
        assert!(error.contains("execution error: not authorized"), "{error}");
    }

    #[test]
    fn a_failed_toggle_without_stderr_surfaces_its_exit_status() {
        let runner = FakeRunner::new([failed("missing\n", "   ")]);

        let error = execute_with_runner(&toggle("Ghostty"), &runner)
            .unwrap_err()
            .to_string();

        assert_eq!(runner.programs(), vec!["/usr/bin/osascript"]);
        assert!(error.contains("exit status: 1"), "{error}");
    }

    #[test]
    fn an_unexpected_toggle_result_never_launches_and_reports_a_clear_error() {
        for result in ["", "   \n", "Missing", "missing app", "hidden extra", "0"] {
            let runner = FakeRunner::new([succeeded(result)]);

            let error = execute_with_runner(&toggle("Ghostty"), &runner)
                .unwrap_err()
                .to_string();

            assert_eq!(runner.programs(), vec!["/usr/bin/osascript"], "{result:?}");
            assert!(error.contains("Ghostty"), "{error}");
            assert!(error.contains("unexpected"), "{error}");
        }
    }

    #[test]
    fn a_failed_launch_of_a_missing_toggle_target_is_reported() {
        let runner = FakeRunner::new([
            succeeded("missing\n"),
            failed("", "Unable to find application"),
        ]);

        let error = execute_with_runner(&toggle("Ghostty"), &runner)
            .unwrap_err()
            .to_string();

        assert_eq!(
            runner.programs(),
            vec!["/usr/bin/osascript", "/usr/bin/open"]
        );
        assert!(error.contains("Unable to find application"), "{error}");
    }

    #[test]
    fn non_toggle_behaviors_run_exactly_one_command_and_ignore_stdout() {
        for behavior in [
            AppBehavior::Launch,
            AppBehavior::Hide,
            AppBehavior::Cycle,
            AppBehavior::NewWindow,
        ] {
            let runner = FakeRunner::new([succeeded("missing\n")]);

            execute_with_runner(
                &AppAction {
                    target: "Ghostty".into(),
                    behavior,
                },
                &runner,
            )
            .unwrap();

            assert_eq!(runner.programs().len(), 1, "{behavior:?}");
        }
    }
}
