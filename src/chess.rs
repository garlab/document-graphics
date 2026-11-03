//! Bounded PGN collections and standard-chess FEN positions.
//! Parsing, legal move replay and rendering perform no file or network access.
use cozy_chess::Board;
pub use cozy_chess::{Color, Piece, Square};
use std::{
    collections::BTreeMap,
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
mod parser;
mod position;
mod render;
pub use position::Position;
pub use render::{RenderOptions, rasterize, to_svg};

pub const MAX_SOURCE: usize = 8 * 1024 * 1024;
pub const MAX_NODES: usize = 100_000;
pub const MAX_VALUE: usize = 64 * 1024;
pub const MAX_DEPTH: usize = 128;
pub const MAX_PLIES: u32 = 16_384;
pub type NodeId = usize;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pgn,
    Fen,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    Invalid {
        offset: usize,
        message: &'static str,
    },
    Unsupported(&'static str),
    Limit(&'static str),
    Cancelled,
    Render(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid { offset, message } => write!(f, "{message} at byte {}", offset + 1),
            Self::Unsupported(s) | Self::Limit(s) => f.write_str(s),
            Self::Cancelled => f.write_str("Chess preview cancelled"),
            Self::Render(s) => f.write_str(s),
        }
    }
}
impl std::error::Error for Error {}
#[derive(Debug)]
pub struct Node {
    pub parent: Option<NodeId>,
    pub first_child: Option<NodeId>,
    pub next_sibling: Option<NodeId>,
    pub source: Range<usize>,
    pub game: usize,
    /// Half-moves since this game's starting position.
    pub ply: u32,
    san: Range<usize>,
    comments: Vec<Range<usize>>,
    annotations: Vec<Range<usize>>,
}
#[derive(Debug)]
pub struct Game {
    pub root: NodeId,
    pub tags: BTreeMap<String, String>,
    start: Board,
}
/// Source spans and tree links; no board or raster is retained per move.
#[derive(Debug)]
pub struct Collection {
    source: Arc<[u8]>,
    nodes: Vec<Node>,
    games: Vec<Game>,
}
impl Collection {
    pub fn parse(source: Arc<[u8]>, format: Format, cancel: &AtomicBool) -> Result<Self, Error> {
        parser::parse(source, format, cancel)
    }
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }
    pub fn games(&self) -> &[Game] {
        &self.games
    }
    pub fn source(&self) -> &[u8] {
        &self.source
    }
    pub fn node_for_offset(&self, offset: usize) -> NodeId {
        self.nodes
            .partition_point(|n| n.source.start <= offset)
            .saturating_sub(1)
    }
    pub fn san(&self, node: NodeId) -> Option<&str> {
        self.nodes
            .get(node)
            .filter(|n| n.parent.is_some())
            .and_then(|n| std::str::from_utf8(&self.source[n.san.clone()]).ok())
    }
    pub fn comment(&self, node: NodeId) -> String {
        self.nodes.get(node).map_or_else(String::new, |n| {
            n.comments
                .iter()
                .map(|r| decode(&self.source[r.clone()]))
                .collect::<Vec<_>>()
                .join("\n\n")
        })
    }
    pub fn annotations(&self, node: NodeId) -> String {
        self.nodes.get(node).map_or_else(String::new, |n| {
            n.annotations
                .iter()
                .map(|r| decode(&self.source[r.clone()]))
                .collect::<Vec<_>>()
                .join(" ")
        })
    }
    pub fn has_comment(&self, node: NodeId) -> bool {
        self.nodes
            .get(node)
            .is_some_and(|n| !n.comments.is_empty() || !n.annotations.is_empty())
    }
    pub fn move_number(&self, node: NodeId) -> u32 {
        let n = &self.nodes[node];
        let start = &self.games[n.game].start;
        u32::from(start.fullmove_number())
            + (n.ply.saturating_sub(1) + u32::from(start.side_to_move() == Color::Black)) / 2
    }
    pub fn move_color(&self, node: NodeId) -> Option<Color> {
        let n = self.nodes.get(node)?;
        n.parent?;
        let start = self.games[n.game].start.side_to_move();
        Some(if n.ply % 2 == 1 { start } else { !start })
    }
    pub fn title(&self, game: usize) -> String {
        let Some(g) = self.games.get(game) else {
            return "Chess game".into();
        };
        match (g.tags.get("White"), g.tags.get("Black")) {
            (Some(w), Some(b)) => format!("{w} · {b}"),
            _ => g
                .tags
                .get("Event")
                .cloned()
                .unwrap_or_else(|| "Chess position".into()),
        }
    }
    pub fn position(&self, node: NodeId, cancel: &AtomicBool) -> Result<Position, Error> {
        position::at(self, node, cancel)
    }
}
fn check(cancel: &AtomicBool) -> Result<(), Error> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
fn invalid(offset: usize, message: &'static str) -> Error {
    Error::Invalid { offset, message }
}
fn decode(bytes: &[u8]) -> String {
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .unwrap_or_else(|_| bytes.iter().copied().map(char::from).collect())
}
#[cfg(test)]
mod tests;
