use super::*;
const MAX_GAMES: usize = 1024;
const MAX_ANNOTATIONS: usize = 300_000;

pub(super) fn parse(
    source: Arc<[u8]>,
    format: Format,
    cancel: &AtomicBool,
) -> Result<Collection, Error> {
    check(cancel)?;
    if source.len() > MAX_SOURCE {
        return Err(Error::Limit("Chess source exceeds 8 MiB"));
    }
    let mut p = Parser {
        doc: Collection {
            source,
            nodes: Vec::new(),
            games: Vec::new(),
        },
        at: 0,
        current: None,
        resumes: Vec::new(),
        tails: Vec::new(),
        annotations: 0,
    };
    if p.doc.source.starts_with(&[0xef, 0xbb, 0xbf]) {
        p.at = 3;
    }
    if format == Format::Fen {
        while p.at < p.doc.source.len() {
            check(cancel)?;
            p.space();
            if p.at == p.doc.source.len() {
                break;
            }
            let start = p.at;
            while p.at < p.doc.source.len() && !matches!(p.doc.source[p.at], b'\r' | b'\n') {
                p.at += 1;
            }
            let fen = std::str::from_utf8(&p.doc.source[start..p.at])
                .map_err(|_| invalid(start, "Invalid FEN"))?;
            let board = Board::from_fen(fen.trim(), false)
                .map_err(|_| invalid(start, "Invalid standard-chess FEN"))?;
            let root = p.new_game(start)?;
            let game = p.doc.games.last_mut().unwrap();
            game.start = board;
            p.doc.nodes[root].source.end = p.at;
        }
    } else {
        while p.at < p.doc.source.len() {
            check(cancel)?;
            p.space();
            let Some(&byte) = p.doc.source.get(p.at) else {
                break;
            };
            let start = p.at;
            match byte {
                b'%' if start == 0 || matches!(p.doc.source[start - 1], b'\n' | b'\r') => {
                    p.line();
                }
                b'[' => p.tag()?,
                b'{' | b';' => {
                    let node = p.ensure_game(start)?;
                    p.at += 1;
                    let begin = p.at;
                    if byte == b'{' {
                        while p.at < p.doc.source.len() && p.doc.source[p.at] != b'}' {
                            p.at += 1;
                            if p.at - begin > MAX_VALUE {
                                return Err(Error::Limit("PGN comment exceeds 64 KiB"));
                            }
                        }
                        if p.at == p.doc.source.len() {
                            return Err(invalid(start, "Unclosed PGN comment"));
                        }
                    } else {
                        p.line();
                    }
                    p.annotation(node, begin..p.at, true)?;
                    if byte == b'{' {
                        p.at += 1;
                    }
                    p.doc.nodes[node].source.end = p.at;
                }
                b'(' => {
                    let node = p
                        .current
                        .ok_or_else(|| invalid(start, "Variation has no preceding move"))?;
                    let parent = p.doc.nodes[node]
                        .parent
                        .ok_or_else(|| invalid(start, "Variation has no preceding move"))?;
                    if p.resumes.len() == MAX_DEPTH {
                        return Err(Error::Limit("PGN variation nesting exceeds 128"));
                    }
                    p.resumes.push((node, p.doc.nodes.len()));
                    p.current = Some(parent);
                    p.at += 1;
                }
                b')' => {
                    let (node, count) = p
                        .resumes
                        .pop()
                        .ok_or_else(|| invalid(start, "Unexpected variation end"))?;
                    if count == p.doc.nodes.len() {
                        return Err(invalid(start, "Empty PGN variation"));
                    }
                    p.current = Some(node);
                    p.at += 1;
                }
                b'$' | b'!' | b'?' => {
                    let node = p
                        .current
                        .ok_or_else(|| invalid(start, "Annotation has no move"))?;
                    p.at += 1;
                    while p.doc.source.get(p.at).is_some_and(|b| {
                        if byte == b'$' {
                            b.is_ascii_digit()
                        } else {
                            matches!(b, b'!' | b'?')
                        }
                    }) {
                        p.at += 1;
                    }
                    if byte == b'$' && (p.at == start + 1 || p.at - start > 4) {
                        return Err(invalid(start, "Invalid numeric annotation"));
                    }
                    p.annotation(node, start..p.at, false)?;
                }
                b']' | b'}' | b'<' | b'>' => {
                    return Err(invalid(start, "Unexpected PGN delimiter"));
                }
                _ => {
                    // Move numbers may touch SAN: 1.e4 or 1...e5.
                    let mut number_end = start;
                    while p.doc.source.get(number_end).is_some_and(u8::is_ascii_digit) {
                        number_end += 1;
                    }
                    if p.doc.source.get(number_end) == Some(&b'.') {
                        p.at = number_end;
                        while p.doc.source.get(p.at) == Some(&b'.') {
                            p.at += 1;
                        }
                        continue;
                    }
                    while p
                        .doc
                        .source
                        .get(p.at)
                        .is_some_and(|b| !b.is_ascii_whitespace() && !b"[]{}();$!?".contains(b))
                    {
                        p.at += 1;
                        if p.at - start > 32 {
                            return Err(invalid(start, "PGN move token exceeds 32 bytes"));
                        }
                    }
                    let token = &p.doc.source[start..p.at];
                    if matches!(token, b"1-0" | b"0-1" | b"1/2-1/2" | b"*") {
                        let result = decode(token);
                        let node = p.ensure_game(start)?;
                        p.finish_start()?;
                        if p.resumes.is_empty() {
                            p.doc.games[p.doc.nodes[node].game]
                                .tags
                                .entry("Result".into())
                                .or_insert(result);
                            p.current = None;
                        }
                    } else if token == b"e.p." {
                        // Optional SAN en-passant suffix; legality is checked by replay.
                    } else {
                        let parent = p.ensure_game(start)?;
                        p.finish_start()?;
                        if p.doc.nodes[parent].ply == MAX_PLIES {
                            return Err(Error::Limit("PGN line exceeds 16,384 half-moves"));
                        }
                        let id = p.add_node(Some(parent), start..p.at, p.doc.nodes[parent].game)?;
                        p.doc.nodes[id].san = start..p.at;
                        p.current = Some(id);
                    }
                }
            }
        }
        if !p.resumes.is_empty() {
            return Err(invalid(p.at, "Unclosed PGN variation"));
        }
        p.finish_start()?;
    }
    if p.doc.games.is_empty() {
        return Err(invalid(0, "No chess games or positions found"));
    }
    Ok(p.doc)
}
struct Parser {
    doc: Collection,
    at: usize,
    current: Option<NodeId>,
    resumes: Vec<(NodeId, usize)>,
    tails: Vec<Option<NodeId>>,
    annotations: usize,
}
impl Parser {
    fn space(&mut self) {
        while self
            .doc
            .source
            .get(self.at)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.at += 1;
        }
    }
    fn line(&mut self) {
        while self
            .doc
            .source
            .get(self.at)
            .is_some_and(|b| !matches!(b, b'\n' | b'\r'))
        {
            self.at += 1;
        }
    }
    fn ensure_game(&mut self, start: usize) -> Result<NodeId, Error> {
        if let Some(id) = self.current {
            Ok(id)
        } else {
            self.new_game(start)
        }
    }
    fn new_game(&mut self, start: usize) -> Result<NodeId, Error> {
        if self.doc.games.len() == MAX_GAMES {
            return Err(Error::Limit("Chess collection exceeds 1,024 games"));
        }
        let root = self.add_node(None, start..start, self.doc.games.len())?;
        self.doc.games.push(Game {
            root,
            tags: BTreeMap::new(),
            start: Board::default(),
        });
        self.current = Some(root);
        Ok(root)
    }
    fn add_node(
        &mut self,
        parent: Option<NodeId>,
        source: Range<usize>,
        game: usize,
    ) -> Result<NodeId, Error> {
        if self.doc.nodes.len() == MAX_NODES {
            return Err(Error::Limit("Chess collection exceeds 100,000 nodes"));
        }
        let id = self.doc.nodes.len();
        let ply = parent.map_or(0, |p| self.doc.nodes[p].ply + 1);
        self.doc.nodes.push(Node {
            parent,
            first_child: None,
            next_sibling: None,
            san: source.start..source.start,
            source,
            game,
            ply,
            comments: Vec::new(),
            annotations: Vec::new(),
        });
        self.tails.push(None);
        if let Some(parent) = parent {
            if let Some(last) = self.tails[parent] {
                self.doc.nodes[last].next_sibling = Some(id);
            } else {
                self.doc.nodes[parent].first_child = Some(id);
            }
            self.tails[parent] = Some(id);
        }
        Ok(id)
    }
    fn annotation(
        &mut self,
        node: NodeId,
        range: Range<usize>,
        comment: bool,
    ) -> Result<(), Error> {
        let n = &mut self.doc.nodes[node];
        let bytes: usize = n.comments.iter().map(Range::len).sum();
        if range.len() > MAX_VALUE
            || bytes + range.len() > MAX_VALUE
            || n.comments.len() + n.annotations.len() >= 128
            || self.annotations == MAX_ANNOTATIONS
        {
            return Err(Error::Limit("PGN annotations exceed their budget"));
        }
        if comment {
            n.comments.push(range);
        } else {
            n.annotations.push(range);
        }
        self.annotations += 1;
        Ok(())
    }
    fn finish_start(&mut self) -> Result<(), Error> {
        let Some(game) = self.doc.games.last_mut() else {
            return Ok(());
        };
        if self.doc.nodes[game.root].first_child.is_some() {
            return Ok(());
        }
        if let Some(v) = game.tags.get("Variant")
            && !["standard", "chess", "normal"]
                .iter()
                .any(|s| v.eq_ignore_ascii_case(s))
        {
            return Err(Error::Unsupported("Only standard chess is supported"));
        }
        if let Some(fen) = game.tags.get("FEN") {
            game.start = Board::from_fen(fen, false).map_err(|_| {
                invalid(
                    self.doc.nodes[game.root].source.start,
                    "Invalid standard-chess FEN",
                )
            })?;
        } else if game.tags.get("SetUp").is_some_and(|s| s == "1") {
            return Err(invalid(self.at, "SetUp requires a FEN tag"));
        }
        Ok(())
    }
    fn tag(&mut self) -> Result<(), Error> {
        let start = self.at;
        if !self.resumes.is_empty() {
            return Err(invalid(start, "PGN tag inside a variation"));
        }
        if let Some(node) = self.current
            && self.doc.nodes[node].parent.is_some()
        {
            self.current = None;
        }
        let root = self.ensure_game(start)?;
        self.at += 1;
        self.space();
        let begin = self.at;
        while self
            .doc
            .source
            .get(self.at)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            self.at += 1;
        }
        if begin == self.at || self.at - begin > 64 {
            return Err(invalid(start, "Invalid PGN tag name"));
        }
        let name = decode(&self.doc.source[begin..self.at]);
        self.space();
        if self.doc.source.get(self.at) != Some(&b'"') {
            return Err(invalid(self.at, "Expected quoted PGN tag"));
        }
        self.at += 1;
        let mut value = Vec::new();
        loop {
            let Some(&b) = self.doc.source.get(self.at) else {
                return Err(invalid(start, "Unclosed PGN tag"));
            };
            self.at += 1;
            match b {
                b'"' => break,
                b'\\' => {
                    let Some(&b) = self.doc.source.get(self.at) else {
                        return Err(invalid(start, "Unclosed PGN tag"));
                    };
                    if !matches!(b, b'\\' | b'"') {
                        return Err(invalid(self.at, "Invalid PGN tag escape"));
                    }
                    self.at += 1;
                    value.push(b);
                }
                _ => value.push(b),
            }
            if value.len() > MAX_VALUE {
                return Err(Error::Limit("PGN tag exceeds 64 KiB"));
            }
        }
        self.space();
        if self.doc.source.get(self.at) != Some(&b']') {
            return Err(invalid(self.at, "Unclosed PGN tag"));
        }
        self.at += 1;
        let game = self.doc.games.last_mut().unwrap();
        if game.tags.len() == 128 {
            return Err(Error::Limit("PGN game exceeds 128 tags"));
        }
        game.tags.insert(name, decode(&value));
        self.doc.nodes[root].source.end = self.at;
        Ok(())
    }
}
