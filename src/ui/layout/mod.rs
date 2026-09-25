use gpui::Global;

pub mod library;
pub mod lyrics;
pub mod navbar;
pub mod player;
pub mod queue;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum SidePanel {
    #[default]
    Closed,
    Queue,
    Lyrics,
}

impl Global for SidePanel {}

impl SidePanel {
    pub fn is_open(self) -> bool {
        self != Self::Closed
    }

    pub fn toggle(&mut self, target: Self) {
        *self = if *self == target {
            Self::Closed
        } else {
            target
        };
    }
}
