use gpui::*;

#[inline(always)]
pub fn flex_col() -> Div {
    div().flex().flex_col()
}

#[inline(always)]
pub fn flex_row() -> Div {
    div().flex().flex_row().items_center()
}

const ELLIPSIS: &str = "…";

pub struct MiddleTruncate {
    text: SharedString,
    color: Rgba,
}

impl MiddleTruncate {
    pub fn new(text: impl Into<SharedString>, color: Rgba) -> Self {
        Self {
            text: text.into(),
            color,
        }
    }

    fn shape(&self, text: SharedString, window: &Window) -> ShapedLine {
        let style = window.text_style();
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color: rgb_to_hsla(self.color),
            background_color: None,
            letter_spacing: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_line(text, style.font_size.to_pixels(window.rem_size()), &[run])
    }
}

impl IntoElement for MiddleTruncate {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for MiddleTruncate {
    type RequestLayoutState = ();
    type PrepaintState = ShapedLine;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        style.min_size.width = px(0.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let full = self.shape(self.text.clone(), window);
        if full.width <= bounds.size.width {
            return full;
        }

        let chars: Vec<char> = self.text.chars().collect();
        let build = |keep: usize| -> String {
            let left = keep - keep / 2;
            let right = keep / 2;
            let mut out: String = chars[..left].iter().collect();
            out.push_str(ELLIPSIS);
            out.extend(&chars[chars.len() - right..]);
            out
        };

        let (mut low, mut high) = (0, chars.len().saturating_sub(1));
        let mut best = self.shape(ELLIPSIS.into(), window);
        while low < high {
            let mid = (low + high).div_ceil(2);
            let candidate = self.shape(build(mid).into(), window);
            if candidate.width <= bounds.size.width {
                best = candidate;
                low = mid;
            } else {
                high = mid - 1;
            }
        }
        best
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        line: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(
            Some(ContentMask {
                bounds,
                ..Default::default()
            }),
            |window| {
                let line_height = window.line_height();
                if let Err(e) = line.paint(
                    bounds.origin,
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                ) {
                    tracing::error!("Failed to paint truncated text: {}", e);
                }
            },
        );
    }
}
