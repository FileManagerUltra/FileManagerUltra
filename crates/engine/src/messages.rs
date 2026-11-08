//! Recent failures and warnings, shown in the jobs panel instead of the status bar.

use std::collections::VecDeque;
use std::time::Instant;

use fm_vfs::FmError;

use crate::registry::CommandSpec;

/// Oldest messages are dropped beyond this many.
pub const MAX_MESSAGES: usize = 50;
/// Longer messages are cut (they may carry a whole remote error body).
const MAX_CHARS: usize = 1000;

#[derive(Debug, Clone)]
pub struct Message {
    pub text: String,
    pub at: Instant,
}

#[derive(Debug, Default)]
pub struct MessageLog {
    items: VecDeque<Message>,
    /// Bumped on every push, so the UI can tell that something new arrived.
    pushes: u64,
}

impl MessageLog {
    pub fn push(&mut self, text: &str) {
        let text: String = text.chars().take(MAX_CHARS).collect();
        if text.trim().is_empty() {
            return;
        }
        if self.items.len() >= MAX_MESSAGES {
            self.items.pop_front();
        }
        self.items.push_back(Message {
            text,
            at: Instant::now(),
        });
        self.pushes = self.pushes.wrapping_add(1);
    }

    /// Newest first.
    pub fn recent(&self) -> impl Iterator<Item = &Message> {
        self.items.iter().rev()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn pushes(&self) -> u64 {
        self.pushes
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "messages.clear",
        label: "Clear Messages",
        menu: &["Jobs", "Clear Messages"],
        shortcut: None,
        params: "none",
        enabled: |s| !s.messages.is_empty(),
        run: |s, params| {
            if !params.is_null() {
                return Err(FmError::InvalidParams(
                    "This command takes no parameters".into(),
                ));
            }
            s.messages.clear();
            Ok(())
        },
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_is_capped_newest_first_and_ignores_blank() {
        let mut log = MessageLog::default();
        log.push("   ");
        assert!(log.is_empty());
        for i in 0..MAX_MESSAGES + 5 {
            log.push(&format!("m{i}"));
        }
        assert_eq!(log.len(), MAX_MESSAGES);
        assert_eq!(log.recent().next().map(|m| m.text.as_str()), Some("m54"));
        log.push(&"x".repeat(MAX_CHARS * 2));
        assert_eq!(log.recent().next().map(|m| m.text.len()), Some(MAX_CHARS));
    }

    #[test]
    fn clear_command_rejects_params_and_empties_the_log() {
        let mut s = crate::Session::new().expect("session");
        s.messages.push("boom");
        assert!(
            s.dispatch("messages.clear", &serde_json::json!({"x": 1}))
                .is_err()
        );
        assert_eq!(s.messages.len(), 1);
        s.dispatch("messages.clear", &serde_json::Value::Null)
            .expect("clear");
        assert!(s.messages.is_empty());
    }

    #[test]
    fn session_moves_notices_into_the_log() {
        let mut s = crate::Session::default();
        assert!(!s.drain_notices());
        s.notice = Some("Cannot open it".into());
        assert!(s.drain_notices());
        assert!(s.notice.is_none());
        assert_eq!(s.messages.len(), 1);
    }
}
