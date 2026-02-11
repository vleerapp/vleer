use crate::data::{
    db::repo::Database,
    models::{Cuid, Song},
};
use gpui::{App, Global};
use std::cell::RefCell;
use tracing::debug;

pub struct Queue {
    items: Vec<Cuid>,
    current_index: Option<usize>,
    shuffle: bool,
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
            repeat_mode: RepeatMode::Off,
            current_song: RefCell::new(None),
        }
    }

    pub fn init(cx: &mut App) {
        cx.set_global(Queue::new());
    }

    pub fn add_song(&mut self, song_id: Cuid) {
        self.items.push(song_id);
        if self.current_index.is_none() && !self.items.is_empty() {
            self.current_index = Some(0);
        }
        debug!("Added song to queue. Queue size: {}", self.items.len());
    }

    pub fn add_songs(&mut self, song_ids: Vec<Cuid>) {
        let was_empty = self.items.is_empty();
        let count = song_ids.len();
        self.items.extend(song_ids);
        if was_empty && !self.items.is_empty() {
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

    pub fn advance_next_id(&mut self) -> Option<Cuid> {
        if self.items.is_empty() {
            return None;
        }
        match self.repeat_mode {
            RepeatMode::One => {
                return self.get_current_song_id();
            }
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

    pub fn advance_previous_id(&mut self) -> Option<Cuid> {
        if self.items.is_empty() {
            return None;
        }
        match self.repeat_mode {
            RepeatMode::One => {
                return self.get_current_song_id();
            }
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

    pub fn next(&mut self, cx: &App) -> Option<Song> {
        if self.items.is_empty() {
            return None;
        }
        match self.repeat_mode {
            RepeatMode::One => {
                return self.get_current_song(cx);
            }
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
        self.get_current_song(cx)
    }

    pub fn previous(&mut self, cx: &App) -> Option<Song> {
        if self.items.is_empty() {
            return None;
        }
        match self.repeat_mode {
            RepeatMode::One => {
                return self.get_current_song(cx);
            }
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
        self.get_current_song(cx)
    }

    pub fn set_current_index(&mut self, index: usize, cx: &App) -> Option<Song> {
        if index < self.items.len() {
            self.current_index = Some(index);
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
        debug!("Queue cleared");
    }

    pub fn remove_at(&mut self, index: usize) -> Option<Cuid> {
        if index < self.items.len() {
            let item = self.items.remove(index);
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
        debug!("Shuffle set to: {}", shuffle);
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

    pub fn has_previous(&self) -> bool {
        if self.items.is_empty() {
            return false;
        }
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

impl Default for Queue {
    fn default() -> Self {
        Self::new()
    }
}
