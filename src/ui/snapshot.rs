use base64::{Engine as _, engine::general_purpose::STANDARD};
use gpui::vector_capture::{
    Capture, CaptureColor, CaptureFill, CaptureOp, CaptureRect, GlyphOutline, OutlineCommand,
};
use std::{collections::HashMap, fmt::Write as _, sync::Arc};

struct Writer {
    view: CaptureRect,
    defs: String,
    body: String,
    next_id: usize,
    clips: HashMap<String, usize>,
    glyphs: HashMap<usize, usize>,
    images: HashMap<usize, usize>,
}

fn n(value: f32) -> String {
    let text = format!("{:.2}", value);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

fn precise(value: f32) -> String {
    let text = format!("{:.7}", value);
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn rgb(color: CaptureColor) -> String {
    format!(
        "rgb({},{},{})",
        (color.r * 255.0).round() as u8,
        (color.g * 255.0).round() as u8,
        (color.b * 255.0).round() as u8
    )
}

fn color_attrs(attr: &str, color: CaptureColor) -> String {
    if color.a >= 0.999 {
        format!(r#" {attr}="{}""#, rgb(color))
    } else {
        format!(r#" {attr}="{}" {attr}-opacity="{}""#, rgb(color), n(color.a))
    }
}

fn rrect_path(rect: CaptureRect, radii: [f32; 4]) -> String {
    let max = (rect.w.min(rect.h) / 2.0).max(0.0);
    let [tl, tr, br, bl] = radii.map(|r| r.clamp(0.0, max));
    let (x, y, w, h) = (rect.x, rect.y, rect.w, rect.h);
    let mut d = String::new();
    let _ = write!(
        d,
        "M{} {}H{}A{} {} 0 0 1 {} {}V{}A{} {} 0 0 1 {} {}H{}A{} {} 0 0 1 {} {}V{}A{} {} 0 0 1 {} {}Z",
        n(x + tl),
        n(y),
        n(x + w - tr),
        n(tr),
        n(tr),
        n(x + w),
        n(y + tr),
        n(y + h - br),
        n(br),
        n(br),
        n(x + w - br),
        n(y + h),
        n(x + bl),
        n(bl),
        n(bl),
        n(x),
        n(y + h - bl),
        n(y + tl),
        n(tl),
        n(tl),
        n(x + tl),
        n(y),
    );
    d
}

fn outline_path(outline: &GlyphOutline) -> String {
    let mut d = String::new();
    for command in &outline.commands {
        match *command {
            OutlineCommand::MoveTo(x, y) => {
                let _ = write!(d, "M{} {}", n(x), n(y));
            }
            OutlineCommand::LineTo(x, y) => {
                let _ = write!(d, "L{} {}", n(x), n(y));
            }
            OutlineCommand::QuadTo(cx, cy, x, y) => {
                let _ = write!(d, "Q{} {} {} {}", n(cx), n(cy), n(x), n(y));
            }
            OutlineCommand::CurveTo(c1x, c1y, c2x, c2y, x, y) => {
                let _ = write!(
                    d,
                    "C{} {} {} {} {} {}",
                    n(c1x),
                    n(c1y),
                    n(c2x),
                    n(c2y),
                    n(x),
                    n(y)
                );
            }
            OutlineCommand::Close => d.push('Z'),
        }
    }
    d
}

fn find_attr(tag: &str, name: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let needle = format!(" {name}={quote}");
        if let Some(start) = tag.find(&needle) {
            let value_start = start + needle.len();
            let end = tag[value_start..].find(quote)?;
            return Some(tag[value_start..value_start + end].to_string());
        }
    }
    None
}

fn strip_attr(tag: &str, name: &str) -> String {
    for quote in ['"', '\''] {
        let needle = format!(" {name}={quote}");
        if let Some(start) = tag.find(&needle) {
            let value_start = start + needle.len();
            if let Some(end) = tag[value_start..].find(quote) {
                return format!("{}{}", &tag[..start], &tag[value_start + end + 1..]);
            }
        }
    }
    tag.to_string()
}

fn parse_length(value: &str) -> Option<f32> {
    value
        .trim()
        .trim_end_matches("px")
        .trim()
        .parse::<f32>()
        .ok()
}

fn place_icon(source: &str, rect: CaptureRect) -> Option<String> {
    let start = source.find("<svg")?;
    let source = &source[start..];
    let end = source.find('>')?;
    let tag = source[..end].trim_end_matches('/');
    let rest = &source[end + 1..];

    let width = find_attr(tag, "width").and_then(|v| parse_length(&v));
    let height = find_attr(tag, "height").and_then(|v| parse_length(&v));
    let view_box = find_attr(tag, "viewBox").or_else(|| {
        Some(format!("0 0 {} {}", n(width?), n(height?)))
    });

    let mut tag = strip_attr(tag, "width");
    tag = strip_attr(&tag, "height");
    tag = strip_attr(&tag, "x");
    tag = strip_attr(&tag, "y");
    if find_attr(&tag, "viewBox").is_none() {
        if let Some(view_box) = view_box {
            tag.push_str(&format!(r#" viewBox="{view_box}""#));
        }
    }
    tag = tag.replacen(
        "<svg",
        &format!(
            r#"<svg x="{}" y="{}" width="{}" height="{}""#,
            n(rect.x),
            n(rect.y),
            n(rect.w),
            n(rect.h)
        ),
        1,
    );
    Some(format!("{tag}>{rest}"))
}

impl Writer {
    fn id(&mut self) -> usize {
        self.next_id += 1;
        self.next_id
    }

    fn clip_for(&mut self, mask: CaptureRect) -> Option<usize> {
        let view = self.view;
        if mask.x <= view.x
            && mask.y <= view.y
            && mask.x + mask.w >= view.x + view.w
            && mask.y + mask.h >= view.y + view.h
        {
            return None;
        }
        let key = format!("{} {} {} {}", n(mask.x), n(mask.y), n(mask.w), n(mask.h));
        if let Some(id) = self.clips.get(&key) {
            return Some(*id);
        }
        let id = self.id();
        let _ = write!(
            self.defs,
            r#"<clipPath id="c{id}"><rect x="{}" y="{}" width="{}" height="{}"/></clipPath>"#,
            n(mask.x),
            n(mask.y),
            n(mask.w),
            n(mask.h)
        );
        self.clips.insert(key, id);
        Some(id)
    }

    fn paint(&mut self, attr: &str, fill: CaptureFill, rect: CaptureRect) -> String {
        match fill {
            CaptureFill::Solid(color) => color_attrs(attr, color),
            CaptureFill::Linear {
                angle_degrees,
                from,
                to,
            } => {
                let id = self.id();
                let theta = angle_degrees.to_radians();
                let (dx, dy) = (theta.sin(), -theta.cos());
                let half = ((rect.w * theta.sin()).abs() + (rect.h * theta.cos()).abs()) / 2.0;
                let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
                let _ = write!(
                    self.defs,
                    r#"<linearGradient id="g{id}" gradientUnits="userSpaceOnUse" x1="{}" y1="{}" x2="{}" y2="{}"><stop offset="{}" stop-color="{}" stop-opacity="{}"/><stop offset="{}" stop-color="{}" stop-opacity="{}"/></linearGradient>"#,
                    n(cx - dx * half),
                    n(cy - dy * half),
                    n(cx + dx * half),
                    n(cy + dy * half),
                    n(from.0),
                    rgb(from.1),
                    n(from.1.a),
                    n(to.0),
                    rgb(to.1),
                    n(to.1.a),
                );
                format!(r#" {attr}="url(#g{id})""#)
            }
        }
    }

    fn shape(&mut self, rect: CaptureRect, radii: [f32; 4], extra: &str) -> String {
        let uniform = radii.iter().all(|r| *r == radii[0]);
        if radii.iter().all(|r| *r <= 0.0) {
            format!(
                r#"<rect x="{}" y="{}" width="{}" height="{}"{extra}/>"#,
                n(rect.x),
                n(rect.y),
                n(rect.w),
                n(rect.h)
            )
        } else if uniform {
            let r = radii[0].min(rect.w.min(rect.h) / 2.0);
            format!(
                r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}"{extra}/>"#,
                n(rect.x),
                n(rect.y),
                n(rect.w),
                n(rect.h),
                n(r)
            )
        } else {
            format!(r#"<path d="{}"{extra}/>"#, rrect_path(rect, radii))
        }
    }

    fn op(&mut self, op: &CaptureOp) {
        match op {
            CaptureOp::Shadow {
                bounds,
                element_bounds,
                radii,
                blur,
                color,
                ..
            } => {
                if color.is_transparent() {
                    return;
                }
                let filter = self.id();
                let cut = self.id();
                let pad = blur * 2.0;
                let view = self.view;
                let element = rrect_path(*element_bounds, *radii);
                let _ = write!(
                    self.defs,
                    r#"<filter id="f{filter}" filterUnits="userSpaceOnUse" x="{}" y="{}" width="{}" height="{}"><feGaussianBlur stdDeviation="{}"/></filter><clipPath id="x{cut}"><path clip-rule="evenodd" d="M{} {}h{}v{}h{}z{element}"/></clipPath>"#,
                    n(bounds.x - pad),
                    n(bounds.y - pad),
                    n(bounds.w + pad * 2.0),
                    n(bounds.h + pad * 2.0),
                    n(blur / 2.0),
                    n(view.x),
                    n(view.y),
                    n(view.w),
                    n(view.h),
                    n(-view.w),
                );
                let paint = self.paint("fill", *color, *bounds);
                let shape = self.shape(*bounds, *radii, &format!(r#" filter="url(#f{filter})"{paint}"#));
                let _ = write!(self.body, r#"<g clip-path="url(#x{cut})">{shape}</g>"#);
            }
            CaptureOp::Quad {
                bounds,
                radii,
                border_widths,
                background,
                border_color,
                dashed,
                ..
            } => {
                if !background.is_transparent() {
                    let paint = self.paint("fill", *background, *bounds);
                    let shape = self.shape(*bounds, *radii, &paint);
                    self.body.push_str(&shape);
                }
                let [top, right, bottom, left] = *border_widths;
                if border_color.is_transparent() || (top + right + bottom + left) <= 0.0 {
                    return;
                }
                if let Some((length, gap)) = dashed {
                    let half = top / 2.0;
                    let inner = CaptureRect {
                        x: bounds.x + half,
                        y: bounds.y + half,
                        w: bounds.w - top,
                        h: bounds.h - top,
                    };
                    let inner_radii = radii.map(|r| (r - half).max(0.0));
                    let paint = self.paint("stroke", *border_color, *bounds);
                    let shape = self.shape(
                        inner,
                        inner_radii,
                        &format!(
                            r#" fill="none" stroke-width="{}" stroke-dasharray="{} {}"{paint}"#,
                            n(top),
                            n(length * top),
                            n(gap * top)
                        ),
                    );
                    self.body.push_str(&shape);
                    return;
                }
                let inner = CaptureRect {
                    x: bounds.x + left,
                    y: bounds.y + top,
                    w: (bounds.w - left - right).max(0.0),
                    h: (bounds.h - top - bottom).max(0.0),
                };
                let inner_radii = [
                    (radii[0] - left.max(top)).max(0.0),
                    (radii[1] - right.max(top)).max(0.0),
                    (radii[2] - right.max(bottom)).max(0.0),
                    (radii[3] - left.max(bottom)).max(0.0),
                ];
                let paint = self.paint("fill", *border_color, *bounds);
                let _ = write!(
                    self.body,
                    r#"<path fill-rule="evenodd" d="{}{}"{paint}/>"#,
                    rrect_path(*bounds, *radii),
                    rrect_path(inner, inner_radii)
                );
            }
            CaptureOp::Path {
                triangles,
                curves,
                color,
                ..
            } => {
                let mut d = String::new();
                let area = |p: [(f32, f32); 3]| {
                    (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1)
                };
                for tri in triangles {
                    let t = if area(*tri) < 0.0 {
                        [tri[2], tri[1], tri[0]]
                    } else {
                        *tri
                    };
                    let _ = write!(
                        d,
                        "M{} {}L{} {}L{} {}Z",
                        n(t[0].0),
                        n(t[0].1),
                        n(t[1].0),
                        n(t[1].1),
                        n(t[2].0),
                        n(t[2].1)
                    );
                }
                for curve in curves {
                    let c = if area(*curve) < 0.0 {
                        [curve[2], curve[1], curve[0]]
                    } else {
                        *curve
                    };
                    let _ = write!(
                        d,
                        "M{} {}Q{} {} {} {}Z",
                        n(c[0].0),
                        n(c[0].1),
                        n(c[1].0),
                        n(c[1].1),
                        n(c[2].0),
                        n(c[2].1)
                    );
                }
                let all: Vec<(f32, f32)> = triangles
                    .iter()
                    .chain(curves.iter())
                    .flat_map(|t| t.iter().copied())
                    .collect();
                let (min_x, max_x) = all.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
                    (lo.min(p.0), hi.max(p.0))
                });
                let (min_y, max_y) = all.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
                    (lo.min(p.1), hi.max(p.1))
                });
                let rect = CaptureRect {
                    x: min_x,
                    y: min_y,
                    w: max_x - min_x,
                    h: max_y - min_y,
                };
                let paint = self.paint("fill", *color, rect);
                let _ = write!(self.body, r#"<path d="{d}"{paint}/>"#);
            }
            CaptureOp::Line {
                rect, wavy, color, ..
            } => {
                if *wavy {
                    let step = (rect.h * 2.0).max(2.0);
                    let mut d = format!("M{} {}", n(rect.x), n(rect.y + rect.h));
                    let mut x = rect.x;
                    let mut up = true;
                    while x < rect.x + rect.w {
                        x = (x + step).min(rect.x + rect.w);
                        let y = if up { rect.y } else { rect.y + rect.h };
                        let _ = write!(d, "L{} {}", n(x), n(y));
                        up = !up;
                    }
                    let _ = write!(
                        self.body,
                        r#"<path d="{d}" fill="none" stroke-width="{}"{}/>"#,
                        n(rect.h.max(1.0)),
                        color_attrs("stroke", *color)
                    );
                } else {
                    let _ = write!(
                        self.body,
                        r#"<rect x="{}" y="{}" width="{}" height="{}"{}/>"#,
                        n(rect.x),
                        n(rect.y),
                        n(rect.w),
                        n(rect.h),
                        color_attrs("fill", *color)
                    );
                }
            }
            CaptureOp::Glyph {
                x,
                y,
                font_size,
                color,
                outline,
                ..
            } => {
                let key = Arc::as_ptr(outline) as usize;
                let id = match self.glyphs.get(&key) {
                    Some(id) => *id,
                    None => {
                        let id = self.id();
                        let _ = write!(
                            self.defs,
                            r#"<path id="p{id}" d="{}"/>"#,
                            outline_path(outline)
                        );
                        self.glyphs.insert(key, id);
                        id
                    }
                };
                let scale = font_size / outline.units_per_em;
                let _ = write!(
                    self.body,
                    r##"<use href="#p{id}" transform="translate({} {}) scale({} {})"{}/>"##,
                    n(*x),
                    n(*y),
                    precise(scale),
                    precise(-scale),
                    color_attrs("fill", *color)
                );
            }
            CaptureOp::Svg {
                bounds,
                source,
                color,
                ..
            } => {
                let Ok(text) = std::str::from_utf8(source) else {
                    return;
                };
                let Some(icon) = place_icon(text, *bounds) else {
                    return;
                };
                let id = self.id();
                let _ = write!(
                    self.defs,
                    r#"<mask id="m{id}" maskUnits="userSpaceOnUse" style="mask-type:alpha" x="{}" y="{}" width="{}" height="{}">{icon}</mask>"#,
                    n(bounds.x),
                    n(bounds.y),
                    n(bounds.w),
                    n(bounds.h)
                );
                let _ = write!(
                    self.body,
                    r#"<rect x="{}" y="{}" width="{}" height="{}" mask="url(#m{id})"{}/>"#,
                    n(bounds.x),
                    n(bounds.y),
                    n(bounds.w),
                    n(bounds.h),
                    color_attrs("fill", *color)
                );
            }
            CaptureOp::Image {
                bounds,
                image_bounds,
                radii,
                png,
                grayscale,
                opacity,
                ..
            } => {
                let key = Arc::as_ptr(png) as usize;
                let image = match self.images.get(&key) {
                    Some(id) => *id,
                    None => {
                        let id = self.id();
                        let _ = write!(
                            self.defs,
                            r#"<image id="i{id}" width="1" height="1" preserveAspectRatio="none" href="data:{};base64,{}"/>"#,
                            if png.starts_with(&[0xFF, 0xD8]) {
                                "image/jpeg"
                            } else {
                                "image/png"
                            },
                            STANDARD.encode(png.as_slice())
                        );
                        self.images.insert(key, id);
                        id
                    }
                };
                let clip = self.id();
                let _ = write!(
                    self.defs,
                    r#"<clipPath id="r{clip}"><path d="{}"/></clipPath>"#,
                    rrect_path(*bounds, *radii)
                );
                let mut extra = String::new();
                if *opacity < 0.999 {
                    let _ = write!(extra, r#" opacity="{}""#, n(*opacity));
                }
                if *grayscale {
                    let filter = self.id();
                    let _ = write!(
                        self.defs,
                        r#"<filter id="s{filter}"><feColorMatrix type="saturate" values="0"/></filter>"#
                    );
                    let _ = write!(extra, r#" filter="url(#s{filter})""#);
                }
                let _ = write!(
                    self.body,
                    r##"<g clip-path="url(#r{clip})"{extra}><use href="#i{image}" transform="translate({} {}) scale({} {})"/></g>"##,
                    n(image_bounds.x),
                    n(image_bounds.y),
                    n(image_bounds.w),
                    n(image_bounds.h)
                );
            }
        }
    }
}

fn mask_of(op: &CaptureOp) -> CaptureRect {
    match op {
        CaptureOp::Shadow { mask, .. }
        | CaptureOp::Quad { mask, .. }
        | CaptureOp::Path { mask, .. }
        | CaptureOp::Line { mask, .. }
        | CaptureOp::Glyph { mask, .. }
        | CaptureOp::Svg { mask, .. }
        | CaptureOp::Image { mask, .. } => *mask,
    }
}

pub fn to_svg(capture: &Capture) -> String {
    let mut writer = Writer {
        view: CaptureRect {
            x: 0.0,
            y: 0.0,
            w: capture.width,
            h: capture.height,
        },
        defs: String::new(),
        body: String::new(),
        next_id: 0,
        clips: HashMap::new(),
        glyphs: HashMap::new(),
        images: HashMap::new(),
    };

    let mut open_clip: Option<usize> = None;
    for op in &capture.ops {
        let clip = writer.clip_for(mask_of(op));
        if clip != open_clip {
            if open_clip.is_some() {
                writer.body.push_str("</g>");
            }
            if let Some(id) = clip {
                let _ = write!(writer.body, r#"<g clip-path="url(#c{id})">"#);
            }
            open_clip = clip;
        }
        writer.op(op);
    }
    if open_clip.is_some() {
        writer.body.push_str("</g>");
    }

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}"><defs>{}</defs>{}</svg>"#,
        writer.defs,
        writer.body,
        w = n(capture.width),
        h = n(capture.height),
    )
}
