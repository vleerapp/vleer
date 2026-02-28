use crate::data::{
    db::repo::Database,
    models::{Cuid, Song},
};
use gpui::{App, Global};
use rand::RngExt;
use rand::seq::SliceRandom;
use std::cell::RefCell;
use tracing::debug;

pub struct Queue {
    items: Vec<Cuid>,
    current_index: Option<usize>,
    shuffle: bool,
    shuffle_order: Vec<usize>,
    shuffle_position: Option<usize>,
    repeat_mode: RepeatMode,
    current_song: RefCell<Option<(Cuid, Song)>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RepeatMode {
    Off,
    All,
    One,
}

impl Global for Queue {}

impl Queue {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            current_index: None,
            shuffle: false,
            shuffle_order: Vec::new(),
            shuffle_position: None,
            repeat_mode: RepeatMode::Off,
            current_song: RefCell::new(None),
        }
    }

    pub fn init(cx: &mut App) {
        cx.set_global(Queue::new());
    }

    pub fn add_song(&mut self, song_id: Cuid) {
        let new_idx = self.items.len();
        self.items.push(song_id);

        if self.shuffle {
            if let Some(pos) = self.shuffle_position {
                let insert_at = rand::rng().random_range(pos + 1..=self.shuffle_order.len());
                self.shuffle_order.insert(insert_at, new_idx);
            } else {
                self.shuffle_order.push(new_idx);
                self.shuffle_position = Some(0);
                self.current_index = Some(new_idx);
            }
        } else if self.current_index.is_none() {
            self.current_index = Some(0);
        }

        debug!("Added song to queue. Queue size: {}", self.items.len());
    }

    pub fn add_songs(&mut self, song_ids: Vec<Cuid>) {
        let was_empty = self.items.is_empty();
        let start_idx = self.items.len();
        let count = song_ids.len();
        self.items.extend(song_ids);

        if self.shuffle {
            let mut new_indices: Vec<usize> = (start_idx..self.items.len()).collect();
            new_indices.shuffle(&mut rand::rng());

            if let Some(pos) = self.shuffle_position {
                let tail: Vec<usize> = self.shuffle_order.drain(pos + 1..).collect();
                self.shuffle_order.extend(new_indices);
                self.shuffle_order.extend(tail);
            } else {
                self.shuffle_order.extend(new_indices);
                if !self.shuffle_order.is_empty() {
                    self.shuffle_position = Some(0);
                    self.current_index = self.shuffle_order.first().copied();
                }
            }
        } else if was_empty && !self.items.is_empty() {
            self.current_index = Some(0);
        }

        debug!(
            "Added {} songs to queue. Queue size: {}",
            count,
            self.items.len()
        );
    }

    pub fn get_current_song(&self, cx: &App) -> Option<Song> {
        let song_id = self
            .current_index
            .and_then(|idx| self.items.get(idx).cloned())?;

        {
            let cache: std::cell::Ref<'_, Option<(Cuid, Song)>> = self.current_song.borrow();
            if let Some((cached_id, cached_song)) = cache.as_ref() {
                if cached_id == &song_id {
                    return Some(cached_song.clone());
                }
            }
        }

        let db = cx.global::<Database>();
        let song = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(db.get_song(song_id.clone()))
                .ok()
                .flatten()
        })?;

        *self.current_song.borrow_mut() = Some((song_id, song.clone()));

        Some(song)
    }

    pub fn get_current_song_id(&self) -> Option<Cuid> {
        self.current_index
            .and_then(|idx| self.items.get(idx).cloned())
    }

    pub fn set_current_song_cache(&self, song_id: Cuid, song: Song) {
        *self.current_song.borrow_mut() = Some((song_id, song));
    }

    pub fn next(&mut self) -> Option<Cuid> {
        if self.items.is_empty() {
            return None;
        }
        if self.repeat_mode == RepeatMode::One {
            return self.get_current_song_id();
        }

        if self.shuffle {
            if self.shuffle_order.is_empty() {
                return None;
            }
            let next_pos = self.shuffle_position.map(|p| p + 1).unwrap_or(0);

            match self.repeat_mode {
                RepeatMode::One => unreachable!(),
                RepeatMode::All => {
                    let wrapped = next_pos % self.shuffle_order.len();
                    self.shuffle_position = Some(wrapped);
                    self.current_index = self.shuffle_order.get(wrapped).copied();
                }
                RepeatMode::Off => {
                    if next_pos < self.shuffle_order.len() {
                        self.shuffle_position = Some(next_pos);
                        self.current_index = self.shuffle_order.get(next_pos).copied();
                    } else {
                        return None;
                    }
                }
            }

            *self.current_song.borrow_mut() = None;
            debug!(
                "Shuffle next. Position: {:?}, Item index: {:?}",
                self.shuffle_position, self.current_index
            );
            return self.get_current_song_id();
        }

        match self.repeat_mode {
            RepeatMode::One => unreachable!(),
            RepeatMode::All => {
                if let Some(idx) = self.current_index {
                    self.current_index = Some((idx + 1) % self.items.len());
                } else {
                    self.current_index = Some(0);
                }
            }
            RepeatMode::Off => {
                if let Some(idx) = self.current_index {
                    if idx + 1 < self.items.len() {
                        self.current_index = Some(idx + 1);
                    } else {
                        return None;
                    }
                } else {
                    self.current_index = Some(0);
                }
            }
        }
        debug!("Moved to next song. Index: {:?}", self.current_index);
        *self.current_song.borrow_mut() = None;
        self.get_current_song_id()
    }

    pub fn previous(&mut self) -> Option<Cuid> {
        if self.items.is_empty() {
            return None;
        }
        if self.repeat_mode == RepeatMode::One {
            return self.get_current_song_id();
        }

        if self.shuffle {
            if self.shuffle_order.is_empty() {
                return None;
            }

            match self.shuffle_position {
                Some(pos) if pos > 0 => {
                    self.shuffle_position = Some(pos - 1);
                    self.current_index = self.shuffle_order.get(pos - 1).copied();
                }
                Some(_) => {
                    if self.repeat_mode == RepeatMode::All {
                        let last = self.shuffle_order.len() - 1;
                        self.shuffle_position = Some(last);
                        self.current_index = self.shuffle_order.get(last).copied();
                    } else {
                        return None;
                    }
                }
                None => {
                    let last = self.shuffle_order.len() - 1;
                    self.shuffle_position = Some(last);
                    self.current_index = self.shuffle_order.get(last).copied();
                }
            }

            *self.current_song.borrow_mut() = None;
            debug!(
                "Shuffle previous. Position: {:?}, Item index: {:?}",
                self.shuffle_position, self.current_index
            );
            return self.get_current_song_id();
        }

        match self.repeat_mode {
            RepeatMode::One => unreachable!(),
            RepeatMode::All => {
                if let Some(idx) = self.current_index {
                    if idx == 0 {
                        self.current_index = Some(self.items.len() - 1);
                    } else {
                        self.current_index = Some(idx - 1);
                    }
                } else {
                    self.current_index = Some(0);
                }
            }
            RepeatMode::Off => {
                if let Some(idx) = self.current_index {
                    if idx > 0 {
                        self.current_index = Some(idx - 1);
                    } else {
                        return None;
                    }
                } else {
                    self.current_index = Some(0);
                }
            }
        }
        debug!("Moved to previous song. Index: {:?}", self.current_index);
        *self.current_song.borrow_mut() = None;
        self.get_current_song_id()
    }

    pub fn set_current_index(&mut self, index: usize, cx: &App) -> Option<Song> {
        if index < self.items.len() {
            self.current_index = Some(index);
            if self.shuffle {
                if let Some(pos) = self.shuffle_order.iter().position(|&x| x == index) {
                    self.shuffle_position = Some(pos);
                }
            }
            debug!("Set current index to {}", index);
            *self.current_song.borrow_mut() = None;
            self.get_current_song(cx)
        } else {
            None
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.current_index = None;
        self.shuffle_order.clear();
        self.shuffle_position = None;
        *self.current_song.borrow_mut() = None;
        debug!("Queue cleared");
    }

    pub fn remove_at(&mut self, index: usize) -> Option<Cuid> {
        if index < self.items.len() {
            let item = self.items.remove(index);

            if self.shuffle {
                if let Some(shuffle_pos) = self.shuffle_order.iter().position(|&x| x == index) {
                    self.shuffle_order.remove(shuffle_pos);
                    if let Some(curr_pos) = self.shuffle_position {
                        if shuffle_pos < curr_pos {
                            self.shuffle_position = Some(curr_pos - 1);
                        } else if shuffle_pos == curr_pos {
                            if self.shuffle_order.is_empty() {
                                self.shuffle_position = None;
                            } else if curr_pos >= self.shuffle_order.len() {
                                self.shuffle_position = Some(self.shuffle_order.len() - 1);
                            }
                        }
                    }
                }
                for idx in self.shuffle_order.iter_mut() {
                    if *idx > index {
                        *idx -= 1;
                    }
                }
                self.current_index = self
                    .shuffle_position
                    .and_then(|pos| self.shuffle_order.get(pos).copied());
            } else {
                if let Some(current) = self.current_index {
                    if current == index {
                        if self.items.is_empty() {
                            self.current_index = None;
                        } else if current >= self.items.len() {
                            self.current_index = Some(self.items.len() - 1);
                        }
                    } else if current > index {
                        self.current_index = Some(current - 1);
                    }
                }
            }

            debug!(
                "Removed song at index {}. Queue size: {}",
                index,
                self.items.len()
            );
            Some(item)
        } else {
            None
        }
    }

    pub fn get_items(&self) -> &[Cuid] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn set_shuffle(&mut self, shuffle: bool) {
        self.shuffle = shuffle;
        if shuffle {
            self.regenerate_shuffle_order();
        }
        debug!("Shuffle set to: {}", shuffle);
    }

    fn regenerate_shuffle_order(&mut self) {
        if self.items.is_empty() {
            self.shuffle_order.clear();
            self.shuffle_position = None;
            return;
        }

        let mut order: Vec<usize> = (0..self.items.len()).collect();
        order.shuffle(&mut rand::rng());

        if let Some(current) = self.current_index {
            if let Some(pos) = order.iter().position(|&x| x == current) {
                order.swap(0, pos);
            }
        }

        self.shuffle_order = order;
        self.shuffle_position = if self.current_index.is_some() {
            Some(0)
        } else {
            None
        };
    }

    pub fn get_shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn set_repeat_mode(&mut self, mode: RepeatMode) {
        self.repeat_mode = mode;
        debug!("Repeat mode set to: {:?}", mode);
    }

    pub fn get_repeat_mode(&self) -> RepeatMode {
        self.repeat_mode
    }

    pub fn cycle_repeat_mode(&mut self) {
        self.repeat_mode = match self.repeat_mode {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        };
        debug!("Repeat mode: {:?}", self.repeat_mode);
    }

    pub fn has_next(&self) -> bool {
        if self.items.is_empty() {
            return false;
        }
        if self.shuffle {
            match self.repeat_mode {
                RepeatMode::One | RepeatMode::All => true,
                RepeatMode::Off => {
                    if let Some(pos) = self.shuffle_position {
                        pos + 1 < self.shuffle_order.len()
                    } else {
                        !self.shuffle_order.is_empty()
                    }
                }
            }
        } else {
            match self.repeat_mode {
                RepeatMode::One | RepeatMode::All => true,
                RepeatMode::Off => {
                    if let Some(idx) = self.current_index {
                        idx + 1 < self.items.len()
                    } else {
                        !self.items.is_empty()
                    }
                }
            }
        }
    }

    pub fn has_previous(&self) -> bool {
        if self.items.is_empty() {
            return false;
        }
        if self.shuffle {
            match self.repeat_mode {
                RepeatMode::One | RepeatMode::All => true,
                RepeatMode::Off => {
                    if let Some(pos) = self.shuffle_position {
                        pos > 0
                    } else {
                        false
                    }
                }
            }
        } else {
            match self.repeat_mode {
                RepeatMode::One | RepeatMode::All => true,
                RepeatMode::Off => {
                    if let Some(idx) = self.current_index {
                        idx > 0
                    } else {
                        false
                    }
                }
            }
        }
    }
}

impl Default for Queue {
    fn default() -> Self {
        Self::new()
    }
}
