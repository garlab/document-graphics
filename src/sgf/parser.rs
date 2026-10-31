use super::*;

struct Frame {
    attach: Option<NodeId>,
    last: Option<NodeId>,
    branched: bool,
    game: usize,
}

pub(super) fn parse(source: Arc<[u8]>, cancel: &AtomicBool) -> Result<Collection, Error> {
    check(cancel)?;
    if source.len() > MAX_SOURCE {
        return Err(Error::Limit("SGF source exceeds 8 MiB"));
    }
    let mut doc = Collection {
        source,
        nodes: Vec::new(),
        properties: Vec::new(),
        values: Vec::new(),
        games: Vec::new(),
    };
    let mut stack: Vec<Frame> = Vec::new();
    let mut tails: Vec<Option<NodeId>> = Vec::new();
    let mut i = if doc.source.starts_with(&[0xef, 0xbb, 0xbf]) {
        3
    } else {
        0
    };
    while i < doc.source.len() {
        check(cancel)?;
        match doc.source[i] {
            c if c.is_ascii_whitespace() => {
                i += 1;
            }
            b'(' => {
                if stack.len() >= 256 {
                    return Err(Error::Limit("SGF variation nesting exceeds 256"));
                }
                let (attach, game) = if let Some(frame) = stack.last_mut() {
                    let parent = frame.last.ok_or_else(|| invalid(i, "Empty SGF sequence"))?;
                    frame.branched = true;
                    (Some(parent), frame.game)
                } else {
                    if doc.games.len() >= 1024 {
                        return Err(Error::Limit("SGF collection exceeds 1024 games"));
                    }
                    let game = doc.games.len();
                    doc.games.push(Game {
                        root: doc.nodes.len(),
                        width: 19,
                        height: 19,
                        encoding: Encoding::Latin1,
                    });
                    (None, game)
                };
                stack.push(Frame {
                    attach,
                    game,
                    last: None,
                    branched: false,
                });
                i += 1;
            }
            b')' => {
                let frame = stack
                    .pop()
                    .ok_or_else(|| invalid(i, "Unexpected closing parenthesis"))?;
                if frame.last.is_none() {
                    return Err(invalid(i, "Empty SGF game tree"));
                }
                i += 1;
            }
            b';' => {
                let frame = stack
                    .last_mut()
                    .ok_or_else(|| invalid(i, "Node outside an SGF game tree"))?;
                if frame.branched {
                    return Err(invalid(i, "Node follows a variation outside its sequence"));
                }
                if doc.nodes.len() >= MAX_NODES {
                    return Err(Error::Limit("SGF exceeds 100,000 nodes"));
                }
                let id = doc.nodes.len();
                let parent = frame.last.or(frame.attach);
                doc.nodes.push(Node {
                    parent,
                    first_child: None,
                    next_sibling: None,
                    source: i..i + 1,
                    game: frame.game,
                    properties: doc.properties.len()..doc.properties.len(),
                });
                tails.push(None);
                if let Some(parent) = parent {
                    if let Some(tail) = tails[parent] {
                        doc.nodes[tail].next_sibling = Some(id);
                    } else {
                        doc.nodes[parent].first_child = Some(id);
                    }
                    tails[parent] = Some(id);
                }
                frame.last = Some(id);
                i += 1;
            }
            b'A'..=b'Z' => {
                let id = stack
                    .last()
                    .filter(|f| !f.branched)
                    .and_then(|f| f.last)
                    .ok_or_else(|| invalid(i, "Property outside a node"))?;
                if doc.properties.len() >= 300_000 || doc.nodes[id].properties.len() >= 128 {
                    return Err(Error::Limit("Too many SGF properties"));
                }
                let start = i;
                while i < doc.source.len() && doc.source[i].is_ascii_uppercase() {
                    i += 1;
                }
                if i - start > 32 {
                    return Err(Error::Limit("SGF property name exceeds 32 bytes"));
                }
                let name = start..i;
                if doc.properties[doc.nodes[id].properties.clone()]
                    .iter()
                    .any(|p| doc.source[p.name.clone()] == doc.source[name.clone()])
                {
                    return Err(invalid(start, "Duplicate SGF property"));
                }
                skip(&doc.source, &mut i);
                let first = doc.values.len();
                while i < doc.source.len() && doc.source[i] == b'[' {
                    if doc.values.len() >= 500_000 {
                        return Err(Error::Limit("Too many SGF values"));
                    }
                    i += 1;
                    let start = i;
                    loop {
                        if i >= doc.source.len() {
                            return Err(invalid(start, "Unterminated SGF value"));
                        }
                        if (i - start).is_multiple_of(4096) {
                            check(cancel)?;
                        }
                        if i - start > MAX_VALUE {
                            return Err(Error::Limit("SGF property value exceeds 128 KiB"));
                        }
                        match doc.source[i] {
                            b']' => break,
                            b'\\' => {
                                i += 1;
                                if i >= doc.source.len() {
                                    return Err(invalid(start, "Unterminated SGF escape"));
                                }
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                    doc.values.push(start..i);
                    i += 1;
                    skip(&doc.source, &mut i);
                }
                if first == doc.values.len() {
                    return Err(invalid(start, "SGF property has no value"));
                }
                doc.properties.push(Property {
                    name,
                    values: first..doc.values.len(),
                });
                doc.nodes[id].properties.end = doc.properties.len();
                doc.nodes[id].source.end = i;
            }
            _ => return Err(invalid(i, "Unexpected SGF input")),
        }
    }
    if !stack.is_empty() {
        return Err(invalid(i, "Unclosed SGF game tree"));
    }
    if doc.nodes.is_empty() {
        return Err(invalid(i, "No SGF game found"));
    }
    for g in 0..doc.games.len() {
        let root = doc.games[g].root;
        let start = doc.nodes[root].source.start;
        let end = doc
            .games
            .get(g + 1)
            .map_or(doc.source.len(), |g| doc.nodes[g.root].source.start);
        let utf8 = std::str::from_utf8(&doc.source[start..end]).is_ok();
        if utf8 {
            doc.games[g].encoding = Encoding::Utf8;
        }
        if doc.text(root, "GM").is_some_and(|s| s.trim() != "1") {
            return Err(Error::Unsupported("Only Go records (GM[1]) are supported"));
        }
        if let Some(ff) = doc.text(root, "FF")
            && !matches!(ff.trim(), "1" | "2" | "3" | "4")
        {
            return Err(Error::Unsupported("Unsupported SGF file-format version"));
        }
        if let Some(ca) = doc.text(root, "CA") {
            doc.games[g].encoding = match ca.trim().to_ascii_lowercase().as_str() {
                "utf-8" | "utf8" if utf8 => Encoding::Utf8,
                "utf-8" | "utf8" => {
                    return Err(invalid(
                        doc.nodes[root].source.start,
                        "Invalid UTF-8 in SGF",
                    ));
                }
                "iso-8859-1" | "iso8859-1" | "latin1" => Encoding::Latin1,
                _ => {
                    return Err(Error::Unsupported(
                        "SGF character encoding is not UTF-8 or ISO-8859-1",
                    ));
                }
            };
        }
        if let Some(size) = doc.text(root, "SZ") {
            let (w, h) = size.split_once(':').unwrap_or((&size, &size));
            let size = |s: &str| s.trim().parse::<u8>().ok().filter(|n| (1..=52).contains(n));
            let (Some(w), Some(h)) = (size(w), size(h)) else {
                return Err(Error::Unsupported(
                    "SGF board sizes must be between 1 and 52",
                ));
            };
            doc.games[g].width = w;
            doc.games[g].height = h;
        }
    }
    doc.nodes.shrink_to_fit();
    doc.properties.shrink_to_fit();
    doc.values.shrink_to_fit();
    doc.games.shrink_to_fit();
    check(cancel)?;
    Ok(doc)
}
fn skip(source: &[u8], i: &mut usize) {
    while *i < source.len() && source[*i].is_ascii_whitespace() {
        *i += 1;
    }
}
