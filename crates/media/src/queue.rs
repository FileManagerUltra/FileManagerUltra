use fm_vfs::{FmError, VfsUri};

/// Maximum queue size, including items loaded from a playlist.
pub const MAX_QUEUE_ITEMS: usize = 10_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

/// Ordered media items with a separate playback order and navigation history.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Queue {
    items: Vec<VfsUri>,
    order: Vec<usize>,
    cursor: Option<usize>,
    current: Option<usize>,
    history: Vec<usize>,
    history_cursor: Option<usize>,
    next_up: Vec<usize>,
    shuffle: bool,
    shuffle_seed: u64,
    repeat: RepeatMode,
}

impl Queue {
    pub fn items(&self) -> &[VfsUri] {
        &self.items
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    pub fn current(&self) -> Option<&VfsUri> {
        self.current.and_then(|index| self.items.get(index))
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    pub fn set_repeat(&mut self, repeat: RepeatMode) {
        self.repeat = repeat;
    }

    pub fn enqueue(&mut self, uri: VfsUri) -> Result<usize, FmError> {
        if self.items.len() >= MAX_QUEUE_ITEMS {
            return Err(FmError::InvalidParams("media queue is full".to_string()));
        }
        let index = self.items.len();
        self.items.push(uri);
        self.order.push(index);
        Ok(index)
    }

    /// Insert a requested batch after the current item, before ordinary playback resumes.
    pub fn insert_next(&mut self, items: Vec<VfsUri>) -> Result<usize, FmError> {
        if items.is_empty() || self.items.len().saturating_add(items.len()) > MAX_QUEUE_ITEMS {
            return Err(FmError::InvalidParams("invalid or full media queue".into()));
        }
        let first = self.current.map_or(0, |index| index + 1);
        let count = items.len();
        let remap = |index: usize| if index >= first { index + count } else { index };
        for index in &mut self.order {
            *index = remap(*index);
        }
        for index in &mut self.history {
            *index = remap(*index);
        }
        for index in &mut self.next_up {
            *index = remap(*index);
        }
        self.current = self.current.map(remap);
        self.items.splice(first..first, items);
        let position = self.cursor.map_or(0, |cursor| cursor + 1);
        self.order.splice(position..position, first..first + count);
        self.next_up.splice(0..0, first..first + count);
        if let Some(cursor) = self.history_cursor {
            self.history.truncate(cursor + 1);
        }
        Ok(first)
    }

    /// Set the current item without changing the user-visible queue order.
    pub fn select(&mut self, index: usize) -> Result<&VfsUri, FmError> {
        if self.items.get(index).is_none() {
            return Err(FmError::InvalidParams(format!(
                "media queue index out of range: {index}"
            )));
        }
        if let Some(position) = self.order.iter().position(|item| *item == index) {
            self.cursor = Some(position);
        }
        self.record_play(index);
        Ok(&self.items[index])
    }

    pub fn remove(&mut self, index: usize) -> Result<VfsUri, FmError> {
        if self.items.get(index).is_none() {
            return Err(FmError::InvalidParams(format!(
                "media queue index out of range: {index}"
            )));
        }
        let removed = self.items.remove(index);
        let remap =
            |item: usize| (item != index).then_some(if item > index { item - 1 } else { item });
        let old_play_cursor = self.cursor;
        self.order = self.order.iter().filter_map(|item| remap(*item)).collect();
        self.next_up = self
            .next_up
            .iter()
            .filter_map(|item| remap(*item))
            .collect();

        let old_history_cursor = self.history_cursor;
        let mut removed_before_cursor = 0;
        self.history = self
            .history
            .iter()
            .enumerate()
            .filter_map(|(position, item)| {
                if *item == index {
                    if old_history_cursor.is_some_and(|cursor| position <= cursor) {
                        removed_before_cursor += 1;
                    }
                    None
                } else {
                    remap(*item)
                }
            })
            .collect();
        self.history_cursor = old_history_cursor
            .map(|cursor| cursor.saturating_sub(removed_before_cursor))
            .filter(|cursor| *cursor < self.history.len());

        if self.current == Some(index) {
            self.current = None;
            self.cursor = None;
            if !self.order.is_empty() {
                let next_position = old_play_cursor
                    .unwrap_or(0)
                    .min(self.order.len().saturating_sub(1));
                let next = self.order[next_position];
                self.record_play(next);
                self.cursor = Some(next_position);
            } else {
                self.history.clear();
                self.history_cursor = None;
            }
        } else {
            self.current = self.current.and_then(remap);
            self.cursor = self
                .current
                .and_then(|current| self.order.iter().position(|item| *item == current));
        }
        Ok(removed)
    }

    /// Move an item in the visible queue while preserving the identities in playback history.
    pub fn move_item(&mut self, from: usize, to: usize) -> Result<(), FmError> {
        let len = self.items.len();
        if from >= len || to >= len {
            return Err(FmError::InvalidParams(
                "media queue index out of range".to_string(),
            ));
        }
        if from == to {
            return Ok(());
        }
        let item = self.items.remove(from);
        self.items.insert(to, item);
        let remap = |index: usize| {
            if index == from {
                to
            } else if from < to && (from < index && index <= to) {
                index - 1
            } else if to < from && (to <= index && index < from) {
                index + 1
            } else {
                index
            }
        };
        for index in &mut self.order {
            *index = remap(*index);
        }
        for index in &mut self.history {
            *index = remap(*index);
        }
        for index in &mut self.next_up {
            *index = remap(*index);
        }
        self.current = self.current.map(remap);
        if !self.shuffle {
            self.order = (0..len).collect();
        }
        self.cursor = self
            .current
            .and_then(|current| self.order.iter().position(|item| *item == current));
        Ok(())
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.order.clear();
        self.cursor = None;
        self.current = None;
        self.history.clear();
        self.history_cursor = None;
        self.next_up.clear();
    }

    /// Enable or disable shuffle. The supplied seed makes ordering reproducible in tests.
    pub fn set_shuffle(&mut self, enabled: bool, seed: u64) {
        if self.shuffle == enabled {
            return;
        }
        self.shuffle = enabled;
        self.order = (0..self.items.len()).collect();
        if enabled {
            self.shuffle_seed = seed;
            let current = self.current;
            let start = usize::from(current.is_some());
            if let Some(current) = current
                && let Some(position) = self.order.iter().position(|item| *item == current)
            {
                self.order.swap(0, position);
            }
            shuffle_tail(&mut self.order, start, seed);
        }
        self.cursor = self
            .current
            .and_then(|current| self.order.iter().position(|item| *item == current));
    }

    /// The item `next_item` would return, without moving. `None` at the end of the queue and
    /// when repeat-all would reshuffle, because that order is not known yet.
    pub fn peek_next(&self) -> Option<&VfsUri> {
        if let Some(next) = self.next_up.first() {
            return self.items.get(*next);
        }
        if self.repeat == RepeatMode::One
            && let Some(current) = self.current
        {
            return self.items.get(current);
        }
        if let Some(history_cursor) = self.history_cursor
            && let Some(next) = self.history.get(history_cursor + 1)
        {
            return self.items.get(*next);
        }
        let position = match self.cursor {
            Some(cursor) if cursor + 1 < self.order.len() => cursor + 1,
            Some(_) if self.repeat == RepeatMode::All && !self.shuffle => 0,
            None => 0,
            _ => return None,
        };
        self.order.get(position).and_then(|i| self.items.get(*i))
    }

    pub fn next_item(&mut self) -> Option<&VfsUri> {
        if self.items.is_empty() {
            return None;
        }
        if let Some(next) = self.next_up.first().copied() {
            self.next_up.remove(0);
            let old = self.order.iter().position(|item| *item == next)?;
            self.order.remove(old);
            let position = self
                .current
                .and_then(|current| self.order.iter().position(|item| *item == current))
                .map_or(0, |cursor| cursor + 1);
            self.order.insert(position, next);
            self.cursor = Some(position);
            self.record_play(next);
            return self.items.get(next);
        }
        if self.repeat == RepeatMode::One
            && let Some(current) = self.current
        {
            return self.items.get(current);
        }
        if let Some(history_cursor) = self.history_cursor
            && let Some(next) = self.history.get(history_cursor + 1).copied()
        {
            self.history_cursor = Some(history_cursor + 1);
            self.current = Some(next);
            self.cursor = self.order.iter().position(|item| *item == next);
            return self.items.get(next);
        }

        let next_position = match self.cursor {
            Some(cursor) if cursor + 1 < self.order.len() => cursor + 1,
            Some(_) if self.repeat == RepeatMode::All => {
                if self.shuffle {
                    let current = self.current;
                    let mut order: Vec<usize> = (0..self.items.len()).collect();
                    self.shuffle_seed = self.shuffle_seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
                    shuffle_tail(&mut order, 0, self.shuffle_seed);
                    if order.len() > 1 && order.first().copied() == current {
                        order.swap(0, 1);
                    }
                    self.order = order;
                }
                0
            }
            None => 0,
            _ => return None,
        };
        let next = *self.order.get(next_position)?;
        self.cursor = Some(next_position);
        self.record_play(next);
        self.items.get(next)
    }

    pub fn previous_item(&mut self) -> Option<&VfsUri> {
        let cursor = self.history_cursor?;
        if cursor == 0 {
            return None;
        }
        let previous = *self.history.get(cursor - 1)?;
        self.history_cursor = Some(cursor - 1);
        self.current = Some(previous);
        self.cursor = self.order.iter().position(|item| *item == previous);
        self.items.get(previous)
    }

    fn record_play(&mut self, index: usize) {
        self.next_up.retain(|item| *item != index);
        if let Some(cursor) = self.history_cursor {
            self.history.truncate(cursor + 1);
        }
        if self.history.last() != Some(&index) {
            self.history.push(index);
            if self.history.len() > MAX_QUEUE_ITEMS {
                self.history.remove(0);
            }
        }
        self.history_cursor = self.history.len().checked_sub(1);
        self.current = Some(index);
    }
}

fn shuffle_tail(items: &mut [usize], start: usize, mut state: u64) {
    if items.len().saturating_sub(start) < 2 {
        return;
    }
    if state == 0 {
        state = 0x9e37_79b9_7f4a_7c15;
    }
    for index in (start + 1..items.len()).rev() {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let random = state.wrapping_mul(0x2545_f491_4f6c_dd1d);
        let target = start + (random as usize % (index - start + 1));
        items.swap(index, target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(count: usize) -> Vec<VfsUri> {
        (0..count)
            .map(|index| VfsUri::parse(&format!("/{index}.mp3")).expect("uri"))
            .collect()
    }

    fn queue(count: usize) -> Queue {
        let mut queue = Queue::default();
        for item in items(count) {
            queue.enqueue(item).expect("enqueue");
        }
        queue
    }

    #[test]
    fn peek_next_matches_next_item_without_moving() {
        for repeat in [RepeatMode::Off, RepeatMode::All, RepeatMode::One] {
            for shuffle in [false, true] {
                let mut q = queue(5);
                q.set_repeat(repeat);
                q.set_shuffle(shuffle, 7);
                q.select(2).expect("select");
                q.insert_next(vec![VfsUri::parse("/up.mp3").expect("uri")])
                    .expect("up next");
                for step in 0..9 {
                    let peeked = q.peek_next().cloned();
                    let before = q.current_index();
                    assert_eq!(q.current_index(), before);
                    let advanced = q.next_item().cloned();
                    let wrap_reshuffle = repeat == RepeatMode::All && shuffle;
                    if peeked.is_some() || !wrap_reshuffle {
                        assert_eq!(peeked, advanced, "{repeat:?} shuffle={shuffle} step {step}");
                    }
                    if advanced.is_none() {
                        break;
                    }
                }
            }
        }
    }

    fn assert_invariants(queue: &Queue) {
        assert_eq!(queue.items.len(), queue.order.len());
        let mut order = queue.order.clone();
        order.sort_unstable();
        assert_eq!(order, (0..queue.items.len()).collect::<Vec<_>>());
        assert!(queue.history.iter().all(|index| *index < queue.items.len()));
        assert!(queue.next_up.iter().all(|index| *index < queue.items.len()));
        assert_eq!(
            queue.current,
            queue
                .history_cursor
                .and_then(|cursor| queue.history.get(cursor).copied())
        );
        assert_eq!(
            queue
                .cursor
                .and_then(|cursor| queue.order.get(cursor).copied()),
            queue.current
        );
    }

    #[test]
    fn queue_edits_keep_current_and_play_order_consistent() {
        let mut queue = queue(4);
        assert_invariants(&queue);
        queue.select(1).expect("select");
        assert_invariants(&queue);
        queue.move_item(2, 3).expect("move");
        assert_invariants(&queue);
        assert_eq!(
            queue.current(),
            Some(&VfsUri::parse("/1.mp3").expect("uri"))
        );
        assert_eq!(queue.current_index(), Some(1));
        assert_eq!(
            queue.next_item(),
            Some(&VfsUri::parse("/3.mp3").expect("uri"))
        );
        assert_eq!(
            queue.remove(1).expect("remove"),
            VfsUri::parse("/1.mp3").expect("uri")
        );
        assert_invariants(&queue);
        assert_eq!(queue.items.len(), queue.order.len());
        assert!(queue.order.iter().all(|index| *index < queue.items.len()));
        queue.clear();
        assert_invariants(&queue);
        assert!(queue.current().is_none());
        assert!(queue.next_item().is_none());
    }

    #[test]
    fn repeat_one_all_and_off_have_distinct_end_behavior() {
        let mut queue = queue(2);
        assert_eq!(
            queue.next_item(),
            Some(&VfsUri::parse("/0.mp3").expect("uri"))
        );
        queue.set_repeat(RepeatMode::One);
        assert_eq!(
            queue.next_item(),
            Some(&VfsUri::parse("/0.mp3").expect("uri"))
        );
        queue.set_repeat(RepeatMode::Off);
        assert_eq!(
            queue.next_item(),
            Some(&VfsUri::parse("/1.mp3").expect("uri"))
        );
        assert_eq!(queue.next_item(), None);
        queue.set_repeat(RepeatMode::All);
        assert_eq!(
            queue.next_item(),
            Some(&VfsUri::parse("/0.mp3").expect("uri"))
        );
    }

    #[test]
    fn moving_and_removing_current_preserves_item_identity_and_history() {
        let mut queue = queue(4);
        queue.select(1).expect("select");
        queue.move_item(1, 3).expect("move current");
        assert_eq!(
            queue.current(),
            Some(&VfsUri::parse("/1.mp3").expect("uri"))
        );
        assert_eq!(queue.current_index(), Some(3));
        assert_invariants(&queue);

        queue.select(0).expect("select next");
        queue.remove(0).expect("remove current");
        assert_eq!(
            queue.current(),
            Some(&VfsUri::parse("/2.mp3").expect("uri"))
        );
        assert_invariants(&queue);
        assert_eq!(
            queue.previous_item(),
            Some(&VfsUri::parse("/1.mp3").expect("uri"))
        );
        assert_invariants(&queue);
    }

    #[test]
    fn shuffle_visits_each_item_once_and_previous_follows_play_history() {
        let mut first_queue = queue(8);
        first_queue.set_shuffle(true, 42);
        let mut played = Vec::new();
        for _ in 0..8 {
            played.push(first_queue.next_item().expect("next").clone());
            assert_invariants(&first_queue);
        }
        let unique: std::collections::BTreeSet<_> = played.iter().cloned().collect();
        assert_eq!(unique.len(), 8);
        assert_eq!(first_queue.previous_item(), Some(&played[6]));
        assert_invariants(&first_queue);
        assert_eq!(first_queue.next_item(), Some(&played[7]));
        assert_invariants(&first_queue);
        let mut same_seed = queue(8);
        same_seed.set_shuffle(true, 42);
        let mut second_order = Vec::new();
        for _ in 0..8 {
            second_order.push(same_seed.next_item().expect("next").clone());
        }
        assert_eq!(played, second_order);
    }

    #[test]
    fn shuffle_cycles_are_bounded_and_toggling_keeps_navigation_history() {
        let mut repeated = queue(8);
        repeated.set_repeat(RepeatMode::All);
        repeated.set_shuffle(true, 17);
        let mut first_cycle = Vec::new();
        let mut second_cycle = Vec::new();
        for _ in 0..16 {
            let item = repeated.next_item().expect("next").clone();
            if first_cycle.len() < 8 {
                first_cycle.push(item);
            } else {
                second_cycle.push(item);
            }
        }
        assert_eq!(
            first_cycle
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            8
        );
        assert_eq!(
            second_cycle
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            8
        );
        assert_ne!(first_cycle[7], second_cycle[0]);
        assert_invariants(&repeated);

        let mut toggled = queue(3);
        toggled.select(0).expect("select");
        toggled.select(1).expect("select");
        toggled.set_shuffle(true, 99);
        assert_eq!(
            toggled.previous_item(),
            Some(&VfsUri::parse("/0.mp3").expect("uri"))
        );
        assert_eq!(
            toggled.next_item(),
            Some(&VfsUri::parse("/1.mp3").expect("uri"))
        );
        assert_invariants(&toggled);
    }

    #[test]
    fn inserting_next_keeps_the_playlist_and_prioritizes_the_batch() {
        for shuffle in [false, true] {
            let mut queue = queue(3);
            queue.select(1).expect("current");
            queue.set_shuffle(shuffle, 42);
            queue.set_repeat(RepeatMode::One);
            let batch = vec![
                VfsUri::parse("/new1.mp3").expect("uri"),
                VfsUri::parse("/new2.mp3").expect("uri"),
            ];
            assert_eq!(queue.insert_next(batch.clone()).expect("insert"), 2);
            assert_eq!(queue.current_index(), Some(1));
            assert_eq!(
                queue
                    .items
                    .iter()
                    .filter_map(VfsUri::file_name)
                    .collect::<Vec<_>>(),
                ["0.mp3", "1.mp3", "new1.mp3", "new2.mp3", "2.mp3"]
            );
            for item in &batch {
                assert_eq!(queue.next_item(), Some(item));
                assert_invariants(&queue);
            }
            assert_eq!(
                queue.next_item(),
                batch.last(),
                "repeat-one resumes after the requested batch"
            );
            assert_eq!(queue.items.len(), 5, "playback never removes entries");
            assert_eq!(
                queue.previous_item().and_then(VfsUri::file_name),
                Some("new1.mp3")
            );
            assert_invariants(&queue);
        }
    }

    #[test]
    fn inserted_requests_survive_queue_edits_and_override_forward_history() {
        let mut queue = queue(3);
        queue.select(0).expect("first");
        queue.select(1).expect("second");
        queue.previous_item();
        let batch = vec![
            VfsUri::parse("/new1.mp3").expect("uri"),
            VfsUri::parse("/new2.mp3").expect("uri"),
        ];
        queue.insert_next(batch.clone()).expect("insert");
        queue.move_item(1, 4).expect("move first scheduled item");
        queue.set_shuffle(true, 9);
        assert_invariants(&queue);
        assert_eq!(
            queue.next_item(),
            batch.first(),
            "explicit request beats forward history and shuffle"
        );
        assert_invariants(&queue);
        let second = queue
            .items
            .iter()
            .position(|item| item == &batch[1])
            .expect("second");
        queue.remove(second).expect("remove pending");
        assert_invariants(&queue);
        assert!(queue.next_up.is_empty());
        assert_eq!(queue.items.len(), 4);
        queue.clear();
        assert!(queue.next_up.is_empty());
        assert_invariants(&queue);

        let mut empty = Queue::default();
        empty.insert_next(batch.clone()).expect("insert into empty");
        assert!(empty.current().is_none());
        assert_eq!(empty.next_item(), batch.first());
        assert_invariants(&empty);
    }

    #[test]
    fn invalid_edits_and_queue_overflow_are_errors() {
        let mut queue = queue(1);
        assert!(queue.select(1).is_err());
        assert!(queue.remove(1).is_err());
        assert!(queue.move_item(0, 1).is_err());
        for index in 1..MAX_QUEUE_ITEMS {
            queue
                .enqueue(VfsUri::parse(&format!("/{index}.mp3")).expect("uri"))
                .expect("fill queue");
        }
        assert!(
            queue
                .enqueue(VfsUri::parse("/overflow.mp3").expect("uri"))
                .is_err()
        );
        let before = queue.clone();
        assert!(
            queue
                .insert_next(vec![VfsUri::parse("/overflow.mp3").expect("uri")])
                .is_err()
        );
        assert!(queue.insert_next(Vec::new()).is_err());
        assert_eq!(queue, before, "invalid batches are atomic");
    }
}
