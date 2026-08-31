use std::{borrow::Cow, fmt, str::FromStr};

use anyhow::{Result, bail};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Key(Cow<'static, str>);

impl Key {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn from_static(value: &'static str) -> Option<Self> {
        is_key(value).then_some(Self(Cow::Borrowed(value)))
    }
}

impl fmt::Display for Key {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for Key {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let key = normalize_key(value);
        if is_key(&key) {
            Ok(Self(Cow::Owned(key)))
        } else {
            bail!("unknown key `{value}`")
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Modifier {
    Hyper,
    Command,
    Control,
    Option,
    Shift,
    Function,
    LeftCommand,
    RightCommand,
    LeftControl,
    RightControl,
    LeftOption,
    RightOption,
    LeftShift,
    RightShift,
}

impl Modifier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hyper => "hyper",
            Self::Command => "command",
            Self::Control => "control",
            Self::Option => "option",
            Self::Shift => "shift",
            Self::Function => "fn",
            Self::LeftCommand => "left_command",
            Self::RightCommand => "right_command",
            Self::LeftControl => "left_control",
            Self::RightControl => "right_control",
            Self::LeftOption => "left_option",
            Self::RightOption => "right_option",
            Self::LeftShift => "left_shift",
            Self::RightShift => "right_shift",
        }
    }

    fn rank(self) -> usize {
        match self {
            Self::Hyper => 0,
            Self::Command => 1,
            Self::LeftCommand => 2,
            Self::RightCommand => 3,
            Self::Control => 4,
            Self::LeftControl => 5,
            Self::RightControl => 6,
            Self::Option => 7,
            Self::LeftOption => 8,
            Self::RightOption => 9,
            Self::Shift => 10,
            Self::LeftShift => 11,
            Self::RightShift => 12,
            Self::Function => 13,
        }
    }

    pub(crate) fn matches(self, physical: Self) -> bool {
        self == physical
            || matches!(
                (self, physical),
                (Self::Command, Self::LeftCommand | Self::RightCommand)
                    | (Self::Control, Self::LeftControl | Self::RightControl)
                    | (Self::Option, Self::LeftOption | Self::RightOption)
                    | (Self::Shift, Self::LeftShift | Self::RightShift)
            )
    }

    pub(crate) fn is_side_specific(self) -> bool {
        matches!(
            self,
            Self::LeftCommand
                | Self::RightCommand
                | Self::LeftControl
                | Self::RightControl
                | Self::LeftOption
                | Self::RightOption
                | Self::LeftShift
                | Self::RightShift
        )
    }
}

impl FromStr for Modifier {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "hyper" => Ok(Self::Hyper),
            "command" | "cmd" => Ok(Self::Command),
            "control" | "ctrl" => Ok(Self::Control),
            "option" | "alt" => Ok(Self::Option),
            "shift" => Ok(Self::Shift),
            "fn" | "function" => Ok(Self::Function),
            "left_command" | "left_cmd" => Ok(Self::LeftCommand),
            "right_command" | "right_cmd" => Ok(Self::RightCommand),
            "left_control" | "left_ctrl" => Ok(Self::LeftControl),
            "right_control" | "right_ctrl" => Ok(Self::RightControl),
            "left_option" | "left_alt" => Ok(Self::LeftOption),
            "right_option" | "right_alt" => Ok(Self::RightOption),
            "left_shift" => Ok(Self::LeftShift),
            "right_shift" => Ok(Self::RightShift),
            _ => bail!("unknown modifier `{value}`"),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Chord {
    pub modifiers: Vec<Modifier>,
    pub key: Key,
}

impl Chord {
    pub fn new(mut modifiers: Vec<Modifier>, key: Key) -> Self {
        modifiers.sort_by_key(|modifier| modifier.rank());
        modifiers.dedup();
        Self { modifiers, key }
    }

    pub fn has(&self, modifier: Modifier) -> bool {
        self.modifiers.contains(&modifier)
    }
}

impl FromStr for Chord {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let parts: Vec<_> = value.split('+').map(str::trim).collect();
        let (key, modifier_parts) = parts
            .split_last()
            .ok_or_else(|| anyhow::anyhow!("key chord cannot be empty"))?;
        if key.is_empty() {
            bail!("key chord cannot end with `+`");
        }

        let modifiers = modifier_parts
            .iter()
            .map(|part| part.parse())
            .collect::<Result<Vec<_>>>()?;
        Ok(Self::new(modifiers, key.parse()?))
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for modifier in &self.modifiers {
            write!(formatter, "{}+", modifier.as_str())?;
        }
        self.key.fmt(formatter)
    }
}

fn normalize_key(value: &str) -> String {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "esc" => "escape".into(),
        "return" => "enter".into(),
        "left_arrow" => "left".into(),
        "right_arrow" => "right".into(),
        "up_arrow" => "up".into(),
        "down_arrow" => "down".into(),
        "backspace" => "delete".into(),
        other => other.into(),
    }
}

fn is_key(value: &str) -> bool {
    matches!(
        value,
        "caps_lock"
            | "escape"
            | "enter"
            | "tab"
            | "space"
            | "delete"
            | "forward_delete"
            | "left"
            | "right"
            | "up"
            | "down"
            | "home"
            | "end"
            | "page_up"
            | "page_down"
            | "minus"
            | "equal"
            | "left_bracket"
            | "right_bracket"
            | "backslash"
            | "semicolon"
            | "quote"
            | "comma"
            | "period"
            | "slash"
            | "grave"
    ) || value.len() == 1 && value.as_bytes()[0].is_ascii_alphanumeric()
        || value
            .strip_prefix('f')
            .and_then(|number| number.parse::<u8>().ok())
            .is_some_and(|number| (1..=20).contains(&number))
}
