//! Recent failures, warnings and one-time hints, shown in the Activity panel instead of the
//! status bar. A message may carry a few action buttons that dispatch commands by id.

use std::collections::VecDeque;
use std::time::Instant;

use fm_vfs::FmError;

use crate::registry::CommandSpec;

/// Oldest messages are dropped beyond this many.
pub const MAX_MESSAGES: usize = 50;
/// Longer messages are cut (they may carry a whole remote error body).
const MAX_CHARS: usize = 1000;
/// Most buttons on one message, and the longest button label.
pub const MAX_ACTIONS: usize = 3;
const MAX_LABEL_CHARS: usize = 40;

/// A button on a message: clicking it dispatches `command` with `params`, like a menu item.
/// The command validates the params; they are never trusted for coming from a message.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageAction {
    pub label: String,
    /// Commands run in order (e.g. change a preference, then play).
    pub commands: Vec<(String, serde_json::Value)>,
}

#[derive(Debug, Clone)]
pub struct Message {
    /// Unique per log, to act on one message from the UI.
    pub id: u64,
    pub text: String,
    pub at: Instant,
    /// A one-time hint's id (see `Prefs::hints_seen`); hints stay on screen longer.
    pub hint: Option<String>,
    pub actions: Vec<MessageAction>,
    /// An action was used: the buttons no longer act.
    pub spent: bool,
}

#[derive(Debug, Default)]
pub struct MessageLog {
    items: VecDeque<Message>,
    /// Bumped on every push, so the UI can tell that something new arrived.
    pushes: u64,
}

impl MessageLog {
    pub fn push(&mut self, text: &str) {
        self.push_with(text, None, Vec::new());
    }

    /// A one-time hint with buttons (capped at [`MAX_ACTIONS`], labels shortened).
    pub fn push_hint(&mut self, text: &str, hint: &str, actions: Vec<MessageAction>) {
        self.push_with(text, Some(hint.to_string()), actions);
    }

    fn push_with(&mut self, text: &str, hint: Option<String>, mut actions: Vec<MessageAction>) {
        let text: String = text.chars().take(MAX_CHARS).collect();
        if text.trim().is_empty() {
            return;
        }
        if self.items.len() >= MAX_MESSAGES {
            self.items.pop_front();
        }
        actions.truncate(MAX_ACTIONS);
        for action in &mut actions {
            action.label = action.label.chars().take(MAX_LABEL_CHARS).collect();
        }
        self.pushes = self.pushes.wrapping_add(1);
        self.items.push_back(Message {
            id: self.pushes,
            text,
            at: Instant::now(),
            hint,
            actions,
            spent: false,
        });
    }

    /// The newest message, when it is a hint (the panel then stays open longer).
    pub fn newest_is_hint(&self) -> bool {
        self.items.back().is_some_and(|m| m.hint.is_some())
    }

    /// Take the commands of action `index` on message `id`, once; later clicks get nothing.
    pub fn use_action(
        &mut self,
        id: u64,
        index: usize,
    ) -> Option<Vec<(String, serde_json::Value)>> {
        let message = self.items.iter_mut().find(|m| m.id == id && !m.spent)?;
        let commands = message.actions.get(index)?.commands.clone();
        message.spent = true;
        Some(commands)
    }

    /// Remove the hint `hint`; `false` when no such hint is in the log.
    pub fn dismiss_hint(&mut self, hint: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|m| m.hint.as_deref() != Some(hint));
        self.items.len() != before
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
        menu: &["Activity", "Clear Messages"],
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
    fn hint_actions_are_bounded_and_act_once() {
        let mut log = MessageLog::default();
        let action = |label: &str| MessageAction {
            label: label.to_string(),
            commands: vec![("media.play".into(), serde_json::json!({"uris": ["/a.mp3"]}))],
        };
        let long = "y".repeat(100);
        log.push_hint(
            "You can also play audio and video here.",
            "media-open",
            vec![action(&long), action("b"), action("c"), action("d")],
        );
        assert!(log.newest_is_hint());
        let message = log.recent().next().expect("hint").clone();
        assert_eq!(message.actions.len(), MAX_ACTIONS);
        assert_eq!(message.actions[0].label.chars().count(), MAX_LABEL_CHARS);
        assert!(log.use_action(message.id, 9).is_none(), "no such button");
        assert_eq!(log.use_action(message.id, 1).map(|c| c.len()), Some(1));
        assert!(
            log.use_action(message.id, 0).is_none(),
            "spent after one click"
        );
        log.push("plain");
        assert!(!log.newest_is_hint());
        assert!(log.dismiss_hint("media-open"));
        assert!(!log.dismiss_hint("media-open"));
        assert_eq!(log.len(), 1);
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
