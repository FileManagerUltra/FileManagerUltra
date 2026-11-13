//! JSON / YAML / TOML / XML: shown raw and highlighted, with an opt-in re-indented form.

use fm_vfs::{Entry, FmError, Vfs};

use crate::{Limits, Preview, StructDoc, text};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Json,
    Yaml,
    Toml,
    Xml,
}

impl Format {
    pub fn from_ext(ext: &str) -> Option<Self> {
        match ext {
            "json" | "geojson" | "webmanifest" => Some(Self::Json),
            "yaml" | "yml" => Some(Self::Yaml),
            "toml" => Some(Self::Toml),
            "xml" | "svg" | "xsd" | "xsl" | "xslt" | "plist" | "xhtml" | "csproj" | "wsdl" => {
                Some(Self::Xml)
            }
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::Yaml => "YAML",
            Self::Toml => "TOML",
            Self::Xml => "XML",
        }
    }

    fn lang(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Yaml => "yaml",
            Self::Toml => "toml",
            Self::Xml => "xml",
        }
    }

    fn note(self) -> Option<&'static str> {
        match self {
            Self::Toml => Some("Comments are not kept in the pretty view."),
            Self::Yaml => Some("Comments and anchors are not kept in the pretty view."),
            _ => None,
        }
    }
}

const MAX_XML_DEPTH: usize = 256;
const MAX_YAML_ALIASES: usize = 64;

/// Re-indent `src`; the error says why it cannot be done.
pub fn pretty(format: Format, src: &str) -> Result<String, String> {
    match format {
        Format::Json => {
            let v: serde_json::Value =
                serde_json::from_str(src).map_err(|e| format!("not valid JSON: {e}"))?;
            serde_json::to_string_pretty(&v).map_err(|e| e.to_string())
        }
        Format::Toml => {
            let v: toml::Table = src.parse().map_err(|e| format!("not valid TOML: {e}"))?;
            toml::to_string_pretty(&v).map_err(|e| e.to_string())
        }
        Format::Yaml => pretty_yaml(src),
        Format::Xml => pretty_xml(src),
    }
}

fn pretty_yaml(src: &str) -> Result<String, String> {
    use serde::Deserialize;
    let aliases = src
        .lines()
        .map(|l| l.matches(" *").count() + l.trim_start().starts_with('*') as usize)
        .sum::<usize>();
    if aliases > MAX_YAML_ALIASES {
        return Err("too many YAML aliases to expand safely".into());
    }
    let mut docs = Vec::new();
    for de in serde_yaml_ng::Deserializer::from_str(src) {
        let v =
            serde_yaml_ng::Value::deserialize(de).map_err(|e| format!("not valid YAML: {e}"))?;
        docs.push(serde_yaml_ng::to_string(&v).map_err(|e| e.to_string())?);
        if docs.len() > 1000 {
            return Err("too many YAML documents".into());
        }
    }
    Ok(docs.join("---\n"))
}

fn pretty_xml(src: &str) -> Result<String, String> {
    use quick_xml::events::Event;
    use quick_xml::{Reader, Writer};
    let mut reader = Reader::from_str(src);
    reader.config_mut().trim_text(true);
    let mut writer = Writer::new_with_indent(Vec::new(), b' ', 2);
    let mut depth = 0usize;
    loop {
        let ev = reader
            .read_event()
            .map_err(|e| format!("not valid XML: {e}"))?;
        match &ev {
            Event::Eof => break,
            Event::Start(_) => {
                depth += 1;
                if depth > MAX_XML_DEPTH {
                    return Err("XML is nested too deeply".into());
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
        writer.write_event(ev).map_err(|e| e.to_string())?;
    }
    String::from_utf8(writer.into_inner()).map_err(|e| e.to_string())
}

pub fn render(
    vfs: &dyn Vfs,
    entry: &Entry,
    limits: &Limits,
    format: Format,
) -> Result<Preview, FmError> {
    let (bytes, more) = text::read_capped(vfs, entry, limits.text_bytes)?;
    let total = entry.size.unwrap_or(bytes.len() as u64);
    Ok(Preview::Structured(doc(
        &bytes, more, total, limits, format,
    )))
}

/// Raw and pretty views of `bytes`, the first part of a `total`-byte file (`more` when cut).
pub fn doc(bytes: &[u8], more: bool, total: u64, limits: &Limits, format: Format) -> StructDoc {
    let src = text::decode(bytes);
    let raw = text::make_doc(&src, format.lang(), limits, more, total);
    let too_big = more || total > limits.pretty_bytes;
    let pretty = if too_big {
        Err(format!(
            "file is larger than {} KiB",
            limits.pretty_bytes / 1024
        ))
    } else {
        pretty(format, &src)
            .map(|p| text::make_doc(&p, format.lang(), limits, false, p.len() as u64))
    };
    StructDoc {
        format,
        raw,
        pretty,
        pretty_note: format.note().map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_pretty_keeps_order_and_rejects_garbage() {
        let p = pretty(Format::Json, "{\"b\":1,\"a\":[1,2]}").expect("ok");
        assert!(p.find("\"b\"") < p.find("\"a\""));
        assert!(p.contains('\n'));
        assert!(pretty(Format::Json, "{oops").is_err());
    }

    #[test]
    fn deep_json_is_refused_not_overflowed() {
        let s = "[".repeat(10_000) + &"]".repeat(10_000);
        assert!(pretty(Format::Json, &s).is_err());
    }

    #[test]
    fn toml_and_yaml_and_xml() {
        assert!(
            pretty(Format::Toml, "a=1\n[t]\nb='x'")
                .expect("toml")
                .contains("[t]")
        );
        assert!(pretty(Format::Toml, "a=").is_err());
        assert!(
            pretty(Format::Yaml, "a: 1\nb: [1, 2]")
                .expect("yaml")
                .contains("a: 1")
        );
        assert!(pretty(Format::Yaml, "a: [").is_err());
        let x = pretty(Format::Xml, "<a><b>t</b><c/></a>").expect("xml");
        assert!(x.contains("\n  <b>t</b>"));
        assert!(pretty(Format::Xml, "<a><b></a>").is_err());
    }

    #[test]
    fn xml_depth_cap_and_yaml_alias_cap() {
        let x = "<a>".repeat(1000) + &"</a>".repeat(1000);
        assert!(pretty(Format::Xml, &x).is_err());
        let y = format!(
            "a: &x 1\n{}",
            "b: *x\n"
                .repeat(200)
                .replace("b:", "b: [ *x,")
                .replace('\n', " *x ]\n")
        );
        assert!(pretty(Format::Yaml, &y).is_err());
    }
}
