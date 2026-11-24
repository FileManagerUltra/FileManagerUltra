//! Extract and compress commands (`archive-extract-compress`): `archive.extract`,
//! `archive.extract_to`, `archive.compress` and `archive.compress_quick`. The work runs
//! as file jobs (`archive_ops`).

use fm_archive::write::{CompressOptions, Level, WRITABLE};
use fm_archive::{FormatId, Secret};
use fm_vfs::{FmError, VfsUri};
use serde_json::{Map, json};

use crate::archive_cmds::{MAX_PASSWORD, ensure_registered, is_archive_name, mount};
use crate::fileops::OpRequest;
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

fn obj(params: &Params) -> Result<&Map<String, Params>, FmError> {
    params
        .as_object()
        .ok_or_else(|| FmError::InvalidParams("expected an object".into()))
}

fn required_uri(map: &Map<String, Params>, key: &str) -> Result<VfsUri, FmError> {
    let raw = map
        .get(key)
        .and_then(Params::as_str)
        .ok_or_else(|| FmError::InvalidParams(format!("{key} must be a uri string")))?;
    VfsUri::parse(raw).map_err(|_| FmError::invalid_uri(raw.to_string()))
}

/// The sources a compress command acts on: `sources`, else the focused selection.
fn sources_of(s: &Session, map: &Map<String, Params>) -> Result<Vec<VfsUri>, FmError> {
    let sources = match map.get("sources") {
        Some(v) => crate::file_cmds::parse_sources(v)?,
        None => crate::file_cmds::focused_selection(s),
    };
    if sources.is_empty() {
        return Err(FmError::InvalidParams(
            "select something to compress".into(),
        ));
    }
    if sources.iter().any(|u| u.file_name().is_none()) {
        return Err(FmError::InvalidParams(
            "a root folder cannot be compressed".into(),
        ));
    }
    Ok(sources)
}

fn archive_file(map: &Map<String, Params>) -> Result<(VfsUri, String), FmError> {
    let uri = required_uri(map, "uri")?;
    let name = uri
        .file_name()
        .ok_or_else(|| FmError::InvalidParams("not a file".into()))?
        .to_string();
    if !is_archive_name(&name) {
        return Err(FmError::Unsupported("this file is not an archive"));
    }
    Ok((uri, name))
}

/// `archive.extract {uri, dest?}` — extract into a new folder named after the archive,
/// next to it (Extract here) or in `dest`.
pub fn extract() -> CommandSpec {
    CommandSpec {
        id: "archive.extract",
        label: "Extract Here",
        menu: &["File", "Archive", "Extract Here"],
        shortcut: None,
        params: "{uri: string, dest?: string} — an archive file; dest defaults to its folder",
        enabled: |_s| true,
        run: |s, params: &Params| {
            let map = obj(params)?;
            let (uri, name) = archive_file(map)?;
            let dest = match map.get("dest") {
                None | Some(Params::Null) => uri
                    .parent()
                    .ok_or_else(|| FmError::InvalidParams("no folder to extract into".into()))?,
                Some(_) => required_uri(map, "dest")?,
            };
            if dest.archive_handle().is_some() {
                return Err(FmError::Unsupported(
                    "extract into a folder outside archives",
                ));
            }
            ensure_registered(s, &uri)?;
            let root = mount(s, &uri)?;
            s.submit_op(OpRequest::Extract {
                root,
                container: uri,
                dest_dir: dest,
                name: crate::archive_ops::extract_name(&name),
            })?;
            Ok(())
        },
    }
}

/// `archive.extract_to {uri}` — choose a folder with the path picker, then extract there.
pub fn extract_to() -> CommandSpec {
    CommandSpec {
        id: "archive.extract_to",
        label: "Extract To…",
        menu: &["File", "Archive", "Extract To…"],
        shortcut: None,
        params: "{uri: string} — an archive file",
        enabled: |_s| true,
        run: |s, params: &Params| {
            let (uri, _) = archive_file(obj(params)?)?;
            let start = uri
                .parent()
                .filter(|p| p.archive_handle().is_none())
                .map(|p| p.to_string());
            s.archives.extract_pending = Some(uri);
            s.dispatch(
                "picker.open",
                &json!({
                    "purpose": "archive.extract",
                    "kind": "folder",
                    "title": "Extract to which folder?",
                    "start": start,
                }),
            )
        },
    }
}

/// `archive.add {uri, sources?, dir?}` — add the selection to the archive file `uri`
/// (into its folder `dir`, default the top), asking about names it already has, and save
/// it in the same job. When the archive is open in the tree or has unsaved changes, the
/// selection is only staged on top of them.
pub fn add() -> CommandSpec {
    CommandSpec {
        id: "archive.add",
        label: "Add to Archive",
        menu: &["File", "Archive", "Add to Archive"],
        shortcut: None,
        params: "{uri: string, sources?: [string], dir?: string} — a writable archive file; sources default to the selection; dir is a folder inside the archive",
        enabled: |_s| true,
        run: |s, params: &Params| {
            let map = obj(params)?;
            let (uri, name) = archive_file(map)?;
            let sources = sources_of(s, map)?;
            if sources.len() > crate::fileops::MAX_SOURCES {
                return Err(FmError::InvalidParams(format!(
                    "too many items (limit {})",
                    crate::fileops::MAX_SOURCES
                )));
            }
            if sources.iter().any(|u| *u == uri || u.is_ancestor_of(&uri)) {
                return Err(FmError::InvalidParams(
                    "an archive cannot be added to itself".into(),
                ));
            }
            ensure_registered(s, &uri)?;
            let root = mount(s, &uri)?;
            let lazy = s
                .archives
                .for_uri(&root)
                .cloned()
                .ok_or_else(|| FmError::NotFound("archive".into()))?;
            match lazy.opened() {
                Some(fs) => fs.writable()?,
                None if !crate::archive_cmds::is_writable_archive_name(&name) => {
                    return Err(FmError::Unsupported("this kind of archive is read-only"));
                }
                None => {}
            }
            let dest = match map.get("dir") {
                None | Some(Params::Null) => root.clone(),
                Some(_) => required_uri(map, "dir")?,
            };
            if dest.archive_handle() != root.archive_handle() {
                return Err(FmError::InvalidParams(
                    "dir must be a folder inside the archive".into(),
                ));
            }
            // An archive that is open (a pane has been in it, the tree shows it) is being
            // edited: the items are staged for the unsaved bar and the close prompt.
            let editing = lazy.is_dirty()
                || s.archives.is_opened(&uri)
                || s.archives
                    .is_opened(&crate::archive_cmds::canonical_container(&uri));
            let save = (!editing).then(|| crate::archive_ops::SaveTarget {
                mount: lazy,
                copy_to: None,
                backup: None,
            });
            s.submit_op(OpRequest::Add {
                sources,
                dest,
                policy: fm_ops::ConflictPolicy::Ask,
                save,
            })?;
            Ok(())
        },
    }
}

/// `archive.add_to {sources?}` — choose an archive file with the path picker, then add the
/// selection to it.
pub fn add_to() -> CommandSpec {
    CommandSpec {
        id: "archive.add_to",
        label: "Add to Archive…",
        menu: &["File", "Archive", "Add to Archive…"],
        shortcut: None,
        params: "{sources?: [string]} — sources default to the selection",
        enabled: |s| !crate::file_cmds::focused_selection(s).is_empty(),
        run: |s, params: &Params| {
            let sources = match params.as_object() {
                Some(map) => sources_of(s, map)?,
                None if params.is_null() => sources_of(s, &Map::new())?,
                None => return Err(FmError::InvalidParams("expected an object".into())),
            };
            let start = sources
                .first()
                .and_then(VfsUri::parent)
                .filter(|p| p.archive_handle().is_none())
                .map(|p| p.to_string());
            s.archives.add_pending = Some(sources);
            s.dispatch(
                "picker.open",
                &json!({
                    "purpose": "archive.add",
                    "kind": "file",
                    "title": "Add to which archive?",
                    "start": start,
                }),
            )
        },
    }
}

fn options(map: &Map<String, Params>) -> Result<(CompressOptions, Option<u64>), FmError> {
    let format = map
        .get("format")
        .and_then(Params::as_str)
        .and_then(FormatId::from_id)
        .filter(|f| WRITABLE.contains(f))
        .ok_or_else(|| {
            FmError::InvalidParams("format must be zip, tar.gz, tar.xz, tar.bz2, tar or 7z".into())
        })?;
    let level = match map.get("level") {
        None | Some(Params::Null) => Level::Normal,
        Some(v) => v
            .as_str()
            .and_then(Level::from_id)
            .ok_or_else(|| FmError::InvalidParams("level must be fast, normal or best".into()))?,
    };
    let password = match map.get("password") {
        None | Some(Params::Null) => None,
        Some(v) => {
            let pw = v
                .as_str()
                .ok_or_else(|| FmError::InvalidParams("password must be a string".into()))?;
            if pw.is_empty() || pw.len() > MAX_PASSWORD {
                return Err(FmError::InvalidParams(format!(
                    "password must be 1 to {MAX_PASSWORD} bytes"
                )));
            }
            Some(Secret::new(pw))
        }
    };
    let encrypt_names = match map.get("encrypt_names") {
        None | Some(Params::Null) => true,
        Some(v) => v
            .as_bool()
            .ok_or_else(|| FmError::InvalidParams("encrypt_names must be true or false".into()))?,
    };
    let split = match map.get("split") {
        None | Some(Params::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .filter(|n| *n >= crate::archive_ops::MIN_PART)
                .ok_or_else(|| {
                    FmError::InvalidParams(
                        "split must be a part size of at least 65536 bytes".into(),
                    )
                })?,
        ),
    };
    let options = CompressOptions {
        format,
        level,
        password,
        zip_crypto: false,
        encrypt_names,
    };
    options.validate()?;
    Ok((options, split))
}

fn submit(
    s: &mut Session,
    sources: Vec<VfsUri>,
    target: VfsUri,
    map: &Map<String, Params>,
) -> Result<(), FmError> {
    if sources.contains(&target) {
        return Err(FmError::InvalidParams(
            "the new archive cannot be one of the items in it".into(),
        ));
    }
    if target.archive_handle().is_some() {
        return Err(FmError::Unsupported("create archives outside archives"));
    }
    let (options, split) = options(map)?;
    s.submit_op(OpRequest::Compress {
        sources,
        target,
        options,
        split,
    })?;
    Ok(())
}

/// `archive.compress {sources?, target, format, level?, password?, encrypt_names?, split?}`
/// — create a new archive from the selection (Compress to…).
pub fn compress() -> CommandSpec {
    CommandSpec {
        id: "archive.compress",
        label: "Compress To…",
        menu: &["File", "Archive", "Compress To…"],
        shortcut: None,
        params: "{sources?: [uri], target: string, format: zip|tar.gz|tar.xz|tar.bz2|tar|7z, level?: fast|normal|best, password?: string, encrypt_names?: bool, split?: bytes}",
        enabled: |s| !crate::file_cmds::focused_selection(s).is_empty(),
        run: |s, params: &Params| {
            let map = obj(params)?;
            let sources = sources_of(s, map)?;
            let target = required_uri(map, "target")?;
            submit(s, sources, target, map)
        },
    }
}

/// The default archive for `sources`: next to them, named after the single item or the
/// folder holding several.
pub fn quick_target(sources: &[VfsUri], suffix: &str) -> Option<VfsUri> {
    let first = sources.first()?;
    let dir = first.parent()?;
    let stem = match sources {
        [one] => fm_archive::format::split_suffix(one.file_name()?).0,
        _ => dir
            .file_name()
            .map(str::to_string)
            .unwrap_or_else(|| "archive".to_string()),
    };
    dir.join(&format!("{stem}{suffix}")).ok()
}

/// `archive.compress_quick {sources?, format}` — "Add to name.zip" / "Add to name.7z": a
/// new archive next to the selection with default settings, without a dialog.
pub fn compress_quick() -> CommandSpec {
    CommandSpec {
        id: "archive.compress_quick",
        label: "Add to Archive",
        menu: &["File", "Archive", "Add to Archive"],
        shortcut: None,
        params: "{sources?: [uri], format: zip|7z}",
        enabled: |s| !crate::file_cmds::focused_selection(s).is_empty(),
        run: |s, params: &Params| {
            let map = obj(params)?;
            let sources = sources_of(s, map)?;
            let suffix = match map.get("format").and_then(Params::as_str) {
                Some("zip") => ".zip",
                Some("7z") => ".7z",
                _ => return Err(FmError::InvalidParams("format must be zip or 7z".into())),
            };
            let target = quick_target(&sources, suffix)
                .ok_or_else(|| FmError::InvalidParams("no folder for the new archive".into()))?;
            let defaults = Map::from_iter([(
                "format".to_string(),
                Params::String(suffix.trim_start_matches('.').to_string()),
            )]);
            submit(s, sources, target, &defaults)
        },
    }
}

fn add_as(s: &mut Session, params: &Params, format: &str) -> Result<(), FmError> {
    let mut map = match params {
        Params::Null => Map::new(),
        Params::Object(m) => m.clone(),
        _ => return Err(FmError::InvalidParams("expected {sources?: [uri]}".into())),
    };
    map.insert("format".into(), Params::String(format.into()));
    s.dispatch("archive.compress_quick", &Params::Object(map))
}

/// `archive.add_zip {sources?}` — the context menu's "Add to name.zip".
pub fn add_zip() -> CommandSpec {
    CommandSpec {
        id: "archive.add_zip",
        label: "Add to Zip",
        menu: &["File", "Archive", "Add to Zip"],
        shortcut: None,
        params: "{sources?: [uri]} — default the selection",
        enabled: |s| !crate::file_cmds::focused_selection(s).is_empty(),
        run: |s, params: &Params| add_as(s, params, "zip"),
    }
}

/// `archive.add_7z {sources?}` — the context menu's "Add to name.7z".
pub fn add_7z() -> CommandSpec {
    CommandSpec {
        id: "archive.add_7z",
        label: "Add to 7z",
        menu: &["File", "Archive", "Add to 7z"],
        shortcut: None,
        params: "{sources?: [uri]} — default the selection",
        enabled: |s| !crate::file_cmds::focused_selection(s).is_empty(),
        run: |s, params: &Params| add_as(s, params, "7z"),
    }
}

/// A format the Compress dialog offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatChoice {
    /// The `format` param of `archive.compress`.
    pub id: &'static str,
    pub label: &'static str,
    pub suffix: &'static str,
    /// Whether it can be encrypted (tar formats cannot).
    pub password: bool,
}

/// Every format `archive.compress` writes, in the order the dialog lists them.
pub const FORMATS: [FormatChoice; 6] = [
    FormatChoice {
        id: "zip",
        label: "Zip",
        suffix: ".zip",
        password: true,
    },
    FormatChoice {
        id: "7z",
        label: "7z",
        suffix: ".7z",
        password: true,
    },
    FormatChoice {
        id: "tar.gz",
        label: "tar.gz",
        suffix: ".tar.gz",
        password: false,
    },
    FormatChoice {
        id: "tar.xz",
        label: "tar.xz",
        suffix: ".tar.xz",
        password: false,
    },
    FormatChoice {
        id: "tar.bz2",
        label: "tar.bz2",
        suffix: ".tar.bz2",
        password: false,
    },
    FormatChoice {
        id: "tar",
        label: "tar (uncompressed)",
        suffix: ".tar",
        password: false,
    },
];

/// The file suffix a new archive of `format` gets.
pub fn suffix_of(format: FormatId) -> &'static str {
    match format {
        FormatId::Zip => ".zip",
        FormatId::TarGz => ".tar.gz",
        FormatId::TarXz => ".tar.xz",
        FormatId::TarBz2 => ".tar.bz2",
        FormatId::Tar => ".tar",
        FormatId::SevenZ => ".7z",
        _ => "",
    }
}

/// Whether `name` is a part of a numbered split archive (`backup.7z.002`): the only sets
/// that can be joined by concatenation (not spanned zips or RAR volumes).
pub fn is_joinable_part(name: &str) -> bool {
    fm_archive::split::parse(name).is_some_and(|p| p.kind == fm_archive::split::SplitKind::Numbered)
}

/// `archive.join {uri, target?}` — concatenate the numbered parts `uri` belongs to into
/// one file next to them (`backup.7z.001`… → `backup.7z`), or at `target`.
pub fn join() -> CommandSpec {
    CommandSpec {
        id: "archive.join",
        label: "Join Parts",
        menu: &["File", "Archive", "Join Parts"],
        shortcut: None,
        params: "{uri?: string, target?: string} — a numbered part (name.7z.001…); uri defaults to the selected file, target to the archive named without the part number",
        enabled: |s| {
            matches!(
                crate::file_cmds::focused_selection(s).as_slice(),
                [one] if one.archive_handle().is_none()
                    && one.file_name().is_some_and(is_joinable_part)
            )
        },
        run: |s, params: &Params| {
            let empty = Map::new();
            let map = match params {
                Params::Null => &empty,
                other => obj(other)?,
            };
            let first = match map.get("uri") {
                Some(_) => required_uri(map, "uri")?,
                None => match crate::file_cmds::focused_selection(s).as_slice() {
                    [one] => one.clone(),
                    _ => return Err(FmError::InvalidParams("select one part".into())),
                },
            };
            if !first.file_name().is_some_and(is_joinable_part) {
                return Err(FmError::Unsupported(
                    "only numbered parts (name.7z.001, …) can be joined",
                ));
            }
            let target = match map.get("target") {
                None | Some(Params::Null) => None,
                Some(_) => Some(required_uri(map, "target")?),
            };
            s.submit_op(OpRequest::Join { first, target })?;
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        extract(),
        extract_to(),
        compress(),
        compress_quick(),
        add_zip(),
        add_7z(),
        add(),
        add_to(),
        join(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn wait_jobs(s: &mut Session) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while s.fileops.jobs().iter().any(|j| j.state.is_running()) {
            s.poll();
            assert!(std::time::Instant::now() < deadline, "job timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        for j in s.fileops.jobs() {
            assert_eq!(j.state, crate::fileops::OpState::Finished, "{}", j.label);
        }
    }

    fn setup() -> (Session, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tmp");
        let p = dir.path();
        std::fs::create_dir_all(p.join("project/docs")).expect("mkdir");
        std::fs::write(p.join("project/docs/a.txt"), b"alpha").expect("w");
        std::fs::write(p.join("project/notes.txt"), b"notes").expect("w");
        (Session::new().expect("session"), dir)
    }

    fn uri(p: &std::path::Path) -> String {
        VfsUri::parse(p.to_str().expect("utf8"))
            .expect("uri")
            .to_string()
    }

    #[test]
    fn compress_a_selection_then_extract_it_here_twice() {
        let (mut s, dir) = setup();
        let p = dir.path();
        let sources = [
            uri(&p.join("project/docs")),
            uri(&p.join("project/notes.txt")),
        ];
        s.dispatch(
            "archive.compress",
            &json!({"sources": sources, "target": uri(&p.join("out.zip")), "format": "zip"}),
        )
        .expect("compress");
        wait_jobs(&mut s);
        assert!(p.join("out.zip").is_file());
        let leftovers: Vec<_> = std::fs::read_dir(p)
            .expect("ls")
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("fmtmp"))
            .collect();
        assert!(leftovers.is_empty(), "no temp file left behind");
        for _ in 0..2 {
            s.dispatch("archive.extract", &json!({"uri": uri(&p.join("out.zip"))}))
                .expect("extract");
            wait_jobs(&mut s);
        }
        assert_eq!(
            std::fs::read(p.join("out/docs/a.txt")).expect("a"),
            b"alpha"
        );
        assert_eq!(
            std::fs::read(p.join("out (2)/notes.txt")).expect("n"),
            b"notes"
        );
    }

    #[test]
    fn quick_add_names_the_archive_and_split_makes_parts() {
        let (mut s, dir) = setup();
        let p = dir.path();
        s.dispatch(
            "archive.compress_quick",
            &json!({"sources": [uri(&p.join("project/notes.txt"))], "format": "7z"}),
        )
        .expect("quick");
        wait_jobs(&mut s);
        assert!(p.join("project/notes.7z").is_file());
        let mut big = std::fs::File::create(p.join("project/big.bin")).expect("big");
        let noise: Vec<u8> = (0..300_000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        big.write_all(&noise).expect("w");
        drop(big);
        s.dispatch(
            "archive.compress",
            &json!({"sources": [uri(&p.join("project/big.bin"))], "target": uri(&p.join("parts.tar")), "format": "tar", "split": 100_000}),
        )
        .expect("split");
        wait_jobs(&mut s);
        for part in [
            "parts.tar.001",
            "parts.tar.002",
            "parts.tar.003",
            "parts.tar.004",
        ] {
            assert!(p.join(part).is_file(), "{part}");
        }
        assert!(!p.join("parts.tar").exists());
    }

    #[test]
    fn dialog_formats_are_the_writable_ones() {
        for f in FORMATS {
            let id = FormatId::from_id(f.id).expect(f.id);
            assert!(WRITABLE.contains(&id), "{}", f.id);
            assert_eq!(suffix_of(id), f.suffix);
            assert_eq!(f.password, !f.id.starts_with("tar"));
        }
        assert_eq!(FORMATS.len(), WRITABLE.len());
    }

    #[test]
    fn bad_params_are_errors() {
        let (mut s, dir) = setup();
        let p = dir.path();
        let notes = uri(&p.join("project/notes.txt"));
        let target = uri(&p.join("x.tar.gz"));
        for bad in [
            json!({"sources": [notes], "target": target, "format": "rar"}),
            json!({"sources": [], "target": target, "format": "zip"}),
            json!({"sources": [notes], "target": target, "format": "zip", "level": "99"}),
            json!({"sources": [notes], "target": target, "format": "tar.gz", "password": "x"}),
            json!({"sources": [notes], "target": target, "format": "zip", "split": 10}),
            json!({"sources": [notes], "target": notes, "format": "zip"}),
            json!({"sources": [notes], "format": "zip"}),
            Params::Null,
        ] {
            assert!(s.dispatch("archive.compress", &bad).is_err(), "{bad}");
        }
        for bad in [Params::Null, json!({"uri": notes}), json!({"uri": 3})] {
            assert!(s.dispatch("archive.extract", &bad).is_err(), "{bad}");
            assert!(s.dispatch("archive.extract_to", &bad).is_err(), "{bad}");
        }
        assert!(
            s.dispatch(
                "archive.compress_quick",
                &json!({"sources": [notes], "format": "tar"})
            )
            .is_err()
        );
        for id in ["archive.add_zip", "archive.add_7z"] {
            assert!(s.dispatch(id, &json!(5)).is_err());
            assert!(s.dispatch(id, &json!({"sources": 5})).is_err());
            assert!(s.dispatch(id, &json!({"sources": []})).is_err());
        }
        assert!(s.fileops.jobs().is_empty(), "nothing was started");
    }
}
