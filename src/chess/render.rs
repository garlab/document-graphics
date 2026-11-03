use super::*;
use crate::{Raster, RasterOptions};
use std::fmt::Write;
#[derive(Debug, Clone, Copy, Default)]
pub struct RenderOptions {
    pub dark: bool,
    pub flipped: bool,
}
/// Render an 8×8 board with original vector piece silhouettes. No system fonts,
/// external images or user-supplied SVG are used for pieces.
pub fn to_svg(
    position: &Position,
    options: RenderOptions,
    cancel: &AtomicBool,
) -> Result<String, Error> {
    check(cancel)?;
    let (light, dark, frame, ink) = if options.dark {
        ("#bfc9b4", "#607b62", "#292e2a", "#d9dfd5")
    } else {
        ("#eee8d5", "#829777", "#e3dfd2", "#39463b")
    };
    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="440" height="440" viewBox="0 0 440 440"><rect width="440" height="440" rx="7" fill="{frame}"/>"#
    );
    for row in 0..8 {
        check(cancel)?;
        for col in 0..8 {
            let (file, rank) = if options.flipped {
                (7 - col, row)
            } else {
                (col, 7 - row)
            };
            let square = Square::new(cozy_chess::File::index(file), cozy_chess::Rank::index(rank));
            let x = 28 + col * 48;
            let y = 28 + row * 48;
            let fill = if (file + rank) % 2 == 0 { dark } else { light };
            let _ = write!(
                svg,
                r#"<rect x="{x}" y="{y}" width="48" height="48" fill="{fill}"/>"#
            );
            if position
                .last_move
                .is_some_and(|(from, to)| from == square || to == square)
            {
                let _ = write!(
                    svg,
                    r##"<rect x="{x}" y="{y}" width="48" height="48" fill="#e1c754" fill-opacity=".55"/>"##
                );
            }
            if let Some((color, piece)) = position.piece(square) {
                let (body, edge, detail) = if color == Color::White {
                    ("#faf8ee", "#30382f", "#596451")
                } else {
                    ("#252e28", "#e1e5d9", "#b8c4ad")
                };
                let _ = write!(
                    svg,
                    r#"<g transform="translate({x} {y})" stroke-linejoin="round" stroke-linecap="round" stroke-width="1.5" fill="{body}" stroke="{edge}">"#
                );
                // Compact, original silhouettes shared by both appearances and orientations.
                match piece {
                    Piece::Pawn => svg.push_str(r#"<path d="M14 38h20l-2-5c-5-2-6-7-4-12a7 7 0 1 0-8 0c2 5 1 10-4 12z"/>"#),
                    Piece::Rook => svg.push_str(r#"<path d="M12 8h6v5h4V8h4v5h4V8h6v12l-5 4v9l4 5H13l4-5v-9l-5-4z"/>"#),
                    Piece::Knight => {
                        svg.push_str(r#"<path d="M13 38h23l-2-9c0-10-3-17-11-21l-5-3 1 7-9 11 4 5 9-5c0 6-6 7-7 10z"/>"#);
                        let _ = write!(svg, r#"<circle cx="22" cy="16" r="1.5" stroke="none" fill="{detail}"/><path d="M13 24l4 1m11-10c4 9 2 14-2 18" fill="none" stroke="{detail}"/>"#);
                    }
                    Piece::Bishop => {
                        svg.push_str(r#"<path d="M13 38h22l-4-6-3-7c8-8 4-13-4-19-8 6-12 11-4 19l-3 7z"/>"#);
                        let _ = write!(svg, r#"<path d="M26 11l-6 8" fill="none" stroke="{detail}"/>"#);
                    }
                    Piece::Queen => svg.push_str(r#"<path d="M13 38h22l-3-7 5-16-9 7-4-12-4 12-9-7 5 16z"/><circle cx="10" cy="12" r="3"/><circle cx="24" cy="7" r="3"/><circle cx="38" cy="12" r="3"/>"#),
                    Piece::King => svg.push_str(r#"<path d="M13 38h22l-3-9c9-11 2-17-8-9-10-8-17-2-8 9z"/><path d="M24 6v12m-5-7h10" fill="none" stroke-width="3"/>"#),
                }
                let _ = write!(
                    svg,
                    r#"<path d="M16 33h16" stroke="{detail}" fill="none"/></g>"#
                );
            }
        }
        let rank = if options.flipped { row + 1 } else { 8 - row };
        let file = char::from(b'a' + if options.flipped { 7 - row } else { row } as u8);
        let at = 52 + row * 48;
        for (x, y, label) in [
            (14, at + 4, rank.to_string()),
            (426, at + 4, rank.to_string()),
            (at, 19, file.to_string()),
            (at, 430, file.to_string()),
        ] {
            let _ = write!(
                svg,
                r#"<text x="{x}" y="{y}" font-family="KaTeX_Main" font-size="13" text-anchor="middle" fill="{ink}">{label}</text>"#
            );
        }
    }
    svg.push_str("</svg>");
    Ok(svg)
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
