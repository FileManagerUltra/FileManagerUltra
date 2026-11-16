//! Mermaid diagrams for the Markdown preview (change `markdown-mermaid`, capabilities
//! `preview-pane` and `security`).
//!
//! Untrusted diagram source becomes a bitmap here, on a worker: bounded source, `merman`
//! (pure Rust) with the resvg-safe SVG pipeline and the app's palette as Mermaid `base`
//! theme variables, then [`crate::svg::rasterize_scaled`]. Labels are measured with the
//! bundled reading face ([`crate::fonts`]) and drawn with the same face, so they fit. Nothing
//! is fetched or read: `merman` has no network or file access, and the rasterizer loads no
//! resources named by the SVG. `merman` is used only in this module.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;
use std::time::Duration;

use merman::svg::{
    DiagramTheme, DiagramThemeCompiler, DiagramThemeSpec, MermaidThemeCompatibility, SvgPipeline,
};
use merman::{OperationControl, RenderError, RenderOutput, RenderRequest, Renderer, SvgRequest};

use crate::{Limits, fonts, svg, thumbs::Thumb};

/// Source bytes of one diagram.
pub const MAX_SOURCE: usize = 64 << 10;
/// One render is abandoned after this long.
pub const DEADLINE: Duration = Duration::from_secs(2);
/// Bytes of the intermediate SVG.
pub const MAX_SVG: usize = 8 << 20;
/// Diagrams drawn per document; later ones stay code blocks.
pub const MAX_PER_DOCUMENT: usize = 50;
/// Longest reason shown under "Diagram not drawn: ".
const MAX_REASON: usize = 160;

/// Whether a fenced code block's info string asks for a Mermaid diagram.
pub fn is_mermaid(language: &str) -> bool {
    language
        .split_whitespace()
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case("mermaid"))
}

/// Colours of a diagram, from the app's theme. Every colour is `#rrggbb`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Palette {
    pub dark: bool,
    pub background: String,
    pub node_fill: String,
    pub node_border: String,
    pub text: String,
    pub line: String,
    pub cluster_fill: String,
    pub cluster_border: String,
    pub label_background: String,
    pub note_fill: String,
    pub note_border: String,
}

impl Palette {
    /// The colours in a fixed order, with their names for error messages.
    fn colours(&self) -> [(&'static str, &str); 10] {
        [
            ("background", &self.background),
            ("node_fill", &self.node_fill),
            ("node_border", &self.node_border),
            ("text", &self.text),
            ("line", &self.line),
            ("cluster_fill", &self.cluster_fill),
            ("cluster_border", &self.cluster_border),
            ("label_background", &self.label_background),
            ("note_fill", &self.note_fill),
            ("note_border", &self.note_border),
        ]
    }

    /// Every colour must be `#rrggbb`: palettes arrive through commands (untrusted).
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in self.colours() {
            if !is_hex_colour(value) {
                return Err(format!("{name} must be a #rrggbb colour"));
            }
        }
        Ok(())
    }

    /// Stable key for caches (independent of process and Rust version).
    pub fn key(&self) -> u64 {
        let mut text = String::from(if self.dark { "d" } else { "l" });
        for (_, value) in self.colours() {
            text.push_str(value);
        }
        fnv1a(text.as_bytes())
    }

    fn theme(&self) -> Result<DiagramTheme, String> {
        let text = self.text.as_str();
        let variables: [(&str, &str); 19] = [
            ("background", &self.background),
            ("mainBkg", &self.node_fill),
            ("primaryColor", &self.node_fill),
            ("primaryBorderColor", &self.node_border),
            ("nodeBorder", &self.node_border),
            ("primaryTextColor", text),
            ("secondaryColor", &self.cluster_fill),
            ("secondaryBorderColor", &self.cluster_border),
            ("secondaryTextColor", text),
            ("tertiaryColor", &self.label_background),
            ("tertiaryTextColor", text),
            ("textColor", text),
            ("titleColor", text),
            ("lineColor", &self.line),
            ("clusterBkg", &self.cluster_fill),
            ("clusterBorder", &self.cluster_border),
            ("edgeLabelBackground", &self.label_background),
            ("noteBkgColor", &self.note_fill),
            ("noteBorderColor", &self.note_border),
        ];
        let mut compat = MermaidThemeCompatibility::default()
            .with_theme("base")
            .and_then(|c| c.with_dark_mode(self.dark))
            .map_err(|e| e.to_string())?;
        for (name, value) in variables {
            compat = compat
                .with_variable(name, value)
                .map_err(|e| e.to_string())?;
        }
        compat = compat
            .with_variable("noteTextColor", text)
            .and_then(|c| c.with_variable("fontFamily", fonts::FAMILY))
            .map_err(|e| e.to_string())?;
        DiagramThemeCompiler::new()
            .compile(DiagramThemeSpec::new().with_mermaid_compatibility(compat))
            .map_err(|e| e.to_string())
    }
}

fn is_hex_colour(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value.bytes().skip(1).all(|b| b.is_ascii_hexdigit())
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, b| {
        (hash ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Stable key of a diagram's source, for caches.
pub fn source_key(source: &str) -> u64 {
    fnv1a(source.as_bytes())
}

/// The diagram type named on the first meaningful line ("flowchart", "sequence"…), for the
/// accessible name. Directives (`%%{…}%%`), comments and front matter are skipped.
pub fn diagram_kind(source: &str) -> Option<&'static str> {
    let mut in_front_matter = false;
    for (index, line) in source.lines().take(200).enumerate() {
        let line = line.trim();
        if line == "---" {
            in_front_matter = index == 0 || !in_front_matter;
            continue;
        }
        if in_front_matter || line.is_empty() || line.starts_with("%%") {
            continue;
        }
        let word = line
            .split(|c: char| c.is_whitespace() || c == ';')
            .next()
            .unwrap_or("");
        return Some(match word {
            "flowchart" | "graph" => "flowchart",
            "sequenceDiagram" => "sequence diagram",
            "classDiagram" | "classDiagram-v2" => "class diagram",
            "stateDiagram" | "stateDiagram-v2" => "state diagram",
            "erDiagram" => "entity-relationship diagram",
            "gantt" => "Gantt chart",
            "pie" => "pie chart",
            "journey" => "user journey",
            "timeline" => "timeline",
            "gitGraph" => "git graph",
            "mindmap" => "mind map",
            _ => return None,
        });
    }
    None
}

/// Why a diagram was not drawn; `Display` is the short reason the preview shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiagramError {
    TooLarge,
    Unsupported,
    Syntax(String),
    TooComplex,
    TimedOut,
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for DiagramError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => write!(f, "the diagram is larger than 64 KiB"),
            Self::Unsupported => write!(f, "this diagram type is not supported"),
            Self::Syntax(detail) => write!(f, "{detail}"),
            Self::TooComplex => write!(f, "the diagram is too large to lay out"),
            Self::TimedOut => write!(f, "drawing took too long"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Failed(detail) => write!(f, "{detail}"),
        }
    }
}

/// One-line, bounded, control-free text for a reason built from renderer messages.
/// merman prefixes parse messages with "Diagram parse error (<internal parser id>): ";
/// readers only need what follows, starting with a capital.
fn strip_parse_prefix(message: &str) -> &str {
    let trimmed = message.trim_start();
    if trimmed.starts_with("Diagram parse error")
        && let Some(at) = trimmed.find("): ")
    {
        return &trimmed[at + 3..];
    }
    trimmed
}

fn reason(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    let mut out: String = line
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_REASON)
        .collect();
    if line.chars().count() > MAX_REASON {
        out.push('…');
    }
    if out.is_empty() {
        "the diagram could not be drawn".into()
    } else {
        out
    }
}

/// Stops a render from another thread (the preview moved on).
#[derive(Clone, Debug)]
pub struct Cancel(OperationControl);

impl Default for Cancel {
    fn default() -> Self {
        Self(OperationControl::new())
    }
}

impl Cancel {
    pub fn cancel(&self) {
        self.0.cancel();
    }
}

/// A drawn diagram: the bitmap and the diagram's own size in points.
#[derive(Clone, Debug)]
pub struct Diagram {
    pub thumb: Thumb,
    pub natural: (f32, f32),
}

/// Settings a document's own `%%{init}%%` or front matter may not change: merman's hardened
/// defaults plus every theme setting, so the app's palette and font always win (readability
/// for low-vision users; labels are measured with the bundled face).
const LOCKED_KEYS: [&str; 7] = [
    "theme",
    "themeVariables",
    "themeCSS",
    "darkMode",
    "fontFamily",
    "altFontFamily",
    "look",
];

fn hardened_parser() -> merman::SemanticParser {
    let mut secure: Vec<serde_json::Value> = merman::default_site_config()
        .as_value()
        .get("secure")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    for key in LOCKED_KEYS {
        if !secure.iter().any(|v| v.as_str() == Some(key)) {
            secure.push(key.into());
        }
    }
    merman::SemanticParser::new().with_site_config(merman::MermaidConfig::from_value(
        serde_json::json!({ "secure": secure }),
    ))
}

/// Measurement fonts, built once from the bundled face.
fn measurement() -> Result<&'static merman::SvgEnvironment, DiagramError> {
    static ENV: OnceLock<Result<merman::SvgEnvironment, String>> = OnceLock::new();
    ENV.get_or_init(|| {
        let control = OperationControl::new();
        let context = merman_export::NativeFontContext::from_font_bytes(
            [fonts::REGULAR.to_vec(), fonts::BOLD.to_vec()],
            Default::default(),
            &control,
        )
        .map_err(|e| e.to_string())?;
        merman::SvgEnvironment::deterministic()
            .with_native_font_context(context)
            .map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(|e| DiagramError::Failed(reason(e)))
}

/// Draw `source` with `palette` at `scale` (preview zoom × pixels per point). Runs on a
/// worker: it can take up to [`DEADLINE`]. Every failure, including a renderer panic, is a
/// [`DiagramError`].
pub fn render(
    source: &str,
    palette: &Palette,
    scale: f32,
    limits: &Limits,
    cancel: &Cancel,
) -> Result<Diagram, DiagramError> {
    let svg = render_svg(source, palette, cancel)?;
    let (thumb, natural) = catch_unwind(|| svg::rasterize_scaled(svg.as_bytes(), scale, limits))
        .map_err(|_| DiagramError::Failed("the diagram could not be drawn".into()))?
        .map_err(|e| DiagramError::Failed(reason(&e)))?;
    Ok(Diagram { thumb, natural })
}

/// The bounded, resvg-safe SVG of a diagram.
fn render_svg(source: &str, palette: &Palette, cancel: &Cancel) -> Result<String, DiagramError> {
    if source.len() > MAX_SOURCE {
        return Err(DiagramError::TooLarge);
    }
    palette.validate().map_err(DiagramError::Failed)?;
    if diagram_kind(source).is_none() {
        return Err(DiagramError::Unsupported);
    }
    let theme = palette
        .theme()
        .map_err(|e| DiagramError::Failed(reason(&e)))?;
    let environment = measurement()?.clone();
    let control = cancel.0.clone().with_deadline(DEADLINE);
    let svg = catch_unwind(AssertUnwindSafe(|| {
        let request = SvgRequest {
            pipeline: Some(SvgPipeline::resvg_safe()),
            environment,
            ..Default::default()
        };
        Renderer::new()
            .with_semantic_parser(hardened_parser())
            .render(RenderRequest::svg(source, control, request).with_theme(theme))
    }))
    .map_err(|_| DiagramError::Failed("the diagram renderer failed".into()))?;
    let svg = match svg {
        Ok(RenderOutput::Svg(Some(svg))) => svg.svg().to_string(),
        Ok(_) | Err(RenderError::NoDiagram) => return Err(DiagramError::Unsupported),
        Err(RenderError::Cancelled(_)) if cancel.0.is_cancelled() => {
            return Err(DiagramError::Cancelled);
        }
        Err(RenderError::Cancelled(_)) => return Err(DiagramError::TimedOut),
        Err(RenderError::Parse(diagnostic)) => {
            use merman::diagnostic::TerminalDiagnosticClass as Class;
            return Err(match diagnostic.class() {
                Class::ResourceLimit => DiagramError::TooComplex,
                Class::Cancelled if cancel.0.is_cancelled() => DiagramError::Cancelled,
                Class::Cancelled => DiagramError::TimedOut,
                // The terminal-safe message never repeats the document's own text.
                _ => {
                    let message = diagnostic.terminal_safe_message();
                    // Grammar-level budgets (merman 0.10: "Flowchart edge limit exceeded")
                    // arrive as plain parse errors with the generic code.
                    if message.to_ascii_lowercase().contains("limit exceeded") {
                        DiagramError::TooComplex
                    } else {
                        DiagramError::Syntax(reason(strip_parse_prefix(&message)))
                    }
                }
            });
        }
        Err(RenderError::ResourceLimitExceeded(_)) => return Err(DiagramError::TooComplex),
        Err(error) => return Err(DiagramError::Failed(reason(&error.to_string()))),
    };
    if svg.len() > MAX_SVG {
        return Err(DiagramError::TooComplex);
    }
    Ok(svg)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::*;

    pub(crate) fn palette(dark: bool) -> Palette {
        let (bg, fg) = if dark {
            ("#1e2126", "#e6e8eb")
        } else {
            ("#ffffff", "#1b1f24")
        };
        Palette {
            dark,
            background: bg.into(),
            node_fill: if dark { "#2a2f36" } else { "#eef2f8" }.into(),
            node_border: "#5b8def".into(),
            text: fg.into(),
            line: "#8a94a6".into(),
            cluster_fill: if dark { "#252a30" } else { "#f6f8fb" }.into(),
            cluster_border: "#7a8597".into(),
            label_background: bg.into(),
            note_fill: "#fff5c2".into(),
            note_border: "#c9a400".into(),
        }
    }

    fn draw(source: &str) -> Result<Diagram, DiagramError> {
        render(
            source,
            &palette(false),
            1.0,
            &Limits::default(),
            &Cancel::default(),
        )
    }

    #[test]
    fn mermaid_info_strings_are_recognised() {
        assert!(is_mermaid("mermaid"));
        assert!(is_mermaid("Mermaid title=x"));
        assert!(!is_mermaid("mermaidjs"));
        assert!(!is_mermaid(""));
        assert!(!is_mermaid("rust"));
    }

    #[test]
    fn diagram_kind_skips_directives_comments_and_front_matter() {
        assert_eq!(diagram_kind("flowchart LR\nA-->B"), Some("flowchart"));
        assert_eq!(
            diagram_kind("---\ntitle: x\n---\n%%{init: {}}%%\n%% c\nsequenceDiagram"),
            Some("sequence diagram")
        );
        assert_eq!(diagram_kind("graph;A-->B"), Some("flowchart"));
        assert_eq!(diagram_kind("quadrantChart\n"), None);
        assert_eq!(diagram_kind(""), None);
    }

    #[test]
    fn palette_validation_rejects_anything_but_hex_colours() {
        assert!(palette(true).validate().is_ok());
        for bad in ["red", "#12345", "#1234567", "red; url(x)", "#12345g", ""] {
            let mut p = palette(false);
            p.line = bad.into();
            assert!(p.validate().is_err(), "{bad}");
            assert!(matches!(draw_with(&p), Err(DiagramError::Failed(_))));
        }
        assert_ne!(palette(true).key(), palette(false).key());
        assert_eq!(palette(true).key(), palette(true).key());
    }

    fn draw_with(p: &Palette) -> Result<Diagram, DiagramError> {
        render(
            "flowchart LR\nA-->B",
            p,
            1.0,
            &Limits::default(),
            &Cancel::default(),
        )
    }

    #[test]
    fn every_supported_type_draws() {
        let examples = [
            "flowchart TB\nsubgraph S[\"Group\"]\nA[\"one<br/>two\"] -->|label| B\nend\nB -.-> C\nC <--> D\nD --- E",
            "sequenceDiagram\nAlice->>Bob: Hello\nNote right of Bob: thinks\nBob-->>Alice: Hi",
            "classDiagram\nclass Animal {\n+name: String\n+eat()\n}\nAnimal <|-- Dog",
            "stateDiagram-v2\n[*] --> Still\nStill --> Moving\nMoving --> [*]",
            "erDiagram\nCUSTOMER ||--o{ ORDER : places",
            "gantt\ndateFormat YYYY-MM-DD\nsection A\nTask :a1, 2026-01-01, 3d",
            "pie title Pets\n\"Dogs\" : 386\n\"Cats\" : 85",
            "journey\ntitle Day\nsection Work\nCode: 5: Me",
            "timeline\ntitle History\n2024 : One\n2025 : Two",
            "gitGraph\ncommit\nbranch dev\ncommit\ncheckout main\nmerge dev",
            "mindmap\n  root((Root))\n    A\n    B",
        ];
        for source in examples {
            let d = draw(source).unwrap_or_else(|e| panic!("{source}: {e}"));
            assert!(d.thumb.width > 10 && d.thumb.height > 10, "{source}");
            assert!(d.natural.0 > 10.0 && d.natural.1 > 10.0, "{source}");
        }
    }

    #[test]
    fn broken_unknown_and_oversized_diagrams_have_reasons() {
        let syntax = draw("flowchart LR\n A[[[(( --> }}} |x|").expect_err("syntax");
        assert!(matches!(syntax, DiagramError::Syntax(_)), "{syntax:?}");
        assert!(
            !syntax.to_string().contains("parse error ("),
            "no internal parser id: {syntax}"
        );
        assert!(!syntax.to_string().is_empty());
        assert_eq!(
            draw("quadrantChart\n").expect_err("type"),
            DiagramError::Unsupported
        );
        assert_eq!(
            draw("not a diagram").expect_err("none"),
            DiagramError::Unsupported
        );
        let huge = format!("flowchart LR\n{}", "A-->B\n".repeat(MAX_SOURCE / 6 + 1));
        assert_eq!(draw(&huge).expect_err("size"), DiagramError::TooLarge);
    }

    #[test]
    fn hostile_graphs_stay_bounded() {
        let mut chain = String::from("flowchart LR\n");
        for i in 0..3000 {
            chain.push_str(&format!("n{i}-->n{}\n", i + 1));
        }
        let started = std::time::Instant::now();
        let result = draw(&chain);
        assert!(started.elapsed() < DEADLINE + Duration::from_secs(1));
        assert_eq!(
            result.expect_err("over the edge limit"),
            DiagramError::TooComplex
        );
        let mut deep = String::from("flowchart TB\n");
        for i in 0..200 {
            deep.push_str(&format!("subgraph s{i}\n"));
        }
        deep.push_str("a --> b\n");
        deep.push_str(&"end\n".repeat(200));
        let started = std::time::Instant::now();
        let _ = draw(&deep);
        assert!(started.elapsed() < DEADLINE + Duration::from_secs(1));
    }

    /// Same shape as a real architecture document: TB, four subgraphs (two without outside
    /// links), chained, labelled and dotted edges, `<br/>` labels.
    const ARCHITECTURE: &str = "flowchart TB
    subgraph App[\"Application\"]
        Browser[\"Customer browser\"]
        Cache[\"Page cache<br/>full-page\"]
        Shop[\"Shop engine 2.4<br/>runtime 8.3\"]
        Theme[\"Theme and checkout\"]
        Browser --> Cache --> Shop
        Shop --- Theme
    end
    subgraph Local[\"Local development\"]
        Dev[\"Developer\"]
        Web[\"Web container\"]
        DB[\"Database\"]
        Queue[\"Queue\"]
        Dev --> Web
        Web --> DB
        Web --> Queue
        Web -.-> Cache2[\"Optional cache\"]
    end
    subgraph Delivery[\"Build and release\"]
        Repo[\"Repository\"] --> PR[\"Pull request\"] --> CI[\"CI/CD\"]
        PR -->|\"merge\"| Main[\"main branch\"]
        Main -->|\"auto-deploy\"| CI
        Main --> Release[\"Release PR<br/>main → production\"] -->|\"approved and merged\"| Tag[\"Tag\"] --> CI
    end
    subgraph Hosted[\"Hosted environments\"]
        Test[\"Per-PR test environment\"]
        Acc[\"Acceptance\"]
        Prod[\"Production\"]
        Errors[\"Error tracking\"]
    end
    CI -->|PR build| Test
    CI -->|main deploy| Acc
    CI -->|tag deploy| Prod
    Test -.-> Errors";

    /// Same shape as a real integration landscape: LR, many labelled edges, `<-->`, `---`.
    const LANDSCAPE: &str = "flowchart LR
    B2B[\"Business customers<br/>(primary)\"] --> Store
    B2C[\"Consumers\"] --> Store
    subgraph Channels[\"Channels\"]
        Store[\"Main store<br/>multilingual\"]
        Sub[\"Sub-brand store\"]
    end
    subgraph Platform[\"Shared platform\"]
        Core[\"Commerce core\"] --- Caps[\"Catalog, CMS, search<br/>accounts, quotes, orders\"]
    end
    Store --> Core
    Sub --> Core
    Erp[\"ERP\"] --> Bridge[\"Integration bridge\"]
    Bridge -->|REST import: orders and invoices| Core
    Core ---|Debtor maps to Company| Erp
    Core <-->|Payments| Pay[\"Payment provider\"]
    Core -->|Delivery-option request| Ship[\"Shipping\"]
    Ship -->|Available shipping methods| Core
    Feed[\"Channel feed\"] -->|Incoming orders| Core
    Auto[\"Automation\"] -->|REST: onboarding| Core
    Core -->|Rating request| Reviews[\"Reviews\"]
    Reviews -->|Rating data| Core";

    /// Nothing in the SVG can make resvg read a file or the network.
    fn assert_offline(svg: &str) {
        assert!(!svg.contains("<foreignObject"), "no HTML labels");
        assert!(!svg.contains("<image"), "no images");
        assert!(!svg.contains("@import"), "no stylesheet imports");
        for (at, _) in svg.match_indices("href=\"") {
            let target = &svg[at + 6..];
            assert!(
                target.starts_with('#'),
                "only same-document links: {}",
                &target[..target.len().min(40)]
            );
        }
        assert!(
            !svg.contains("url(http") && !svg.contains("url(file"),
            "no external url()"
        );
    }

    #[test]
    fn real_world_shapes_draw_every_label_offline() {
        for (source, labels) in [
            (
                ARCHITECTURE,
                &[
                    "Build and release",
                    "Hosted environments",
                    "approved and merged",
                    "tag deploy",
                    "Release PR",
                ][..],
            ),
            (
                LANDSCAPE,
                &[
                    "Shared platform",
                    "Payments",
                    "Available shipping methods",
                    "Rating data",
                ][..],
            ),
        ] {
            let svg = render_svg(source, &palette(true), &Cancel::default()).expect("svg");
            assert_offline(&svg);
            for label in labels {
                assert!(svg.contains(label), "label {label} is drawn as SVG text");
            }
            let d = draw(source).expect("bitmap");
            assert!(d.natural.0 > 300.0, "{:?}", d.natural);
        }
    }

    /// Image nodes may reach the SVG as `<image>` elements; the rasterizer refuses every image
    /// reference (see `svg::tests::image_references_are_never_read`), and merman itself has no
    /// network or file access, so nothing named by a diagram is ever read.
    #[test]
    fn references_inside_diagrams_are_never_loaded() {
        let source = "flowchart LR
    A[\"<img src='https://example.org/x.png'>\"] --> B[\"<img src='/etc/passwd'>\"]
    click A \"https://example.org\" \"tip\"
    click B callback
    C@{ img: \"https://example.org/y.png\", label: \"pic\" } --> A";
        if let Ok(svg) = render_svg(source, &palette(false), &Cancel::default()) {
            assert!(!svg.contains("<foreignObject") && !svg.contains("@import"));
            assert!(!svg.contains("/etc/passwd"), "HTML in labels is not kept");
            // Drawing it reads nothing and still produces a picture.
            let (thumb, _) =
                svg::rasterize_scaled(svg.as_bytes(), 1.0, &Limits::default()).expect("draws");
            assert!(thumb.width > 10);
        }
    }

    #[test]
    fn the_palette_wins_over_the_documents_own_theme() {
        let forest = "%%{init: {\"theme\": \"forest\", \"themeVariables\": {\"primaryColor\": \"#ff00ff\", \"lineColor\": \"#00ff00\"}}}%%\nflowchart LR\nA-->B";
        let svg = render_svg(forest, &palette(true), &Cancel::default()).expect("svg");
        let lower = svg.to_ascii_lowercase();
        assert!(
            !lower.contains("#ff00ff") && !lower.contains("#00ff00"),
            "document colours ignored"
        );
        assert!(
            lower.contains(&palette(true).node_fill),
            "palette node fill used"
        );
    }

    #[test]
    fn a_cancelled_render_reports_cancelled() {
        let cancel = Cancel::default();
        cancel.cancel();
        let result = render(
            "flowchart LR\nA-->B",
            &palette(false),
            1.0,
            &Limits::default(),
            &cancel,
        );
        assert_eq!(result.expect_err("cancelled"), DiagramError::Cancelled);
    }

    #[test]
    fn reasons_are_one_bounded_line() {
        let long = format!("{}\nsecond line", "x".repeat(500));
        let r = reason(&long);
        assert!(r.chars().count() <= MAX_REASON + 1 && !r.contains('\n'));
        assert_eq!(reason("  \n"), "the diagram could not be drawn");
        assert_eq!(reason("a\u{7}b"), "ab");
    }
}
