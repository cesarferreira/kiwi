use std::{fs, process::Command};

use crate::{
    config::{Action, FeedbackPolicy},
    key::Chord,
};

use super::feedback::{
    ActionJob, NOTIFICATION_SCRIPT, action_completion, notification_command, notification_for,
};

fn job(policy: FeedbackPolicy) -> ActionJob {
    ActionJob {
        chord: "hyper+p".parse::<Chord>().unwrap(),
        action: Action::RunCommand("printf \"hello\"".into()),
        feedback: policy,
    }
}

#[test]
fn feedback_decision_matrix_covers_success_failure_and_off() {
    assert!(notification_for(&job(FeedbackPolicy::Off), Ok(())).is_none());
    assert!(notification_for(&job(FeedbackPolicy::Off), Err("failed")).is_none());
    assert!(notification_for(&job(FeedbackPolicy::Errors), Ok(())).is_none());
    assert!(notification_for(&job(FeedbackPolicy::Errors), Err("failed")).is_some());
    assert!(notification_for(&job(FeedbackPolicy::All), Ok(())).is_some());
    assert!(notification_for(&job(FeedbackPolicy::All), Err("failed")).is_some());
}

#[test]
fn action_completion_retains_full_failure_and_builds_concise_notification() {
    let completion = action_completion(
        &job(FeedbackPolicy::Errors),
        Err(anyhow::anyhow!("first detail\nsecond detail")),
    );

    assert_eq!(
        completion.failure.as_deref(),
        Some("first detail\nsecond detail")
    );
    let notification = completion.notification.unwrap();
    assert!(notification.body.contains("first detail"));
    assert!(!notification.body.contains("second detail"));
}

#[test]
fn failure_message_contains_action_context_and_first_meaningful_error_line() {
    let notification = notification_for(
        &job(FeedbackPolicy::Errors),
        Err("\n\npermission denied\nfull diagnostic detail"),
    )
    .unwrap();

    assert_eq!(notification.title, "Kiwi action failed");
    assert!(notification.body.contains("hyper+p"));
    assert!(notification.body.contains("command"));
    assert!(notification.body.contains("printf \"hello\""));
    assert!(notification.body.contains("permission denied"));
    assert!(!notification.body.contains("full diagnostic detail"));
}

#[test]
fn notification_text_is_unicode_safe_and_bounded() {
    let long_value = "🥝".repeat(400);
    let job = ActionJob {
        chord: "hyper+p".parse().unwrap(),
        action: Action::OpenUrl(long_value),
        feedback: FeedbackPolicy::All,
    };

    let notification = notification_for(&job, Ok(())).unwrap();

    assert!(notification.title.chars().count() <= 80);
    assert!(notification.body.chars().count() <= 240);
    assert!(notification.body.ends_with('…'));
}

#[test]
fn bounded_failure_message_retains_its_diagnostic() {
    let job = ActionJob {
        chord: "hyper+p".parse().unwrap(),
        action: Action::RunCommand("x".repeat(400)),
        feedback: FeedbackPolicy::Errors,
    };

    let notification = notification_for(&job, Err("important failure")).unwrap();

    assert!(notification.body.chars().count() <= 240);
    assert!(notification.body.contains("important failure"));
}

#[test]
fn notifier_uses_static_script_and_passes_content_after_argv_separator() {
    let notification = notification_for(
        &job(FeedbackPolicy::Errors),
        Err("quote: \"'; display dialog \"unsafe\""),
    )
    .unwrap();
    let command = notification_command(&notification);

    assert_eq!(command.program, "/usr/bin/osascript");
    assert_eq!(&command.args[..3], ["-e", NOTIFICATION_SCRIPT, "--"]);
    assert_eq!(command.args[3], notification.title);
    assert_eq!(command.args[4], notification.body);
    assert!(!command.args[1].contains("unsafe"));
}

#[test]
fn notification_applescript_compiles_without_posting() {
    let output_path = std::env::temp_dir().join(format!(
        "kiwi-notification-script-{}.scpt",
        std::process::id()
    ));
    let output = Command::new("/usr/bin/osacompile")
        .args(["-e", NOTIFICATION_SCRIPT, "-o"])
        .arg(&output_path)
        .output()
        .unwrap();
    let _ = fs::remove_file(output_path);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
