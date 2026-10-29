use std::{
    io::{self, BufRead, Read, Write},
    process::{Child, Command, Stdio},
    sync::mpsc::{Receiver, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::BindingSummary;

const MAX_ROWS: usize = 64;
const MAX_TEXT_CHARS: usize = 160;
const MAX_INPUT_BYTES: usize = 100_000;
pub(crate) const MAX_VISIBLE_DURATION: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CheatsheetModel {
    pub generation: u64,
    pub rows: std::sync::Arc<[BindingSummary]>,
}

#[derive(Deserialize)]
struct CheatsheetInput {
    generation: u64,
    rows: Vec<BindingSummary>,
}

#[derive(Serialize)]
struct CheatsheetOutput<'a> {
    generation: u64,
    rows: &'a [BindingSummary],
}

pub(crate) fn parse_model(input: &[u8]) -> Result<CheatsheetModel> {
    if input.len() > MAX_INPUT_BYTES {
        bail!("cheatsheet model exceeds {MAX_INPUT_BYTES} bytes");
    }
    let model: CheatsheetInput =
        serde_json::from_slice(input).context("invalid cheatsheet model JSON")?;
    if model.rows.len() > MAX_ROWS {
        bail!("cheatsheet model exceeds {MAX_ROWS} rows");
    }
    if model.rows.iter().any(|row| {
        [&row.key, &row.kind, &row.action].iter().any(|value| {
            value.chars().count() > MAX_TEXT_CHARS || value.chars().any(char::is_control)
        })
    }) {
        bail!("cheatsheet model text exceeds {MAX_TEXT_CHARS} characters");
    }
    Ok(CheatsheetModel {
        generation: model.generation,
        rows: model.rows.into(),
    })
}

#[doc(hidden)]
pub fn run_overlay_helper() -> Result<()> {
    let mut input = Vec::new();
    std::io::stdin()
        .lock()
        .take((MAX_INPUT_BYTES + 1) as u64)
        .read_until(b'\n', &mut input)
        .context("could not read cheatsheet model")?;
    let model = parse_model(&input)?;
    thread::Builder::new()
        .name("kiwi-cheatsheet-parent-watch".into())
        .spawn(|| {
            let _ = wait_for_parent_eof(&mut std::io::stdin().lock());
            std::process::exit(0);
        })
        .context("could not start cheatsheet parent watcher")?;
    show_native_overlay(&model)
}

fn wait_for_parent_eof(reader: &mut impl Read) -> io::Result<()> {
    let mut byte = [0_u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => return Ok(()),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

fn show_native_overlay(model: &CheatsheetModel) -> Result<()> {
    use objc2::{MainThreadMarker, MainThreadOnly, runtime::AnyObject};
    use objc2_app_kit::{
        NSApplication, NSBackingStoreType, NSBox, NSBoxType, NSColor, NSFloatingWindowLevel,
        NSFont, NSFontAttributeName, NSForegroundColorAttributeName, NSPanel, NSScreen,
        NSTextField, NSTitlePosition, NSWindowStyleMask,
    };
    use objc2_foundation::{
        NSAttributedString, NSDictionary, NSMutableAttributedString, NSPoint, NSRect, NSSize,
        NSString,
    };

    const PADDING: f64 = 22.0;
    let mtm = MainThreadMarker::new().context("cheatsheet helper must run on its main thread")?;
    let screen = NSScreen::mainScreen(mtm).context("no macOS main screen is available")?;
    let visible = screen.visibleFrame();

    let text = NSMutableAttributedString::from_nsstring(&NSString::from_str(""));
    for (span, tint) in render_spans(model) {
        let (red, green, blue) = tint.rgb();
        let color = NSColor::colorWithSRGBRed_green_blue_alpha(red, green, blue, 1.0);
        let font = NSFont::monospacedSystemFontOfSize_weight(tint.font_size(), tint.weight());
        let objects: [&AnyObject; 2] = [color.as_ref(), font.as_ref()];
        // SAFETY: AppKit attribute keys are immutable statics, and each value
        // has the type AppKit documents for its key.
        let attributes = unsafe {
            NSDictionary::from_slices(
                &[NSForegroundColorAttributeName, NSFontAttributeName],
                &objects,
            )
        };
        // SAFETY: `attributes` holds only valid foreground color and font values.
        let span = unsafe {
            NSAttributedString::new_with_attributes(&NSString::from_str(&span), &attributes)
        };
        text.appendAttributedString(&span);
    }

    let label = NSTextField::labelWithAttributedString(&text, mtm);
    label.setMaximumNumberOfLines(0);
    label.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingTail);
    let fitting = label.fittingSize();
    let max_width = (visible.size.width - 80.0).max(240.0);
    let max_height = (visible.size.height - 40.0).max(180.0);
    let width = (fitting.width + PADDING * 2.0).min(max_width).ceil();
    let height = (fitting.height + PADDING * 2.0).min(max_height).ceil();
    let origin = NSPoint::new(
        visible.origin.x + (visible.size.width - width) / 2.0,
        visible.origin.y + (visible.size.height - height) / 2.0,
    );

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(overlay_activation_policy());
    let window = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        NSRect::new(origin, NSSize::new(width, height)),
        NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
        NSBackingStoreType::Buffered,
        false,
    );
    unsafe { window.setReleasedWhenClosed(false) };
    window.setLevel(NSFloatingWindowLevel);
    window.setOpaque(false);
    window.setHasShadow(true);
    window.setIgnoresMouseEvents(true);
    window.setBackgroundColor(Some(&NSColor::clearColor()));

    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height));
    let background = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
    background.setBoxType(NSBoxType::Custom);
    background.setTitlePosition(NSTitlePosition::NoTitle);
    background.setCornerRadius(14.0);
    background.setBorderWidth(1.5);
    background.setBorderColor(&NSColor::colorWithSRGBRed_green_blue_alpha(
        0.55, 0.85, 0.30, 0.65,
    ));
    background.setFillColor(&NSColor::colorWithSRGBRed_green_blue_alpha(
        0.07, 0.08, 0.10, 0.97,
    ));

    label.setFrame(NSRect::new(
        NSPoint::new(PADDING, PADDING),
        NSSize::new(width - PADDING * 2.0, height - PADDING * 2.0),
    ));
    let content = window
        .contentView()
        .context("cheatsheet window has no content view")?;
    content.addSubview(&background);
    content.addSubview(&label);
    window.orderFrontRegardless();
    app.run();
    Ok(())
}

fn overlay_activation_policy() -> objc2_app_kit::NSApplicationActivationPolicy {
    objc2_app_kit::NSApplicationActivationPolicy::Accessory
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Tint {
    Title,
    Muted,
    Key,
    Action,
    App,
    Url,
    Command,
    Keys,
    Window,
    Sequence,
}

impl Tint {
    fn for_kind(kind: &str) -> Self {
        match kind {
            "app" => Self::App,
            "url" => Self::Url,
            "command" => Self::Command,
            "keys" => Self::Keys,
            "window" => Self::Window,
            "sequence" => Self::Sequence,
            _ => Self::Muted,
        }
    }

    fn rgb(self) -> (f64, f64, f64) {
        match self {
            Self::Title | Self::Key => (0.55, 0.85, 0.30),
            Self::Muted => (0.50, 0.53, 0.60),
            Self::Action => (0.90, 0.91, 0.94),
            Self::App => (0.40, 0.70, 1.00),
            Self::Url => (0.75, 0.55, 1.00),
            Self::Command => (1.00, 0.62, 0.30),
            Self::Keys => (1.00, 0.85, 0.35),
            Self::Window => (0.35, 0.85, 0.80),
            Self::Sequence => (1.00, 0.50, 0.70),
        }
    }

    fn weight(self) -> f64 {
        match self {
            Self::Title | Self::Key => 0.4,
            Self::Action | Self::Muted => 0.0,
            _ => 0.23,
        }
    }

    fn font_size(self) -> f64 {
        if self == Self::Title { 16.0 } else { 14.0 }
    }
}

fn render_spans(model: &CheatsheetModel) -> Vec<(String, Tint)> {
    let key_width = model
        .rows
        .iter()
        .map(|row| row.key.chars().count())
        .max()
        .unwrap_or(3)
        .max(3);
    let type_width = model
        .rows
        .iter()
        .map(|row| row.kind.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    let mut spans = vec![
        ("🥝 kiwi".to_string(), Tint::Title),
        ("  hyper shortcuts\n\n".to_string(), Tint::Muted),
        (
            format!("{:<key_width$}  {:<type_width$}  ACTION\n", "KEY", "TYPE"),
            Tint::Muted,
        ),
    ];
    for (index, row) in model.rows.iter().enumerate() {
        let newline = if index + 1 < model.rows.len() {
            "\n"
        } else {
            ""
        };
        spans.push((format!("{:<key_width$}  ", row.key), Tint::Key));
        spans.push((
            format!("{:<type_width$}  ", row.kind),
            Tint::for_kind(&row.kind),
        ));
        spans.push((format!("{}{newline}", row.action), Tint::Action));
    }
    spans
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerEffect {
    None,
    Schedule,
    Spawn,
    Stop,
}

pub(crate) struct WorkerState {
    phase: WorkerPhase,
}

enum WorkerPhase {
    Idle,
    Pending {
        deadline: Duration,
        model: CheatsheetModel,
    },
    Spawning {
        model: CheatsheetModel,
    },
    Visible {
        model: CheatsheetModel,
        expires_at: Duration,
    },
    BlockedUntilHide,
}

impl Default for WorkerState {
    fn default() -> Self {
        Self {
            phase: WorkerPhase::Idle,
        }
    }
}

impl WorkerState {
    pub(crate) fn show(
        &mut self,
        now: Duration,
        delay_ms: u64,
        model: CheatsheetModel,
    ) -> WorkerEffect {
        let deadline = now + Duration::from_millis(delay_ms);
        match &self.phase {
            WorkerPhase::Idle => {
                self.phase = WorkerPhase::Pending { deadline, model };
                WorkerEffect::Schedule
            }
            WorkerPhase::Pending {
                model: current,
                deadline: pending,
            } if current == &model && *pending <= deadline => WorkerEffect::None,
            WorkerPhase::Pending { .. } => {
                self.phase = WorkerPhase::Pending { deadline, model };
                WorkerEffect::Schedule
            }
            WorkerPhase::Spawning { model: current }
            | WorkerPhase::Visible { model: current, .. }
                if current == &model =>
            {
                WorkerEffect::None
            }
            WorkerPhase::Spawning { .. } | WorkerPhase::Visible { .. } => {
                self.phase = WorkerPhase::Pending { deadline, model };
                WorkerEffect::Stop
            }
            WorkerPhase::BlockedUntilHide => WorkerEffect::None,
        }
    }

    pub(crate) fn hide(&mut self) -> WorkerEffect {
        let phase = std::mem::replace(&mut self.phase, WorkerPhase::Idle);
        match phase {
            WorkerPhase::Visible { .. } => WorkerEffect::Stop,
            WorkerPhase::Idle
            | WorkerPhase::Pending { .. }
            | WorkerPhase::Spawning { .. }
            | WorkerPhase::BlockedUntilHide => WorkerEffect::None,
        }
    }

    pub(crate) fn deadline_elapsed(&mut self, now: Duration) -> WorkerEffect {
        match &self.phase {
            WorkerPhase::Pending { deadline, .. } if now >= *deadline => {
                let WorkerPhase::Pending { model, .. } =
                    std::mem::replace(&mut self.phase, WorkerPhase::Idle)
                else {
                    unreachable!("phase was checked as pending");
                };
                self.phase = WorkerPhase::Spawning { model };
                WorkerEffect::Spawn
            }
            WorkerPhase::Visible { expires_at, .. } if now >= *expires_at => {
                self.phase = WorkerPhase::BlockedUntilHide;
                WorkerEffect::Stop
            }
            _ => WorkerEffect::None,
        }
    }

    pub(crate) fn spawn_succeeded(&mut self, now: Duration) {
        let WorkerPhase::Spawning { model } = std::mem::replace(&mut self.phase, WorkerPhase::Idle)
        else {
            return;
        };
        self.phase = WorkerPhase::Visible {
            model,
            expires_at: now + MAX_VISIBLE_DURATION,
        };
    }

    pub(crate) fn spawn_failed(&mut self) {
        if matches!(self.phase, WorkerPhase::Spawning { .. }) {
            self.phase = WorkerPhase::Idle;
        }
    }

    fn child_exited(&mut self) {
        if matches!(self.phase, WorkerPhase::Visible { .. }) {
            self.phase = WorkerPhase::BlockedUntilHide;
        }
    }

    fn next_deadline(&self) -> Option<Duration> {
        match self.phase {
            WorkerPhase::Pending { deadline, .. } => Some(deadline),
            WorkerPhase::Visible { expires_at, .. } => Some(expires_at),
            WorkerPhase::Idle | WorkerPhase::Spawning { .. } | WorkerPhase::BlockedUntilHide => {
                None
            }
        }
    }

    fn spawning_model(&self) -> Option<&CheatsheetModel> {
        match &self.phase {
            WorkerPhase::Spawning { model } => Some(model),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum CheatsheetCommand {
    Show {
        model: CheatsheetModel,
        delay_ms: u64,
    },
    Hide,
}

pub(crate) fn worker(receiver: Receiver<CheatsheetCommand>) {
    let started = Instant::now();
    let mut state = WorkerState::default();
    let mut child = None;
    loop {
        let now = started.elapsed();
        let deadline = state.next_deadline().map(|deadline| {
            if child.is_some() {
                deadline.min(now + Duration::from_secs(1))
            } else {
                deadline
            }
        });
        let received = match deadline {
            Some(deadline) => receiver.recv_timeout(deadline.saturating_sub(now)),
            None => match receiver.recv() {
                Ok(command) => Ok(command),
                Err(_) => break,
            },
        };
        if reap_exited_child(&mut child) {
            state.child_exited();
        }
        match received {
            Ok(CheatsheetCommand::Show { model, delay_ms }) => {
                let effect = state.show(started.elapsed(), delay_ms, model);
                apply_stop(effect, &mut child);
                if delay_ms == 0 && state.deadline_elapsed(started.elapsed()) == WorkerEffect::Spawn
                {
                    spawn_from_state(&mut state, &mut child, started.elapsed());
                }
            }
            Ok(CheatsheetCommand::Hide) => {
                let effect = state.hide();
                apply_stop(effect, &mut child);
            }
            Err(RecvTimeoutError::Timeout) => {
                let effect = state.deadline_elapsed(started.elapsed());
                if effect == WorkerEffect::Spawn {
                    spawn_from_state(&mut state, &mut child, started.elapsed());
                } else {
                    apply_stop(effect, &mut child);
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    stop_child(&mut child);
}

fn spawn_from_state(state: &mut WorkerState, child: &mut Option<Child>, now: Duration) {
    if let Some(spawned) = spawn_visible(state.spawning_model()) {
        *child = Some(spawned);
        state.spawn_succeeded(now);
    } else {
        state.spawn_failed();
    }
}

fn spawn_visible(model: Option<&CheatsheetModel>) -> Option<Child> {
    let model = model?;
    match spawn_overlay(model) {
        Ok(child) => Some(child),
        Err(error) => {
            eprintln!("kiwi cheatsheet failed: {error:#}");
            None
        }
    }
}

fn reap_exited_child(child: &mut Option<Child>) -> bool {
    let exited = child
        .as_mut()
        .is_some_and(|child| matches!(child.try_wait(), Ok(Some(_))));
    if exited {
        child.take();
    }
    exited
}

fn spawn_overlay(model: &CheatsheetModel) -> Result<Child> {
    let executable = std::env::current_exe().context("could not locate the kiwi binary")?;
    let mut child = Command::new(executable)
        .arg("__cheatsheet-overlay")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("could not start cheatsheet overlay helper")?;
    let input = child
        .stdin
        .as_mut()
        .context("cheatsheet helper stdin was not piped")?;
    let output = CheatsheetOutput {
        generation: model.generation,
        rows: &model.rows,
    };
    if let Err(error) = serde_json::to_writer(&mut *input, &output)
        .and_then(|()| input.write_all(b"\n").map_err(serde_json::Error::io))
        .and_then(|()| input.flush().map_err(serde_json::Error::io))
    {
        stop_child(&mut Some(child));
        return Err(error).context("could not send cheatsheet model");
    }
    Ok(child)
}

fn apply_stop(effect: WorkerEffect, child: &mut Option<Child>) {
    if effect == WorkerEffect::Stop {
        stop_child(child);
    }
}

fn stop_child(child: &mut Option<Child>) {
    if let Some(mut child) = child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, io, time::Duration};

    use crate::config::BindingSummary;

    use super::{
        CheatsheetModel, MAX_VISIBLE_DURATION, WorkerEffect, WorkerState, parse_model,
        wait_for_parent_eof,
    };

    #[test]
    fn worker_deadline_cancels_fast_release_and_deduplicates_down() {
        let model = CheatsheetModel {
            generation: 1,
            rows: Vec::new().into(),
        };
        let mut state = WorkerState::default();

        assert_eq!(
            state.show(Duration::from_millis(10), 300, model.clone()),
            WorkerEffect::Schedule
        );
        assert_eq!(
            state.show(Duration::from_millis(20), 300, model),
            WorkerEffect::None
        );
        assert_eq!(
            state.hide(),
            WorkerEffect::None,
            "pending display is cancelled without spawning"
        );
        assert_eq!(
            state.deadline_elapsed(Duration::from_millis(400)),
            WorkerEffect::None
        );
    }

    #[test]
    fn immediate_request_pulls_a_pending_delay_forward() {
        let model = CheatsheetModel {
            generation: 5,
            rows: Vec::new().into(),
        };
        let mut state = WorkerState::default();
        let now = Duration::from_secs(1);

        assert_eq!(state.show(now, 1000, model.clone()), WorkerEffect::Schedule);
        let asked_at = now + Duration::from_millis(200);
        assert_eq!(state.show(asked_at, 0, model), WorkerEffect::Schedule);
        assert_eq!(state.deadline_elapsed(asked_at), WorkerEffect::Spawn);
    }

    #[test]
    fn worker_spawns_at_deadline_and_hides_visible_child_immediately() {
        let model = CheatsheetModel {
            generation: 2,
            rows: Vec::new().into(),
        };
        let mut state = WorkerState::default();

        assert_eq!(
            state.show(Duration::from_secs(1), 300, model),
            WorkerEffect::Schedule
        );
        assert_eq!(
            state.deadline_elapsed(Duration::from_millis(1299)),
            WorkerEffect::None
        );
        assert_eq!(
            state.deadline_elapsed(Duration::from_millis(1300)),
            WorkerEffect::Spawn
        );
        state.spawn_succeeded(Duration::from_millis(1300));
        assert_eq!(state.hide(), WorkerEffect::Stop);
    }

    #[test]
    fn visible_overlay_expires_and_stays_blocked_until_fresh_release_down() {
        let model = CheatsheetModel {
            generation: 3,
            rows: Vec::new().into(),
        };
        let mut state = WorkerState::default();
        let shown_at = Duration::from_secs(2);

        assert_eq!(
            state.show(shown_at, 0, model.clone()),
            WorkerEffect::Schedule
        );
        assert_eq!(state.deadline_elapsed(shown_at), WorkerEffect::Spawn);
        state.spawn_succeeded(shown_at);
        assert_eq!(
            state.deadline_elapsed(shown_at + MAX_VISIBLE_DURATION - Duration::from_millis(1)),
            WorkerEffect::None
        );
        assert_eq!(
            state.deadline_elapsed(shown_at + MAX_VISIBLE_DURATION),
            WorkerEffect::Stop
        );
        assert_eq!(
            state.show(shown_at + MAX_VISIBLE_DURATION, 0, model.clone()),
            WorkerEffect::None
        );
        assert_eq!(state.hide(), WorkerEffect::None);
        assert_eq!(
            state.show(shown_at + MAX_VISIBLE_DURATION, 0, model),
            WorkerEffect::Schedule
        );
    }

    #[test]
    fn failed_spawn_does_not_mark_the_model_visible_and_can_retry() {
        let model = CheatsheetModel {
            generation: 4,
            rows: Vec::new().into(),
        };
        let mut state = WorkerState::default();
        let now = Duration::from_secs(1);

        assert_eq!(state.show(now, 0, model.clone()), WorkerEffect::Schedule);
        assert_eq!(state.deadline_elapsed(now), WorkerEffect::Spawn);
        state.spawn_failed();
        assert_eq!(
            state.show(now + Duration::from_millis(1), 0, model),
            WorkerEffect::Schedule
        );
    }

    #[test]
    fn helper_input_is_structured_and_rejects_oversized_models() {
        let valid = br#"{"generation":3,"rows":[{"key":"shift+t","type":"app","action":"Arc"}]}"#;
        assert_eq!(parse_model(valid).unwrap().generation, 3);

        let rows = (0..65)
            .map(|_| BindingSummary {
                key: "a".into(),
                kind: "app".into(),
                action: "Arc".into(),
            })
            .collect::<Vec<_>>();
        let too_many = serde_json::to_vec(&super::CheatsheetOutput {
            generation: 0,
            rows: &rows,
        })
        .unwrap();
        assert!(
            parse_model(&too_many)
                .unwrap_err()
                .to_string()
                .contains("rows")
        );

        let long_text = format!(
            r#"{{"generation":0,"rows":[{{"key":"a","type":"app","action":"{}"}}]}}"#,
            "x".repeat(161)
        );
        assert!(parse_model(long_text.as_bytes()).is_err());
        assert!(parse_model(&vec![b'x'; 100_001]).is_err());
        assert!(
            parse_model(
                br#"{"generation":0,"rows":[{"key":"a","type":"app","action":"Arc\nINJECT"}]}"#
            )
            .is_err()
        );
    }

    #[test]
    fn parent_watch_discards_input_with_constant_one_byte_memory() {
        struct TrackingReader {
            remaining: usize,
            largest_buffer: Cell<usize>,
        }

        impl io::Read for TrackingReader {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                self.largest_buffer
                    .set(self.largest_buffer.get().max(buffer.len()));
                if self.remaining == 0 {
                    return Ok(0);
                }
                self.remaining -= 1;
                buffer[0] = b'x';
                Ok(1)
            }
        }

        let mut reader = TrackingReader {
            remaining: 100_000,
            largest_buffer: Cell::new(0),
        };
        wait_for_parent_eof(&mut reader).unwrap();

        assert_eq!(reader.remaining, 0);
        assert_eq!(reader.largest_buffer.get(), 1);
    }

    #[test]
    fn spans_keep_columns_aligned_and_tint_each_action_type() {
        let row = |key: &str, kind: &str, action: &str| BindingSummary {
            key: key.into(),
            kind: kind.into(),
            action: action.into(),
        };
        let model = CheatsheetModel {
            generation: 0,
            rows: vec![
                row("t", "app", "Ghostty"),
                row("shift+y", "url", "https://youtube.com"),
            ]
            .into(),
        };

        let spans = super::render_spans(&model);
        let text: String = spans.iter().map(|(text, _)| text.as_str()).collect();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines[2], "KEY      TYPE  ACTION");
        assert_eq!(lines[3], "t        app   Ghostty");
        assert_eq!(lines[4], "shift+y  url   https://youtube.com");
        assert!(
            !text.ends_with('\n'),
            "no trailing blank line to pad the window"
        );
        assert!(spans.contains(&("app   ".into(), super::Tint::App)));
        assert!(spans.contains(&("url   ".into(), super::Tint::Url)));
    }

    #[test]
    fn helper_uses_accessory_activation_policy() {
        assert_eq!(
            super::overlay_activation_policy(),
            objc2_app_kit::NSApplicationActivationPolicy::Accessory
        );
    }
}
