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
    /// Group of each item, parallel to `items`; a group is always one contiguous run.
    item_group: Vec<Option<u32>>,
    group_meta: Vec<GroupMeta>,
    next_group: u32,
}

/// Folders per queue.
pub const MAX_GROUPS: usize = 1_000;
/// Characters in a folder name.
pub const MAX_GROUP_NAME: usize = 120;

#[derive(Clone, Debug, PartialEq, Eq)]
struct GroupMeta {
    id: u32,
    name: String,
    /// Source folder of an automatic group, the key for resume points.
    dir: Option<String>,
    /// A locked folder keeps its items: nothing joins, leaves or is reordered inside it.
    locked: bool,
}

/// A named run of queue items. Empty groups sit after the last item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub id: u32,
    pub start: usize,
    pub len: usize,
    pub name: String,
    pub dir: Option<String>,
    pub locked: bool,
}

/// The group an item belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupRef<'a> {
    pub id: u32,
    pub name: &'a str,
    pub dir: Option<&'a str>,
    /// The item is the last of its group.
    pub last: bool,
    pub locked: bool,
}

fn locked_error() -> FmError {
    FmError::InvalidParams("the folder is locked: unlock it to change its items".to_string())
}

fn valid_group_name(name: &str) -> Result<String, FmError> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_GROUP_NAME
        || name.chars().any(char::is_control)
    {
        return Err(FmError::InvalidParams(format!(
            "a folder name is 1–{MAX_GROUP_NAME} characters without control characters"
        )));
    }
    Ok(name.to_string())
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
        self.item_group.push(None);
        self.order.push(index);
        Ok(index)
    }

    /// Insert a requested batch after the current item, before ordinary playback resumes.
    /// Inside an unlocked folder the items join it; a locked folder is skipped, so they go
    /// right after it.
    pub fn insert_next(&mut self, items: Vec<VfsUri>) -> Result<usize, FmError> {
        let first = self.current.map_or(0, |index| index + 1);
        let first = match self.enclosing_group(first) {
            Some(id) if self.group_is_locked(Some(id)) => self.past_group(first, id),
            _ => first,
        };
        self.insert_at(first, items)
    }

    /// Like `insert_next`, but never inside a folder: the items go after it, so a new folder
    /// can be formed from them.
    pub fn insert_next_outside_groups(&mut self, items: Vec<VfsUri>) -> Result<usize, FmError> {
        let first = self.current.map_or(0, |index| index + 1);
        let first = match self.enclosing_group(first) {
            Some(id) => self.past_group(first, id),
            None => first,
        };
        self.insert_at(first, items)
    }

    /// The folder an insert at `at` would land inside (items of it on both sides).
    fn enclosing_group(&self, at: usize) -> Option<u32> {
        let before = (*self.item_group.get(at.checked_sub(1)?)?)?;
        (self.item_group.get(at) == Some(&Some(before))).then_some(before)
    }

    /// The first slot at or after `at` that is no longer in folder `id`.
    fn past_group(&self, mut at: usize, id: u32) -> usize {
        while self.item_group.get(at) == Some(&Some(id)) {
            at += 1;
        }
        at
    }

    fn insert_at(&mut self, first: usize, items: Vec<VfsUri>) -> Result<usize, FmError> {
        if items.is_empty() || self.items.len().saturating_add(items.len()) > MAX_QUEUE_ITEMS {
            return Err(FmError::InvalidParams("invalid or full media queue".into()));
        }
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
        let around = (
            first.checked_sub(1).and_then(|at| self.item_group.get(at)),
            self.item_group.get(first),
        );
        let joined = match around {
            (Some(Some(before)), Some(Some(after))) if before == after => Some(*before),
            _ => None,
        };
        self.item_group
            .splice(first..first, std::iter::repeat_n(joined, count));
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
        self.item_group.remove(index);
        self.normalize_groups();
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
        // Without the moved row, the neighbours of its new place decide where it lands.
        let without = |at: usize| self.item_group.get(if at < from { at } else { at + 1 });
        let inside = to
            .checked_sub(1)
            .and_then(without)
            .zip(without(to))
            .is_some_and(|(before, after)| before.is_some() && before == after);
        let target = if inside {
            without(to).copied().flatten()
        } else {
            None
        };
        if self.is_locked(from) || self.group_is_locked(target) {
            return Err(locked_error());
        }
        let item = self.items.remove(from);
        self.items.insert(to, item);
        let own = self.item_group.remove(from);
        self.item_group.insert(to, own);
        let neighbour = |at: Option<usize>| at.and_then(|at| self.item_group.get(at)).copied();
        let before = neighbour(to.checked_sub(1)).flatten();
        let after = neighbour(Some(to + 1)).flatten();
        let adopted = match (before, after) {
            (Some(a), Some(b)) if a == b => Some(a),
            _ if own.is_some() && (before == own || after == own) => own,
            _ => None,
        };
        if let Some(slot) = self.item_group.get_mut(to) {
            *slot = adopted;
        }
        self.normalize_groups();
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
        self.item_group.clear();
        self.group_meta.clear();
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
        if self.ends_group() {
            return None;
        }
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

    /// Folders in queue order; empty ones come last.
    pub fn groups(&self) -> Vec<Group> {
        let mut out = Vec::new();
        let mut at = 0;
        while at < self.item_group.len() {
            let Some(Some(id)) = self.item_group.get(at).copied() else {
                at += 1;
                continue;
            };
            let mut end = at + 1;
            while self.item_group.get(end) == Some(&Some(id)) {
                end += 1;
            }
            if let Some(meta) = self.group_meta.iter().find(|meta| meta.id == id) {
                out.push(Group {
                    id,
                    start: at,
                    len: end - at,
                    name: meta.name.clone(),
                    dir: meta.dir.clone(),
                    locked: meta.locked,
                });
            }
            at = end;
        }
        for meta in &self.group_meta {
            if !out.iter().any(|group| group.id == meta.id) {
                out.push(Group {
                    id: meta.id,
                    start: self.items.len(),
                    len: 0,
                    name: meta.name.clone(),
                    dir: meta.dir.clone(),
                    locked: meta.locked,
                });
            }
        }
        out
    }

    /// The folder of an item, without building the whole folder list.
    pub fn group_of(&self, index: usize) -> Option<GroupRef<'_>> {
        let id = (*self.item_group.get(index)?)?;
        let meta = self.group_meta.iter().find(|meta| meta.id == id)?;
        Some(GroupRef {
            id,
            name: &meta.name,
            dir: meta.dir.as_deref(),
            last: self.item_group.get(index + 1) != Some(&Some(id)),
            locked: meta.locked,
        })
    }

    /// Playback stops here: the current item ends its folder and nothing else is lined up.
    /// Not with shuffle or repeat, which have their own idea of what comes next.
    pub fn ends_group(&self) -> bool {
        !self.shuffle
            && self.repeat == RepeatMode::Off
            && self.next_up.is_empty()
            && self
                .history_cursor
                .is_none_or(|cursor| cursor + 1 >= self.history.len())
            && self
                .current
                .and_then(|current| self.group_of(current))
                .is_some_and(|group| group.last)
    }

    /// Index of the first item of the folder the current item belongs to.
    pub fn current_group_start(&self) -> Option<usize> {
        let current = self.current?;
        let id = (*self.item_group.get(current)?)?;
        let mut start = current;
        while start > 0 && self.item_group.get(start - 1) == Some(&Some(id)) {
            start -= 1;
        }
        Some(start)
    }

    pub fn create_group(&mut self, name: &str, dir: Option<String>) -> Result<u32, FmError> {
        let name = valid_group_name(name)?;
        if self.group_meta.len() >= MAX_GROUPS {
            return Err(FmError::InvalidParams(format!(
                "at most {MAX_GROUPS} folders per queue"
            )));
        }
        let id = self.next_group;
        self.next_group = self.next_group.wrapping_add(1);
        let locked = dir.is_some();
        self.group_meta.push(GroupMeta {
            id,
            name,
            dir,
            locked,
        });
        Ok(id)
    }

    /// Put `len` consecutive items starting at `start` into a new folder.
    pub fn group_range(
        &mut self,
        start: usize,
        len: usize,
        name: &str,
        dir: Option<String>,
    ) -> Result<u32, FmError> {
        let range = start..start.saturating_add(len);
        if len == 0 || range.end > self.items.len() {
            return Err(FmError::InvalidParams("folder range out of bounds".into()));
        }
        let id = self.create_group(name, dir)?;
        for slot in &mut self.item_group[range] {
            *slot = Some(id);
        }
        self.normalize_groups();
        Ok(id)
    }

    pub fn rename_group(&mut self, id: u32, name: &str) -> Result<(), FmError> {
        let name = valid_group_name(name)?;
        let meta = self
            .group_meta
            .iter_mut()
            .find(|meta| meta.id == id)
            .ok_or_else(|| FmError::InvalidParams(format!("unknown folder: {id}")))?;
        meta.name = name;
        Ok(())
    }

    pub fn set_group_locked(&mut self, id: u32, locked: bool) -> Result<(), FmError> {
        let meta = self
            .group_meta
            .iter_mut()
            .find(|meta| meta.id == id)
            .ok_or_else(|| FmError::InvalidParams(format!("unknown folder: {id}")))?;
        meta.locked = locked;
        Ok(())
    }

    fn group_is_locked(&self, id: Option<u32>) -> bool {
        id.is_some_and(|id| self.group_meta.iter().any(|m| m.id == id && m.locked))
    }

    /// The item sits in a locked folder, so it may not be removed, moved or regrouped.
    pub fn is_locked(&self, index: usize) -> bool {
        self.group_is_locked(self.item_group.get(index).copied().flatten())
    }

    /// Dissolve a folder; its items stay where they are.
    pub fn delete_group(&mut self, id: u32) -> Result<(), FmError> {
        if !self.group_meta.iter().any(|meta| meta.id == id) {
            return Err(FmError::InvalidParams(format!("unknown folder: {id}")));
        }
        for slot in &mut self.item_group {
            if *slot == Some(id) {
                *slot = None;
            }
        }
        self.group_meta.retain(|meta| meta.id != id);
        Ok(())
    }

    /// Move an item to the end of a folder (an empty folder is at the end of the queue).
    /// Returns the item's new index.
    pub fn add_to_group(&mut self, index: usize, id: u32) -> Result<usize, FmError> {
        if index >= self.items.len() {
            return Err(FmError::InvalidParams(format!(
                "media queue index out of range: {index}"
            )));
        }
        if !self.group_meta.iter().any(|meta| meta.id == id) {
            return Err(FmError::InvalidParams(format!("unknown folder: {id}")));
        }
        if self.group_is_locked(Some(id)) || self.is_locked(index) {
            return Err(locked_error());
        }
        let run = self.groups().into_iter().find(|group| group.id == id);
        let target = match run {
            Some(group) if group.len > 0 => {
                if index >= group.start && index < group.start + group.len {
                    return Ok(index);
                }
                if index < group.start {
                    group.start + group.len - 1
                } else {
                    group.start + group.len
                }
            }
            _ => self.items.len() - 1,
        };
        self.move_item(index, target)?;
        if let Some(slot) = self.item_group.get_mut(target) {
            *slot = Some(id);
        }
        self.normalize_groups();
        Ok(target)
    }

    /// Take an item out of its folder, placing it just after it. Returns the new index.
    pub fn remove_from_group(&mut self, index: usize) -> Result<usize, FmError> {
        let Some(slot) = self.item_group.get(index).copied() else {
            return Err(FmError::InvalidParams(format!(
                "media queue index out of range: {index}"
            )));
        };
        let Some(id) = slot else {
            return Ok(index);
        };
        if self.group_is_locked(Some(id)) {
            return Err(locked_error());
        }
        let Some(group) = self.groups().into_iter().find(|group| group.id == id) else {
            return Ok(index);
        };
        let last = group.start + group.len - 1;
        let target = if index == group.start { index } else { last };
        if target != index {
            self.move_item(index, target)?;
        }
        if let Some(slot) = self.item_group.get_mut(target) {
            *slot = None;
        }
        self.normalize_groups();
        Ok(target)
    }

    /// Move an item to just after row `after`, joining that row's folder. Returns the new index.
    pub fn place_after(&mut self, index: usize, after: usize) -> Result<usize, FmError> {
        self.place_item(index, after, true, true)
    }

    /// Move an item to just before row `before`, joining that row's folder. Returns the new index.
    pub fn place_before(&mut self, index: usize, before: usize) -> Result<usize, FmError> {
        self.place_item(index, before, false, true)
    }

    /// Move an item to just outside the folder of `row` (before or after that whole folder);
    /// a row without a folder acts like `place_before`/`place_after`. Returns the new index.
    pub fn place_beside_group(
        &mut self,
        index: usize,
        row: usize,
        after: bool,
    ) -> Result<usize, FmError> {
        self.place_item(index, row, after, false)
    }

    fn place_item(
        &mut self,
        index: usize,
        row: usize,
        after: bool,
        join: bool,
    ) -> Result<usize, FmError> {
        let (Some(_), Some(group)) = (self.items.get(index), self.item_group.get(row).copied())
        else {
            return Err(FmError::InvalidParams(
                "media queue index out of range".to_string(),
            ));
        };
        if self.is_locked(index) || (join && self.group_is_locked(group)) {
            return Err(locked_error());
        }
        let span = group
            .filter(|_| !join)
            .and_then(|id| self.groups().into_iter().find(|g| g.id == id))
            .map_or((row, row), |g| (g.start, g.start + g.len - 1));
        let at = if after { span.1 + 1 } else { span.0 };
        let slot = if join { group } else { None };
        let to = if at == index || at == index + 1 {
            index
        } else if index < at {
            at - 1
        } else {
            at
        };
        if to != index {
            self.move_item(index, to)?;
        }
        if let Some(entry) = self.item_group.get_mut(to) {
            *entry = slot;
        }
        self.normalize_groups();
        Ok(to)
    }

    /// Move a whole folder to just before row `before` (before that row's folder, if it is in
    /// one). Returns the folder's new first index.
    pub fn place_group_before(&mut self, id: u32, before: usize) -> Result<usize, FmError> {
        if before >= self.items.len() {
            return Err(FmError::InvalidParams(
                "media queue index out of range".to_string(),
            ));
        }
        let at = self
            .groups()
            .iter()
            .find(|g| before >= g.start && before < g.start + g.len)
            .map_or(before, |g| g.start);
        self.place_group_at(id, at)
    }

    /// Move a whole folder to just after row `after` (after that row's folder, if it is in one).
    /// Returns the folder's new first index.
    pub fn place_group_after(&mut self, id: u32, after: usize) -> Result<usize, FmError> {
        if after >= self.items.len() {
            return Err(FmError::InvalidParams(
                "media queue index out of range".to_string(),
            ));
        }
        let at = self
            .groups()
            .iter()
            .find(|g| after >= g.start && after < g.start + g.len)
            .map_or(after + 1, |g| g.start + g.len);
        self.place_group_at(id, at)
    }

    /// Move a folder so it starts at the row boundary `at` (counted before the move).
    fn place_group_at(&mut self, id: u32, at: usize) -> Result<usize, FmError> {
        let groups = self.groups();
        let Some(group) = groups.iter().find(|g| g.id == id && g.len > 0) else {
            return Err(FmError::InvalidParams(format!("unknown folder: {id}")));
        };
        let (start, len) = (group.start, group.len);
        if at >= start && at <= start + len {
            return Ok(start);
        }
        // Detach while moving so edits cannot re-sort or drop the folder; automatic folders
        // would be forgotten for being empty, so keep their details.
        let meta = self.group_meta.iter().find(|m| m.id == id).cloned();
        for slot in self.item_group.iter_mut().skip(start).take(len) {
            *slot = None;
        }
        let first = if at > start + len {
            for _ in 0..len {
                self.move_item(start, at - 1)?;
            }
            at - len
        } else {
            for k in 0..len {
                self.move_item(start + k, at + k)?;
            }
            at
        };
        for slot in self.item_group.iter_mut().skip(first).take(len) {
            *slot = Some(id);
        }
        if let Some(meta) = meta
            && !self.group_meta.iter().any(|m| m.id == id)
        {
            self.group_meta.push(meta);
        }
        self.normalize_groups();
        Ok(first)
    }

    /// Keep each folder one run, and forget automatic folders that lost all their items.
    fn normalize_groups(&mut self) {
        let mut seen: Vec<u32> = Vec::new();
        let mut previous = None;
        for slot in &mut self.item_group {
            match *slot {
                Some(id) if Some(id) != previous => {
                    if seen.contains(&id) {
                        *slot = None;
                    } else {
                        seen.push(id);
                    }
                }
                _ => {}
            }
            previous = *slot;
        }
        self.group_meta
            .retain(|meta| meta.dir.is_none() || seen.contains(&meta.id));
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

    fn names(q: &Queue) -> Vec<String> {
        q.items()
            .iter()
            .map(|u| u.file_name().unwrap_or("").to_string())
            .collect()
    }

    fn shape(q: &Queue) -> Vec<(String, usize, usize)> {
        q.groups()
            .into_iter()
            .map(|g| (g.name, g.start, g.len))
            .collect()
    }

    #[test]
    fn folders_are_runs_that_follow_edits() {
        let mut q = queue(6);
        q.group_range(1, 3, "book", Some("/book".into()))
            .expect("group");
        assert_eq!(shape(&q), [("book".to_string(), 1, 3)]);
        // Removing a member shrinks it; removing all drops an automatic folder.
        q.remove(2).expect("remove");
        assert_eq!(shape(&q), [("book".to_string(), 1, 2)]);
        q.remove(1).expect("remove");
        q.remove(1).expect("remove");
        assert!(q.groups().is_empty());
    }

    #[test]
    fn moving_items_in_and_out_of_a_folder_changes_membership() {
        let mut q = queue(5);
        q.group_range(1, 3, "f", None).expect("group");
        // Out the top, then back into the middle of the run.
        q.move_item(1, 0).expect("up");
        assert_eq!(shape(&q), [("f".to_string(), 2, 2)]);
        q.move_item(0, 2).expect("into");
        assert_eq!(q.groups()[0].len, 3);
        assert_eq!(q.group_of(2).map(|g| g.name), Some("f"));
    }

    #[test]
    fn dragging_into_and_out_of_folders_keeps_them_contiguous() {
        let mut q = queue(5);
        let id = q.create_group("empty", None).expect("create");
        assert_eq!(shape(&q), [("empty".to_string(), 5, 0)]);
        assert_eq!(q.add_to_group(0, id).expect("add"), 4);
        assert_eq!(names(&q), ["1.mp3", "2.mp3", "3.mp3", "4.mp3", "0.mp3"]);
        q.add_to_group(1, id).expect("add");
        assert_eq!(shape(&q), [("empty".to_string(), 3, 2)]);
        // Taking a middle member out lands it just after the folder.
        let mid = q.group_range(0, 2, "head", None).expect("head");
        q.add_to_group(4, mid).expect("add");
        q.remove_from_group(1).expect("out");
        for g in q.groups() {
            let ids: Vec<_> = (g.start..g.start + g.len)
                .map(|i| q.group_of(i).map(|r| r.id))
                .collect();
            assert!(ids.iter().all(|i| *i == Some(g.id)));
        }
        assert!(q.add_to_group(99, id).is_err());
        assert!(q.add_to_group(0, 4242).is_err());
        assert!(q.rename_group(id, "  ").is_err());
        assert!(q.rename_group(id, "o\nk").is_err());
        q.delete_group(id).expect("delete");
        assert!(q.groups().iter().all(|g| g.id != id));
    }

    #[test]
    fn playback_stops_at_the_end_of_a_folder_only_without_shuffle_or_repeat() {
        let mut q = queue(4);
        q.group_range(1, 2, "f", None).expect("group");
        q.select(1).expect("select");
        assert!(!q.ends_group());
        assert!(q.peek_next().is_some());
        q.select(2).expect("select");
        assert!(q.ends_group());
        assert!(q.peek_next().is_none());
        assert_eq!(q.current_group_start(), Some(1));
        q.set_repeat(RepeatMode::All);
        assert!(!q.ends_group());
        q.set_repeat(RepeatMode::Off);
        q.set_shuffle(true, 3);
        assert!(!q.ends_group());
    }

    #[test]
    fn inserting_inside_a_folder_joins_it_unless_asked_to_stay_outside() {
        let mut q = queue(4);
        q.group_range(0, 4, "all", None).expect("group");
        q.select(1).expect("select");
        q.insert_next(vec![VfsUri::parse("/x.mp3").expect("uri")])
            .expect("insert");
        assert_eq!(shape(&q), [("all".to_string(), 0, 5)]);
        let first = q
            .insert_next_outside_groups(vec![VfsUri::parse("/y.mp3").expect("uri")])
            .expect("insert");
        assert_eq!(first, 5);
        assert_eq!(shape(&q), [("all".to_string(), 0, 5)]);
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
    fn items_and_folders_can_be_placed_after_a_row() {
        let mut q = queue(6);
        q.group_range(1, 2, "f", Some("/m".into())).expect("group");
        let id = q.groups()[0].id;
        q.set_group_locked(id, false).expect("unlock");
        assert_eq!(q.place_after(5, 2).expect("join"), 3);
        assert_eq!(q.groups()[0].len, 3);
        assert_eq!(q.place_after(0, 4).expect("out"), 4);
        assert_eq!(q.groups()[0].start, 0);
        assert_eq!(q.groups()[0].len, 3);
        assert_eq!(q.place_group_after(id, 4).expect("down"), 2);
        assert_eq!((q.groups()[0].start, q.groups()[0].len), (2, 3));
        assert_eq!(q.place_group_after(id, 0).expect("up"), 1);
        assert_eq!((q.groups()[0].start, q.groups()[0].len), (1, 3));
        assert!(q.place_group_after(id, 99).is_err());
        assert!(q.place_after(0, 99).is_err());
    }

    #[test]
    fn a_locked_folder_keeps_its_items_but_can_still_move_whole() {
        let mut q = queue(6);
        let id = q
            .group_range(1, 2, "book", Some("/b".into()))
            .expect("group");
        assert!(q.groups()[0].locked, "automatic folders start locked");
        assert!(q.place_after(5, 1).is_err(), "join");
        assert!(q.place_before(5, 2).is_err(), "join before");
        assert!(q.add_to_group(5, id).is_err(), "add");
        assert!(q.remove_from_group(1).is_err(), "ungroup");
        assert!(q.place_beside_group(1, 1, false).is_err(), "leave");
        assert!(q.move_item(1, 4).is_err(), "move out");
        assert!(q.move_item(5, 2).is_err(), "move in");
        assert!(q.is_locked(2) && !q.is_locked(0));
        // Boundaries are not inside the folder, and the folder itself may travel.
        q.place_beside_group(5, 1, false).expect("before folder");
        assert_eq!(q.groups()[0].start, 2);
        q.place_group_after(id, 5).expect("whole folder");
        assert_eq!(q.groups()[0].len, 2);
        q.set_group_locked(id, false).expect("unlock");
        assert!(q.add_to_group(0, id).is_ok());
    }

    #[test]
    fn play_next_inside_a_locked_folder_goes_after_it() {
        let mut q = queue(5);
        let id = q
            .group_range(1, 3, "book", Some("/b".into()))
            .expect("group");
        q.select(2).expect("current inside the folder");
        let extra = VfsUri::parse("/extra.mkv").expect("uri");
        let at = q.insert_next(vec![extra.clone()]).expect("insert");
        assert_eq!(at, 4, "right after the folder");
        assert_eq!(q.groups()[0].len, 3, "the locked folder keeps its items");
        assert!(!q.is_locked(at));
        assert_eq!(q.items().get(at), Some(&extra));
        // Unlocked, the same insert joins the folder.
        q.set_group_locked(id, false).expect("unlock");
        let joined = q.insert_next(items(1)).expect("insert");
        assert_eq!(joined, 3);
        assert_eq!(q.groups()[0].len, 4);
    }

    #[test]
    fn moving_an_automatic_folder_keeps_it() {
        let mut q = queue(6);
        q.group_range(0, 3, "book", Some("/book".into()))
            .expect("group");
        let id = q.groups()[0].id;
        assert_eq!(q.place_group_after(id, 5).expect("down"), 3);
        let g = q.groups();
        assert_eq!((g.len(), g[0].id, g[0].start, g[0].len), (1, id, 3, 3));
        assert_eq!(q.place_group_after(id, 0).expect("up"), 1);
        assert_eq!(q.groups()[0].start, 1);
    }

    #[test]
    fn placing_before_the_first_row_puts_an_item_at_the_top() {
        let mut q = queue(4);
        assert_eq!(q.place_before(2, 0).expect("top"), 0);
        assert_eq!(q.items()[0].to_string(), "file:///2.mp3");
        q.group_range(1, 2, "f", None).expect("group");
        let id = q.groups()[0].id;
        assert_eq!(q.place_group_before(id, 0).expect("folder top"), 0);
        assert_eq!(q.groups()[0].start, 0);
        assert_eq!(q.place_before(3, 1).expect("into folder"), 1);
        assert_eq!(q.groups()[0].len, 3);
    }

    #[test]
    fn items_can_be_placed_before_or_after_a_whole_folder() {
        let mut q = queue(5);
        q.group_range(1, 2, "f", None).expect("group");
        assert_eq!(q.place_beside_group(4, 2, false).expect("before"), 1);
        let g = q.groups();
        assert_eq!((g[0].start, g[0].len), (2, 2));
        assert_eq!(q.place_beside_group(0, 3, true).expect("after"), 3);
        let g = q.groups();
        assert_eq!((g[0].start, g[0].len), (1, 2));
        // The folder's own last item steps out behind it.
        assert_eq!(q.place_beside_group(2, 2, true).expect("out"), 2);
        let g = q.groups();
        assert_eq!((g[0].start, g[0].len), (1, 1));
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
