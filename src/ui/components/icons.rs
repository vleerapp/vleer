pub const ALBUM: &str = "!bundled:icons/album.svg";
pub const ARROW_LEFT: &str = "!bundled:icons/arrow-left.svg";
pub const ARROW_RIGHT: &str = "!bundled:icons/arrow-right.svg";
pub const ARROW_DOWN: &str = "!bundled:icons/arrow-down.svg";
pub const ARROW_UP: &str = "!bundled:icons/arrow-up.svg";
pub const ARTIST: &str = "!bundled:icons/artist.svg";
pub const DURATION: &str = "!bundled:icons/duration.svg";
pub const FAVORITE: &str = "!bundled:icons/favorite.svg";
pub const UNFAVORITE: &str = "!bundled:icons/unfavorite.svg";
pub const HOME: &str = "!bundled:icons/home.svg";
pub const NEXT: &str = "!bundled:icons/next.svg";
pub const PAUSE: &str = "!bundled:icons/pause.svg";
pub const PIN: &str = "!bundled:icons/pin.svg";
pub const UNPIN: &str = "!bundled:icons/unpin.svg";
pub const PLAY: &str = "!bundled:icons/play.svg";
pub const PLAYLIST: &str = "!bundled:icons/playlist.svg";
pub const PLUS: &str = "!bundled:icons/plus.svg";
pub const PREVIOUS: &str = "!bundled:icons/previous.svg";
pub const QUEUE: &str = "!bundled:icons/queue.svg";
pub const REPLAY: &str = "!bundled:icons/replay.svg";
pub const REPLAY_1: &str = "!bundled:icons/replay-1.svg";
pub const SEARCH: &str = "!bundled:icons/search.svg";
pub const SETTINGS: &str = "!bundled:icons/settings.svg";
pub const SHUFFLE: &str = "!bundled:icons/shuffle.svg";
pub const SONGS: &str = "!bundled:icons/songs.svg";
pub const VOLUME_1: &str = "!bundled:icons/volume-1.svg";
pub const VOLUME_2: &str = "!bundled:icons/volume-2.svg";
pub const VOLUME_3: &str = "!bundled:icons/volume-3.svg";
pub const VOLUME_4: &str = "!bundled:icons/volume-4.svg";
pub const VOLUME_MUTE: &str = "!bundled:icons/volume-mute.svg";
pub const X: &str = "!bundled:icons/x.svg";
pub const MAXIMIZE: &str = "!bundled:icons/maximize.svg";
pub const UNMAXIMIZE: &str = "!bundled:icons/unmaximize.svg";
pub const MINIMIZE: &str = "!bundled:icons/minimize.svg";
pub const PROPERTIES: &str = "!bundled:icons/properties.svg";
pub const TRASH: &str = "!bundled:icons/trash.svg";
pub const PLAY_NEXT: &str = "!bundled:icons/play-next.svg";
pub const PLAY_LAST: &str = "!bundled:icons/play-last.svg";

use gpui::*;

use crate::ui::variables::Variables;

#[derive(IntoElement)]
pub struct Icon {
    svg: Stateful<Svg>,
}

impl Styled for Icon {
    fn style(&mut self) -> &mut StyleRefinement {
        self.svg.style()
    }
}

impl InteractiveElement for Icon {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.svg.interactivity()
    }
}

impl StatefulInteractiveElement for Icon {}

impl RenderOnce for Icon {
    fn render(mut self, _: &mut gpui::Window, cx: &mut gpui::App) -> impl gpui::IntoElement {
        let variables = cx.global::<Variables>();

        if self.svg.text_style().color.is_none() {
            self.svg = self.svg.text_color(variables.text_secondary);
        }

        self.svg.w(px(16.0)).h(px(16.0)).flex_shrink_0()
    }
}

pub fn icon(icon: impl Into<SharedString>) -> Icon {
    let icon_str: SharedString = icon.into();
    Icon {
        svg: svg().path(icon_str.clone()).id(icon_str.clone()),
    }
}
