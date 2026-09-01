use kiwi_keymapper::{
    config::Config,
    conflicts::{catalog, find_conflicts},
};

fn compiled(binding: &str) -> kiwi_keymapper::config::CompiledConfig {
    Config::from_toml(&format!(
        r#"
[bindings]
{binding}
"#
    ))
    .unwrap()
    .compile()
    .unwrap()
}

#[test]
fn bundled_catalog_parses_and_covers_required_sources() {
    let catalog = catalog().unwrap();
    let sources: std::collections::BTreeSet<_> =
        catalog.iter().map(|entry| entry.source.as_str()).collect();

    assert!(catalog.len() >= 20);
    for source in [
        "macos", "safari", "chrome", "finder", "terminal", "ghostty", "slack",
    ] {
        assert!(sources.contains(source), "missing catalog source {source}");
    }
    assert!(catalog.iter().all(|entry| !entry.label.trim().is_empty()));
}

#[test]
fn generic_catalog_chord_matches_generic_and_side_specific_bindings() {
    for shortcut in ["command+space", "left_command+space"] {
        let conflicts = find_conflicts(&compiled(&format!(
            r#""{shortcut}" = {{ app = "Finder" }}"#
        )))
        .unwrap();

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].shortcut, shortcut);
        assert_eq!(conflicts[0].source, "macos");
        assert_eq!(conflicts[0].label, "Spotlight");
    }
}

#[test]
fn screenshot_binding_matches_catalog() {
    let conflicts =
        find_conflicts(&compiled(r#""command+shift+3" = { command = "capture" }"#)).unwrap();

    assert!(
        conflicts.iter().any(|conflict| {
            conflict.source == "macos" && conflict.label.contains("Screenshot")
        })
    );
}

#[test]
fn a_chord_shared_by_several_apps_reports_every_row_in_catalog_order() {
    let conflicts = find_conflicts(&compiled(r#""command+t" = { app = "Arc" }"#)).unwrap();

    let rows: Vec<_> = conflicts
        .iter()
        .map(|conflict| (conflict.source.as_str(), conflict.label.as_str()))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("safari", "New tab"),
            ("chrome", "New tab"),
            ("ghostty", "New tab"),
        ]
    );
    assert!(
        conflicts
            .iter()
            .all(|conflict| conflict.shortcut == "command+t")
    );
}

#[test]
fn hyper_binding_does_not_expand_or_inspect_emitted_keys() {
    let conflicts = find_conflicts(&compiled(r#""hyper+t" = { keys = "command+space" }"#)).unwrap();

    assert!(conflicts.is_empty());
}
