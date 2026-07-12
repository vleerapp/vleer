use parking_lot::RwLock;
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusColor {
    Accent,
    Warning,
    Destructive,
}

#[derive(Clone)]
pub struct StatusEntry {
    pub text: String,
    pub ratio: Option<f32>,
    pub color: StatusColor,
}

#[derive(Default)]
pub struct StatusReporter {
    entries: RwLock<BTreeMap<String, StatusEntry>>,
}

impl StatusReporter {
    pub fn set(&self, key: &str, text: impl Into<String>, ratio: Option<f32>, color: StatusColor) {
        self.entries.write().insert(
            key.to_string(),
            StatusEntry {
                text: text.into(),
                ratio,
                color,
            },
        );
    }

    pub fn clear(&self, key: &str) {
        self.entries.write().remove(key);
    }

    pub fn entries(&self) -> Vec<StatusEntry> {
        self.entries.read().values().cloned().collect()
    }
}

static STATUS: OnceLock<StatusReporter> = OnceLock::new();

pub fn status() -> &'static StatusReporter {
    STATUS.get_or_init(StatusReporter::default)
}
