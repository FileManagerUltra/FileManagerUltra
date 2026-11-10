use std::collections::BTreeMap;

use fm_vfs::{FmError, Scheme, VfsUri};

use crate::MAX_QUEUE_ITEMS;

pub const MAX_PLAYLIST_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PLAYLIST_LINE_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaylistFormat {
    M3u,
    M3u8,
    Pls,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Playlist {
    pub items: Vec<VfsUri>,
    /// URL or credential-bearing entries are never resolved or fetched.
    pub skipped: usize,
}

pub fn parse_playlist(
    input: &str,
    format: PlaylistFormat,
    playlist_dir: &VfsUri,
) -> Result<Playlist, FmError> {
    if input.len() > MAX_PLAYLIST_BYTES {
        return Err(invalid_playlist("playlist exceeds the size limit"));
    }
    if !matches!(
        playlist_dir.scheme(),
        Scheme::File | Scheme::Smb | Scheme::Sftp
    ) || playlist_dir.authority().user.is_some()
    {
        return Err(invalid_playlist("invalid playlist directory"));
    }
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let entries = match format {
        PlaylistFormat::M3u | PlaylistFormat::M3u8 => parse_m3u(input)?,
        PlaylistFormat::Pls => parse_pls(input)?,
    };
    let mut playlist = Playlist {
        items: Vec::new(),
        skipped: 0,
    };
    for entry in entries {
        if let Some(uri) = resolve_entry(&entry, playlist_dir)? {
            playlist.items.push(uri);
        } else {
            playlist.skipped += 1;
        }
    }
    Ok(playlist)
}

pub fn write_m3u8(items: &[VfsUri]) -> Result<String, FmError> {
    if items.len() > MAX_QUEUE_ITEMS {
        return Err(invalid_playlist("too many playlist entries"));
    }
    let mut output = String::from("#EXTM3U\n");
    for item in items {
        let line = match item.scheme() {
            Scheme::File if item.authority().user.is_none() => item.path_str().to_string(),
            Scheme::Smb | Scheme::Sftp if item.authority().user.is_none() => item.to_string(),
            _ => return Err(invalid_playlist("unsupported playlist item")),
        };
        if line.len() > MAX_PLAYLIST_LINE_BYTES || line.chars().any(char::is_control) {
            return Err(invalid_playlist("invalid playlist entry"));
        }
        if output.len() + line.len() + 1 > MAX_PLAYLIST_BYTES {
            return Err(invalid_playlist("playlist exceeds the size limit"));
        }
        output.push_str(&line);
        output.push('\n');
    }
    Ok(output)
}

fn parse_m3u(input: &str) -> Result<Vec<String>, FmError> {
    let mut entries = Vec::new();
    for line in input.lines() {
        if line.len() > MAX_PLAYLIST_LINE_BYTES {
            return Err(invalid_playlist("playlist line exceeds the size limit"));
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if entries.len() == MAX_QUEUE_ITEMS {
            return Err(invalid_playlist("too many playlist entries"));
        }
        entries.push(line.to_string());
    }
    Ok(entries)
}

fn parse_pls(input: &str) -> Result<Vec<String>, FmError> {
    let mut in_playlist_section = false;
    let mut saw_playlist_section = false;
    let mut declared_count = None;
    let mut files = BTreeMap::new();

    for line in input.lines() {
        if line.len() > MAX_PLAYLIST_LINE_BYTES {
            return Err(invalid_playlist("playlist line exceeds the size limit"));
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_playlist_section = line.eq_ignore_ascii_case("[playlist]");
            saw_playlist_section |= in_playlist_section;
            continue;
        }
        if !in_playlist_section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(invalid_playlist("malformed PLS entry"));
        };
        let key = key.trim();
        let value = value.trim();
        if key.eq_ignore_ascii_case("NumberOfEntries") {
            let count = value
                .parse::<usize>()
                .map_err(|_| invalid_playlist("invalid PLS entry count"))?;
            if count > MAX_QUEUE_ITEMS {
                return Err(invalid_playlist("too many playlist entries"));
            }
            declared_count = Some(count);
        } else if let Some(number) = key
            .get(..4)
            .filter(|prefix| prefix.eq_ignore_ascii_case("File"))
            .and_then(|_| key.get(4..))
        {
            let index = number
                .parse::<usize>()
                .map_err(|_| invalid_playlist("invalid PLS file index"))?;
            if index == 0 || index > MAX_QUEUE_ITEMS || value.is_empty() {
                return Err(invalid_playlist("invalid PLS file entry"));
            }
            if files.insert(index, value.to_string()).is_some() {
                return Err(invalid_playlist("duplicate PLS file index"));
            }
        }
    }
    if !saw_playlist_section {
        return Err(invalid_playlist("missing PLS playlist section"));
    }
    if declared_count.is_some_and(|count| count != files.len()) {
        return Err(invalid_playlist("PLS entry count does not match its files"));
    }
    Ok(files.into_values().collect())
}

fn resolve_entry(entry: &str, base: &VfsUri) -> Result<Option<VfsUri>, FmError> {
    if entry.chars().any(char::is_control) {
        return Err(invalid_playlist(
            "playlist entry contains control characters",
        ));
    }
    if let Some((scheme, rest)) = entry.split_once("://") {
        if !scheme.eq_ignore_ascii_case("smb") && !scheme.eq_ignore_ascii_case("sftp") {
            return Ok(None);
        }
        if rest
            .split('/')
            .next()
            .is_some_and(|authority| authority.contains('@'))
        {
            return Ok(None);
        }
        let uri = VfsUri::parse(entry).map_err(|_| invalid_playlist("invalid playlist URI"))?;
        if !matches!(uri.scheme(), Scheme::Smb | Scheme::Sftp) || uri.authority().user.is_some() {
            return Ok(None);
        }
        return Ok(Some(uri));
    }

    let normalized = entry.replace('\\', "/");
    if normalized.starts_with('/')
        || (normalized.len() >= 3
            && normalized.as_bytes().get(1) == Some(&b':')
            && normalized.as_bytes().get(2) == Some(&b'/'))
    {
        let uri = VfsUri::parse(&normalized)
            .map_err(|_| invalid_playlist("invalid absolute playlist path"))?;
        if uri.scheme() != Scheme::File {
            return Err(invalid_playlist("unsupported playlist path"));
        }
        return Ok(Some(uri));
    }

    let mut uri = base.clone();
    for component in normalized.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                uri = uri
                    .parent()
                    .ok_or_else(|| invalid_playlist("relative path escapes its root"))?;
            }
            name => {
                uri = uri
                    .join(name)
                    .map_err(|_| invalid_playlist("invalid relative playlist path"))?;
            }
        }
    }
    Ok(Some(uri))
}

fn invalid_playlist(message: &str) -> FmError {
    FmError::InvalidParams(message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> VfsUri {
        VfsUri::parse("/music/playlists").expect("base")
    }

    #[test]
    fn m3u_resolves_relative_and_absolute_entries_and_ignores_metadata() {
        let parsed = parse_playlist(
            "#EXTM3U\n#EXTINF:12,Artist - Song\n../song01.mp3\nsub/song02.flac\n/music/song03.ogg\n",
            PlaylistFormat::M3u,
            &base(),
        )
        .expect("playlist");
        let expected = [
            "/music/song01.mp3",
            "/music/playlists/sub/song02.flac",
            "/music/song03.ogg",
        ];
        assert_eq!(
            parsed
                .items
                .iter()
                .map(VfsUri::path_str)
                .collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn pls_entries_are_sorted_by_file_number() {
        let parsed = parse_playlist(
            "[playlist]\nFile2=two.mp3\nTitle2=Two\nFile1=one.mp3\nNumberOfEntries=2\n",
            PlaylistFormat::Pls,
            &base(),
        )
        .expect("playlist");
        assert_eq!(
            parsed
                .items
                .iter()
                .map(VfsUri::path_str)
                .collect::<Vec<_>>(),
            ["/music/playlists/one.mp3", "/music/playlists/two.mp3"]
        );
    }

    #[test]
    fn m3u8_round_trips_local_and_remote_paths() {
        let items = [
            VfsUri::parse("/music/one.mp3").expect("local"),
            VfsUri::parse("smb://nas/share/two.mp3").expect("remote"),
        ];
        let encoded = write_m3u8(&items).expect("write");
        let decoded = parse_playlist(&encoded, PlaylistFormat::M3u8, &base()).expect("read");
        assert_eq!(decoded.items, items);
        assert_eq!(decoded.skipped, 0);
    }

    #[test]
    fn hostile_playlist_schemes_caps_and_path_escape_are_rejected() {
        for uri in [
            "http://example.test/a.mp3",
            "file:///etc/passwd",
            "ftp://host/a",
        ] {
            let parsed = parse_playlist(uri, PlaylistFormat::M3u, &base()).expect("skip URL");
            assert!(parsed.items.is_empty());
            assert_eq!(parsed.skipped, 1);
        }
        assert!(parse_playlist("../..", PlaylistFormat::M3u, &base()).is_ok());
        let root = VfsUri::root_file();
        assert!(parse_playlist("../escape.mp3", PlaylistFormat::M3u, &root).is_err());
        assert!(
            parse_playlist(
                &"a".repeat(MAX_PLAYLIST_LINE_BYTES + 1),
                PlaylistFormat::M3u,
                &base()
            )
            .is_err()
        );
        assert!(
            parse_playlist(
                &"a".repeat(MAX_PLAYLIST_BYTES + 1),
                PlaylistFormat::M3u,
                &base()
            )
            .is_err()
        );
        let too_many = std::iter::repeat_n("song.mp3", MAX_QUEUE_ITEMS + 1)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(parse_playlist(&too_many, PlaylistFormat::M3u, &base()).is_err());
    }

    #[test]
    fn malformed_pls_and_embedded_credentials_are_rejected() {
        assert!(
            parse_playlist(
                "[playlist]\nFile1=one.mp3\nNumberOfEntries=2\n",
                PlaylistFormat::Pls,
                &base()
            )
            .is_err()
        );
        assert_eq!(
            parse_playlist("sftp://alice@host/path.mp3", PlaylistFormat::M3u, &base())
                .expect("skip userinfo")
                .skipped,
            1
        );
        assert!(write_m3u8(&[VfsUri::parse("sftp://alice@host/path.mp3").expect("uri")]).is_err());
    }

    #[test]
    fn skips_urls_without_losing_valid_entries_or_bypassing_caps() {
        let input = "\u{feff}#EXTM3U\none.mp3\nhttps://alice:secret@host/two.mp3\nfile:///three.mp3\nfour.mp3";
        let parsed = parse_playlist(input, PlaylistFormat::M3u8, &base()).expect("parse");
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(parsed.skipped, 2);
        let input = "[playlist]\nFile1=one.mp3\nFile2=https://host/two.mp3\nNumberOfEntries=2\n";
        let parsed = parse_playlist(input, PlaylistFormat::Pls, &base()).expect("parse");
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.skipped, 1);
        let too_many = "https://host/song.mp3\n".repeat(MAX_QUEUE_ITEMS + 1);
        assert!(parse_playlist(&too_many, PlaylistFormat::M3u8, &base()).is_err());
    }
}
