//! Sort commands: column key + directories-first toggle. Sorting is in-memory
//! over the cached entries via the pane's view index — never a re-read.

use fm_layout::SortKey;
use fm_vfs::FmError;

use crate::registry::{CommandSpec, Params};
use crate::session::Session;

fn set_key(s: &mut Session, key: SortKey) -> Result<(), FmError> {
    let focused = s.layout.focused();
    let pane = s
        .layout
        .state_mut(focused)
        .ok_or_else(|| FmError::NotFound("focused pane".to_string()))?;
    if pane.listing.sort.key != key {
        pane.listing.sort.key = key;
        pane.listing.sort.dir = fm_layout::SortDir::Asc;
    }
    s.invalidate_view(focused);
    Ok(())
}

fn run_name(s: &mut Session, _p: &Params) -> Result<(), FmError> {
    set_key(s, SortKey::Name)
}
fn run_size(s: &mut Session, _p: &Params) -> Result<(), FmError> {
    set_key(s, SortKey::Size)
}
fn run_mtime(s: &mut Session, _p: &Params) -> Result<(), FmError> {
    set_key(s, SortKey::Mtime)
}
fn run_type(s: &mut Session, _p: &Params) -> Result<(), FmError> {
    set_key(s, SortKey::Type)
}

/// `view.sort.name`
pub fn by_name() -> CommandSpec {
    CommandSpec {
        id: "view.sort.name",
        label: "Name",
        menu: &["View", "Sort", "Name"],
        shortcut: None,
        params: "none",
        enabled: |_s| true,
        run: run_name,
    }
}

/// `view.sort.size`
pub fn by_size() -> CommandSpec {
    CommandSpec {
        id: "view.sort.size",
        label: "Size",
        menu: &["View", "Sort", "Size"],
        shortcut: None,
        params: "none",
        enabled: |_s| true,
        run: run_size,
    }
}

/// `view.sort.mtime`
pub fn by_mtime() -> CommandSpec {
    CommandSpec {
        id: "view.sort.mtime",
        label: "Modified",
        menu: &["View", "Sort", "Modified"],
        shortcut: None,
        params: "none",
        enabled: |_s| true,
        run: run_mtime,
    }
}

/// `view.sort.type`
pub fn by_type() -> CommandSpec {
    CommandSpec {
        id: "view.sort.type",
        label: "Type",
        menu: &["View", "Sort", "Type"],
        shortcut: None,
        params: "none",
        enabled: |_s| true,
        run: run_type,
    }
}

/// `view.sort.dirs_first` — toggle directories-ahead-of-files.
pub fn dirs_first() -> CommandSpec {
    CommandSpec {
        id: "view.sort.dirs_first",
        label: "Directories First",
        menu: &["View", "Sort", "Directories First"],
        shortcut: Some("Ctrl+H"),
        params: "none",
        enabled: |_s| true,
        run: |s, _p: &Params| {
            let focused = s.layout.focused();
            let pane = s
                .layout
                .state_mut(focused)
                .ok_or_else(|| FmError::NotFound("focused pane".to_string()))?;
            pane.listing.sort.dirs_first = !pane.listing.sort.dirs_first;
            s.invalidate_view(focused);
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![by_name(), by_size(), by_mtime(), by_type(), dirs_first()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Params;
    use fm_testkit::MemFs;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn loaded_session() -> (Session, MemFs, fm_vfs::VfsUri) {
        let mut s = Session::default();
        let mem = MemFs::new();
        mem.mkdirs_str("/d").expect("mkdirs");
        mem.put_str("/d/big.bin", &[0u8; 64]).expect("put");
        mem.put_str("/d/small.txt", b"s").expect("put");
        s.backends.register(Arc::new(mem.clone()));
        let uri = fm_vfs::VfsUri::parse("/d").expect("uri");
        s.layout.state_mut(1).expect("pane").location = uri.clone();
        s.submit_read(uri.clone()).expect("submit");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            s.poll();
            if s.dirs.state(&uri).is_ready() {
                break;
            }
            assert!(Instant::now() < deadline, "read never ready");
            std::thread::sleep(Duration::from_millis(5));
        }
        (s, mem, uri)
    }

    fn names(s: &mut Session) -> Vec<String> {
        let (entries, ix) = s.pane_view(1).expect("view");
        ix.order
            .iter()
            .map(|i| entries.get(*i as usize).expect("entry").name.clone())
            .collect()
    }

    #[test]
    fn size_sort_orders_files_and_header_click_semantics_apply() {
        let (mut s, _mem, _uri) = loaded_session();
        assert_eq!(names(&mut s), vec!["big.bin", "small.txt"]); // name asc
        (by_size().run)(&mut s, &Params::Null).expect("size sort");
        // dirs_first default: no dirs here; small(1) before big(64)
        assert_eq!(names(&mut s), vec!["small.txt", "big.bin"]);
        // Clicking Size again flips to descending.
        {
            let pane = s.layout.state_mut(1).expect("pane");
            pane.listing.sort.header_click(SortKey::Size);
        }
        s.invalidate_view(1);
        assert_eq!(names(&mut s), vec!["big.bin", "small.txt"]);
    }

    #[test]
    fn toggling_dirs_first_flips_the_grouping() {
        let (mut s, _mem, _uri) = loaded_session();
        // Add a dir and re-read.
        _mem.mkdirs_str("/d/zdir").expect("mkdirs");
        (crate::nav_cmds::refresh().run)(&mut s, &Params::Null).expect("refresh");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            s.poll();
            let n = s.dirs.entries(&_uri).map(|e| e.len()).unwrap_or(0);
            if n == 3 {
                break;
            }
            assert!(Instant::now() < deadline, "refresh never landed");
            std::thread::sleep(Duration::from_millis(5));
        }
        (by_name().run)(&mut s, &Params::Null).expect("name sort");
        assert_eq!(names(&mut s), vec!["zdir", "big.bin", "small.txt"]);
        (dirs_first().run)(&mut s, &Params::Null).expect("toggle");
        assert_eq!(names(&mut s), vec!["big.bin", "small.txt", "zdir"]);
    }

    #[test]
    fn sort_commands_survive_a_corrupt_session() {
        let mut s = Session::default();
        s.layout.focus.focused = 404;
        assert!((by_name().run)(&mut s, &Params::Null).is_err());
        assert!((dirs_first().run)(&mut s, &Params::Null).is_err());
    }
}
