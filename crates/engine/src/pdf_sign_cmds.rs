//! Signing (design D7–D9): `pdf.sign.*` — make a visual signature (drawn, typed or an
//! image), place, move and remove it, sign digitally with a PKCS#12 certificate, and
//! remember up to [`MAX_SAVED`] visual signatures on this computer.
//!
//! The certificate password is a [`Secret`]: it is never stored, logged or shown, and is
//! dropped as soon as signing finishes. Remembered signatures live in the app's data
//! folder (`signatures/`, owner-only) and never leave it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use fm_layout::PaneId;
use fm_pdf::forms::FieldKind;
use fm_pdf::sign::{Appearance, Request};
use fm_pdf::visual::{self, Placement, Visual, VisualKind};
use fm_pdf::{PdfError, Secret};
use fm_vfs::{FmError, VfsUri};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::pdf_form_cmds::{
    DigitalJob, FormMsg, SaveTarget, read_capped, ready_session, session_mut,
};
use crate::pdf_view::{PdfMsg, push};
use crate::pdf_view_cmds::{MAX_PASSWORD, pdf_pane};
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// Remembered visual signatures at most.
pub const MAX_SAVED: usize = 8;
/// Largest remembered signature file.
pub const MAX_SAVED_BYTES: usize = 8 << 20;
/// Placed visual signatures per document at most.
pub const MAX_PLACEMENTS: usize = 32;

/// A remembered visual signature.
#[derive(Clone, Debug)]
pub struct SavedSignature {
    /// File stem in the signatures folder.
    pub id: String,
    pub visual: Arc<Visual>,
}

/// On-disk form of a drawn or typed signature (images are stored as they are).
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Stored {
    Drawn {
        strokes: Vec<Vec<[f32; 2]>>,
        width: f32,
        height: f32,
    },
    Typed {
        name: String,
    },
}

/// The signatures folder: `<data>/signatures`, or a test folder.
pub(crate) fn default_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::prefs::project_dirs().map(|d| d.data_local_dir().join("signatures"))
}

fn args(params: &Params, allowed: &[&str]) -> Result<Map<String, Value>, FmError> {
    let map = crate::file_cmds::object(params)?
        .cloned()
        .unwrap_or_default();
    for k in map.keys() {
        if k != "pane" && !allowed.contains(&k.as_str()) {
            return Err(FmError::InvalidParams(format!("unknown parameter {k}")));
        }
    }
    Ok(map)
}

fn text(map: &Map<String, Value>, key: &str, max_chars: usize) -> Result<Option<String>, FmError> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.len() <= max_chars * 4 && s.chars().count() <= max_chars => {
            Ok(Some(s.clone()))
        }
        Some(_) => Err(FmError::InvalidParams(format!(
            "{key} must be text of at most {max_chars} characters"
        ))),
    }
}

fn rect(v: Option<&Value>) -> Result<[f32; 4], FmError> {
    let bad = || FmError::InvalidParams("rect must be [x0, y0, x1, y1] in page points".into());
    let a = v
        .and_then(Value::as_array)
        .filter(|a| a.len() == 4)
        .ok_or_else(bad)?;
    let mut r = [0f32; 4];
    for (slot, x) in r.iter_mut().zip(a) {
        let f = x
            .as_f64()
            .filter(|f| f.is_finite() && f.abs() < 1e6)
            .ok_or_else(bad)?;
        *slot = f as f32;
    }
    let [x0, y0, x1, y1] = r;
    Ok([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)])
}

fn strokes(v: Option<&Value>) -> Result<Vec<Vec<[f32; 2]>>, FmError> {
    let bad = || FmError::InvalidParams("strokes must be lists of [x, y] points".into());
    let list = v
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty() && a.len() <= visual::MAX_STROKES)
        .ok_or_else(bad)?;
    let mut total = 0usize;
    let mut out = Vec::with_capacity(list.len());
    for s in list {
        let pts = s.as_array().ok_or_else(bad)?;
        total += pts.len();
        if total > visual::MAX_POINTS * visual::MAX_STROKES {
            return Err(bad());
        }
        let mut stroke = Vec::with_capacity(pts.len());
        for p in pts {
            let xy = p.as_array().filter(|a| a.len() == 2).ok_or_else(bad)?;
            let c = |i: usize| {
                xy.get(i)
                    .and_then(Value::as_f64)
                    .filter(|f| f.is_finite() && f.abs() < 1e5)
                    .map(|f| f as f32)
            };
            stroke.push([c(0).ok_or_else(bad)?, c(1).ok_or_else(bad)?]);
        }
        out.push(stroke);
    }
    Ok(out)
}

fn pad(map: &Map<String, Value>, key: &str) -> Result<f32, FmError> {
    map.get(key)
        .and_then(Value::as_f64)
        .filter(|f| f.is_finite() && *f >= 1.0 && *f <= 10_000.0)
        .map(|f| f as f32)
        .ok_or_else(|| FmError::InvalidParams(format!("{key} must be the pad's size in points")))
}

fn pdf_err(e: PdfError) -> FmError {
    FmError::from(e)
}

/// `pdf.sign.visual`: make the visual signature that the next `pdf.sign.place` uses.
fn make_visual(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = args(
        params,
        &[
            "kind",
            "strokes",
            "width",
            "height",
            "name",
            "uri",
            "remove_white",
            "id",
        ],
    )?;
    let pane = pdf_pane(s, params)?;
    let kind = map
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| FmError::InvalidParams("kind must be drawn, typed, image or saved".into()))?
        .to_string();
    let inbox = s.pdf_inbox.clone();
    let send = move |result: Result<Arc<Visual>, String>| {
        push(
            &inbox,
            PdfMsg::Form(Box::new(FormMsg::Visual { pane, result })),
        );
    };
    let built: Option<VisualKind> = match kind.as_str() {
        "drawn" => Some(VisualKind::Drawn {
            strokes: strokes(map.get("strokes"))?,
            width: pad(&map, "width")?,
            height: pad(&map, "height")?,
        }),
        "typed" => {
            let name = text(&map, "name", visual::MAX_TYPED)?
                .filter(|n| !n.trim().is_empty())
                .ok_or_else(|| FmError::InvalidParams("type a name".into()))?;
            if let Err(c) = visual::typed_supported(&name) {
                return Err(FmError::InvalidParams(format!(
                    "The signature font has no “{c}”; draw the signature or use an image."
                )));
            }
            Some(VisualKind::Typed { name })
        }
        "image" => None,
        "saved" => {
            let id = text(&map, "id", 64)?
                .ok_or_else(|| FmError::InvalidParams("id is required".into()))?;
            let saved = s
                .pdf_saved
                .iter()
                .find(|x| x.id == id)
                .cloned()
                .ok_or_else(|| FmError::InvalidParams("no such remembered signature".into()))?;
            let session = ready_session(s, pane)?;
            session.pending_visual = Some(saved.visual);
            session.error = None;
            return Ok(());
        }
        _ => {
            return Err(FmError::InvalidParams(
                "kind must be drawn, typed, image or saved".into(),
            ));
        }
    };
    let image = if built.is_none() {
        let uri = text(&map, "uri", 4096)?
            .ok_or_else(|| FmError::InvalidParams("uri is required for an image".into()))?;
        let remove_white = match map.get("remove_white") {
            None => false,
            Some(v) => v.as_bool().ok_or_else(|| {
                FmError::InvalidParams("remove_white must be true or false".into())
            })?,
        };
        Some((VfsUri::parse(&uri)?, remove_white))
    } else {
        None
    };
    let session = ready_session(s, pane)?;
    session.visual_loading = true;
    session.error = None;
    match (built, image) {
        (Some(kind), _) => {
            s.pdf
                .run(move || send(Visual::new(kind).map_err(|e| e.to_string())));
        }
        (None, Some((uri, remove_white))) => {
            let vfs = s.backends.require(&uri)?;
            let lane = crate::backends::lane_for(uri.scheme());
            s.jobs.submit(lane, move |_| {
                let result = read_capped(&*vfs, &uri, visual::MAX_IMAGE_BYTES).and_then(|data| {
                    Visual::new(VisualKind::Image { data, remove_white }).map_err(|e| e.to_string())
                });
                send(result);
                Ok(())
            });
        }
        (None, None) => {}
    }
    Ok(())
}

/// Empty signature field `name` of the session: its first widget's page and rectangle.
fn signature_field(
    s: &mut Session,
    pane: PaneId,
    name: &str,
) -> Result<(usize, [f32; 4]), FmError> {
    let session = ready_session(s, pane)?;
    let field = session
        .fields
        .iter()
        .find(|f| f.name == name)
        .ok_or_else(|| FmError::InvalidParams(format!("there is no field named “{name}”")))?;
    match field.kind {
        FieldKind::Signature { signed: false } => {}
        FieldKind::Signature { signed: true } => {
            return Err(FmError::InvalidParams(
                "this signature field is already signed".into(),
            ));
        }
        _ => {
            return Err(FmError::InvalidParams(format!(
                "“{name}” is not a signature field"
            )));
        }
    }
    let w = field.widgets.first().ok_or_else(|| {
        FmError::InvalidParams("the signature field has no place on a page".into())
    })?;
    Ok((w.page, w.rect))
}

fn page_size(s: &mut Session, pane: PaneId, page: usize) -> Result<(f32, f32), FmError> {
    let shown = s
        .pdf_shown(pane)
        .ok_or_else(|| FmError::InvalidParams("the PDF is not loaded".into()))?;
    shown
        .doc
        .info()
        .pages
        .get(page)
        .map(|p| (p.width, p.height))
        .ok_or_else(|| FmError::InvalidParams(format!("page {} does not exist", page + 1)))
}

fn page_index(map: &Map<String, Value>) -> Result<usize, FmError> {
    map.get("page")
        .and_then(Value::as_u64)
        .and_then(|p| usize::try_from(p).ok())
        .ok_or_else(|| FmError::InvalidParams("page must be a page index".into()))
}

/// `pdf.sign.place {page, rect}` or `{field}`.
fn place(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = args(params, &["page", "rect", "field"])?;
    let pane = pdf_pane(s, params)?;
    let field = text(&map, "field", 1024)?;
    let (page, rect) = match &field {
        Some(name) => signature_field(s, pane, name)?,
        None => {
            let page = page_index(&map)?;
            let r = rect(map.get("rect"))?;
            visual::check_rect(r, page_size(s, pane, page)?).map_err(pdf_err)?;
            (page, r)
        }
    };
    let session = ready_session(s, pane)?;
    if session.is_saving() {
        return Err(FmError::InvalidParams("wait for the save to finish".into()));
    }
    if session.placements.len() >= MAX_PLACEMENTS {
        return Err(FmError::InvalidParams(format!(
            "A document takes at most {MAX_PLACEMENTS} placed signatures."
        )));
    }
    if let Some(f) = &field
        && session
            .placements
            .iter()
            .any(|p| p.field.as_deref() == Some(f))
    {
        return Err(FmError::InvalidParams(
            "this field already holds a signature".into(),
        ));
    }
    let visual = session
        .pending_visual
        .clone()
        .ok_or_else(|| FmError::InvalidParams("make a signature first".into()))?;
    let id = session.next_placement_id();
    session.placements.push(Placement {
        id,
        page,
        rect,
        field,
        visual,
    });
    session.editing = true;
    session.revision += 1;
    session.error = None;
    session.prompt = None;
    s.rebuild_pdf(pane);
    Ok(())
}

fn placement_id(map: &Map<String, Value>) -> Result<u64, FmError> {
    map.get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| FmError::InvalidParams("id must be a placed signature".into()))
}

/// `pdf.sign.move {id, rect, page?}`: placements in fields stay where the field is.
fn move_placement(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = args(params, &["id", "rect", "page"])?;
    let pane = pdf_pane(s, params)?;
    let id = placement_id(&map)?;
    let r = rect(map.get("rect"))?;
    let current = ready_session(s, pane)?
        .placements
        .iter()
        .find(|p| p.id == id)
        .map(|p| (p.page, p.field.is_some()))
        .ok_or_else(|| FmError::InvalidParams("no such placed signature".into()))?;
    if current.1 {
        return Err(FmError::InvalidParams(
            "a signature in a signature field cannot be moved".into(),
        ));
    }
    let page = if map.contains_key("page") {
        page_index(&map)?
    } else {
        current.0
    };
    visual::check_rect(r, page_size(s, pane, page)?).map_err(pdf_err)?;
    let session = ready_session(s, pane)?;
    if session.is_saving() {
        return Err(FmError::InvalidParams("wait for the save to finish".into()));
    }
    if let Some(p) = session.placements.iter_mut().find(|p| p.id == id) {
        p.rect = r;
        p.page = page;
    }
    session.revision += 1;
    session.prompt = None;
    s.rebuild_pdf(pane);
    Ok(())
}

fn remove(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = args(params, &["id"])?;
    let pane = pdf_pane(s, params)?;
    let id = placement_id(&map)?;
    let session = ready_session(s, pane)?;
    if session.is_saving() {
        return Err(FmError::InvalidParams("wait for the save to finish".into()));
    }
    let before = session.placements.len();
    session.placements.retain(|p| p.id != id);
    if session.placements.len() == before {
        return Err(FmError::InvalidParams("no such placed signature".into()));
    }
    session.revision += 1;
    session.prompt = None;
    s.rebuild_pdf(pane);
    Ok(())
}

/// `pdf.sign.digital`: build the filled document, then sign it as the last step of the
/// save. Fails cleanly (nothing written) on a wrong password or an unusable certificate.
fn digital(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = args(
        params,
        &[
            "cert",
            "password",
            "field",
            "page",
            "rect",
            "appearance",
            "reason",
            "location",
            "contact",
            "uri",
            "overwrite",
        ],
    )?;
    let pane = pdf_pane(s, params)?;
    let cert = text(&map, "cert", 4096)?
        .ok_or_else(|| FmError::InvalidParams("cert must be the certificate file".into()))?;
    let cert = VfsUri::parse(&cert)?;
    let password = match map.get("password") {
        Some(Value::String(p)) if p.len() <= MAX_PASSWORD * 4 => Secret::new(p),
        _ => return Err(FmError::InvalidParams("password must be text".into())),
    };
    let field = text(&map, "field", 1024)?;
    let appearance = match map.get("appearance").map(Value::as_str) {
        None | Some(Some("name_date")) => "name_date",
        Some(Some("none")) => "none",
        Some(Some("visual")) => "visual",
        _ => {
            return Err(FmError::InvalidParams(
                "appearance must be none, visual or name_date".into(),
            ));
        }
    };
    let mut request = Request {
        reason: text(&map, "reason", fm_pdf::sign::MAX_INFO_CHARS)?,
        location: text(&map, "location", fm_pdf::sign::MAX_INFO_CHARS)?,
        contact: text(&map, "contact", fm_pdf::sign::MAX_INFO_CHARS)?,
        ..Request::default()
    };
    if let Some(name) = &field {
        let (page, r) = signature_field(s, pane, name)?;
        request.field = Some(name.clone());
        request.page = page;
        request.rect = Some(r);
    } else if appearance != "none" {
        let page = page_index(&map)?;
        let r = rect(map.get("rect"))?;
        visual::check_rect(r, page_size(s, pane, page)?).map_err(pdf_err)?;
        request.page = page;
        request.rect = Some(r);
    }
    let target = match (text(&map, "uri", 4096)?, map.get("overwrite")) {
        (Some(uri), None) => SaveTarget::As(VfsUri::parse(&uri)?),
        (None, Some(Value::Bool(true))) => SaveTarget::Overwrite,
        (None, None | Some(Value::Bool(false))) => SaveTarget::Copy,
        _ => {
            return Err(FmError::InvalidParams(
                "give either uri or overwrite: true, not both".into(),
            ));
        }
    };
    let session = ready_session(s, pane)?;
    if session.is_saving() {
        return Err(FmError::InvalidParams("a save is already running".into()));
    }
    request.appearance = match appearance {
        "none" => Appearance::None,
        "visual" => Appearance::Visual(
            session
                .pending_visual
                .clone()
                .ok_or_else(|| FmError::InvalidParams("make a visual signature first".into()))?,
        ),
        _ => Appearance::NameAndDate,
    };
    if appearance == "none" && field.is_none() {
        request.rect = None;
    }
    request.validate().map_err(pdf_err)?;
    // The signing job reports through the session's save state.
    s.start_save(
        pane,
        target,
        true,
        Some(DigitalJob {
            cert,
            password,
            request,
        }),
    )
}

fn load_dir(dir: &Path) -> Result<Vec<SavedSignature>, String> {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let mut names: Vec<(std::time::SystemTime, PathBuf)> = rd
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| {
            let t = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (t, e.path())
        })
        .take(64)
        .collect();
    names.sort();
    let mut out = Vec::new();
    for (_, path) in names {
        if out.len() >= MAX_SAVED {
            break;
        }
        let Some(file) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some((id, ext)) = file.split_once('.') else {
            continue;
        };
        if id.is_empty() || id.len() > 32 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.len() > MAX_SAVED_BYTES as u64 {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let kind = match ext {
            "json" => match serde_json::from_slice::<Stored>(&bytes) {
                Ok(Stored::Drawn {
                    strokes,
                    width,
                    height,
                }) => VisualKind::Drawn {
                    strokes,
                    width,
                    height,
                },
                Ok(Stored::Typed { name }) => VisualKind::Typed { name },
                Err(_) => continue,
            },
            "img" => VisualKind::Image {
                data: bytes,
                remove_white: false,
            },
            "white.img" => VisualKind::Image {
                data: bytes,
                remove_white: true,
            },
            _ => continue,
        };
        // A damaged or hostile file is skipped, not fatal.
        if let Ok(visual) = Visual::new(kind) {
            out.push(SavedSignature {
                id: id.to_string(),
                visual,
            });
        }
    }
    Ok(out)
}

fn signatures_dir(s: &Session) -> Result<PathBuf, FmError> {
    s.pdf_signatures_dir.clone().ok_or_else(|| {
        FmError::InvalidParams("there is no folder for remembered signatures".into())
    })
}

/// File work to do in the signatures folder before it is listed again.
type DirTask = Box<dyn FnOnce(&Path) -> Result<(), String> + Send>;

fn reload(s: &mut Session, dir: PathBuf, then: Option<DirTask>) {
    let inbox = s.pdf_inbox.clone();
    s.pdf_saved_loading = true;
    s.pdf.run(move || {
        let result = match then {
            Some(f) => f(&dir).and_then(|()| load_dir(&dir)),
            None => load_dir(&dir),
        };
        push(&inbox, PdfMsg::Saved(result));
    });
}

fn saved_load(s: &mut Session, params: &Params) -> Result<(), FmError> {
    args(params, &[])?;
    let dir = signatures_dir(s)?;
    reload(s, dir, None);
    Ok(())
}

fn saved_store(s: &mut Session, params: &Params) -> Result<(), FmError> {
    args(params, &[])?;
    let pane = pdf_pane(s, params)?;
    let dir = signatures_dir(s)?;
    let visual = session_mut(s, pane)?
        .pending_visual
        .clone()
        .ok_or_else(|| FmError::InvalidParams("make a signature first".into()))?;
    if s.pdf_saved.len() >= MAX_SAVED {
        return Err(FmError::InvalidParams(format!(
            "At most {MAX_SAVED} signatures are remembered; forget one first."
        )));
    }
    let (ext, bytes) = match &visual.kind {
        VisualKind::Drawn {
            strokes,
            width,
            height,
        } => (
            "json",
            serde_json::to_vec(&Stored::Drawn {
                strokes: strokes.clone(),
                width: *width,
                height: *height,
            })
            .map_err(|e| FmError::Io(std::io::Error::other(e)))?,
        ),
        VisualKind::Typed { name } => (
            "json",
            serde_json::to_vec(&Stored::Typed { name: name.clone() })
                .map_err(|e| FmError::Io(std::io::Error::other(e)))?,
        ),
        VisualKind::Image { data, remove_white } => (
            if *remove_white { "white.img" } else { "img" },
            data.clone(),
        ),
    };
    if bytes.len() > MAX_SAVED_BYTES {
        return Err(FmError::InvalidParams(
            "the signature is too large to remember".into(),
        ));
    }
    let id = format!(
        "sig{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let file = format!("{id}.{ext}");
    reload(
        s,
        dir,
        Some(Box::new(move |dir: &Path| {
            crate::prefs::write_atomic(dir, &file, &bytes).map_err(|e| e.to_string())
        })),
    );
    Ok(())
}

fn saved_delete(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = args(params, &["id"])?;
    let id =
        text(&map, "id", 64)?.ok_or_else(|| FmError::InvalidParams("id is required".into()))?;
    if !s.pdf_saved.iter().any(|x| x.id == id) {
        return Err(FmError::InvalidParams(
            "no such remembered signature".into(),
        ));
    }
    let dir = signatures_dir(s)?;
    reload(
        s,
        dir,
        Some(Box::new(move |dir: &Path| {
            for ext in ["json", "img", "white.img"] {
                match std::fs::remove_file(dir.join(format!("{id}.{ext}"))) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
            Ok(())
        })),
    );
    Ok(())
}

fn active(s: &Session) -> bool {
    let pane = s.layout.focused();
    s.pdf_view(pane)
        .and_then(|v| v.fill.as_ref())
        .is_some_and(|f| f.state == crate::pdf_form_cmds::FillState::Ready)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "pdf.sign.visual",
            label: "Make Signature",
            menu: &["Preview", "PDF", "Make Signature"],
            shortcut: None,
            params: "{kind: \"drawn\", strokes: [[[x, y], ...], ...], width, height} | {kind: \"typed\", name} | {kind: \"image\", uri, remove_white?} | {kind: \"saved\", id}",
            enabled: active,
            run: make_visual,
        },
        CommandSpec {
            id: "pdf.sign.place",
            label: "Place Signature",
            menu: &["Preview", "PDF", "Place Signature"],
            shortcut: None,
            params: "{page: number, rect: [x0, y0, x1, y1] (page points)} | {field: string (empty signature field)}",
            enabled: active,
            run: place,
        },
        CommandSpec {
            id: "pdf.sign.move",
            label: "Move Signature",
            menu: &["Preview", "PDF", "Move Signature"],
            shortcut: None,
            params: "{id: number, rect: [x0, y0, x1, y1], page?: number}",
            enabled: active,
            run: move_placement,
        },
        CommandSpec {
            id: "pdf.sign.remove",
            label: "Remove Signature",
            menu: &["Preview", "PDF", "Remove Signature"],
            shortcut: None,
            params: "{id: number}",
            enabled: active,
            run: remove,
        },
        CommandSpec {
            id: "pdf.sign.digital",
            label: "Sign Digitally",
            menu: &["Preview", "PDF", "Sign Digitally"],
            shortcut: None,
            params: "{cert: uri (.p12/.pfx), password: string, field?: string | page + rect, appearance?: \"none\"|\"visual\"|\"name_date\", reason?, location?, contact?, uri? | overwrite?: true}",
            enabled: active,
            run: digital,
        },
        CommandSpec {
            id: "pdf.sign.saved.load",
            label: "Load Remembered Signatures",
            menu: &["Preview", "PDF", "Remembered Signatures"],
            shortcut: None,
            params: "{}",
            enabled: |s| s.pdf_signatures_dir.is_some(),
            run: saved_load,
        },
        CommandSpec {
            id: "pdf.sign.saved.store",
            label: "Remember Signature",
            menu: &["Preview", "PDF", "Remember Signature"],
            shortcut: None,
            params: "{pane?: number} (the signature made last)",
            enabled: active,
            run: saved_store,
        },
        CommandSpec {
            id: "pdf.sign.saved.delete",
            label: "Forget Signature",
            menu: &["Preview", "PDF", "Forget Signature"],
            shortcut: None,
            params: "{id: string}",
            enabled: |s| !s.pdf_saved.is_empty(),
            run: saved_delete,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf_form_cmds::SaveState;
    use crate::pdf_form_cmds::tests::{filling, wait};
    use serde_json::json;

    fn visual_ready(s: &Session, pane: PaneId) -> bool {
        s.pdf_view(pane)
            .and_then(|v| v.fill.as_ref())
            .is_some_and(|f| !f.visual_loading && f.pending_visual.is_some())
    }

    #[test]
    fn draw_place_move_remove() {
        let (mut s, pane, _tmp) = filling(fm_pdf::fixtures::form());
        let p = |v: Value| {
            let mut v = v;
            v["pane"] = json!(pane);
            v
        };
        assert!(
            s.dispatch(
                "pdf.sign.place",
                &p(json!({"page": 0, "rect": [10, 10, 110, 60]}))
            )
            .is_err(),
            "nothing to place yet"
        );
        s.dispatch(
            "pdf.sign.visual",
            &p(json!({"kind": "drawn", "strokes": [[[0, 0], [50, 20], [100, 5]]], "width": 100, "height": 30})),
        )
        .expect("draw");
        wait(&mut s, |s| visual_ready(s, pane));
        for bad in [
            json!({"page": 0, "rect": [10, 10, 11, 11]}),
            json!({"page": 0, "rect": [10, 10, 9000, 60]}),
            json!({"page": 7, "rect": [10, 10, 110, 60]}),
            json!({"page": 0, "rect": [10, "x", 110, 60]}),
            json!({"field": "name"}),
            json!({"field": "nope"}),
        ] {
            assert!(
                s.dispatch("pdf.sign.place", &p(bad.clone())).is_err(),
                "{bad}"
            );
        }
        s.dispatch(
            "pdf.sign.place",
            &p(json!({"page": 0, "rect": [10, 10, 110, 60]})),
        )
        .expect("place");
        s.dispatch("pdf.sign.place", &p(json!({"field": "sig"})))
            .expect("into field");
        let ids: Vec<u64> = s
            .pdf_view(pane)
            .and_then(|v| v.fill.as_ref())
            .map(|f| f.placements.iter().map(|p| p.id).collect())
            .unwrap_or_default();
        assert_eq!(ids.len(), 2);
        s.dispatch(
            "pdf.sign.move",
            &p(json!({"id": ids[0], "rect": [20, 20, 120, 70]})),
        )
        .expect("move");
        assert!(
            s.dispatch(
                "pdf.sign.move",
                &p(json!({"id": ids[1], "rect": [20, 20, 120, 70]}))
            )
            .is_err()
        );
        s.dispatch("pdf.sign.remove", &p(json!({"id": ids[1]})))
            .expect("remove");
        assert!(
            s.dispatch("pdf.sign.remove", &p(json!({"id": 999})))
                .is_err()
        );
        s.dispatch("pdf.save", &p(json!({"confirm": true})))
            .expect("save");
        wait(&mut s, |s| {
            s.pdf_view(pane).is_some_and(|v| v.fill.is_none())
        });
        assert!(_tmp.path().join("form (signed).pdf").exists());
    }

    #[test]
    fn typed_names_outside_the_font_are_refused() {
        let (mut s, pane, _tmp) = filling(fm_pdf::fixtures::form());
        assert!(
            s.dispatch(
                "pdf.sign.visual",
                &json!({"kind": "typed", "name": "李雷", "pane": pane})
            )
            .is_err()
        );
        s.dispatch(
            "pdf.sign.visual",
            &json!({"kind": "typed", "name": "Jane Doe", "pane": pane}),
        )
        .expect("typed");
        wait(&mut s, |s| visual_ready(s, pane));
    }

    #[test]
    fn digital_signature_wrong_password_writes_nothing_then_signs() {
        let (mut s, pane, tmp) = filling(fm_pdf::fixtures::form());
        let p12 = fm_pdf::fixtures::certs::p12(&fm_pdf::fixtures::certs::P12Spec::default());
        let cert = tmp.path().join("me.p12");
        std::fs::write(&cert, p12).expect("p12");
        let cert = cert.to_string_lossy().to_string();
        s.dispatch(
            "pdf.sign.digital",
            &json!({"cert": cert, "password": "wrong", "field": "sig", "pane": pane}),
        )
        .expect("start");
        wait(&mut s, |s| {
            s.pdf_view(pane)
                .and_then(|v| v.fill.as_ref())
                .is_some_and(|f| matches!(f.save, SaveState::Failed(_)))
        });
        assert!(!tmp.path().join("form (signed).pdf").exists());
        let notice = s.notice.clone().unwrap_or_default();
        assert!(!notice.contains("wrong"), "{notice}");
        s.dispatch(
            "pdf.sign.digital",
            &json!({"cert": cert, "password": "test-pass", "field": "sig", "reason": "Approved", "pane": pane}),
        )
        .expect("sign");
        wait(&mut s, |s| {
            s.pdf_view(pane).is_some_and(|v| v.fill.is_none())
        });
        let bytes = std::fs::read(tmp.path().join("form (signed).pdf")).expect("signed");
        let doc = fm_pdf::Document::open(bytes.into(), None).expect("open");
        let reports = fm_pdf::verify::verify_all(&doc);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].status, fm_pdf::verify::SigStatus::Valid);
    }

    #[test]
    fn remembered_signatures_are_owner_only_and_capped() {
        let (mut s, pane, tmp) = filling(fm_pdf::fixtures::form());
        let dir = tmp.path().join("signatures");
        s.pdf_signatures_dir = Some(dir.clone());
        s.dispatch(
            "pdf.sign.visual",
            &json!({"kind": "typed", "name": "Jane", "pane": pane}),
        )
        .expect("typed");
        wait(&mut s, |s| visual_ready(s, pane));
        s.dispatch("pdf.sign.saved.store", &json!({"pane": pane}))
            .expect("store");
        wait(&mut s, |s| !s.pdf_saved_loading);
        assert_eq!(s.pdf_saved.len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir).expect("dir").permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
            for e in std::fs::read_dir(&dir).expect("ls") {
                let mode = e.expect("e").metadata().expect("m").permissions().mode();
                assert_eq!(mode & 0o777, 0o600);
            }
        }
        // Junk in the folder is skipped.
        std::fs::write(dir.join("evil.json"), b"{\"kind\":\"typed\",\"name\":").expect("w");
        std::fs::write(dir.join("../escape.json"), b"{}").expect("w");
        s.dispatch("pdf.sign.saved.load", &json!({})).expect("load");
        wait(&mut s, |s| !s.pdf_saved_loading);
        assert_eq!(s.pdf_saved.len(), 1);
        let id = s.pdf_saved[0].id.clone();
        s.dispatch(
            "pdf.sign.visual",
            &json!({"kind": "saved", "id": id, "pane": pane}),
        )
        .expect("use saved");
        s.dispatch("pdf.sign.saved.delete", &json!({"id": id}))
            .expect("forget");
        wait(&mut s, |s| !s.pdf_saved_loading);
        assert!(s.pdf_saved.is_empty());
        assert!(
            s.dispatch("pdf.sign.saved.delete", &json!({"id": "../x"}))
                .is_err()
        );
    }
}
