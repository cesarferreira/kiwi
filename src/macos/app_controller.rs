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
            error "application is not running: " & appTarget number 1
        end if
        set targetProcess to item 1 of matchingProcesses

        if appOperation is "hide" then
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

pub struct MacOsAppController;

impl AppController for MacOsAppController {
    fn execute(&self, action: &AppAction) -> Result<()> {
        let spec = command_for(action);
        let output = Command::new(spec.program)
            .args(&spec.args)
            .output()
            .with_context(|| format!("could not start app action for `{}`", action.target))?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr);
            let detail = detail.trim();
            if detail.is_empty() {
                bail!(
                    "app action for `{}` exited with {}",
                    action.target,
                    output.status
                );
            }
            bail!("app action for `{}` failed: {detail}", action.target);
        }
        Ok(())
    }
}

pub fn command_for(action: &AppAction) -> CommandSpec {
    let target_kind = classify_target(&action.target);
    match action.behavior {
        AppBehavior::Launch => CommandSpec {
            program: "/usr/bin/open",
            args: open_args(target_kind, false, &action.target),
        },
        AppBehavior::Hide | AppBehavior::Cycle => {
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
