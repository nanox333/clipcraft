//! A small renderer for the icon files under `assets/icons`.
//!
//! The icons are plain SVG files, so they can be opened and edited in any vector editor. Only the
//! subset they use is read: `path` (M L H V C Q A Z, absolute and relative), `circle`, `rect`
//! (with `rx`), `line`, `polyline` and `polygon`, with `fill`, `stroke` and `stroke-width` set on
//! the root or on each shape. Anything else is skipped, never a panic: a file that cannot be read
//! draws nothing. Strokes get round ends and corners; fills are triangulated once on load, so
//! concave shapes work too.
//!
//! Parsing is bounded: files over [`MAX_BYTES`] are refused, and a path stops at [`MAX_POINTS`].

use eframe::egui::{Color32, Mesh, Painter, Pos2, Shape, Stroke, Vec2};

const MAX_BYTES: usize = 64 * 1024;
const MAX_POINTS: usize = 4096;
/// The icons are drawn on a 24-unit square.
const VIEW: f32 = 24.0;

type Pt = (f32, f32);

#[derive(Debug, PartialEq)]
struct Shaped {
    points: Vec<Pt>,
    /// Real corners and ends, where a round dot is added; curve samples are left smooth.
    corners: Vec<Pt>,
    closed: bool,
    triangles: Vec<[Pt; 3]>,
    stroke: Option<f32>,
}

/// One parsed icon.
#[derive(Debug, Default, PartialEq)]
pub struct Svg {
    shapes: Vec<Shaped>,
}

#[derive(Clone, Copy)]
struct Style {
    fill: bool,
    stroke: bool,
    width: f32,
}

impl Svg {
    /// Reads an icon file. `None` when it has nothing drawable or is too large.
    pub fn parse(source: &str) -> Option<Self> {
        if source.len() > MAX_BYTES {
            return None;
        }
        let mut root = Style {
            fill: false,
            stroke: false,
            width: 2.0,
        };
        let mut shapes = Vec::new();
        for tag in tags(source) {
            let style = style_of(&tag, root);
            if tag.name == "svg" {
                root = style;
                continue;
            }
            let outlines: Vec<(Vec<Pt>, Vec<Pt>, bool)> = match tag.name.as_str() {
                "path" => tag.get("d").map(path_outlines).unwrap_or_default(),
                "circle" => circle(&tag).into_iter().collect(),
                "rect" => rect(&tag).into_iter().collect(),
                "line" => line(&tag).into_iter().collect(),
                "polyline" | "polygon" => poly(&tag, tag.name == "polygon").into_iter().collect(),
                _ => Vec::new(),
            };
            for (points, corners, closed) in outlines {
                if points.len() < 2 {
                    continue;
                }
                let triangles = if style.fill && closed {
                    triangulate(&points)
                } else {
                    Vec::new()
                };
                shapes.push(Shaped {
                    points,
                    corners,
                    closed,
                    triangles,
                    stroke: style.stroke.then_some(style.width),
                });
            }
        }
        (!shapes.is_empty()).then_some(Self { shapes })
    }

    /// Draws the icon centred on `centre`; one icon unit is `unit` pixels. `fill_closed` also
    /// fills every closed outline (a "liked" heart, say).
    pub fn paint(
        &self,
        painter: &Painter,
        centre: Pos2,
        unit: f32,
        color: Color32,
        fill_closed: bool,
    ) {
        let map = |(x, y): Pt| centre + Vec2::new(x - VIEW / 2.0, y - VIEW / 2.0) * unit;
        for shape in &self.shapes {
            let triangles = if fill_closed && shape.triangles.is_empty() && shape.closed {
                triangulate(&shape.points)
            } else {
                shape.triangles.clone()
            };
            if !triangles.is_empty() {
                let mut mesh = Mesh::default();
                for triangle in &triangles {
                    let base = mesh.vertices.len() as u32;
                    for point in triangle {
                        mesh.colored_vertex(map(*point), color);
                    }
                    mesh.add_triangle(base, base + 1, base + 2);
                }
                painter.add(Shape::mesh(mesh));
            }
            let width = match shape.stroke {
                Some(width) => (width * unit).max(1.0),
                None => continue,
            };
            let mut path: Vec<Pos2> = shape.points.iter().copied().map(map).collect();
            if shape.closed {
                path.extend(path.first().copied());
            }
            painter.add(Shape::line(path, Stroke::new(width, color)));
            for corner in &shape.corners {
                painter.circle_filled(map(*corner), width / 2.0, color);
            }
        }
    }
}

struct Tag {
    name: String,
    attributes: Vec<(String, String)>,
}

impl Tag {
    fn get(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn number(&self, name: &str) -> Option<f32> {
        self.get(name)
            .and_then(|value| value.trim().parse::<f32>().ok())
            .filter(|value| value.is_finite())
    }
}

/// Every opening or self-closing element, with its attributes. Comments and closing tags are
/// skipped.
fn tags(source: &str) -> Vec<Tag> {
    let mut found = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.find('<') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('>') else { break };
        let body = &rest[..end];
        rest = &rest[end + 1..];
        if body.starts_with('/') || body.starts_with('!') || body.starts_with('?') {
            continue;
        }
        let body = body.trim_end_matches('/').trim();
        let name_end = body.find(|c: char| c.is_whitespace()).unwrap_or(body.len());
        let name = body[..name_end].to_owned();
        let mut attributes = Vec::new();
        let mut text = &body[name_end..];
        while let Some(equals) = text.find('=') {
            let key = text[..equals].trim().to_owned();
            let after = text[equals + 1..].trim_start();
            let Some(quote) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else {
                break;
            };
            let Some(close) = after[1..].find(quote) else {
                break;
            };
            attributes.push((key, after[1..1 + close].to_owned()));
            text = &after[close + 2..];
        }
        found.push(Tag { name, attributes });
        if found.len() > 256 {
            break;
        }
    }
    found
}

fn style_of(tag: &Tag, inherited: Style) -> Style {
    let paint = |name: &str, current: bool| match tag.get(name) {
        Some("none") => false,
        Some(_) => true,
        None => current,
    };
    Style {
        fill: paint("fill", inherited.fill),
        stroke: paint("stroke", inherited.stroke),
        width: tag
            .number("stroke-width")
            .unwrap_or(inherited.width)
            .max(0.0),
    }
}

type Outline = (Vec<Pt>, Vec<Pt>, bool);

fn circle(tag: &Tag) -> Option<Outline> {
    let (cx, cy, r) = (tag.number("cx")?, tag.number("cy")?, tag.number("r")?);
    (r > 0.0).then(|| ellipse(cx, cy, r, r))
}

fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32) -> Outline {
    let points = (0..48)
        .map(|step| {
            let angle = step as f32 / 48.0 * std::f32::consts::TAU;
            (cx + rx * angle.cos(), cy + ry * angle.sin())
        })
        .collect();
    (points, Vec::new(), true)
}

fn rect(tag: &Tag) -> Option<Outline> {
    let (x, y) = (
        tag.number("x").unwrap_or(0.0),
        tag.number("y").unwrap_or(0.0),
    );
    let (w, h) = (tag.number("width")?, tag.number("height")?);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let r = tag.number("rx").unwrap_or(0.0).clamp(0.0, w.min(h) / 2.0);
    if r == 0.0 {
        let corners = vec![(x, y), (x + w, y), (x + w, y + h), (x, y + h)];
        return Some((corners.clone(), corners, true));
    }
    let mut points = Vec::new();
    // Four quarter circles, clockwise from the top-right corner.
    for (cx, cy, from) in [
        (x + w - r, y + r, -90.0_f32),
        (x + w - r, y + h - r, 0.0),
        (x + r, y + h - r, 90.0),
        (x + r, y + r, 180.0),
    ] {
        for step in 0..=8 {
            let angle = (from + step as f32 * 90.0 / 8.0).to_radians();
            points.push((cx + r * angle.cos(), cy + r * angle.sin()));
        }
    }
    Some((points, Vec::new(), true))
}

fn line(tag: &Tag) -> Option<Outline> {
    let ends = vec![
        (tag.number("x1")?, tag.number("y1")?),
        (tag.number("x2")?, tag.number("y2")?),
    ];
    Some((ends.clone(), ends, false))
}

fn poly(tag: &Tag, closed: bool) -> Option<Outline> {
    let numbers = numbers(tag.get("points")?);
    let points: Vec<Pt> = numbers.chunks_exact(2).map(|p| (p[0], p[1])).collect();
    Some((points.clone(), points, closed))
}

fn numbers(text: &str) -> Vec<f32> {
    let mut scanner = Scanner::new(text);
    let mut out = Vec::new();
    while let Some(value) = scanner.number() {
        out.push(value);
        if out.len() > MAX_POINTS {
            break;
        }
    }
    out
}

/// Reads numbers and command letters from path data, the way SVG writes them: separators are
/// optional, a sign or a second `.` starts a new number.
struct Scanner<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Scanner<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            at: 0,
        }
    }

    fn skip(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b',')
        {
            self.at += 1;
        }
    }

    fn command(&mut self) -> Option<u8> {
        self.skip();
        let byte = *self.bytes.get(self.at)?;
        byte.is_ascii_alphabetic().then(|| {
            self.at += 1;
            byte
        })
    }

    fn peek_number(&mut self) -> bool {
        self.skip();
        self.bytes
            .get(self.at)
            .is_some_and(|b| b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.'))
    }

    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let byte = *self.bytes.get(self.at)?;
        self.at += 1;
        match byte {
            b'0' => Some(false),
            b'1' => Some(true),
            _ => None,
        }
    }

    fn number(&mut self) -> Option<f32> {
        self.skip();
        let start = self.at;
        let mut end = start;
        let mut seen_dot = false;
        let mut seen_digit = false;
        while let Some(&byte) = self.bytes.get(end) {
            match byte {
                b'+' | b'-' if end == start => {}
                b'0'..=b'9' => seen_digit = true,
                b'.' if !seen_dot => seen_dot = true,
                b'e' | b'E' if seen_digit => {
                    end += 1;
                    if matches!(self.bytes.get(end), Some(b'+' | b'-')) {
                        end += 1;
                    }
                    while self.bytes.get(end).is_some_and(u8::is_ascii_digit) {
                        end += 1;
                    }
                    break;
                }
                _ => break,
            }
            end += 1;
        }
        if !seen_digit {
            return None;
        }
        self.at = end;
        std::str::from_utf8(self.bytes.get(start..end)?)
            .ok()?
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite())
    }
}

/// The sub-paths of a `d` attribute, as flattened points, corner points and whether each closes.
fn path_outlines(data: &str) -> Vec<Outline> {
    let mut scanner = Scanner::new(data);
    let mut outlines: Vec<Outline> = Vec::new();
    let (mut current, mut start) = ((0.0_f32, 0.0_f32), (0.0_f32, 0.0_f32));
    let mut points: Vec<Pt> = Vec::new();
    let mut corners: Vec<Pt> = Vec::new();
    let mut command = 0_u8;
    let finish =
        |points: &mut Vec<Pt>, corners: &mut Vec<Pt>, closed: bool, out: &mut Vec<Outline>| {
            if points.len() >= 2 {
                out.push((std::mem::take(points), std::mem::take(corners), closed));
            } else {
                points.clear();
                corners.clear();
            }
        };
    let mut total = 0_usize;
    loop {
        if !scanner.peek_number() {
            match scanner.command() {
                Some(next) => command = next,
                None => break,
            }
        } else if command == 0 {
            break;
        } else if command == b'M' {
            command = b'L';
        } else if command == b'm' {
            command = b'l';
        }
        total += 1;
        if total > MAX_POINTS {
            break;
        }
        let relative = command.is_ascii_lowercase();
        let base = if relative { current } else { (0.0, 0.0) };
        let take = |scanner: &mut Scanner<'_>, count: usize| -> Option<Vec<f32>> {
            (0..count).map(|_| scanner.number()).collect()
        };
        match command.to_ascii_uppercase() {
            b'M' => {
                finish(&mut points, &mut corners, false, &mut outlines);
                let Some(v) = take(&mut scanner, 2) else {
                    break;
                };
                current = (base.0 + v[0], base.1 + v[1]);
                start = current;
                points.push(current);
                corners.push(current);
            }
            b'L' => {
                let Some(v) = take(&mut scanner, 2) else {
                    break;
                };
                current = (base.0 + v[0], base.1 + v[1]);
                points.push(current);
                corners.push(current);
            }
            b'H' => {
                let Some(v) = take(&mut scanner, 1) else {
                    break;
                };
                current = (base.0 + v[0], current.1);
                points.push(current);
                corners.push(current);
            }
            b'V' => {
                let Some(v) = take(&mut scanner, 1) else {
                    break;
                };
                current = (current.0, base.1 + v[0]);
                points.push(current);
                corners.push(current);
            }
            b'C' => {
                let Some(v) = take(&mut scanner, 6) else {
                    break;
                };
                let p1 = (base.0 + v[0], base.1 + v[1]);
                let p2 = (base.0 + v[2], base.1 + v[3]);
                let end = (base.0 + v[4], base.1 + v[5]);
                for step in 1..=16 {
                    let t = step as f32 / 16.0;
                    let u = 1.0 - t;
                    points.push((
                        u * u * u * current.0
                            + 3.0 * u * u * t * p1.0
                            + 3.0 * u * t * t * p2.0
                            + t * t * t * end.0,
                        u * u * u * current.1
                            + 3.0 * u * u * t * p1.1
                            + 3.0 * u * t * t * p2.1
                            + t * t * t * end.1,
                    ));
                }
                current = end;
                corners.push(current);
            }
            b'Q' => {
                let Some(v) = take(&mut scanner, 4) else {
                    break;
                };
                let p1 = (base.0 + v[0], base.1 + v[1]);
                let end = (base.0 + v[2], base.1 + v[3]);
                for step in 1..=12 {
                    let t = step as f32 / 12.0;
                    let u = 1.0 - t;
                    points.push((
                        u * u * current.0 + 2.0 * u * t * p1.0 + t * t * end.0,
                        u * u * current.1 + 2.0 * u * t * p1.1 + t * t * end.1,
                    ));
                }
                current = end;
                corners.push(current);
            }
            b'A' => {
                let (Some(rx), Some(ry), Some(rotation)) =
                    (scanner.number(), scanner.number(), scanner.number())
                else {
                    break;
                };
                let (Some(large), Some(sweep)) = (scanner.flag(), scanner.flag()) else {
                    break;
                };
                let (Some(x), Some(y)) = (scanner.number(), scanner.number()) else {
                    break;
                };
                let end = (base.0 + x, base.1 + y);
                arc(
                    &mut points,
                    current,
                    end,
                    (rx, ry, rotation),
                    (large, sweep),
                );
                current = end;
                corners.push(current);
            }
            b'Z' => {
                current = start;
                finish(&mut points, &mut corners, true, &mut outlines);
                points.push(current);
                corners.push(current);
            }
            _ => break,
        }
    }
    finish(&mut points, &mut corners, false, &mut outlines);
    // A closed outline repeats its first point at the end; the drawing closes it itself.
    for (points, corners, closed) in &mut outlines {
        if *closed && points.len() > 2 && points.first() == points.last() {
            points.pop();
            corners.pop();
        }
    }
    outlines
}

/// Appends the points of an SVG elliptical arc from `from` to `to` (endpoint form, SVG 1.1 F.6).
fn arc(
    points: &mut Vec<Pt>,
    from: Pt,
    to: Pt,
    (rx, ry, rotation): (f32, f32, f32),
    (large, sweep): (bool, bool),
) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if from == to {
        return;
    }
    if rx == 0.0 || ry == 0.0 {
        points.push(to);
        return;
    }
    let phi = rotation.to_radians();
    let (sin, cos) = phi.sin_cos();
    let (dx, dy) = ((from.0 - to.0) / 2.0, (from.1 - to.1) / 2.0);
    let (x1, y1) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    let scale = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if scale > 1.0 {
        let root = scale.sqrt();
        rx *= root;
        ry *= root;
    }
    let numerator = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut factor = if denominator == 0.0 {
        0.0
    } else {
        (numerator / denominator).max(0.0).sqrt()
    };
    if large == sweep {
        factor = -factor;
    }
    let (cx1, cy1) = (factor * rx * y1 / ry, -factor * ry * x1 / rx);
    let centre = (
        cos * cx1 - sin * cy1 + (from.0 + to.0) / 2.0,
        sin * cx1 + cos * cy1 + (from.1 + to.1) / 2.0,
    );
    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| {
        let dot = ux * vx + uy * vy;
        let length = (ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt();
        let mut value = (dot / length.max(f32::EPSILON)).clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            value = -value;
        }
        value
    };
    let start = angle(1.0, 0.0, (x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut sweep_angle = angle(
        (x1 - cx1) / rx,
        (y1 - cy1) / ry,
        (-x1 - cx1) / rx,
        (-y1 - cy1) / ry,
    );
    if !sweep && sweep_angle > 0.0 {
        sweep_angle -= std::f32::consts::TAU;
    } else if sweep && sweep_angle < 0.0 {
        sweep_angle += std::f32::consts::TAU;
    }
    let steps = ((sweep_angle.abs() / std::f32::consts::TAU * 48.0).ceil() as usize).clamp(4, 48);
    for step in 1..=steps {
        let theta = start + sweep_angle * step as f32 / steps as f32;
        let (ex, ey) = (rx * theta.cos(), ry * theta.sin());
        points.push((
            cos * ex - sin * ey + centre.0,
            sin * ex + cos * ey + centre.1,
        ));
    }
}

/// Ear-clipping triangulation of a simple polygon, in either winding. A polygon that cannot be
/// split (self-crossing) falls back to a fan so something still draws.
fn triangulate(points: &[Pt]) -> Vec<[Pt; 3]> {
    let count = points.len();
    if count < 3 {
        return Vec::new();
    }
    let area: f32 = (0..count)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % count]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum();
    let mut order: Vec<usize> = if area >= 0.0 {
        (0..count).collect()
    } else {
        (0..count).rev().collect()
    };
    let cross = |a: Pt, b: Pt, c: Pt| (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    let inside = |p: Pt, a: Pt, b: Pt, c: Pt| {
        cross(a, b, p) >= 0.0 && cross(b, c, p) >= 0.0 && cross(c, a, p) >= 0.0
    };
    let mut triangles = Vec::new();
    let mut guard = count * count;
    while order.len() > 3 && guard > 0 {
        guard -= 1;
        let n = order.len();
        let ear = (0..n).find(|&i| {
            let (a, b, c) = (
                points[order[(i + n - 1) % n]],
                points[order[i]],
                points[order[(i + 1) % n]],
            );
            cross(a, b, c) > 1e-6
                && !order.iter().any(|&other| {
                    let p = points[other];
                    p != a && p != b && p != c && inside(p, a, b, c)
                })
        });
        match ear {
            Some(i) => {
                triangles.push([
                    points[order[(i + n - 1) % n]],
                    points[order[i]],
                    points[order[(i + 1) % n]],
                ]);
                order.remove(i);
            }
            None => break,
        }
    }
    if let [first, rest @ ..] = order.as_slice() {
        for pair in rest.windows(2) {
            triangles.push([points[*first], points[pair[0]], points[pair[1]]]);
        }
    }
    triangles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_subset_the_icons_use() {
        let svg = Svg::parse(
            r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                 <path d="M4 4 H20 v16 h-16 Z"/>
                 <circle cx="12" cy="12" r="5"/>
                 <rect x="1" y="1" width="6" height="6" rx="2" fill="currentColor"/>
                 <line x1="0" y1="0" x2="5" y2="5"/>
                 <polygon points="0,0 4,0 4,4" fill="currentColor"/>
               </svg>"#,
        )
        .expect("parses");
        assert_eq!(svg.shapes.len(), 5);
        assert!(svg.shapes[0].closed && svg.shapes[0].points.len() == 4);
        assert!(!svg.shapes[2].triangles.is_empty());
        assert!(svg.shapes[0].triangles.is_empty());
    }

    #[test]
    fn arcs_and_curves_stay_near_their_ends() {
        let outlines = path_outlines("M3 12 A9 9 0 0 1 21 12 C21 16 17 20 12 20 Q8 20 3 12");
        let (points, _, closed) = &outlines[0];
        assert!(!closed);
        // The half circle passes through the top of its circle.
        assert!(points.iter().any(|p| (p.1 - 3.0).abs() < 0.2));
        assert!(
            points
                .iter()
                .all(|p| p.0 > -0.5 && p.0 < 24.5 && p.1 > -0.5 && p.1 < 24.5)
        );
        assert_eq!(
            points.last().map(|p| (p.0.round(), p.1.round())),
            Some((3.0, 12.0))
        );
    }

    #[test]
    fn numbers_may_run_together() {
        let outlines = path_outlines("M1.5.5-2-3l.5.5");
        assert_eq!(outlines[0].0, vec![(1.5, 0.5), (-2.0, -3.0), (-1.5, -2.5)]);
    }

    #[test]
    fn a_concave_polygon_is_filled_without_covering_its_notch() {
        // An arrow head: concave at (2, 2).
        let triangles = triangulate(&[(0.0, 0.0), (4.0, 0.0), (2.0, 2.0), (4.0, 4.0), (0.0, 4.0)]);
        assert_eq!(triangles.len(), 3);
        let area: f32 = triangles
            .iter()
            .map(|t| {
                ((t[1].0 - t[0].0) * (t[2].1 - t[0].1) - (t[1].1 - t[0].1) * (t[2].0 - t[0].0))
                    .abs()
                    / 2.0
            })
            .sum();
        assert!((area - 12.0).abs() < 1e-3, "{area}");
    }

    #[test]
    fn shipped_icons_stay_inside_the_view_box() {
        for icon in crate::kit::Icon::ALL {
            let svg = Svg::parse(icon.source()).expect("parses");
            for shape in &svg.shapes {
                for (x, y) in &shape.points {
                    assert!(
                        (-0.5..=VIEW + 0.5).contains(x) && (-0.5..=VIEW + 0.5).contains(y),
                        "{icon:?} draws outside its square at {x},{y}"
                    );
                }
            }
        }
    }

    #[test]
    fn hostile_files_never_panic() {
        for source in [
            "",
            "<",
            "<svg",
            "<path d=\"",
            "<path d=\"M\"/>",
            "<path d=\"M1 1 A\"/>",
            "<path d=\"M1 1 A1 1 0 5 5 3 3\"/>",
            "<path d=\"M1e999 1 L2 2\"/>",
            "<path d=\"M1 1 L NaN 2\"/>",
            "<circle cx=\"x\" cy=\"1\" r=\"1\"/>",
            "<rect width=\"-1\" height=\"1\"/>",
            "<path d=\"M0 0 A0 0 0 0 0 5 5 Z Z Z\"/>",
        ] {
            let _ = Svg::parse(source);
        }
        let huge = format!("<path d=\"M0 0 {}\"/>", "L1 1 ".repeat(100_000));
        assert!(Svg::parse(&huge).is_none());
    }
}
