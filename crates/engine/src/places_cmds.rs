//! Places: the standard user folders (Home, Desktop, Documents…) plus the user's own
//! bookmarks (kept in prefs), shown above the folder tree and in the Go menu.

use fm_vfs::{FmError, VfsUri};

use crate::file_cmds::{focused_dir, object, parse_uri};
use crate::prefs::{Bookmark, MAX_BOOKMARK_NAME, MAX_BOOKMARKS};
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// One clickable entry under Places.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    pub uri: VfsUri,
    /// A user bookmark (can be renamed/removed); otherwise a standard folder.
    pub bookmark: bool,
    pub kind: PlaceKind,
}

/// What a place is, so the UI can pick an icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceKind {
    Home,
    Desktop,
    Documents,
    Downloads,
    Music,
    Pictures,
    Videos,
    /// Windows: the drive list.
    Computer,
    Bookmark,
}

/// The standard folders, resolved once at start-up (it reads `user-dirs.dirs` on Linux).
#[derive(Clone, Debug, Default)]
pub struct StandardPlaces(pub Vec<Place>);

impl StandardPlaces {
    pub fn detect() -> Self {
        let mut out = Vec::new();
        let mut push = |name: &str, kind: PlaceKind, path: Option<&std::path::Path>| {
            if let Some(uri) = path
                .and_then(std::path::Path::to_str)
                .and_then(|p| VfsUri::parse(p).ok())
                && !out.iter().any(|p: &Place| p.uri == uri)
            {
                out.push(Place {
                    name: name.to_string(),
                    uri,
                    bookmark: false,
                    kind,
                });
            }
        };
        if let Some(dirs) = directories::UserDirs::new() {
            push("Home", PlaceKind::Home, Some(dirs.home_dir()));
            push("Desktop", PlaceKind::Desktop, dirs.desktop_dir());
            push("Documents", PlaceKind::Documents, dirs.document_dir());
            push("Downloads", PlaceKind::Downloads, dirs.download_dir());
            push("Music", PlaceKind::Music, dirs.audio_dir());
            push("Pictures", PlaceKind::Pictures, dirs.picture_dir());
            push("Videos", PlaceKind::Videos, dirs.video_dir());
        }
        if cfg!(windows) {
            out.push(Place {
                name: "This PC".to_string(),
                uri: VfsUri::root_file(),
                bookmark: false,
                kind: PlaceKind::Computer,
            });
        }
        Self(out)
    }
}

impl Session {
    /// Standard folders followed by the user's bookmarks (invalid saved ones skipped).
    pub fn places(&self) -> Vec<Place> {
        let mut out = self.standard_places.0.clone();
        out.extend(self.prefs.bookmarks.iter().filter_map(|b| {
            Some(Place {
                name: b.name.clone(),
                uri: VfsUri::parse(&b.uri).ok()?,
                bookmark: true,
                kind: PlaceKind::Bookmark,
            })
        }));
        out
    }

    pub fn is_bookmarked(&self, uri: &VfsUri) -> bool {
        self.prefs
            .bookmarks
            .iter()
            .any(|b| VfsUri::parse(&b.uri).is_ok_and(|u| u == *uri))
    }
}

fn target(s: &Session, params: &Params) -> Result<VfsUri, FmError> {
    match object(params)?.and_then(|m| m.get("uri")) {
        Some(v) => parse_uri(v),
        None => focused_dir(s)
            .ok_or_else(|| FmError::InvalidParams("no folder to bookmark".to_string())),
    }
}

fn name_param(params: &Params) -> Result<Option<String>, FmError> {
    match object(params)?.and_then(|m| m.get("name")) {
        None => Ok(None),
        Some(v) => {
            let name = v
                .as_str()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .ok_or_else(|| FmError::InvalidParams("name must be a non-empty string".into()))?;
            if name.chars().count() > MAX_BOOKMARK_NAME || name.chars().any(char::is_control) {
                return Err(FmError::InvalidParams(format!(
                    "bookmark names are one line of at most {MAX_BOOKMARK_NAME} characters"
                )));
            }
            Ok(Some(name.to_string()))
        }
    }
}

fn position(s: &Session, uri: &VfsUri) -> Option<usize> {
    s.prefs
        .bookmarks
        .iter()
        .position(|b| VfsUri::parse(&b.uri).is_ok_and(|u| u == *uri))
}

fn default_name(uri: &VfsUri) -> String {
    let name = uri
        .file_name()
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| crate::item_cmds::display_path(uri));
    name.chars().take(MAX_BOOKMARK_NAME).collect()
}

/// `bookmark.add` — pin a folder (default: the focused one) under Places.
pub fn add() -> CommandSpec {
    CommandSpec {
        id: "bookmark.add",
        label: "Bookmark This Folder",
        menu: &["Go", "Bookmark This Folder"],
        shortcut: Some("Ctrl+D"),
        params: "uri?: string, name?: string",
        enabled: |s| focused_dir(s).is_some_and(|d| !s.is_bookmarked(&d)),
        run: |s, params| {
            let uri = target(s, params)?;
            let name = name_param(params)?.unwrap_or_else(|| default_name(&uri));
            if position(s, &uri).is_some() {
                s.info = Some((format!("“{name}” is already bookmarked"), now()));
                return Ok(());
            }
            if s.prefs.bookmarks.len() >= MAX_BOOKMARKS {
                return Err(FmError::InvalidParams(format!(
                    "at most {MAX_BOOKMARKS} bookmarks"
                )));
            }
            s.prefs.bookmarks.push(Bookmark {
                name: name.clone(),
                uri: uri.to_string(),
            });
            s.info = Some((format!("Bookmarked “{name}”"), now()));
            Ok(())
        },
    }
}

fn now() -> std::time::Instant {
    std::time::Instant::now()
}

fn required_uri(params: &Params) -> Result<VfsUri, FmError> {
    let v = object(params)?
        .and_then(|m| m.get("uri"))
        .ok_or_else(|| FmError::InvalidParams("uri is required".to_string()))?;
    parse_uri(v)
}

/// `bookmark.remove` — unpin a bookmark.
pub fn remove() -> CommandSpec {
    CommandSpec {
        id: "bookmark.remove",
        label: "Remove Bookmark",
        menu: &["Go", "Remove Bookmark"],
        shortcut: None,
        params: "uri: string",
        enabled: |s| !s.prefs.bookmarks.is_empty(),
        run: |s, params| {
            let uri = required_uri(params)?;
            let ix = position(s, &uri)
                .ok_or_else(|| FmError::NotFound(format!("bookmark {}", default_name(&uri))))?;
            s.prefs.bookmarks.remove(ix);
            Ok(())
        },
    }
}

/// `bookmark.rename` — change the label shown under Places.
pub fn rename() -> CommandSpec {
    CommandSpec {
        id: "bookmark.rename",
        label: "Rename Bookmark…",
        menu: &["Go", "Rename Bookmark…"],
        shortcut: None,
        params: "uri: string, name: string",
        enabled: |s| !s.prefs.bookmarks.is_empty(),
        run: |s, params| {
            let uri = required_uri(params)?;
            let name = name_param(params)?
                .ok_or_else(|| FmError::InvalidParams("name is required".to_string()))?;
            let ix = position(s, &uri)
                .ok_or_else(|| FmError::NotFound(format!("bookmark {}", default_name(&uri))))?;
            if let Some(b) = s.prefs.bookmarks.get_mut(ix) {
                b.name = name;
            }
            Ok(())
        },
    }
}

/// `bookmark.move` — move a bookmark one place up (`by: -1`), down (`by: 1`) or to position
/// `to` (0-based among the bookmarks; drag and drop).
pub fn move_() -> CommandSpec {
    CommandSpec {
        id: "bookmark.move",
        label: "Move Bookmark",
        menu: &["Go", "Move Bookmark"],
        shortcut: None,
        params: "uri: string, by?: -1|1, to?: index (one of by/to)",
        enabled: |s| s.prefs.bookmarks.len() > 1,
        run: |s, params| {
            let uri = required_uri(params)?;
            let map = object(params)?;
            let by = map.and_then(|m| m.get("by"));
            let to = map.and_then(|m| m.get("to"));
            let ix = position(s, &uri)
                .ok_or_else(|| FmError::NotFound(format!("bookmark {}", default_name(&uri))))?;
            let last = s.prefs.bookmarks.len().saturating_sub(1);
            let target = match (by, to) {
                (Some(by), None) => {
                    let by = by
                        .as_i64()
                        .filter(|b| *b == -1 || *b == 1)
                        .ok_or_else(|| FmError::InvalidParams("by must be -1 or 1".to_string()))?;
                    if by < 0 {
                        ix.saturating_sub(1)
                    } else {
                        (ix + 1).min(last)
                    }
                }
                (None, Some(to)) => {
                    let to = to.as_u64().ok_or_else(|| {
                        FmError::InvalidParams("to must be a non-negative index".to_string())
                    })?;
                    usize::try_from(to).unwrap_or(usize::MAX).min(last)
                }
                _ => {
                    return Err(FmError::InvalidParams(
                        "give exactly one of by or to".to_string(),
                    ));
                }
            };
            if target != ix {
                let b = s.prefs.bookmarks.remove(ix);
                s.prefs.bookmarks.insert(target, b);
            }
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![add(), remove(), rename(), move_()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn move_to_an_index_reorders_and_bad_params_fail() {
        let mut s = Session::new().expect("session");
        s.prefs.bookmarks.clear();
        for u in ["/a", "/b", "/c"] {
            s.dispatch("bookmark.add", &json!({ "uri": u }))
                .expect("add");
        }
        let names = |s: &Session| -> Vec<String> {
            s.prefs.bookmarks.iter().map(|b| b.name.clone()).collect()
        };
        s.dispatch("bookmark.move", &json!({"uri": "/c", "to": 0}))
            .expect("to 0");
        assert_eq!(names(&s), ["c", "a", "b"]);
        s.dispatch("bookmark.move", &json!({"uri": "/c", "to": 99}))
            .expect("clamped");
        assert_eq!(names(&s), ["a", "b", "c"]);
        for bad in [
            json!({"uri": "/a"}),
            json!({"uri": "/a", "to": -1}),
            json!({"uri": "/a", "by": 1, "to": 0}),
            json!({"uri": "/a", "by": 2}),
        ] {
            assert!(s.dispatch("bookmark.move", &bad).is_err(), "{bad}");
        }
        assert_eq!(names(&s), ["a", "b", "c"]);
    }

    #[test]
    fn add_rename_move_remove_round_trip() {
        let mut s = Session::new().expect("session");
        s.prefs.bookmarks.clear();
        s.dispatch("bookmark.add", &json!({"uri": "/srv/photos"}))
            .expect("add");
        s.dispatch("bookmark.add", &json!({"uri": "/srv/photos"}))
            .expect("dup is a no-op");
        s.dispatch(
            "bookmark.add",
            &json!({"uri": "sftp://h/var/www", "name": "Web"}),
        )
        .expect("add remote");
        let names: Vec<_> = s
            .places()
            .into_iter()
            .filter(|p| p.bookmark)
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["photos", "Web"]);
        s.dispatch(
            "bookmark.move",
            &json!({"uri": "sftp://h/var/www", "by": -1}),
        )
        .expect("move");
        s.dispatch(
            "bookmark.move",
            &json!({"uri": "sftp://h/var/www", "by": -1}),
        )
        .expect("move at the top is a no-op");
        s.dispatch(
            "bookmark.rename",
            &json!({"uri": "/srv/photos", "name": " Pics "}),
        )
        .expect("rename");
        let names: Vec<_> = s.prefs.bookmarks.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["Web", "Pics"]);
        assert!(s.prefs.bookmarks.iter().all(Bookmark::is_valid));
        s.dispatch("bookmark.remove", &json!({"uri": "/srv/photos"}))
            .expect("remove");
        assert_eq!(s.prefs.bookmarks.len(), 1);
        assert!(s.is_bookmarked(&VfsUri::parse("sftp://h/var/www").expect("uri")));
    }

    #[test]
    fn bad_params_and_the_cap_are_errors() {
        let mut s = Session::new().expect("session");
        s.prefs.bookmarks.clear();
        for (id, p) in [
            ("bookmark.add", json!({"uri": 5})),
            ("bookmark.add", json!({"uri": "/a", "name": ""})),
            (
                "bookmark.add",
                json!({"uri": "/a", "name": "x".repeat(MAX_BOOKMARK_NAME + 1)}),
            ),
            ("bookmark.add", json!({"uri": "/a", "name": "a\nb"})),
            ("bookmark.remove", json!({})),
            ("bookmark.remove", json!({"uri": "/not/bookmarked"})),
            ("bookmark.rename", json!({"uri": "/a"})),
            ("bookmark.move", json!({"uri": "/a", "by": 7})),
        ] {
            assert!(s.dispatch(id, &p).is_err(), "{id} {p}");
        }
        for i in 0..MAX_BOOKMARKS {
            s.dispatch("bookmark.add", &json!({"uri": format!("/b/{i}")}))
                .expect("add");
        }
        assert!(
            s.dispatch("bookmark.add", &json!({"uri": "/one/more"}))
                .is_err()
        );
    }
}
