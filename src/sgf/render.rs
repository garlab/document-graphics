use super::*;
use crate::{Raster, RasterOptions};
use std::fmt::Write;

#[derive(Debug, Clone, Copy, Default)]
pub struct RenderOptions {
    pub dark: bool,
    pub move_numbers: bool,
}

/// Produce a self-contained board diagram. User text is escaped as XML text.
pub fn to_svg(
    position: &Position,
    options: RenderOptions,
    cancel: &AtomicBool,
) -> Result<String, Error> {
    check(cancel)?;
    position.validate()?;
    let w = (position.width as usize - 1) * 24 + 72;
    let h = (position.height as usize - 1) * 24 + 72;
    let wood = if options.dark { "#bd9864" } else { "#e7c58a" };
    let mut s = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}"><rect width="{w}" height="{h}" rx="6" fill="{wood}"/><defs><radialGradient id="b" cx=".3" cy=".25" r=".8"><stop stop-color="#494949"/><stop offset="1" stop-color="#111111"/></radialGradient><radialGradient id="w" cx=".3" cy=".25" r=".8"><stop stop-color="#ffffff"/><stop offset="1" stop-color="#dedbd5"/></radialGradient></defs><g stroke="#624c2e" stroke-width=".7">"##
    );
    for x in 0..position.width {
        let x = xy(x);
        let _ = write!(s, r#"<path d="M{x} 36V{}"/>"#, h - 36);
    }
    for y in 0..position.height {
        let y = xy(y);
        let _ = write!(s, r#"<path d="M36 {y}H{}"/>"#, w - 36);
    }
    s.push_str("</g>");
    if position.width == position.height && position.width >= 9 && position.width % 2 == 1 {
        let n = position.width;
        let edge = if n >= 13 { 3 } else { 2 };
        for x in [edge, n / 2, n - edge - 1] {
            for y in [edge, n / 2, n - edge - 1] {
                if n < 19 && (x == n / 2) != (y == n / 2) {
                    continue;
                }
                let _ = write!(
                    s,
                    r##"<circle cx="{}" cy="{}" r="2" fill="#624c2e"/>"##,
                    xy(x),
                    xy(y)
                );
            }
        }
    }
    for x in 0..position.width {
        let label: String = Point { x, y: 0 }
            .coordinate(position.height)
            .chars()
            .take_while(char::is_ascii_alphabetic)
            .collect();
        text(&mut s, xy(x) as f32, 17., &label, "#624c2e", 10.);
        text(
            &mut s,
            xy(x) as f32,
            (h - 12) as f32,
            &label,
            "#624c2e",
            10.,
        );
    }
    for y in 0..position.height {
        let label = (position.height - y).to_string();
        text(&mut s, 16., xy(y) as f32 + 3., &label, "#624c2e", 10.);
        text(
            &mut s,
            (w - 16) as f32,
            xy(y) as f32 + 3.,
            &label,
            "#624c2e",
            10.,
        );
        check(cancel)?;
        for x in 0..position.width {
            let at = Point { x, y };
            if let Some(color) = position.stone(at) {
                let (fill, stroke) = if color == Color::Black {
                    ("url(#b)", "#171717")
                } else {
                    ("url(#w)", "#aaa69e")
                };
                let _ = write!(
                    s,
                    r#"<circle cx="{}" cy="{}" r="10.7" fill="{fill}" stroke="{stroke}" stroke-width=".7"/>"#,
                    xy(x),
                    xy(y)
                );
                if options.move_numbers && position.number(at) != 0 {
                    let label = position.number(at).to_string();
                    text(
                        &mut s,
                        xy(x) as f32,
                        xy(y) as f32 + 3.5,
                        &label,
                        ink(position, at),
                        (17. / label.len() as f32).min(11.),
                    );
                }
            }
        }
    }
    for mark in &position.marks {
        check(cancel)?;
        let x = xy(mark.at.x) as f32;
        let y = xy(mark.at.y) as f32;
        let ink = ink(position, mark.at);
        match &mark.kind {
            MarkKind::Label(label) => {
                if position.stone(mark.at).is_none() {
                    let _ = write!(
                        s,
                        r#"<rect x="{}" y="{}" width="22" height="20" fill="{wood}"/>"#,
                        x - 11.,
                        y - 10.
                    );
                }
                text(
                    &mut s,
                    x,
                    y + 4.,
                    label,
                    ink,
                    (22. / label.chars().count().max(1) as f32).min(12.),
                );
            }
            MarkKind::Circle => {
                let _ = write!(
                    s,
                    r#"<circle cx="{x}" cy="{y}" r="5.8" fill="none" stroke="{ink}" stroke-width="1.5"/>"#
                );
            }
            MarkKind::Square => {
                let _ = write!(
                    s,
                    r#"<rect x="{}" y="{}" width="10" height="10" fill="none" stroke="{ink}" stroke-width="1.5"/>"#,
                    x - 5.,
                    y - 5.
                );
            }
            MarkKind::Triangle => {
                let _ = write!(
                    s,
                    r#"<path d="M{x} {}L{} {}H{}Z" fill="none" stroke="{ink}" stroke-width="1.5"/>"#,
                    y - 6.,
                    x + 6.,
                    y + 5.,
                    x - 6.
                );
            }
            MarkKind::Cross => {
                let _ = write!(
                    s,
                    r#"<path d="M{} {}l10 10m0 -10l-10 10" fill="none" stroke="{ink}" stroke-width="1.5"/>"#,
                    x - 5.,
                    y - 5.
                );
            }
            MarkKind::Selected => {
                let _ = write!(
                    s,
                    r##"<rect x="{}" y="{}" width="21" height="21" fill="#b32e30" fill-opacity=".4"/>"##,
                    x - 10.5,
                    y - 10.5
                );
            }
            MarkKind::Territory(color) => {
                let color = if *color == Color::Black {
                    "#111"
                } else {
                    "#fff"
                };
                let _ = write!(
                    s,
                    r#"<rect x="{}" y="{}" width="6" height="6" fill="{color}" stroke="{ink}" stroke-width=".6"/>"#,
                    x - 3.,
                    y - 3.
                );
            }
            MarkKind::Dim => {
                let _ = write!(
                    s,
                    r#"<rect x="{}" y="{}" width="24" height="24" fill="{wood}" fill-opacity=".6"/>"#,
                    x - 12.,
                    y - 12.
                );
            }
        }
    }
    for line in &position.lines {
        let (x, y, tx, ty) = (
            xy(line.from.x) as f32,
            xy(line.from.y) as f32,
            xy(line.to.x) as f32,
            xy(line.to.y) as f32,
        );
        let _ = write!(
            s,
            r##"<path d="M{x} {y}L{tx} {ty}" stroke="#ba3232" stroke-width="2"/>"##
        );
        let distance = ((tx - x).powi(2) + (ty - y).powi(2)).sqrt();
        if line.arrow && distance > 0. {
            let (dx, dy) = ((tx - x) / distance, (ty - y) / distance);
            let _ = write!(
                s,
                r##"<path d="M{tx} {ty}L{:.2} {:.2}L{:.2} {:.2}Z" fill="#ba3232"/>"##,
                tx - dx * 9. + dy * 4.,
                ty - dy * 9. - dx * 4.,
                tx - dx * 9. - dy * 4.,
                ty - dy * 9. + dx * 4.
            );
        }
    }
    if !options.move_numbers
        && let Some(Played {
            point: Some(at), ..
        }) = position.last_move
        && position.stone(at).is_some()
        && !position.marks.iter().any(|m| m.at == at)
    {
        let _ = write!(
            s,
            r##"<circle cx="{}" cy="{}" r="3.5" fill="none" stroke="#de4c43" stroke-width="2"/>"##,
            xy(at.x),
            xy(at.y)
        );
    }
    s.push_str("</svg>");
    if s.len() > crate::svg::MAX_SOURCE {
        return Err(Error::Limit("SGF diagram exceeds its SVG size limit"));
    }
    Ok(s)
}
pub fn rasterize(
    position: &Position,
    options: RenderOptions,
    fitting: RasterOptions,
    cancel: &AtomicBool,
) -> Result<Raster, Error> {
    let svg = to_svg(position, options, cancel)?;
    let raster = crate::svg::rasterize(&svg, fitting).map_err(|e| Error::Render(e.to_string()))?;
    check(cancel)?;
    Ok(raster)
}
fn xy(n: u8) -> usize {
    36 + n as usize * 24
}
fn ink(position: &Position, at: Point) -> &'static str {
    if position.stone(at) == Some(Color::Black) {
        "#fff"
    } else {
        "#1b1b1b"
    }
}
fn text(out: &mut String, x: f32, y: f32, value: &str, color: &str, size: f32) {
    let _ = write!(
        out,
        r#"<text x="{x}" y="{y}" text-anchor="middle" font-family="KaTeX_Main" font-size="{size:.2}" fill="{color}">"#
    );
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out.push_str("</text>");
}
