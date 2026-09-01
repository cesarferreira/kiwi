use std::str::FromStr;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    config::{CompiledConfig, chord_matches},
    key::Chord,
};

const CATALOG: &str = include_str!("../data/shortcut_catalog.toml");

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub chord: String,
    pub source: String,
    pub label: String,
    pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    shortcuts: Vec<CatalogEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Conflict {
    pub shortcut: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub action: String,
    pub source: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

pub fn catalog() -> Result<Vec<CatalogEntry>> {
    let catalog: Catalog = toml::from_str(CATALOG).context("invalid bundled shortcut catalog")?;
    for entry in &catalog.shortcuts {
        Chord::from_str(&entry.chord).with_context(|| {
            format!(
                "invalid chord `{}` in bundled shortcut catalog",
                entry.chord
            )
        })?;
    }
    Ok(catalog.shortcuts)
}

pub fn find_conflicts(config: &CompiledConfig) -> Result<Vec<Conflict>> {
    let catalog = catalog()?
        .into_iter()
        .map(|entry| {
            let chord = Chord::from_str(&entry.chord)
                .expect("catalog chords were validated while loading the catalog");
            (entry, chord)
        })
        .collect::<Vec<_>>();
    let mut conflicts = Vec::new();

    for (shortcut, action) in &config.bindings {
        let configured = Chord::from_str(shortcut)
            .expect("compiled binding shortcuts are normalized valid chords");
        for (entry, catalog_chord) in &catalog {
            if catalog_matches(catalog_chord, &configured) {
                let (kind, action) = action.type_and_value();
                conflicts.push(Conflict {
                    shortcut: shortcut.clone(),
                    kind,
                    action,
                    source: entry.source.clone(),
                    label: entry.label.clone(),
                    url: entry.url.clone(),
                });
            }
        }
    }

    Ok(conflicts)
}

fn catalog_matches(catalog: &Chord, configured: &Chord) -> bool {
    chord_matches(catalog, configured)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{catalog, catalog_matches};

    #[test]
    fn bundled_catalog_is_valid_and_covers_every_required_source() {
        let catalog = catalog().unwrap();
        let sources: BTreeSet<_> = catalog.iter().map(|entry| entry.source.as_str()).collect();

        for source in [
            "macos", "safari", "chrome", "finder", "terminal", "ghostty", "slack",
        ] {
            assert!(sources.contains(source), "missing catalog source {source}");
        }
        assert!(catalog.iter().all(|entry| !entry.label.trim().is_empty()));
    }

    #[test]
    fn generic_catalog_modifier_matches_either_physical_side() {
        let catalog = "command+space".parse().unwrap();

        assert!(catalog_matches(
            &catalog,
            &"left_command+space".parse().unwrap()
        ));
        assert!(catalog_matches(
            &catalog,
            &"right_command+space".parse().unwrap()
        ));
    }
}
