//! Hostile-input properties for `VfsUri` (capability `security`): parsing is total,
//! normalization can never escape the root, and display round-trips.

use fm_vfs::{FmError, Scheme, VfsUri};
use proptest::prelude::*;

fn any_uri() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>(),
        "[ -~]{0,200}", // printable ASCII
        "/[^\\x00-\\x1f]{0,100}",
        "(\\.\\.|\\.|/|//|///){0,50}",
        "file://[^\\x00-\\x1f]{0,100}",
        "smb://[^\\x00-\\x1f]{0,100}",
        "sftp://[^\\x00-\\x1f]{0,100}",
        "[A-Za-z]:[/\\\\][^\\x00-\\x1f]{0,100}",
        "\\\\\\\\[^\\x00-\\x1f]{0,100}",
        "a{1,4000}",
        Just(format!("/{}/{}", "x".repeat(4096), "y".repeat(100))),
    ]
}

fn assert_normalized(uri: &VfsUri) {
    let path = uri.path_str();
    assert!(path.starts_with('/'), "not absolute: {path}");
    assert!(
        path == "/" || !path.ends_with('/'),
        "trailing slash: {path}"
    );
    if path != "/" {
        for seg in path.split('/').skip(1) {
            assert!(!seg.is_empty(), "empty component in {path}");
            assert_ne!(seg, ".", "dot component in {path}");
            assert_ne!(seg, "..", "dotdot component in {path}");
        }
    }
}

proptest! {
    #[test]
    fn parse_never_panics(input in any_uri()) {
        let _ = VfsUri::parse(&input);
    }

    #[test]
    fn parse_bytes_never_panics(input in any_uri()) {
        let _ = VfsUri::parse_bytes(input.as_bytes());
    }

    #[test]
    fn ok_implies_normalized(input in any_uri()) {
        if let Ok(uri) = VfsUri::parse(&input) {
            assert_normalized(&uri);
        }
    }

    #[test]
    fn display_round_trips(input in any_uri()) {
        if let Ok(uri) = VfsUri::parse(&input) {
            let shown = uri.to_string();
            let again = VfsUri::parse(&shown).expect("display form must re-parse");
            prop_assert_eq!(uri, again);
        }
    }

    #[test]
    fn parent_chain_reaches_root(input in any_uri()) {
        if let Ok(uri) = VfsUri::parse(&input) {
            let mut cur = uri.clone();
            let mut steps = 0;
            while let Some(parent) = cur.parent() {
                prop_assert!(parent.is_ancestor_of(&cur));
                prop_assert!(parent.path_str().len() <= cur.path_str().len());
                cur = parent;
                steps += 1;
                prop_assert!(steps <= 4096, "parent chain did not terminate");
            }
            prop_assert_eq!(cur.path_str(), "/");
            prop_assert_eq!(cur.parent(), None);
            let root = cur.root();
            prop_assert_eq!(cur.path_str(), root.path_str());
        }
    }

    #[test]
    fn join_stays_descendant(input in any_uri(), name in "[^/\\x00-\\x1f]{1,64}") {
        if let Ok(uri) = VfsUri::parse(&input)
            && let Ok(child) = uri.join(&name)
        {
            prop_assert!(uri.is_ancestor_of(&child));
            prop_assert_eq!(child.file_name(), Some(name.as_str()));
            assert_normalized(&child);
        }
    }

    #[test]
    fn ancestor_is_antisymmetric(a in any_uri(), b in any_uri()) {
        if let (Ok(a), Ok(b)) = (VfsUri::parse(&a), VfsUri::parse(&b)) {
            prop_assert!(!(a.is_ancestor_of(&b) && b.is_ancestor_of(&a)));
        }
    }

    #[test]
    fn invalid_uri_display_redacts(input in any_uri()) {
        if let Err(err) = VfsUri::parse(&input) {
            prop_assert!(matches!(err, FmError::InvalidUri(_)));
            let shown = err.to_string();
            prop_assert!(!shown.contains('\0'));
        }
    }

    #[test]
    fn file_scheme_never_carries_authority(input in any_uri()) {
        if let Ok(uri) = VfsUri::parse(&input)
            && uri.scheme() == Scheme::File
        {
            prop_assert!(uri.authority().is_local());
        }
    }
}
