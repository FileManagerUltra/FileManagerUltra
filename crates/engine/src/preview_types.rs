//! User-configurable filename associations, in deterministic priority order.

use std::collections::BTreeMap;

use fm_preview::{Format, Kind};
use fm_vfs::FmError;
use serde::{Deserialize, Serialize};

use crate::glob::Glob;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PreviewType {
    Logs,
    Images,
    Video,
    Audio,
    Pdf,
    Markdown,
    Json,
    Yaml,
    Toml,
    Xml,
    Html,
    Csv,
    Text,
}

impl PreviewType {
    /// Also the matching priority shown in Preferences.
    pub const ALL: [Self; 13] = [
        Self::Logs,
        Self::Images,
        Self::Video,
        Self::Audio,
        Self::Pdf,
        Self::Markdown,
        Self::Json,
        Self::Yaml,
        Self::Toml,
        Self::Xml,
        Self::Html,
        Self::Csv,
        Self::Text,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Logs => "Logs",
            Self::Images => "Images",
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Pdf => "PDF",
            Self::Markdown => "Markdown",
            Self::Json => "JSON",
            Self::Yaml => "YAML",
            Self::Toml => "TOML",
            Self::Xml => "XML",
            Self::Html => "HTML",
            Self::Csv => "CSV",
            Self::Text => "Text",
        }
    }

    pub fn default_patterns(self) -> Vec<String> {
        self.default_text()
            .split("; ")
            .map(str::to_string)
            .collect()
    }

    fn default_text(self) -> String {
        match self {
            Self::Logs => {
                let mut patterns = vec!["*.log".to_string(), "*.log.gz".into(), "*.log.[0-9]*".into(), "*.log-[0-9]*".into()];
                for base in ["syslog", "messages", "secure", "maillog", "cron", "debug", "dmesg"] {
                    for suffix in ["", ".gz", ".[0-9]*", "-[0-9]*"] {
                        patterns.push(format!("{base}{suffix}"));
                    }
                }
                patterns.join("; ")
            }
            Self::Images => "*.png; *.jpg; *.jpeg; *.gif; *.webp; *.bmp; *.ico".into(),
            Self::Video => "*.mp4; *.m4v; *.mkv; *.mov; *.avi; *.webm; *.wmv; *.flv; *.mpg; *.mpeg; *.3gp; *.ogv".into(),
            Self::Audio => "*.mp3; *.flac; *.ogg; *.wav; *.m4a; *.aac".into(),
            Self::Pdf => "*.pdf".into(),
            Self::Markdown => "*.md; *.markdown".into(),
            Self::Json => "*.json; *.geojson; *.webmanifest".into(),
            Self::Yaml => "*.yaml; *.yml".into(),
            Self::Toml => "*.toml".into(),
            Self::Xml => "*.xml; *.svg; *.xsd; *.xsl; *.xslt; *.plist; *.xhtml; *.csproj; *.wsdl".into(),
            Self::Html => "*.html; *.htm".into(),
            Self::Csv => "*.csv; *.tsv".into(),
            Self::Text => "*.txt; *.text".into(),
        }
    }

    fn kind(self) -> Kind {
        match self {
            Self::Logs => Kind::Log,
            Self::Images => Kind::Image("image"),
            Self::Video => Kind::Video,
            Self::Audio => Kind::Audio,
            Self::Pdf => Kind::Pdf,
            Self::Markdown => Kind::Markdown,
            Self::Json => Kind::Structured(Format::Json),
            Self::Yaml => Kind::Structured(Format::Yaml),
            Self::Toml => Kind::Structured(Format::Toml),
            Self::Xml => Kind::Structured(Format::Xml),
            Self::Html => Kind::Text("html".into()),
            Self::Csv => Kind::Text("csv".into()),
            Self::Text => Kind::Text("text".into()),
        }
    }
}

/// One file type and its filename patterns, one pattern per entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilterGroup {
    #[serde(rename = "type")]
    pub kind: PreviewType,
    pub patterns: Vec<String>,
}

/// Ordered groups: earlier groups win when a filename matches several types.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PreviewFilters(pub Vec<FilterGroup>);

/// Saved either as the current ordered list or as the older one-string-per-type table.
#[derive(Deserialize)]
#[serde(untagged)]
enum Saved {
    Groups(Vec<MaybeGroup>),
    Legacy(BTreeMap<PreviewType, String>),
    /// Hand-edited junk: use defaults rather than discarding every other preference.
    Other(serde::de::IgnoredAny),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum MaybeGroup {
    Group(FilterGroup),
    Other(serde::de::IgnoredAny),
}

impl<'de> Deserialize<'de> for PreviewFilters {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let groups = match Saved::deserialize(deserializer)? {
            Saved::Groups(groups) => groups
                .into_iter()
                .filter_map(|g| match g {
                    MaybeGroup::Group(group) => Some(group),
                    MaybeGroup::Other(_) => None,
                })
                .collect(),
            Saved::Other(_) => Vec::new(),
            Saved::Legacy(map) => PreviewType::ALL
                .into_iter()
                .filter_map(|kind| {
                    map.get(&kind).map(|text| FilterGroup {
                        kind,
                        patterns: text
                            .split([';', ',', '|'])
                            .map(str::trim)
                            .filter(|p| !p.is_empty())
                            .map(str::to_string)
                            .collect(),
                    })
                })
                .collect(),
        };
        Ok(Self::from_groups(groups))
    }
}

impl Default for PreviewFilters {
    fn default() -> Self {
        Self::from_groups(Vec::new())
    }
}

impl PreviewFilters {
    /// Keep the first group of each type and append defaults for any missing type.
    pub fn from_groups(groups: Vec<FilterGroup>) -> Self {
        let mut out: Vec<FilterGroup> = Vec::new();
        for group in groups {
            if !out.iter().any(|g| g.kind == group.kind) {
                out.push(group);
            }
        }
        for kind in PreviewType::ALL {
            if !out.iter().any(|g| g.kind == kind) {
                out.push(FilterGroup {
                    kind,
                    patterns: kind.default_patterns(),
                });
            }
        }
        Self(out)
    }

    pub fn patterns(&self, kind: PreviewType) -> &[String] {
        self.0
            .iter()
            .find(|g| g.kind == kind)
            .map(|g| g.patterns.as_slice())
            .unwrap_or_default()
    }

    pub fn compile(&self) -> Result<PreviewRules, FmError> {
        let mut rules = Vec::new();
        let mut seen = Vec::new();
        for group in &self.0 {
            let kind = group.kind;
            if seen.contains(&kind) {
                return Err(FmError::InvalidParams(format!(
                    "{} appears more than once",
                    kind.label()
                )));
            }
            seen.push(kind);
            if group.patterns.len() > crate::glob::MAX_PATTERNS {
                return Err(FmError::InvalidParams(format!(
                    "{} has more than {} patterns",
                    kind.label(),
                    crate::glob::MAX_PATTERNS
                )));
            }
            for pattern in &group.patterns {
                validate_pattern(kind, pattern)?;
            }
            let joined = group.patterns.join(";");
            if joined.len() > crate::glob::MAX_PATTERN_LEN {
                return Err(FmError::InvalidParams(format!(
                    "{} patterns exceed {} bytes in total",
                    kind.label(),
                    crate::glob::MAX_PATTERN_LEN
                )));
            }
            if !group.patterns.is_empty() {
                rules.push((
                    kind,
                    Glob::parse(&joined).map_err(|error| {
                        FmError::InvalidParams(format!("{} patterns: {error}", kind.label()))
                    })?,
                ));
            }
        }
        Ok(PreviewRules(rules))
    }
}

fn validate_pattern(kind: PreviewType, pattern: &str) -> Result<(), FmError> {
    let label = kind.label();
    if pattern.trim().is_empty() {
        return Err(FmError::InvalidParams(format!(
            "{label} patterns cannot be empty; remove the row instead"
        )));
    }
    if pattern.len() > 256 {
        return Err(FmError::InvalidParams(format!(
            "{label} patterns are limited to 256 bytes each"
        )));
    }
    if pattern
        .chars()
        .any(|c| c.is_control() || matches!(c, '/' | '\\' | ';' | ',' | '|'))
    {
        return Err(FmError::InvalidParams(format!(
            "{label} patterns must be one filename pattern, without paths, separators (; , |) or control characters"
        )));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct PreviewRules(Vec<(PreviewType, Glob)>);

impl PreviewRules {
    pub fn kind(&self, name: &str) -> Option<Kind> {
        self.0
            .iter()
            .find(|(_, glob)| glob.matches(name))
            .map(|(kind, _)| kind.kind())
    }
}
