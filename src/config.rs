use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::Path,
    str::FromStr,
    sync::Arc,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer, Serialize, de};

use crate::key::{Chord, Key, Modifier};

const MAX_BINDING_SUMMARY_ROWS: usize = 64;
const MAX_BINDING_SUMMARY_TEXT_CHARS: usize = 160;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BindingSummary {
    pub key: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub action: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    App(AppAction),
    OpenUrl(String),
    RunCommand(String),
    SendKeys(Chord),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppAction {
    pub target: String,
    pub behavior: AppBehavior,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AppBehavior {
    #[default]
    Launch,
    Toggle,
    Hide,
    Cycle,
    NewWindow,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FeedbackPolicy {
    Off,
    #[default]
    Errors,
    All,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FeedbackStyle {
    #[default]
    Notification,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    #[serde(deserialize_with = "deserialize_feedback_policy")]
    pub feedback: FeedbackPolicy,
    #[serde(deserialize_with = "deserialize_feedback_style")]
    pub style: FeedbackStyle,
    pub cheatsheet: bool,
    pub cheatsheet_delay_ms: u64,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            feedback: FeedbackPolicy::Errors,
            style: FeedbackStyle::Notification,
            cheatsheet: true,
            cheatsheet_delay_ms: 1000,
        }
    }
}

impl FromStr for AppBehavior {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "launch" => Ok(Self::Launch),
            "toggle" => Ok(Self::Toggle),
            "hide" => Ok(Self::Hide),
            "cycle" => Ok(Self::Cycle),
            "new_window" => Ok(Self::NewWindow),
            other => bail!(
                "unknown app behavior `{other}`; expected `launch`, `toggle`, `hide`, `cycle`, or `new_window`"
            ),
        }
    }
}

impl Action {
    pub fn type_and_value(&self) -> (&'static str, String) {
        match self {
            Self::App(action) => ("app", action.display_value()),
            Self::OpenUrl(value) => ("url", value.clone()),
            Self::RunCommand(value) => ("command", value.clone()),
            Self::SendKeys(value) => ("keys", value.to_string()),
        }
    }
}

impl AppAction {
    fn display_value(&self) -> String {
        match self.behavior {
            AppBehavior::Launch => self.target.clone(),
            AppBehavior::Toggle => format!("{} (toggle)", self.target),
            AppBehavior::Hide => format!("{} (hide)", self.target),
            AppBehavior::Cycle => format!("{} (cycle)", self.target),
            AppBehavior::NewWindow => format!("{} (new window)", self.target),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub hyper: HyperSpec,
    #[serde(default)]
    pub bindings: BTreeMap<String, BindingSpec>,
    #[serde(default)]
    pub ui: UiConfig,
}

impl Config {
    pub fn from_toml(contents: &str) -> Result<Self> {
        toml::from_str(contents).context("invalid TOML config")
    }

    pub fn from_path(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("could not read config {}", path.display()))?;
        Self::from_toml(&contents)
    }

    pub fn compile(self) -> Result<CompiledConfig> {
        if self.ui.cheatsheet_delay_ms > 5000 {
            bail!("`ui.cheatsheet_delay_ms` must be in 0..=5000");
        }
        let hyper = self.hyper.compile()?;
        let mut bindings = BTreeMap::new();
        let mut compiled_bindings: HashMap<Key, Vec<(Chord, Action)>> = HashMap::new();
        let mut hyper_binding_summaries = Vec::new();

        for (source, binding) in self.bindings {
            if !binding.enabled {
                continue;
            }
            let chord = Chord::from_str(&source)
                .map_err(|error| anyhow::anyhow!("invalid binding `{source}`: {error:#}"))?;
            let normalized = chord.to_string();
            if bindings.contains_key(&normalized) {
                bail!("duplicate binding `{normalized}` after normalization");
            }
            let action = binding
                .compile()
                .map_err(|error| anyhow::anyhow!("invalid binding `{source}`: {error:#}"))?;
            if chord.has(Modifier::Hyper) {
                hyper_binding_summaries
                    .push((normalized.clone(), binding_summary(&chord, &action)));
            }
            bindings.insert(normalized, action.clone());
            compiled_bindings
                .entry(chord.key.clone())
                .or_default()
                .push((chord, action));
        }

        hyper_binding_summaries.sort_by(|left, right| left.0.cmp(&right.0));
        if self.ui.cheatsheet && hyper_binding_summaries.len() > MAX_BINDING_SUMMARY_ROWS {
            bail!(
                "`[ui].cheatsheet` requires at most {MAX_BINDING_SUMMARY_ROWS} enabled Hyper bindings; found {}",
                hyper_binding_summaries.len()
            );
        }
        let hyper_binding_summary = hyper_binding_summaries
            .into_iter()
            .map(|(_, summary)| summary)
            .collect::<Vec<_>>()
            .into();

        Ok(CompiledConfig {
            hyper,
            bindings,
            ui: self.ui,
            compiled_bindings,
            hyper_binding_summary,
        })
    }
}

#[derive(Clone, Debug)]
pub struct CompiledConfig {
    pub hyper: Hyper,
    pub bindings: BTreeMap<String, Action>,
    pub ui: UiConfig,
    compiled_bindings: HashMap<Key, Vec<(Chord, Action)>>,
    hyper_binding_summary: Arc<[BindingSummary]>,
}

impl CompiledConfig {
    pub fn hyper_binding_summary(&self) -> Arc<[BindingSummary]> {
        Arc::clone(&self.hyper_binding_summary)
    }

    pub(crate) fn action_for(&self, actual: &Chord) -> Option<&Action> {
        self.compiled_bindings
            .get(&actual.key)?
            .iter()
            .filter(|(binding, _)| chord_matches(binding, actual))
            .max_by_key(|(binding, _)| {
                binding
                    .modifiers
                    .iter()
                    .filter(|modifier| modifier.is_side_specific())
                    .count()
            })
            .map(|(_, action)| action)
    }
}

fn binding_summary(chord: &Chord, action: &Action) -> BindingSummary {
    let mut parts: Vec<_> = chord
        .modifiers
        .iter()
        .filter(|modifier| **modifier != Modifier::Hyper)
        .map(|modifier| modifier.as_str())
        .collect();
    parts.push(chord.key.as_str());
    let (kind, action) = action.type_and_value();
    BindingSummary {
        key: parts.join("+"),
        kind: bounded_summary_text(kind),
        action: bounded_summary_text(&action),
    }
}

fn bounded_summary_text(value: &str) -> String {
    let mut output: String = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(MAX_BINDING_SUMMARY_TEXT_CHARS)
        .collect();
    if value.chars().count() > MAX_BINDING_SUMMARY_TEXT_CHARS {
        output.pop();
        output.push('…');
    }
    output
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HyperSpec {
    pub key: String,
    pub tap: String,
    pub modifiers: Vec<String>,
}

impl Default for HyperSpec {
    fn default() -> Self {
        Self {
            key: "caps_lock".into(),
            tap: "escape".into(),
            modifiers: vec![
                "command".into(),
                "control".into(),
                "option".into(),
                "shift".into(),
            ],
        }
    }
}

impl HyperSpec {
    fn compile(self) -> Result<Hyper> {
        let key = self.key.parse().context("invalid hyper key")?;
        let tap = self.tap.parse().context("invalid hyper tap action")?;
        let modifiers = self
            .modifiers
            .iter()
            .map(|modifier| Modifier::from_str(modifier))
            .collect::<Result<Vec<_>>>()
            .context("invalid hyper modifier")?;
        if modifiers.contains(&Modifier::Hyper) {
            bail!("hyper modifiers cannot contain `hyper`");
        }
        Ok(Hyper {
            key,
            tap,
            modifiers,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Hyper {
    pub key: Key,
    pub tap: Chord,
    pub modifiers: Vec<Modifier>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingSpec {
    pub app: Option<String>,
    pub behavior: Option<String>,
    pub url: Option<String>,
    pub command: Option<String>,
    pub keys: Option<String>,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

impl BindingSpec {
    fn compile(self) -> Result<Action> {
        let count = [
            self.app.is_some(),
            self.url.is_some(),
            self.command.is_some(),
            self.keys.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();
        if count != 1 {
            bail!("a binding must define exactly one of `app`, `url`, `command`, or `keys`");
        }

        if self.behavior.is_some() && self.app.is_none() {
            bail!("`behavior` is only valid with `app`");
        }

        if let Some(app) = self.app {
            require_nonempty("app", &app)?;
            let behavior = self
                .behavior
                .as_deref()
                .unwrap_or("launch")
                .parse::<AppBehavior>()?;
            return Ok(Action::App(AppAction {
                target: app,
                behavior,
            }));
        }
        if let Some(url) = self.url {
            require_nonempty("url", &url)?;
            return Ok(Action::OpenUrl(url));
        }
        if let Some(command) = self.command {
            require_nonempty("command", &command)?;
            return Ok(Action::RunCommand(command));
        }
        let keys = self.keys.expect("exactly one action was present");
        let chord: Chord = keys.parse().context("invalid `keys` action")?;
        if chord.has(Modifier::Hyper) {
            bail!("a `keys` action cannot emit the virtual `hyper` modifier");
        }
        Ok(Action::SendKeys(chord))
    }
}

fn enabled_by_default() -> bool {
    true
}

fn require_nonempty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("`{field}` cannot be empty");
    }
    Ok(())
}

fn deserialize_feedback_policy<'de, D>(deserializer: D) -> Result<FeedbackPolicy, D::Error>
where
    D: Deserializer<'de>,
{
    match String::deserialize(deserializer)?.as_str() {
        "off" => Ok(FeedbackPolicy::Off),
        "errors" => Ok(FeedbackPolicy::Errors),
        "all" => Ok(FeedbackPolicy::All),
        other => Err(de::Error::custom(format!(
            "unknown feedback `{other}`; expected `off`, `errors`, or `all`"
        ))),
    }
}

fn deserialize_feedback_style<'de, D>(deserializer: D) -> Result<FeedbackStyle, D::Error>
where
    D: Deserializer<'de>,
{
    match String::deserialize(deserializer)?.as_str() {
        "notification" => Ok(FeedbackStyle::Notification),
        other => Err(de::Error::custom(format!(
            "unknown feedback style `{other}`; expected `notification`"
        ))),
    }
}

pub(crate) fn chord_matches(binding: &Chord, actual: &Chord) -> bool {
    binding.key == actual.key
        && binding.modifiers.iter().all(|wanted| {
            actual
                .modifiers
                .iter()
                .any(|physical| wanted.matches(*physical))
        })
        && actual.modifiers.iter().all(|physical| {
            binding
                .modifiers
                .iter()
                .any(|wanted| wanted.matches(*physical))
        })
}
