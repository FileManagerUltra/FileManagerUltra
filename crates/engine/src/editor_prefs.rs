//! Editor preferences and the file kinds the internal/external choice is made per
//! (capabilities `text-editor` and `editor-open-preference`).

use fm_editor::{DEFAULT_MAX_STEPS, MAX_STEPS, MIN_STEPS};
use fm_vfs::{FmError, VfsUri};
use serde::{Deserialize, Serialize};

/// Which editor a kind of file opens in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditorTarget {
    /// The built-in editor (Edit mode of a Preview pane).
    #[default]
    Internal,
    /// The system default application.
    External,
}

/// File kinds the open preference distinguishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorKind {
    Text,
    Code,
    Markup,
    Config,
    Log,
}

impl EditorKind {
    pub const ALL: [Self; 5] = [
        Self::Text,
        Self::Code,
        Self::Markup,
        Self::Config,
        Self::Log,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Code => "code",
            Self::Markup => "markup",
            Self::Config => "config",
            Self::Log => "log",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.key() == key)
    }

    /// The kind of a file from its name, or `None` for names that are not known to be text
    /// (those keep opening in the default app on Enter).
    pub fn of_name(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();
        if let Some(lang) = fm_editor::detect_lang(&lower, "") {
            use fm_editor::Lang::*;
            return Some(match lang {
                Markdown | Html | Xml | Css => Self::Markup,
                Toml | Json | Yaml => Self::Config,
                _ => Self::Code,
            });
        }
        // Rotated logs too (`app.log.1`, `app.log-20240101`), as the preview sees them;
        // gzipped ones are not text to edit.
        if fm_preview::sniff::is_log_name(&lower) && !lower.ends_with(".gz") {
            return Some(Self::Log);
        }
        let (stem, ext) = lower.rsplit_once('.').unwrap_or((lower.as_str(), ""));
        Some(match ext {
            "out" => Self::Log,
            "ini" | "cfg" | "conf" | "env" | "properties" | "editorconfig" | "gitignore"
            | "gitattributes" | "lock" => Self::Config,
            "txt" | "text" | "csv" | "tsv" | "rst" | "tex" | "diff" | "patch" => Self::Text,
            "" if matches!(
                stem,
                "readme" | "license" | "changelog" | "authors" | "notice" | "todo"
            ) =>
            {
                Self::Text
            }
            "" if matches!(stem, "makefile" | "dockerfile" | "cmakelists") => Self::Code,
            "" if stem.starts_with('.') => Self::Config,
            _ => return None,
        })
    }

    pub fn of_uri(uri: &VfsUri) -> Option<Self> {
        Self::of_name(uri.file_name()?)
    }
}

/// The internal/external choice per kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorOpen {
    pub text: EditorTarget,
    pub code: EditorTarget,
    pub markup: EditorTarget,
    pub config: EditorTarget,
    pub log: EditorTarget,
}

impl EditorOpen {
    pub fn get(&self, kind: EditorKind) -> EditorTarget {
        match kind {
            EditorKind::Text => self.text,
            EditorKind::Code => self.code,
            EditorKind::Markup => self.markup,
            EditorKind::Config => self.config,
            EditorKind::Log => self.log,
        }
    }

    pub fn set(&mut self, kind: EditorKind, target: EditorTarget) {
        match kind {
            EditorKind::Text => self.text = target,
            EditorKind::Code => self.code = target,
            EditorKind::Markup => self.markup = target,
            EditorKind::Config => self.config = target,
            EditorKind::Log => self.log = target,
        }
    }
}

pub const DEFAULT_SYNTAX_MAX_MIB: u64 = 8;
pub const MAX_SYNTAX_MIB: u64 = 64;
/// Editor line height in percent of the font's: roomy by default, adjustable for reading.
pub const DEFAULT_LINE_SPACING: u16 = 145;
pub const MIN_LINE_SPACING: u16 = 100;
pub const MAX_LINE_SPACING: u16 = 200;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorPrefs {
    pub open: EditorOpen,
    /// Save a dirty buffer when the editor loses focus.
    pub autosave: bool,
    /// How many undo steps are remembered (10–100000).
    pub undo_steps: usize,
    pub wrap: bool,
    pub line_numbers: bool,
    /// Syntax colouring on/off.
    pub syntax: bool,
    /// Files above this many MiB are shown without colouring.
    pub syntax_max_mib: u64,
    /// Colour brackets by nesting depth.
    pub rainbow: bool,
    /// Line height as a percentage of the font's own (100-200).
    pub line_spacing_pct: u16,
    /// Colours for code, in the editor and the code preview.
    pub scheme: EditorScheme,
}

/// Colour scheme for code (editor and code preview); each has a dark and a light
/// version that follow the app's theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditorScheme {
    /// The app's own palette.
    #[default]
    App,
    /// Soft grey background, orange keywords, green strings.
    Warm,
    /// At least 7:1 contrast for every colour, for low vision.
    HighContrast,
}

impl EditorScheme {
    pub const ALL: [EditorScheme; 3] = [Self::App, Self::Warm, Self::HighContrast];

    pub fn id(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Warm => "warm",
            Self::HighContrast => "high_contrast",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::App => "App colours",
            Self::Warm => "Warm",
            Self::HighContrast => "High contrast",
        }
    }

    pub fn of_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id() == id)
    }
}

impl Default for EditorPrefs {
    fn default() -> Self {
        Self {
            open: EditorOpen::default(),
            autosave: true,
            undo_steps: DEFAULT_MAX_STEPS,
            wrap: false,
            line_numbers: true,
            syntax: true,
            syntax_max_mib: DEFAULT_SYNTAX_MAX_MIB,
            rainbow: true,
            line_spacing_pct: DEFAULT_LINE_SPACING,
            scheme: EditorScheme::App,
        }
    }
}

impl EditorPrefs {
    /// The undo depth, safe against hand-edited values.
    /// Line height as a factor of the font's, clamped to the allowed range.
    pub fn line_spacing(&self) -> f32 {
        f32::from(
            self.line_spacing_pct
                .clamp(MIN_LINE_SPACING, MAX_LINE_SPACING),
        ) / 100.0
    }

    pub fn undo_steps(&self) -> usize {
        self.undo_steps.clamp(MIN_STEPS, MAX_STEPS)
    }

    /// The size above which files are not coloured, in bytes.
    pub fn syntax_max_bytes(&self) -> usize {
        let mib = self.syntax_max_mib.clamp(1, MAX_SYNTAX_MIB);
        usize::try_from(mib << 20).unwrap_or(fm_editor::SYNTAX_MAX_BYTES)
    }
}

fn bad(message: impl Into<String>) -> FmError {
    FmError::InvalidParams(message.into())
}

/// A one-key object parameter, or an error naming what was expected.
pub(crate) fn one_field<'a>(
    params: &'a serde_json::Value,
    key: &str,
    expected: &str,
) -> Result<&'a serde_json::Value, FmError> {
    let map = params
        .as_object()
        .ok_or_else(|| bad(format!("expected {{{key}: {expected}}}")))?;
    if map.keys().any(|k| k != key) {
        return Err(bad(format!("only {key} is accepted")));
    }
    map.get(key)
        .ok_or_else(|| bad(format!("{key}: {expected} required")))
}

pub fn specs() -> Vec<crate::CommandSpec> {
    use crate::CommandSpec;
    vec![
        CommandSpec {
            id: "prefs.editor_open",
            label: "Open Text Files In…",
            menu: &["Edit", "Preferences", "Open text files in"],
            shortcut: None,
            params: "{kind: \"text\"|\"code\"|\"markup\"|\"config\"|\"log\"|\"all\", target: \"internal\"|\"external\"}: which editor Enter, double-click and F4 use",
            enabled: |_| true,
            run: |s, params| {
                let map = params
                    .as_object()
                    .ok_or_else(|| bad("expected {kind, target}"))?;
                if map.keys().any(|k| k != "kind" && k != "target") {
                    return Err(bad("only kind and target are accepted"));
                }
                let text = |k: &str| {
                    map.get(k)
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| bad(format!("{k} must be a string")))
                };
                let target = match text("target")? {
                    "internal" => EditorTarget::Internal,
                    "external" => EditorTarget::External,
                    _ => return Err(bad("target must be internal or external")),
                };
                let kinds: Vec<EditorKind> = match text("kind")? {
                    "all" => EditorKind::ALL.to_vec(),
                    key => vec![EditorKind::from_key(key).ok_or_else(|| {
                        bad("kind must be text, code, markup, config, log or all")
                    })?],
                };
                for kind in kinds {
                    s.prefs.editor.open.set(kind, target);
                }
                Ok(())
            },
        },
        CommandSpec {
            id: "prefs.editor_autosave",
            label: "Autosave on Focus Loss",
            menu: &["Edit", "Preferences", "Autosave editor on focus loss"],
            shortcut: None,
            params: "{enabled: bool}: save a dirty editor buffer when it loses focus",
            enabled: |_| true,
            run: |s, params| {
                let v = one_field(params, "enabled", "bool")?;
                s.prefs.editor.autosave = v.as_bool().ok_or_else(|| bad("enabled: bool"))?;
                Ok(())
            },
        },
        CommandSpec {
            id: "prefs.editor_undo_steps",
            label: "Editor Undo Steps",
            menu: &["Edit", "Preferences", "Editor undo steps"],
            shortcut: None,
            params: "{steps: integer 10-100000}: how many undo steps the editor remembers",
            enabled: |_| true,
            run: |s, params| {
                let v = one_field(params, "steps", "integer")?;
                let n = v
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|n| (MIN_STEPS..=MAX_STEPS).contains(n))
                    .ok_or_else(|| bad(format!("steps: integer {MIN_STEPS}-{MAX_STEPS}")))?;
                s.prefs.editor.undo_steps = n;
                s.apply_editor_undo_steps();
                Ok(())
            },
        },
        CommandSpec {
            id: "prefs.editor_scheme",
            label: "Editor Colour Scheme",
            menu: &["Edit", "Preferences", "Editor colour scheme"],
            shortcut: None,
            params: "{scheme: \"app\" | \"warm\" | \"high_contrast\"}: colours for code in the editor and the code preview",
            enabled: |_| true,
            run: |s, params| {
                let v = one_field(params, "scheme", "string")?;
                let scheme = v
                    .as_str()
                    .and_then(EditorScheme::of_id)
                    .ok_or_else(|| bad("scheme: \"app\", \"warm\" or \"high_contrast\""))?;
                s.prefs.editor.scheme = scheme;
                Ok(())
            },
        },
        CommandSpec {
            id: "prefs.editor_line_spacing",
            label: "Editor Line Spacing",
            menu: &["Edit", "Preferences", "Editor line spacing"],
            shortcut: None,
            params: "{percent: integer 100-200}: the editor's line height as a percentage of the font's",
            enabled: |_| true,
            run: |s, params| {
                let v = one_field(params, "percent", "integer")?;
                let n = v
                    .as_u64()
                    .and_then(|n| u16::try_from(n).ok())
                    .filter(|n| (MIN_LINE_SPACING..=MAX_LINE_SPACING).contains(n))
                    .ok_or_else(|| {
                        bad(format!(
                            "percent: integer {MIN_LINE_SPACING}-{MAX_LINE_SPACING}"
                        ))
                    })?;
                s.prefs.editor.line_spacing_pct = n;
                Ok(())
            },
        },
        CommandSpec {
            id: "prefs.editor_view",
            label: "Editor View Defaults",
            menu: &["Edit", "Preferences", "Editor View Defaults"],
            shortcut: None,
            params: "{wrap?: bool, line_numbers?: bool}: how newly opened files are shown",
            enabled: |_| true,
            run: |s, params| {
                if params.is_null() {
                    return Ok(());
                }
                let map = params
                    .as_object()
                    .ok_or_else(|| bad("expected {wrap?, line_numbers?}"))?;
                if map.is_empty() || map.keys().any(|k| k != "wrap" && k != "line_numbers") {
                    return Err(bad("only wrap and line_numbers are accepted"));
                }
                let flag = |k: &str| {
                    map.get(k)
                        .map(|v| v.as_bool().ok_or_else(|| bad(format!("{k}: bool"))))
                        .transpose()
                };
                let (wrap, numbers) = (flag("wrap")?, flag("line_numbers")?);
                if let Some(w) = wrap {
                    s.prefs.editor.wrap = w;
                }
                if let Some(n) = numbers {
                    s.prefs.editor.line_numbers = n;
                }
                Ok(())
            },
        },
        CommandSpec {
            id: "prefs.editor_syntax",
            label: "Editor Syntax Colouring",
            menu: &["Edit", "Preferences", "Editor syntax colouring"],
            shortcut: None,
            params: "{enabled?: bool, max_mib?: integer 1-64}: colouring on/off and the size above which files stay plain",
            enabled: |_| true,
            run: |s, params| {
                if params.is_null() {
                    return Ok(());
                }
                let map = params
                    .as_object()
                    .ok_or_else(|| bad("expected {enabled?, max_mib?}"))?;
                if map.keys().any(|k| k != "enabled" && k != "max_mib") || map.is_empty() {
                    return Err(bad("only enabled and max_mib are accepted"));
                }
                let enabled = map
                    .get("enabled")
                    .map(|v| v.as_bool().ok_or_else(|| bad("enabled: bool")))
                    .transpose()?;
                let max = map
                    .get("max_mib")
                    .map(|v| {
                        v.as_u64()
                            .filter(|n| (1..=MAX_SYNTAX_MIB).contains(n))
                            .ok_or_else(|| bad(format!("max_mib: integer 1-{MAX_SYNTAX_MIB}")))
                    })
                    .transpose()?;
                if let Some(e) = enabled {
                    s.prefs.editor.syntax = e;
                }
                if let Some(m) = max {
                    s.prefs.editor.syntax_max_mib = m;
                }
                s.editor_syntax_prefs_changed();
                Ok(())
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::*;
    use crate::Session;
    use serde_json::json;

    #[test]
    fn kinds_come_from_names() {
        assert_eq!(EditorKind::of_name("main.rs"), Some(EditorKind::Code));
        assert_eq!(EditorKind::of_name("a.TOML"), Some(EditorKind::Config));
        assert_eq!(EditorKind::of_name("README"), Some(EditorKind::Text));
        assert_eq!(EditorKind::of_name("app.log"), Some(EditorKind::Log));
        assert_eq!(
            EditorKind::of_name("alternatives.log.1"),
            Some(EditorKind::Log)
        );
        assert_eq!(
            EditorKind::of_name("app.log-20240101"),
            Some(EditorKind::Log)
        );
        assert_ne!(EditorKind::of_name("app.log.2.gz"), Some(EditorKind::Log));
        assert_eq!(EditorKind::of_name("page.html"), Some(EditorKind::Markup));
        assert_eq!(EditorKind::of_name("photo.jpg"), None);
        assert_eq!(EditorKind::of_name("blob.bin"), None);
    }

    #[test]
    fn open_preference_sets_one_kind_or_all() {
        let mut s = Session::new().expect("session");
        s.dispatch(
            "prefs.editor_open",
            &json!({"kind": "log", "target": "external"}),
        )
        .expect("set");
        assert_eq!(s.prefs.editor.open.log, EditorTarget::External);
        assert_eq!(s.prefs.editor.open.code, EditorTarget::Internal);
        s.dispatch(
            "prefs.editor_open",
            &json!({"kind": "all", "target": "external"}),
        )
        .expect("set all");
        assert!(
            EditorKind::ALL
                .iter()
                .all(|k| s.prefs.editor.open.get(*k) == EditorTarget::External)
        );
    }

    #[test]
    fn bad_params_leave_prefs_unchanged() {
        let mut s = Session::new().expect("session");
        let before = s.prefs.editor.clone();
        for (id, bad) in [
            (
                "prefs.editor_open",
                json!({"kind": "nope", "target": "external"}),
            ),
            (
                "prefs.editor_open",
                json!({"kind": "log", "target": "maybe"}),
            ),
            ("prefs.editor_open", json!({"kind": "log"})),
            ("prefs.editor_open", json!("log")),
            ("prefs.editor_open", json!(null)),
            ("prefs.editor_autosave", json!({"enabled": "yes"})),
            ("prefs.editor_autosave", json!({})),
            ("prefs.editor_undo_steps", json!({"steps": 5})),
            ("prefs.editor_undo_steps", json!({"steps": 1_000_000})),
            ("prefs.editor_undo_steps", json!({"steps": "many"})),
            ("prefs.editor_undo_steps", json!({"steps": -1})),
            ("prefs.editor_syntax", json!({"max_mib": 0})),
            ("prefs.editor_line_spacing", json!({"percent": 99})),
            ("prefs.editor_line_spacing", json!({"percent": 300})),
            ("prefs.editor_line_spacing", json!({"percent": 1.5})),
            ("prefs.editor_line_spacing", json!({})),
            ("prefs.editor_scheme", json!({"scheme": "islands"})),
            ("prefs.editor_scheme", json!({"scheme": 2})),
            ("prefs.editor_scheme", json!({})),
            ("prefs.editor_view", json!({})),
            ("prefs.editor_view", json!({"wrap": 1})),
            ("prefs.editor_syntax", json!({})),
        ] {
            assert!(
                matches!(s.dispatch(id, &bad), Err(FmError::InvalidParams(_))),
                "{id} {bad}"
            );
        }
        assert_eq!(s.prefs.editor, before);
    }

    #[test]
    fn autosave_and_undo_steps_are_settable_and_clamped_when_read() {
        let mut s = Session::new().expect("session");
        s.dispatch("prefs.editor_autosave", &json!({"enabled": false}))
            .expect("off");
        assert!(!s.prefs.editor.autosave);
        s.dispatch("prefs.editor_undo_steps", &json!({"steps": 50}))
            .expect("steps");
        assert_eq!(s.prefs.editor.undo_steps(), 50);
        s.prefs.editor.undo_steps = 3;
        assert_eq!(s.prefs.editor.undo_steps(), MIN_STEPS);
        s.dispatch("prefs.editor_line_spacing", &json!({"percent": 160}))
            .expect("spacing");
        assert!((s.prefs.editor.line_spacing() - 1.6).abs() < 1e-6);
        s.prefs.editor.line_spacing_pct = 900;
        assert!(
            (s.prefs.editor.line_spacing() - 2.0).abs() < 1e-6,
            "clamped when read"
        );
        s.dispatch("prefs.editor_scheme", &json!({"scheme": "high_contrast"}))
            .expect("scheme");
        assert_eq!(s.prefs.editor.scheme, EditorScheme::HighContrast);
        let saved = serde_json::to_string(&s.prefs.editor).expect("json");
        assert!(saved.contains("\"high_contrast\""), "{saved}");
    }
}
