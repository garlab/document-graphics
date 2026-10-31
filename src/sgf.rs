//! Bounded SGF collections and Go positions. No file access or application cache.
//!
//! FF[4] Go (GM[1]), including variations, setup stones, captures and annotations.
//! SGF records describe positions even when moves violate competitive rules;
//! replay deliberately does not enforce turn order, ko, or move legality.
use std::{
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

mod parser;
mod position;
mod render;
pub use position::{Color, Line, Mark, MarkKind, Played, Point, Position};
pub use render::{RenderOptions, rasterize, to_svg};

pub const MAX_SOURCE: usize = 8 * 1024 * 1024;
pub const MAX_NODES: usize = 100_000;
pub const MAX_VALUE: usize = 128 * 1024;
pub type NodeId = usize;

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
            Self::Cancelled => f.write_str("SGF preview cancelled"),
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
    properties: Range<usize>,
}
#[derive(Debug)]
pub struct Game {
    pub root: NodeId,
    pub width: u8,
    pub height: u8,
    encoding: Encoding,
}
#[derive(Debug, Clone, Copy)]
enum Encoding {
    Utf8,
    Latin1,
}
#[derive(Debug)]
struct Property {
    name: Range<usize>,
    values: Range<usize>,
}

/// Compact source ranges retain unknown properties without copying their values.
/// Node IDs are local to this collection. No position or image is stored per node.
#[derive(Debug)]
pub struct Collection {
    source: Arc<[u8]>,
    nodes: Vec<Node>,
    properties: Vec<Property>,
    values: Vec<Range<usize>>,
    games: Vec<Game>,
}
impl Collection {
    pub fn parse(source: Arc<[u8]>, cancel: &AtomicBool) -> Result<Self, Error> {
        parser::parse(source, cancel)
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
    pub fn text(&self, node: NodeId, property: &str) -> Option<String> {
        self.values(node, property).next()
    }
    pub fn children(&self, node: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(self.nodes.get(node).and_then(|n| n.first_child), |&id| {
            self.nodes[id].next_sibling
        })
    }
    pub fn title(&self, game: usize) -> String {
        let Some(game) = self.games.get(game) else {
            return "Go game".into();
        };
        if let Some(title) = self.text(game.root, "GN").filter(|s| !s.is_empty()) {
            return title;
        }
        let player = |name| self.text(game.root, name).filter(|s| !s.is_empty());
        match (player("PB"), player("PW")) {
            (Some(b), Some(w)) => format!("{b} · {w}"),
            _ => "Go game".into(),
        }
    }
    pub fn position(&self, node: NodeId, cancel: &AtomicBool) -> Result<Position, Error> {
        position::at(self, node, cancel)
    }
    fn ranges(&self, node: NodeId, name: &str) -> &[Range<usize>] {
        self.nodes
            .get(node)
            .and_then(|n| {
                self.properties[n.properties.clone()]
                    .iter()
                    .find(|p| &self.source[p.name.clone()] == name.as_bytes())
            })
            .map_or(&[], |p| &self.values[p.values.clone()])
    }
    fn values<'a>(&'a self, node: NodeId, name: &str) -> impl Iterator<Item = String> + 'a {
        let text = matches!(name, "C" | "GC");
        let encoding = self
            .nodes
            .get(node)
            .map_or(Encoding::Utf8, |n| self.games[n.game].encoding);
        self.ranges(node, name)
            .iter()
            .map(move |range| decode(&self.source[range.clone()], encoding, text))
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

fn decode(bytes: &[u8], encoding: Encoding, text: bool) -> String {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = bytes[i] == b'\\';
        if escaped {
            i += 1;
            if i == bytes.len() {
                break;
            }
        }
        let c = bytes[i];
        i += 1;
        if matches!(c, b'\r' | b'\n') {
            if i < bytes.len() && matches!(bytes[i], b'\r' | b'\n') && bytes[i] != c {
                i += 1;
            }
            if !escaped {
                out.push(if text { b'\n' } else { b' ' });
            }
        } else if c.is_ascii_whitespace() {
            out.push(b' ');
        } else {
            out.push(c);
        }
    }
    match encoding {
        Encoding::Utf8 => String::from_utf8_lossy(&out).into_owned(),
        Encoding::Latin1 => out.into_iter().map(char::from).collect(),
    }
}

#[cfg(test)]
mod tests;
