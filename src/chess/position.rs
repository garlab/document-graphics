use super::*;
use cozy_chess::{File, Move, Rank};

#[derive(Clone, Debug)]
pub struct Position {
    node: NodeId,
    source: Arc<[u8]>,
    board: Board,
    /// Actual source/destination squares (castling points at the king's destination).
    pub last_move: Option<(Square, Square)>,
}
impl Position {
    pub fn node(&self) -> NodeId {
        self.node
    }
    pub fn piece(&self, square: Square) -> Option<(Color, Piece)> {
        Some((self.board.color_on(square)?, self.board.piece_on(square)?))
    }
    pub fn side_to_move(&self) -> Color {
        self.board.side_to_move()
    }
    pub fn fullmove_number(&self) -> u16 {
        self.board.fullmove_number()
    }
    pub fn in_check(&self) -> bool {
        !self.board.checkers().is_empty()
    }
    pub fn fen(&self) -> String {
        self.board.to_string()
    }
    pub fn advance(
        &self,
        doc: &Collection,
        node: NodeId,
        cancel: &AtomicBool,
    ) -> Result<Self, Error> {
        check(cancel)?;
        if !Arc::ptr_eq(&self.source, &doc.source)
            || doc.nodes.get(node).and_then(|n| n.parent) != Some(self.node)
        {
            return Err(invalid(
                0,
                "Position can only advance to a child in the same collection",
            ));
        }
        let mut result = self.clone();
        result.apply(doc, node)?;
        check(cancel)?;
        Ok(result)
    }
    fn apply(&mut self, doc: &Collection, node: NodeId) -> Result<(), Error> {
        let offset = doc.nodes[node].san.start;
        let san = doc
            .san(node)
            .ok_or_else(|| invalid(offset, "Invalid SAN move"))?;
        let mv = san_move(&self.board, san)
            .ok_or_else(|| invalid(offset, "Illegal or ambiguous chess move"))?;
        let castle = self.board.color_on(mv.to) == Some(self.board.side_to_move());
        self.last_move = Some((
            mv.from,
            if castle {
                Square::new(
                    if mv.to.file() > mv.from.file() {
                        File::G
                    } else {
                        File::C
                    },
                    mv.from.rank(),
                )
            } else {
                mv.to
            },
        ));
        self.board
            .try_play(mv)
            .map_err(|_| invalid(offset, "Illegal chess move"))?;
        self.node = node;
        Ok(())
    }
}
pub(super) fn at(doc: &Collection, node: NodeId, cancel: &AtomicBool) -> Result<Position, Error> {
    check(cancel)?;
    let entry = doc
        .nodes
        .get(node)
        .ok_or_else(|| invalid(0, "Unknown chess node"))?;
    let game = &doc.games[entry.game];
    let mut position = Position {
        node: game.root,
        source: doc.source.clone(),
        board: game.start.clone(),
        last_move: None,
    };
    let mut ancestors = Vec::new();
    let mut id = node;
    while let Some(parent) = doc.nodes[id].parent {
        check(cancel)?;
        ancestors.push(id);
        id = parent;
    }
    for id in ancestors.into_iter().rev() {
        check(cancel)?;
        position.apply(doc, id)?;
    }
    Ok(position)
}

/// Resolve SAN against legal moves, including pinned-piece disambiguation,
/// promotions, en passant and castling. At most one standard board's moves are visited.
fn san_move(board: &Board, san: &str) -> Option<Move> {
    if !san.is_ascii() || san.len() > 32 {
        return None;
    }
    let san = san.trim_end_matches(['+', '#', '!', '?']);
    let castle = match san {
        "O-O" | "0-0" => Some(true),
        "O-O-O" | "0-0-0" => Some(false),
        _ => None,
    };
    let mut s = san;
    let piece = if castle.is_some() {
        Piece::King
    } else {
        match s.as_bytes().first()? {
            b'K' => {
                s = &s[1..];
                Piece::King
            }
            b'Q' => {
                s = &s[1..];
                Piece::Queen
            }
            b'R' => {
                s = &s[1..];
                Piece::Rook
            }
            b'B' => {
                s = &s[1..];
                Piece::Bishop
            }
            b'N' => {
                s = &s[1..];
                Piece::Knight
            }
            _ => Piece::Pawn,
        }
    };
    let mut promotion = None;
    let mut destination = None;
    let mut file = None;
    let mut rank = None;
    let mut capture = false;
    if castle.is_none() {
        if s.len() >= 2 && s.as_bytes()[s.len() - 2] == b'=' {
            promotion = Some(match s.as_bytes()[s.len() - 1] {
                b'Q' => Piece::Queen,
                b'R' => Piece::Rook,
                b'B' => Piece::Bishop,
                b'N' => Piece::Knight,
                _ => return None,
            });
            s = &s[..s.len() - 2];
        }
        if s.len() < 2 {
            return None;
        }
        destination = Some(s[s.len() - 2..].parse::<Square>().ok()?);
        s = &s[..s.len() - 2];
        if let Some(prefix) = s.strip_suffix('x') {
            capture = true;
            s = prefix;
        }
        if let Some(b'a'..=b'h') = s.as_bytes().first() {
            file = Some(s[..1].parse::<File>().ok()?);
            s = &s[1..];
        }
        if let Some(b'1'..=b'8') = s.as_bytes().first() {
            rank = Some(s[..1].parse::<Rank>().ok()?);
            s = &s[1..];
        }
        if !s.is_empty() || (piece == Piece::Pawn && (rank.is_some() || capture != file.is_some()))
        {
            return None;
        }
    }
    let mut selected = None;
    let mut ambiguous = false;
    board.generate_moves(|moves| {
        for mv in moves {
            if board.piece_on(mv.from) != Some(piece) {
                continue;
            }
            let is_castle = board.color_on(mv.to) == Some(board.side_to_move());
            if let Some(short) = castle {
                if !is_castle || (mv.to.file() > mv.from.file()) != short {
                    continue;
                }
            } else {
                if is_castle || Some(mv.to) != destination || mv.promotion != promotion {
                    continue;
                }
                if file.is_some_and(|f| mv.from.file() != f)
                    || rank.is_some_and(|r| mv.from.rank() != r)
                {
                    continue;
                }
                let is_capture = board.color_on(mv.to) == Some(!board.side_to_move())
                    || (piece == Piece::Pawn && mv.from.file() != mv.to.file());
                if capture != is_capture {
                    continue;
                }
            }
            if selected.is_some() {
                ambiguous = true;
                return true;
            }
            selected = Some(mv);
        }
        false
    });
    if ambiguous { None } else { selected }
}
