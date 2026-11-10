//! Remote media playback: the file is first copied to a private (0700) temp
//! folder on the backend's own job lane, then played like a local file.
//! Playlists, folder play and the persisted queue stay local-only.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};

use fm_media::{PlaybackState, PlaybackStatus};
use fm_vfs::{FmError, VfsUri};

use crate::Session;
use crate::backends::lane_for;

/// Spooled files kept for quick replays; the oldest is deleted beyond this.
pub const MAX_SPOOLED: usize = 3;

#[derive(Debug)]
struct Pending {
    uri: VfsUri,
    receiver: Receiver<Result<PathBuf, String>>,
    cancelled: Arc<AtomicBool>,
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

#[derive(Debug, Default)]
pub struct Spool {
    pending: Option<Pending>,
    /// Oldest first.
    ready: Vec<(VfsUri, PathBuf)>,
}

impl Drop for Spool {
    fn drop(&mut self) {
        for (_, path) in self.ready.drain(..) {
            remove(&path);
        }
    }
}

fn remove(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

impl Spool {
    /// The local copy of `uri`, if it has been downloaded.
    pub fn path(&self, uri: &VfsUri) -> Option<&Path> {
        self.ready
            .iter()
            .find(|(ready, _)| ready == uri)
            .map(|(_, path)| path.as_path())
    }

    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn is_pending_for(&self, uri: &VfsUri) -> bool {
        self.pending.as_ref().is_some_and(|p| &p.uri == uri)
    }

    /// `(bytes done, bytes total)`; the total is 0 while unknown.
    pub fn progress(&self) -> Option<(u64, u64)> {
        self.pending.as_ref().map(|p| {
            (
                p.done.load(Ordering::Relaxed),
                p.total.load(Ordering::Relaxed),
            )
        })
    }

    /// Abandon a download unless it is for `keep`.
    pub fn cancel_except(&mut self, keep: Option<&VfsUri>) {
        if self.pending.as_ref().is_some_and(|p| Some(&p.uri) != keep) {
            self.pending = None;
        }
    }

    fn insert(&mut self, uri: VfsUri, path: PathBuf) {
        if let Some(index) = self.ready.iter().position(|(ready, _)| *ready == uri) {
            let (_, old) = self.ready.remove(index);
            if old != path {
                remove(&old);
            }
        }
        self.ready.push((uri, path));
        while self.ready.len() > MAX_SPOOLED {
            let (_, oldest) = self.ready.remove(0);
            remove(&oldest);
        }
    }
}

impl Session {
    /// Start copying `uri` locally; playback resumes from [`Session::media_tick`].
    pub(crate) fn media_spool_begin(&mut self, uri: VfsUri) -> Result<(), FmError> {
        let vfs = self.backends.require(&uri)?;
        let (tx, rx) = sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicU64::new(0));
        let total = Arc::new(AtomicU64::new(0));
        let job = (cancelled.clone(), done.clone(), total.clone());
        let source = uri.clone();
        self.jobs.submit(lane_for(uri.scheme()), move |ctx| {
            let (cancelled, done, total) = job;
            let stopped = || ctx.is_cancelled() || cancelled.load(Ordering::Acquire);
            if stopped() {
                return Ok(());
            }
            let result = (|| {
                let size = vfs.stat(&source)?.size;
                let name = crate::open_cmds::safe_file_name(source.file_name().unwrap_or("media"));
                crate::open_cmds::download_to_temp(
                    vfs.as_ref(),
                    &source,
                    &name,
                    size,
                    &stopped,
                    &mut |d, t| {
                        done.store(d, Ordering::Relaxed);
                        total.store(t, Ordering::Relaxed);
                        ctx.report(d, t);
                    },
                )
            })()
            .map_err(|error| fm_vfs::redact::redact(&error.to_string()));
            let _ = tx.send(result);
            Ok(())
        });
        self.media.spool.pending = Some(Pending {
            uri,
            receiver: rx,
            cancelled,
            done,
            total,
        });
        Ok(())
    }

    /// Collect a finished download. Returns whether the visible state changed.
    pub(crate) fn media_spool_poll(&mut self) -> bool {
        let Some(pending) = &self.media.spool.pending else {
            return false;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Disconnected) => {
                Err("Downloading the media file did not complete.".into())
            }
            Err(TryRecvError::Empty) => return false,
        };
        let Some(pending) = self.media.spool.pending.take() else {
            return false;
        };
        let wanted = self.media.queue.current() == Some(&pending.uri);
        match result {
            Ok(path) => {
                self.media.spool.insert(pending.uri.clone(), path);
                if wanted
                    && self.media.playing
                    && let Err(error) = self.media_resume()
                {
                    let message = fm_vfs::redact::redact(&error.to_string());
                    self.media_apply_failure(message);
                }
            }
            Err(error) if wanted && self.media.playing => self.media_apply_failure(error),
            Err(error) => self.messages.push(&error),
        }
        true
    }

    fn media_apply_failure(&mut self, error: String) {
        let position = self.media.position;
        self.media_apply_status(PlaybackStatus {
            token: 0,
            state: PlaybackState::Failed,
            info: None,
            position,
            error: Some(error),
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;
    use fm_testkit::MemFs;
    use fm_vfs::Authority;

    const HOST: &str = "sftp://nas/music";

    fn session_with(files: &[(&str, &[u8])]) -> Session {
        let mut s = Session::new().expect("session");
        let fs = MemFs::new().with_scheme(fm_vfs::Scheme::Sftp);
        for (name, bytes) in files {
            let uri = VfsUri::parse(&format!("{HOST}/{name}")).expect("uri");
            let parent = uri.parent().expect("parent");
            fs.mkdirs(&parent).expect("dir");
            fs.put_file(&uri, bytes).expect("file");
        }
        let probe = VfsUri::parse(&format!("{HOST}/x.mp3")).expect("uri");
        let authority: Authority = probe.authority().clone();
        s.backends.register_at(&authority, Arc::new(fs));
        s
    }

    fn wait(s: &mut Session, mut done: impl FnMut(&Session) -> bool) {
        for _ in 0..400 {
            s.poll();
            if done(s) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("timed out");
    }

    fn remote(name: &str) -> VfsUri {
        VfsUri::parse(&format!("{HOST}/{name}")).expect("uri")
    }

    #[test]
    fn remote_files_download_privately_then_play_and_are_cleaned_up() {
        let mut s = session_with(&[("a.mp3", &[7; 4096])]);
        let uri = remote("a.mp3");
        s.media.queue.enqueue(uri.clone()).expect("enqueue");
        s.media.queue.select(0).expect("select");
        s.media.playing = true;
        s.media_spool_begin(uri.clone()).expect("begin");
        assert!(s.media.spool.is_pending_for(&uri));
        wait(&mut s, |s| s.media.spool.path(&uri).is_some());
        let path = s.media.spool.path(&uri).expect("spooled").to_path_buf();
        assert_eq!(std::fs::read(&path).expect("read"), vec![7; 4096]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path.parent().expect("dir"))
                .expect("meta")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        drop(s);
        assert!(!path.exists(), "spool dir survives the session");
    }

    #[test]
    fn only_the_newest_spooled_files_are_kept() {
        let mut spool = Spool::default();
        let mut paths = Vec::new();
        for n in 0..=MAX_SPOOLED {
            let dir =
                std::env::temp_dir().join(format!("fm-open-test-spool-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("dir");
            let path = dir.join("f.mp3");
            std::fs::write(&path, b"x").expect("file");
            spool.insert(remote(&format!("{n}.mp3")), path.clone());
            paths.push(path);
        }
        assert!(!paths[0].exists(), "oldest evicted");
        assert!(paths[1..].iter().all(|p| p.exists()));
        assert!(spool.path(&remote("0.mp3")).is_none());
        drop(spool);
        assert!(paths.iter().all(|p| !p.exists()));
    }

    #[test]
    fn a_failed_download_skips_the_item_with_a_notice_and_leaves_no_files() {
        let mut s = session_with(&[("ok.mp3", &[1; 16])]);
        let missing = remote("missing.mp3");
        s.media.queue.enqueue(missing.clone()).expect("enqueue");
        s.media.queue.select(0).expect("select");
        s.media.playing = true;
        s.media_spool_begin(missing.clone()).expect("begin");
        wait(&mut s, |s| !s.media.spool.is_pending());
        assert!(s.media.spool.path(&missing).is_none());
        assert!(!s.media.playing, "single failed item stops the queue");
        assert!(!s.messages.is_empty() || s.media.error.is_some());
    }

    #[test]
    fn progress_is_reported_while_pending_and_abandoned_downloads_are_cancelled() {
        let mut s = session_with(&[("a.mp3", &[0; 1024])]);
        let uri = remote("a.mp3");
        s.media_spool_begin(uri.clone()).expect("begin");
        assert!(s.media.spool.progress().is_some());
        s.media.spool.cancel_except(Some(&remote("other.mp3")));
        assert!(s.media.spool.progress().is_none());
        assert!(!s.media.spool.is_pending());
    }
}
