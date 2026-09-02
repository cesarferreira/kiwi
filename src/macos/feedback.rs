use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::{
    config::{Action, FeedbackPolicy},
    key::Chord,
};

pub(crate) const NOTIFICATION_SCRIPT: &str = r#"on run argv
    set notificationTitle to item 1 of argv
    set notificationBody to item 2 of argv
    display notification notificationBody with title notificationTitle
end run"#;

const MAX_TITLE_CHARS: usize = 80;
const MAX_BODY_CHARS: usize = 240;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActionJob {
    pub chord: Chord,
    pub action: Action,
    pub feedback: FeedbackPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Notification {
    pub title: String,
    pub body: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActionCompletion {
    pub failure: Option<String>,
    pub notification: Option<Notification>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NotificationCommand {
    pub program: &'static str,
    pub args: Vec<String>,
}

pub(crate) trait Notifier {
    fn notify(&self, notification: &Notification) -> Result<()>;
}

pub(crate) struct MacOsNotifier;

impl Notifier for MacOsNotifier {
    fn notify(&self, notification: &Notification) -> Result<()> {
        let spec = notification_command(notification);
        let output = Command::new(spec.program)
            .args(&spec.args)
            .output()
            .context("could not start macOS notification")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "macOS notification exited with {}: {}",
                output.status,
                stderr.trim()
            );
        }
        Ok(())
    }
}

pub(crate) fn notification_for(job: &ActionJob, outcome: Result<(), &str>) -> Option<Notification> {
    let failed = outcome.is_err();
    let should_notify = match job.feedback {
        FeedbackPolicy::Off => false,
        FeedbackPolicy::Errors => failed,
        FeedbackPolicy::All => true,
    };
    if !should_notify {
        return None;
    }

    let title = if failed {
        "Kiwi action failed"
    } else {
        "Kiwi action completed"
    };
    let (action_type, value) = job.action.type_and_value();
    let context = format!("{} · {action_type}: {value}", job.chord);
    let body = match outcome {
        Err(error) => {
            let detail = error
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or("action failed");
            let detail = truncate(detail, 80);
            let context_limit = MAX_BODY_CHARS.saturating_sub(detail.chars().count() + 1);
            format!("{}\n{detail}", truncate(&context, context_limit))
        }
        Ok(()) => truncate(&context, MAX_BODY_CHARS),
    };

    Some(Notification {
        title: truncate(title, MAX_TITLE_CHARS),
        body,
    })
}

pub(crate) fn action_completion(job: &ActionJob, outcome: anyhow::Result<()>) -> ActionCompletion {
    let failure = outcome.err().map(|error| format!("{error:#}"));
    let notification = match failure.as_deref() {
        Some(error) => notification_for(job, Err(error)),
        None => notification_for(job, Ok(())),
    };
    ActionCompletion {
        failure,
        notification,
    }
}

pub(crate) fn notification_command(notification: &Notification) -> NotificationCommand {
    NotificationCommand {
        program: "/usr/bin/osascript",
        args: vec![
            "-e".into(),
            NOTIFICATION_SCRIPT.into(),
            "--".into(),
            notification.title.clone(),
            notification.body.clone(),
        ],
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.into();
    }
    let mut output: String = value.chars().take(max_chars.saturating_sub(1)).collect();
    output.push('…');
    output
}
