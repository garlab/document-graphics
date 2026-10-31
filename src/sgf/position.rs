use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Black,
    White,
}
impl Color {
    pub fn other(self) -> Self {
        match self {
            Self::Black => Self::White,
            Self::White => Self::Black,
        }
    }
    fn index(self) -> usize {
        if self == Self::Black { 0 } else { 1 }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub x: u8,
    pub y: u8,
}
impl Point {
    pub fn coordinate(self, height: u8) -> String {
        const LETTERS: &[u8] = b"ABCDEFGHJKLMNOPQRSTUVWXYZ";
        let x = self.x as usize;
        let column = if x < LETTERS.len() {
            (LETTERS[x] as char).to_string()
        } else {
            format!(
                "{}{}",
                LETTERS[x / LETTERS.len() - 1] as char,
                LETTERS[x % LETTERS.len()] as char
            )
        };
        format!("{column}{}", height.saturating_sub(self.y))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Played {
    pub color: Color,
    pub point: Option<Point>,
    pub number: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkKind {
    Circle,
    Square,
    Triangle,
    Cross,
    Selected,
    Label(String),
    Territory(Color),
    Dim,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub at: Point,
    pub kind: MarkKind,
}
#[derive(Debug, Clone)]
pub struct Line {
    pub from: Point,
    pub to: Point,
    pub arrow: bool,
}

/// A single board. Applications may cache a bounded number of these independently.
#[derive(Debug, Clone)]
pub struct Position {
    pub width: u8,
    pub height: u8,
    pub to_play: Color,
    /// Prisoners taken by Black, then White. Setup stones never count as captures.
    pub captures: [u32; 2],
    pub move_number: u32,
    pub last_move: Option<Played>,
    pub marks: Vec<Mark>,
    pub lines: Vec<Line>,
    node: NodeId,
    source: Arc<[u8]>,
    board: Vec<Option<Color>>,
    numbers: Vec<u32>,
}
impl Position {
    pub(super) fn validate(&self) -> Result<(), Error> {
        if !(1..=52).contains(&self.width)
            || !(1..=52).contains(&self.height)
            || self.board.len() != self.width as usize * self.height as usize
            || self.marks.len() > 512
            || self.lines.len() > 512
        {
            return Err(Error::Limit("Invalid or oversized SGF diagram"));
        }
        let inside = |p: Point| p.x < self.width && p.y < self.height;
        if self
            .marks
            .iter()
            .any(|m| !inside(m.at) || matches!(&m.kind, MarkKind::Label(s) if s.chars().count()>32))
            || self.lines.iter().any(|l| !inside(l.from) || !inside(l.to))
            || self
                .last_move
                .is_some_and(|m| m.point.is_some_and(|p| !inside(p)))
        {
            return Err(Error::Limit("Invalid or oversized SGF board annotation"));
        }
        Ok(())
    }
    pub fn node(&self) -> NodeId {
        self.node
    }
    pub fn stone(&self, point: Point) -> Option<Color> {
        if point.x >= self.width || point.y >= self.height {
            return None;
        }
        self.board
            .get(point.y as usize * self.width as usize + point.x as usize)
            .copied()
            .flatten()
    }
    pub fn number(&self, point: Point) -> u32 {
        if point.x >= self.width || point.y >= self.height {
            return 0;
        }
        self.numbers
            .get(point.y as usize * self.width as usize + point.x as usize)
            .copied()
            .unwrap_or(0)
    }
    /// Incremental forward navigation; the target must be a direct child.
    pub fn advance(
        &self,
        doc: &Collection,
        node: NodeId,
        cancel: &AtomicBool,
    ) -> Result<Self, Error> {
        if !Arc::ptr_eq(&self.source, &doc.source)
            || doc
                .nodes
                .get(node)
                .is_none_or(|n| n.parent != Some(self.node))
        {
            return Err(invalid(
                0,
                "Position does not precede the requested SGF node",
            ));
        }
        self.validate()?;
        let mut next = self.clone();
        next.apply(doc, node, &mut Work::new(self.board.len(), cancel))?;
        Ok(next)
    }
    fn index(&self, p: Point) -> usize {
        p.y as usize * self.width as usize + p.x as usize
    }
    fn apply(&mut self, doc: &Collection, node: NodeId, work: &mut Work<'_>) -> Result<(), Error> {
        check(work.cancel)?;
        let offset = doc.nodes[node].source.start;
        let black = doc.text(node, "B");
        let white = doc.text(node, "W");
        if black.is_some() && white.is_some() {
            return Err(invalid(offset, "B and W cannot share an SGF node"));
        }
        let has_move = black.is_some() || white.is_some();
        if doc.ranges(node, "B").len() > 1 || doc.ranges(node, "W").len() > 1 {
            return Err(invalid(offset, "A move must have one value"));
        }
        for key in ["SZ", "GM", "CA", "FF"] {
            if doc.nodes[node].parent.is_some() && !doc.ranges(node, key).is_empty() {
                return Err(invalid(offset, "Root property outside the SGF root"));
            }
        }
        for (key, color) in [
            ("AE", None),
            ("AB", Some(Color::Black)),
            ("AW", Some(Color::White)),
        ] {
            if has_move && !doc.ranges(node, key).is_empty() {
                return Err(invalid(
                    offset,
                    "Moves and setup stones cannot share an SGF node",
                ));
            }
            for value in doc.values(node, key) {
                for point in points(&value, self.width, self.height, offset)? {
                    work.step()?;
                    let i = self.index(point);
                    self.board[i] = color;
                    self.numbers[i] = 0;
                }
            }
        }
        if let Some(value) = black.as_ref().or(white.as_ref()) {
            self.move_number = self.move_number.saturating_add(1);
            if let Some(number) = doc.text(node, "MN") {
                self.move_number = number
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| invalid(offset, "Invalid SGF move number"))?;
            }
            let color = if black.is_some() {
                Color::Black
            } else {
                Color::White
            };
            let point =
                if value.is_empty() || (value == "tt" && self.width <= 19 && self.height <= 19) {
                    None
                } else {
                    Some(point(value, self.width, self.height, offset)?)
                };
            if let Some(p) = point {
                let i = self.index(p);
                self.board[i] = Some(color);
                self.numbers[i] = self.move_number;
                for j in neighbors(i, self.width as usize, self.board.len()) {
                    if self.board[j] == Some(color.other()) {
                        self.remove_dead(j, color, work)?;
                    }
                }
                self.remove_dead(i, color.other(), work)?;
            }
            self.last_move = Some(Played {
                color,
                point,
                number: self.move_number,
            });
            self.to_play = color.other();
        } else if doc.nodes[node].parent.is_none()
            && doc
                .text(node, "HA")
                .and_then(|s| s.parse::<u16>().ok())
                .is_some_and(|n| n >= 2)
        {
            self.to_play = Color::White;
        }
        if let Some(player) = doc.text(node, "PL") {
            self.to_play = match player.as_str() {
                "B" => Color::Black,
                "W" => Color::White,
                _ => return Err(invalid(offset, "Invalid player to move")),
            };
        }
        // Most markup is node-local. Dimmed points inherit until DD[] clears them.
        self.marks.retain(|m| matches!(m.kind, MarkKind::Dim));
        if !doc.ranges(node, "DD").is_empty() {
            self.marks.clear();
        }
        self.lines.clear();
        for (key, kind) in [
            ("CR", MarkKind::Circle),
            ("SQ", MarkKind::Square),
            ("TR", MarkKind::Triangle),
            ("MA", MarkKind::Cross),
            ("SL", MarkKind::Selected),
            ("TB", MarkKind::Territory(Color::Black)),
            ("TW", MarkKind::Territory(Color::White)),
            ("DD", MarkKind::Dim),
        ] {
            for value in doc.values(node, key) {
                for at in points(&value, self.width, self.height, offset)? {
                    work.step()?;
                    if self.marks.len() >= 512 {
                        return Err(Error::Limit("SGF node exceeds 512 board marks"));
                    }
                    self.marks.push(Mark {
                        at,
                        kind: kind.clone(),
                    });
                }
            }
        }
        for value in doc.values(node, "LB") {
            let (p, label) = value
                .split_once(':')
                .ok_or_else(|| invalid(offset, "Invalid SGF label"))?;
            if self.marks.len() >= 512 || label.chars().count() > 32 {
                return Err(Error::Limit("SGF board label limit exceeded"));
            }
            self.marks.push(Mark {
                at: point(p, self.width, self.height, offset)?,
                kind: MarkKind::Label(label.into()),
            });
        }
        for (key, arrow) in [("AR", true), ("LN", false)] {
            for value in doc.values(node, key) {
                if self.lines.len() >= 512 {
                    return Err(Error::Limit("SGF node exceeds 512 lines"));
                }
                let (a, b) = value
                    .split_once(':')
                    .ok_or_else(|| invalid(offset, "Invalid SGF line"))?;
                self.lines.push(Line {
                    from: point(a, self.width, self.height, offset)?,
                    to: point(b, self.width, self.height, offset)?,
                    arrow,
                });
            }
        }
        self.node = node;
        Ok(())
    }
    fn remove_dead(
        &mut self,
        start: usize,
        captor: Color,
        work: &mut Work<'_>,
    ) -> Result<(), Error> {
        let Some(color) = self.board[start] else {
            return Ok(());
        };
        work.seen.fill(false);
        work.stack.clear();
        work.group.clear();
        work.stack.push(start);
        while let Some(i) = work.stack.pop() {
            if work.seen[i] {
                continue;
            }
            work.seen[i] = true;
            work.step()?;
            work.group.push(i);
            for j in neighbors(i, self.width as usize, self.board.len()) {
                match self.board[j] {
                    None => return Ok(()),
                    Some(c) if c == color && !work.seen[j] => work.stack.push(j),
                    _ => {}
                }
            }
        }
        for &i in &work.group {
            self.board[i] = None;
            self.numbers[i] = 0;
        }
        self.captures[captor.index()] =
            self.captures[captor.index()].saturating_add(work.group.len() as u32);
        Ok(())
    }
}
fn neighbors(i: usize, width: usize, len: usize) -> impl Iterator<Item = usize> {
    [
        if !i.is_multiple_of(width) {
            Some(i - 1)
        } else {
            None
        },
        if i % width + 1 < width {
            Some(i + 1)
        } else {
            None
        },
        i.checked_sub(width),
        if i + width < len {
            Some(i + width)
        } else {
            None
        },
    ]
    .into_iter()
    .flatten()
}
struct Work<'a> {
    seen: Vec<bool>,
    stack: Vec<usize>,
    group: Vec<usize>,
    steps: usize,
    cancel: &'a AtomicBool,
}
impl<'a> Work<'a> {
    fn new(size: usize, cancel: &'a AtomicBool) -> Self {
        Self {
            seen: vec![false; size],
            stack: Vec::new(),
            group: Vec::new(),
            steps: 0,
            cancel,
        }
    }
    fn step(&mut self) -> Result<(), Error> {
        self.steps += 1;
        if self.steps > 20_000_000 {
            return Err(Error::Limit(
                "SGF position reconstruction exceeded its work limit",
            ));
        }
        if self.steps.is_multiple_of(1024) {
            check(self.cancel)?;
        }
        Ok(())
    }
}
pub(super) fn at(doc: &Collection, node: NodeId, cancel: &AtomicBool) -> Result<Position, Error> {
    check(cancel)?;
    let selected = doc
        .nodes
        .get(node)
        .ok_or_else(|| invalid(0, "Unknown SGF node"))?;
    let game = &doc.games[selected.game];
    let size = game.width as usize * game.height as usize;
    let mut result = Position {
        width: game.width,
        height: game.height,
        to_play: Color::Black,
        captures: [0, 0],
        move_number: 0,
        last_move: None,
        marks: Vec::new(),
        lines: Vec::new(),
        node: game.root,
        source: doc.source.clone(),
        board: vec![None; size],
        numbers: vec![0; size],
    };
    let mut path = Vec::new();
    let mut current = Some(node);
    while let Some(id) = current {
        check(cancel)?;
        path.push(id);
        current = doc.nodes[id].parent;
    }
    let mut work = Work::new(size, cancel);
    for id in path.into_iter().rev() {
        work.step()?;
        result.apply(doc, id, &mut work)?;
    }
    Ok(result)
}
pub(super) fn point(s: &str, width: u8, height: u8, offset: usize) -> Result<Point, Error> {
    let index = |b| match b {
        b'a'..=b'z' => Some(b - b'a'),
        b'A'..=b'Z' => Some(b - b'A' + 26),
        _ => None,
    };
    let b = s.as_bytes();
    if b.len() == 2
        && let (Some(x), Some(y)) = (index(b[0]), index(b[1]))
        && x < width
        && y < height
    {
        return Ok(Point { x, y });
    }
    Err(invalid(offset, "SGF point is outside the board"))
}
fn points(s: &str, width: u8, height: u8, offset: usize) -> Result<Vec<Point>, Error> {
    if s.is_empty() {
        return Ok(Vec::new());
    }
    if let Some((a, b)) = s.split_once(':') {
        let a = point(a, width, height, offset)?;
        let b = point(b, width, height, offset)?;
        if a.x > b.x || a.y > b.y {
            return Err(invalid(offset, "Reversed SGF point rectangle"));
        }
        Ok((a.y..=b.y)
            .flat_map(|y| (a.x..=b.x).map(move |x| Point { x, y }))
            .collect())
    } else {
        Ok(vec![point(s, width, height, offset)?])
    }
}
