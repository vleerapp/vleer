#![allow(missing_docs)]

use crate::{
    Background, BackgroundTag, Bounds, Corners, Edges, FontId, GlyphId, Hsla, Path, Pixels, Size,
};
use palette::FromColor as _;
use collections::HashMap;
use std::{
    cell::RefCell,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Copy, Debug, Default)]
pub struct CaptureRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl From<Bounds<Pixels>> for CaptureRect {
    fn from(b: Bounds<Pixels>) -> Self {
        Self {
            x: b.origin.x.0,
            y: b.origin.y.0,
            w: b.size.width.0,
            h: b.size.height.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CaptureColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl From<Hsla> for CaptureColor {
    fn from(c: Hsla) -> Self {
        let rgb = palette::Srgba::from_color(c);
        Self {
            r: rgb.red,
            g: rgb.green,
            b: rgb.blue,
            a: rgb.alpha,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CaptureFill {
    Solid(CaptureColor),
    Linear {
        angle_degrees: f32,
        from: (f32, CaptureColor),
        to: (f32, CaptureColor),
    },
}

impl CaptureFill {
    pub fn is_transparent(&self) -> bool {
        match self {
            CaptureFill::Solid(c) => c.a == 0.0,
            CaptureFill::Linear { from, to, .. } => from.1.a == 0.0 && to.1.a == 0.0,
        }
    }
}

fn scene_color(c: crate::SceneHsla) -> CaptureColor {
    let hsla: Hsla = c.into();
    hsla.into()
}

impl From<Background> for CaptureFill {
    fn from(bg: Background) -> Self {
        match bg.tag {
            BackgroundTag::LinearGradient => CaptureFill::Linear {
                angle_degrees: bg.gradient_angle_or_pattern_height,
                from: (bg.colors[0].percentage, scene_color(bg.colors[0].color)),
                to: (bg.colors[1].percentage, scene_color(bg.colors[1].color)),
            },
            _ => CaptureFill::Solid(scene_color(bg.solid)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum OutlineCommand {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    QuadTo(f32, f32, f32, f32),
    CurveTo(f32, f32, f32, f32, f32, f32),
    Close,
}

#[derive(Clone, Debug, Default)]
pub struct GlyphOutline {
    pub units_per_em: f32,
    pub commands: Vec<OutlineCommand>,
}

#[derive(Clone, Debug)]
pub enum CaptureOp {
    Shadow {
        bounds: CaptureRect,
        element_bounds: CaptureRect,
        radii: [f32; 4],
        blur: f32,
        color: CaptureFill,
        mask: CaptureRect,
    },
    Quad {
        bounds: CaptureRect,
        radii: [f32; 4],
        border_widths: [f32; 4],
        background: CaptureFill,
        border_color: CaptureFill,
        dashed: Option<(f32, f32)>,
        mask: CaptureRect,
    },
    Path {
        triangles: Vec<[(f32, f32); 3]>,
        curves: Vec<[(f32, f32); 3]>,
        color: CaptureFill,
        mask: CaptureRect,
    },
    Line {
        rect: CaptureRect,
        wavy: bool,
        color: CaptureColor,
        mask: CaptureRect,
    },
    Glyph {
        x: f32,
        y: f32,
        font_size: f32,
        color: CaptureColor,
        outline: Arc<GlyphOutline>,
        mask: CaptureRect,
    },
    Svg {
        bounds: CaptureRect,
        source: Arc<Vec<u8>>,
        color: CaptureColor,
        mask: CaptureRect,
    },
    Image {
        bounds: CaptureRect,
        image_bounds: CaptureRect,
        radii: [f32; 4],
        png: Arc<Vec<u8>>,
        grayscale: bool,
        opacity: f32,
        mask: CaptureRect,
    },
}

#[derive(Clone, Debug)]
pub struct Capture {
    pub width: f32,
    pub height: f32,
    pub scale_factor: f32,
    pub ops: Vec<CaptureOp>,
}

pub(crate) fn corners(c: Corners<Pixels>) -> [f32; 4] {
    [
        c.top_left.0,
        c.top_right.0,
        c.bottom_right.0,
        c.bottom_left.0,
    ]
}

pub(crate) fn edges(e: Edges<Pixels>) -> [f32; 4] {
    [e.top.0, e.right.0, e.bottom.0, e.left.0]
}

pub(crate) fn path_op(path: &Path<Pixels>, color: Background, mask: Bounds<Pixels>) -> CaptureOp {
    let mut triangles = Vec::new();
    let mut curves = Vec::new();
    for tri in path.vertices.chunks_exact(3) {
        let pts = [
            (tri[0].xy_position.x.0, tri[0].xy_position.y.0),
            (tri[1].xy_position.x.0, tri[1].xy_position.y.0),
            (tri[2].xy_position.x.0, tri[2].xy_position.y.0),
        ];
        let is_curve = tri[1].st_position.x == 0.5;
        if is_curve {
            curves.push(pts);
        } else {
            triangles.push(pts);
        }
    }
    CaptureOp::Path {
        triangles,
        curves,
        color: color.into(),
        mask: mask.into(),
    }
}

struct Recorder {
    ops: Vec<CaptureOp>,
    outlines: HashMap<(FontId, GlyphId), Arc<GlyphOutline>>,
    images: HashMap<(usize, usize), Arc<Vec<u8>>>,
}

thread_local! {
    static RECORDER: RefCell<Option<Recorder>> = const { RefCell::new(None) };
}

static ARMED: AtomicBool = AtomicBool::new(false);
type Sink = Box<dyn Fn(Capture) + Send + Sync>;
static SINK: OnceLock<Sink> = OnceLock::new();

pub fn set_sink(sink: impl Fn(Capture) + Send + Sync + 'static) {
    let _ = SINK.set(Box::new(sink));
}

pub(crate) fn arm() {
    ARMED.store(true, Ordering::SeqCst);
}

pub(crate) fn begin_if_armed() -> bool {
    if SINK.get().is_some() && ARMED.swap(false, Ordering::SeqCst) {
        RECORDER.with(|r| {
            *r.borrow_mut() = Some(Recorder {
                ops: Vec::new(),
                outlines: HashMap::default(),
                images: HashMap::default(),
            })
        });
        true
    } else {
        false
    }
}

pub(crate) fn finish(size: Size<Pixels>, scale_factor: f32) {
    let recorder = RECORDER.with(|r| r.borrow_mut().take());
    if let (Some(recorder), Some(sink)) = (recorder, SINK.get()) {
        sink(Capture {
            width: size.width.0,
            height: size.height.0,
            scale_factor,
            ops: recorder.ops,
        });
    }
}

pub(crate) fn recording() -> bool {
    RECORDER.with(|r| r.borrow().is_some())
}

pub(crate) fn record(op: impl FnOnce() -> CaptureOp) {
    RECORDER.with(|r| {
        if let Some(recorder) = r.borrow_mut().as_mut() {
            recorder.ops.push(op());
        }
    });
}

pub(crate) fn cached_outline(
    key: (FontId, GlyphId),
    load: impl FnOnce() -> Option<GlyphOutline>,
) -> Option<Arc<GlyphOutline>> {
    RECORDER.with(|r| {
        let mut r = r.borrow_mut();
        let recorder = r.as_mut()?;
        if let Some(found) = recorder.outlines.get(&key) {
            return Some(found.clone());
        }
        let outline = Arc::new(load()?);
        recorder.outlines.insert(key, outline.clone());
        Some(outline)
    })
}

pub(crate) fn cached_image(
    key: (usize, usize),
    encode: impl FnOnce() -> Option<Vec<u8>>,
) -> Option<Arc<Vec<u8>>> {
    RECORDER.with(|r| {
        let mut r = r.borrow_mut();
        let recorder = r.as_mut()?;
        if let Some(found) = recorder.images.get(&key) {
            return Some(found.clone());
        }
        let png = Arc::new(encode()?);
        recorder.images.insert(key, png.clone());
        Some(png)
    })
}
