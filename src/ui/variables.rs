use gpui::*;

pub struct Variables {
    pub background: Rgba,
    pub element: Rgba,
    pub element_hover: Rgba,
    pub border: Rgba,
    pub accent: Rgba,
    pub text: Rgba,
    pub text_secondary: Rgba,
    pub text_muted: Rgba,

    pub padding_8: f32,
    pub padding_16: f32,
    pub padding_24: f32,
    pub padding_32: f32,
}

impl Default for Variables {
    fn default() -> Self {
        Self {
            background: rgb(0x121212),
            element: rgb(0x1A1A1A),
            element_hover: rgb(0x242424),
            border: rgb(0x535353),
            accent: rgb(0xA058FF),
            text: rgb(0xE6E6E6),
            text_secondary: rgb(0xABABAB),
            text_muted: rgb(0x303030),

            padding_8: 8.0,
            padding_16: 16.0,
            padding_24: 24.0,
            padding_32: 32.0,
        }
    }
}

impl Global for Variables {}

impl Variables {
    pub fn init(cx: &mut App) {
        cx.set_global(Variables::default());
    }
}
