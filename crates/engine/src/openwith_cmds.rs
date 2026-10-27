//! Open With: choose an installed application for a local file.
//!
//! Linux (and other freedesktop systems) scan the XDG `applications/*.desktop` entries; the
//! file's type comes from the shared-mime-info glob table and the user's defaults from
//! `mimeapps.list`. macOS lists `/Applications` and launches with `open -a`. Windows hands
//! the file to the system's own Open With dialog.
//!
//! The scan runs once on its own thread; launching only accepts an application id from that
//! scan, expands the `Exec` line without a shell and starts the program detached.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use fm_vfs::{FmError, VfsUri};

use crate::file_cmds::{focused_selection, object, parse_uri};
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// Most applications kept from a scan; largest file read while scanning.
const MAX_APPS: usize = 2_000;
const MAX_SCAN_FILE: u64 = 4 * 1024 * 1024;
const MAX_ARGS: usize = 256;

/// An installed application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    /// Desktop-file id (`org.gnome.gedit.desktop`) or the bundle path on macOS.
    pub id: String,
    pub name: String,
    /// `Exec` line (freedesktop) — never run through a shell.
    pub exec: String,
    pub mime_types: Vec<String>,
}

/// Result of the one-time scan.
#[derive(Clone, Debug, Default)]
pub struct AppCatalog {
    pub apps: Vec<App>,
    /// File extension (lower-case, no dot) → mime type.
    pub globs: HashMap<String, String>,
    /// Mime type → default application ids, most preferred first.
    pub defaults: HashMap<String, Vec<String>>,
}

/// The Open With dialog.
#[derive(Debug)]
pub struct OpenWith {
    pub target: VfsUri,
    catalog: Arc<Mutex<Option<Arc<AppCatalog>>>>,
}

/// One line of the dialog's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppChoice {
    pub id: String,
    pub name: String,
    /// Registered for this file's type.
    pub recommended: bool,
    pub default: bool,
}

impl OpenWith {
    /// `None` while the scan is still running.
    pub fn choices(&self) -> Option<(Option<String>, Vec<AppChoice>)> {
        let catalog = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()?;
        let mime = mime_for(&catalog, self.target.file_name().unwrap_or_default());
        Some((mime.clone(), rank(&catalog, mime.as_deref())))
    }
}

/// Mime type of a file name by its extension (`archive.tar.gz` tries `tar.gz` first).
pub fn mime_for(catalog: &AppCatalog, name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    let mut rest = lower.as_str();
    while let Some((_, ext)) = rest.split_once('.') {
        if let Some(m) = catalog.globs.get(ext) {
            return Some(m.clone());
        }
        rest = ext;
    }
    None
}

/// Applications for `mime` first (defaults on top), then every other one, by name.
pub fn rank(catalog: &AppCatalog, mime: Option<&str>) -> Vec<AppChoice> {
    let defaults = mime
        .and_then(|m| catalog.defaults.get(m))
        .cloned()
        .unwrap_or_default();
    let family = mime
        .and_then(|m| m.split_once('/'))
        .map(|(f, _)| format!("{f}/*"));
    let mut out: Vec<AppChoice> = catalog
        .apps
        .iter()
        .map(|a| AppChoice {
            id: a.id.clone(),
            name: a.name.clone(),
            recommended: mime.is_some_and(|m| {
                a.mime_types
                    .iter()
                    .any(|t| t == m || Some(t) == family.as_ref())
            }) || defaults.contains(&a.id),
            default: defaults.first() == Some(&a.id),
        })
        .collect();
    out.sort_by(|a, b| {
        b.default
            .cmp(&a.default)
            .then(b.recommended.cmp(&a.recommended))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

/// Parse the `[Desktop Entry]` group of a `.desktop` file. Hidden, `NoDisplay`, terminal
/// and non-application entries are skipped.
pub fn parse_desktop(id: &str, text: &str) -> Option<App> {
    let mut in_entry = false;
    let mut fields: HashMap<&str, &str> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            fields.entry(k.trim()).or_insert(v.trim());
        }
    }
    let flag = |k: &str| fields.get(k).is_some_and(|v| *v == "true");
    if fields.get("Type") != Some(&"Application")
        || flag("NoDisplay")
        || flag("Hidden")
        || flag("Terminal")
    {
        return None;
    }
    let name = fields.get("Name").filter(|n| !n.is_empty())?;
    let exec = fields.get("Exec").filter(|e| !e.is_empty())?;
    Some(App {
        id: id.to_string(),
        name: name.chars().take(200).collect(),
        exec: exec.to_string(),
        mime_types: fields
            .get("MimeType")
            .map(|m| {
                m.split(';')
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// Split an `Exec` value into arguments (desktop-entry quoting) and substitute the file.
/// `%f %F %u %U` become `path`; other field codes are dropped; `%%` is a literal `%`.
/// Without any file code the path is appended.
pub fn expand_exec(exec: &str, path: &str) -> Result<Vec<String>, FmError> {
    let bad = || FmError::InvalidParams("the application's Exec line is malformed".to_string());
    let mut args: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut has_token = false;
    let mut quoted = false;
    let mut used_file = false;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                has_token = true;
            }
            '\\' if quoted => cur.push(chars.next().ok_or_else(bad)?),
            c if c.is_whitespace() && !quoted => {
                if has_token {
                    args.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            '%' => {
                has_token = true;
                match chars.next().ok_or_else(bad)? {
                    '%' => cur.push('%'),
                    'f' | 'F' | 'u' | 'U' => {
                        cur.push_str(path);
                        used_file = true;
                    }
                    _ => {}
                }
            }
            c => {
                cur.push(c);
                has_token = true;
            }
        }
        if args.len() > MAX_ARGS {
            return Err(bad());
        }
    }
    if quoted {
        return Err(bad());
    }
    if has_token && !cur.is_empty() {
        args.push(cur);
    }
    args.retain(|a| !a.is_empty());
    if args.is_empty() {
        return Err(bad());
    }
    if !used_file {
        args.push(path.to_string());
    }
    Ok(args)
}

fn read_small(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_SCAN_FILE {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

fn data_dirs() -> Vec<PathBuf> {
    let home = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf());
    let mut dirs = Vec::new();
    match std::env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
        Some(d) if d.is_absolute() => dirs.push(d),
        _ => dirs.extend(home.as_ref().map(|h| h.join(".local/share"))),
    }
    dirs.extend(
        home.as_ref()
            .map(|h| h.join(".local/share/flatpak/exports/share")),
    );
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share"));
    let system = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
    dirs.extend(
        system
            .split(':')
            .map(PathBuf::from)
            .filter(|d| d.is_absolute()),
    );
    dirs
}

fn scan_apps_dir(
    dir: &Path,
    prefix: &str,
    depth: usize,
    out: &mut Vec<App>,
    seen: &mut Vec<String>,
) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut items: Vec<_> = rd.flatten().collect();
    items.sort_by_key(|e| e.file_name());
    for item in items {
        if out.len() >= MAX_APPS {
            return;
        }
        let name = item.file_name().to_string_lossy().into_owned();
        let path = item.path();
        if path.is_dir() && depth < 2 {
            scan_apps_dir(&path, &format!("{prefix}{name}-"), depth + 1, out, seen);
            continue;
        }
        if !name.ends_with(".desktop") {
            continue;
        }
        let id = format!("{prefix}{name}");
        // The first data dir wins: a user's override hides the system entry.
        if seen.contains(&id) {
            continue;
        }
        seen.push(id.clone());
        if let Some(app) = read_small(&path).and_then(|t| parse_desktop(&id, &t)) {
            out.push(app);
        }
    }
}

/// Parse `globs2` lines (`weight:type:glob`), keeping simple `*.ext` globs; the first
/// (highest-weight) mapping of an extension wins.
pub fn parse_globs(text: &str, into: &mut HashMap<String, String>) {
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(3, ':');
        let (_, Some(mime), Some(glob)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let glob = glob.split(':').next().unwrap_or(glob);
        if let Some(ext) = glob.strip_prefix("*.")
            && !ext.contains(['*', '?', '['])
        {
            into.entry(ext.to_lowercase())
                .or_insert_with(|| mime.to_string());
        }
    }
}

/// Parse the `[Default Applications]` (and `[Added Associations]`) groups of a
/// `mimeapps.list`; earlier files win.
pub fn parse_mimeapps(text: &str, into: &mut HashMap<String, Vec<String>>) {
    let mut group = "";
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            group = line;
            continue;
        }
        if group != "[Default Applications]" && group != "[Added Associations]" {
            continue;
        }
        if let Some((mime, ids)) = line.split_once('=') {
            let list = into.entry(mime.trim().to_string()).or_default();
            for id in ids.split(';').map(str::trim).filter(|i| !i.is_empty()) {
                if !list.iter().any(|x| x == id) {
                    list.push(id.to_string());
                }
            }
        }
    }
}

fn scan() -> AppCatalog {
    let mut catalog = AppCatalog::default();
    if cfg!(target_os = "macos") {
        if let Ok(rd) = std::fs::read_dir("/Applications") {
            for item in rd.flatten() {
                let path = item.path();
                if path.extension().is_some_and(|e| e == "app")
                    && let (Some(id), Some(name)) =
                        (path.to_str(), path.file_stem().and_then(|s| s.to_str()))
                {
                    catalog.apps.push(App {
                        id: id.to_string(),
                        name: name.to_string(),
                        exec: String::new(),
                        mime_types: Vec::new(),
                    });
                }
            }
        }
        return catalog;
    }
    let dirs = data_dirs();
    let mut seen = Vec::new();
    for dir in &dirs {
        scan_apps_dir(
            &dir.join("applications"),
            "",
            0,
            &mut catalog.apps,
            &mut seen,
        );
        if let Some(text) = read_small(&dir.join("mime/globs2")) {
            parse_globs(&text, &mut catalog.globs);
        }
    }
    let config = directories::BaseDirs::new().map(|b| b.config_dir().to_path_buf());
    let mut lists: Vec<PathBuf> = config
        .into_iter()
        .map(|c| c.join("mimeapps.list"))
        .collect();
    lists.extend(dirs.iter().map(|d| d.join("applications/mimeapps.list")));
    for list in lists {
        if let Some(text) = read_small(&list) {
            parse_mimeapps(&text, &mut catalog.defaults);
        }
    }
    catalog
}

fn launch(app: &App, path: &Path) -> Result<(), FmError> {
    use std::process::{Command, Stdio};
    let text = path
        .to_str()
        .ok_or_else(|| FmError::InvalidParams("the path is not valid UTF-8".to_string()))?;
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg("-a").arg(&app.id).arg(path);
        c
    } else {
        let args = expand_exec(&app.exec, text)?;
        let (program, rest) = args
            .split_first()
            .ok_or_else(|| FmError::InvalidParams("the application has no command".to_string()))?;
        let mut c = Command::new(program);
        c.args(rest);
        c
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = path.parent() {
        cmd.current_dir(dir);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| FmError::backend("open", format!("cannot start {}: {e}", app.name)))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

impl Session {
    fn app_catalog(&mut self) -> Arc<Mutex<Option<Arc<AppCatalog>>>> {
        if let Some(c) = &self.app_catalog {
            return c.clone();
        }
        let slot: Arc<Mutex<Option<Arc<AppCatalog>>>> = Arc::new(Mutex::new(None));
        let fill = slot.clone();
        let spawned = std::thread::Builder::new()
            .name("fm-apps".into())
            .spawn(move || {
                let catalog = Arc::new(scan());
                *fill
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(catalog);
            });
        if spawned.is_err() {
            *slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(Arc::new(AppCatalog::default()));
        }
        self.app_catalog = Some(slot.clone());
        slot
    }
}

fn target(s: &Session, map: Option<&crate::file_cmds::Obj>) -> Result<VfsUri, FmError> {
    match map.and_then(|m| m.get("uri")) {
        Some(v) => parse_uri(v),
        None => focused_selection(s)
            .into_iter()
            .next()
            .ok_or_else(|| FmError::InvalidParams("select a file first".to_string())),
    }
}

/// `file.open_with.dialog` — choose an application for the selected file.
pub fn dialog() -> CommandSpec {
    CommandSpec {
        id: "file.open_with.dialog",
        label: "Open With…",
        menu: &["File", "Open With…"],
        shortcut: None,
        params: "uri?: string",
        enabled: |s| focused_selection(s).len() == 1,
        run: |s, params| {
            let uri = target(s, object(params)?)?;
            let path = crate::open_cmds::local_path(&uri)?;
            if cfg!(windows) {
                // The system dialog lists and remembers applications itself.
                let mut cmd = std::process::Command::new("rundll32.exe");
                cmd.arg("shell32.dll,OpenAs_RunDLL").arg(&path);
                cmd.spawn()
                    .map_err(|e| FmError::backend("open", format!("cannot show Open With: {e}")))?;
                return Ok(());
            }
            let catalog = s.app_catalog();
            s.open_with = Some(OpenWith {
                target: uri,
                catalog,
            });
            Ok(())
        },
    }
}

/// `file.open_with` — open a local file with an application from the last scan.
pub fn open_with() -> CommandSpec {
    CommandSpec {
        id: "file.open_with",
        label: "Open With Application",
        menu: &["File", "Open With Application"],
        shortcut: None,
        params: "app: string (id from the Open With list), uri?: string",
        enabled: |s| s.open_with.is_some(),
        run: |s, params| {
            let map = object(params)?;
            let id = map
                .and_then(|m| m.get("app"))
                .and_then(Params::as_str)
                .ok_or_else(|| FmError::InvalidParams("app: string is required".to_string()))?;
            let uri = match map.and_then(|m| m.get("uri")) {
                Some(v) => parse_uri(v)?,
                None => s
                    .open_with
                    .as_ref()
                    .map(|o| o.target.clone())
                    .ok_or_else(|| FmError::InvalidParams("uri is required".to_string()))?,
            };
            let path = crate::open_cmds::local_path(&uri)?;
            let catalog = s
                .app_catalog
                .as_ref()
                .and_then(|c| c.lock().ok().and_then(|g| g.clone()))
                .ok_or_else(|| FmError::NotFound("the application list".to_string()))?;
            let app = catalog
                .apps
                .iter()
                .find(|a| a.id == id)
                .ok_or_else(|| FmError::NotFound(format!("application {id}")))?;
            launch(app, &path)?;
            s.open_with = None;
            Ok(())
        },
    }
}

pub fn close() -> CommandSpec {
    CommandSpec {
        id: "file.open_with.close",
        label: "Close Open With",
        menu: &["File", "Close Open With"],
        shortcut: None,
        params: "none",
        enabled: |s| s.open_with.is_some(),
        run: |s, params| {
            object(params)?;
            s.open_with = None;
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![dialog(), open_with(), close()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn desktop_entries_parse_and_hidden_ones_are_skipped() {
        let text = "[Desktop Entry]\nType=Application\nName=Text Editor\nName[nl]=Teksteditor\n\
                    Exec=gedit %U\nMimeType=text/plain;text/x-log;\n\n[Desktop Action new]\nName=New\nExec=gedit --new\n";
        let app = parse_desktop("org.gnome.gedit.desktop", text).expect("app");
        assert_eq!(app.name, "Text Editor");
        assert_eq!(app.exec, "gedit %U");
        assert_eq!(app.mime_types, ["text/plain", "text/x-log"]);
        for skip in ["NoDisplay=true", "Hidden=true", "Terminal=true"] {
            let t = format!("[Desktop Entry]\nType=Application\nName=x\nExec=x\n{skip}\n");
            assert!(parse_desktop("x.desktop", &t).is_none(), "{skip}");
        }
        assert!(parse_desktop("x", "[Desktop Entry]\nType=Link\nName=x\nExec=x").is_none());
        assert!(parse_desktop("x", "garbage\0\u{7}").is_none());
    }

    #[test]
    fn exec_lines_expand_without_a_shell() {
        let p = "/home/u/my file.txt";
        assert_eq!(expand_exec("gedit %U", p).expect("x"), ["gedit", p]);
        assert_eq!(
            expand_exec("\"/opt/My App/bin\" --x=%f %i %c", p).expect("x"),
            ["/opt/My App/bin", &format!("--x={p}")]
        );
        assert_eq!(expand_exec("vlc", p).expect("x"), ["vlc", p]);
        assert_eq!(
            expand_exec("echo 100%% %F", p).expect("x"),
            ["echo", "100%", p]
        );
        assert_eq!(
            expand_exec("sh -c \"rm -rf \\$HOME\" %f", p).expect("x"),
            ["sh", "-c", "rm -rf $HOME", p],
            "quoting yields literal args; nothing is interpreted"
        );
        for bad in ["", "\"unterminated", "x %", "   "] {
            assert!(expand_exec(bad, p).is_err(), "{bad:?}");
        }
        assert!(expand_exec(&"a ".repeat(MAX_ARGS + 2), p).is_err());
    }

    #[test]
    fn mime_globs_defaults_and_ranking() {
        let mut catalog = AppCatalog::default();
        parse_globs(
            "# comment\n80:text/plain:*.txt\n50:text/x-other:*.txt\n50:application/x-compressed-tar:*.tar.gz\n50:text/x-readme:README*\nbroken\n",
            &mut catalog.globs,
        );
        assert_eq!(mime_for(&catalog, "A.TXT").as_deref(), Some("text/plain"));
        assert_eq!(
            mime_for(&catalog, "x.tar.gz").as_deref(),
            Some("application/x-compressed-tar")
        );
        assert_eq!(mime_for(&catalog, "README"), None);
        parse_mimeapps(
            "[Default Applications]\ntext/plain=b.desktop;a.desktop\n[Removed Associations]\ntext/plain=c.desktop\n",
            &mut catalog.defaults,
        );
        let app = |id: &str, name: &str, mime: &[&str]| App {
            id: id.into(),
            name: name.into(),
            exec: "x".into(),
            mime_types: mime.iter().map(|m| m.to_string()).collect(),
        };
        catalog.apps = vec![
            app("z.desktop", "Zed", &[]),
            app("a.desktop", "Alpha", &["text/plain"]),
            app("b.desktop", "Beta", &[]),
            app("c.desktop", "Code", &["text/*"]),
        ];
        let ranked = rank(&catalog, Some("text/plain"));
        let ids: Vec<_> = ranked.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["b.desktop", "a.desktop", "c.desktop", "z.desktop"]);
        assert!(ranked[0].default && ranked[2].recommended && !ranked[3].recommended);
    }

    #[test]
    fn scanning_this_machine_is_total() {
        let catalog = scan();
        assert!(catalog.apps.len() <= MAX_APPS);
        assert!(catalog.apps.iter().all(|a| !a.name.is_empty()));
    }

    #[test]
    fn bad_params_are_errors() {
        let mut s = Session::new().expect("session");
        for (id, p) in [
            ("file.open_with.dialog", json!({"uri": "sftp://h/x.txt"})),
            ("file.open_with.dialog", json!({"uri": 7})),
            ("file.open_with", json!({})),
            (
                "file.open_with",
                json!({"app": "nope.desktop", "uri": "/tmp/x"}),
            ),
        ] {
            assert!(s.dispatch(id, &p).is_err(), "{id} {p}");
        }
    }
}
