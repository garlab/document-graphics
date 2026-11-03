use super::*;
fn parse(s: &str) -> Collection {
    Collection::parse(
        Arc::from(s.as_bytes()),
        Format::Pgn,
        &AtomicBool::new(false),
    )
    .unwrap()
}
fn position(s: &str) -> Position {
    let d = parse(s);
    d.position(d.nodes.len() - 1, &AtomicBool::new(false))
        .unwrap()
}
#[test]
fn variations_comments_tags_and_collections() {
    let d = parse(
        "[White \"Alice\"]\n[Black \"Bob\"]\n{Opening} 1.e4! e5 (1...c5 {Sicilian} 2.Nf3 $1 (2.Nc3)) 2.Nf3 Nc6 1/2-1/2\n[Event \"Next\"]\n1.d4 *",
    );
    assert_eq!(d.games.len(), 2);
    assert_eq!(d.title(0), "Alice · Bob");
    assert_eq!(d.nodes[2].next_sibling, Some(3));
    assert_eq!(d.nodes[3].parent, Some(1));
    assert_eq!(d.nodes[5].parent, Some(3));
    assert_eq!(d.nodes[6].parent, Some(2));
    assert_eq!(d.comment(0), "Opening");
    assert_eq!(d.comment(3), "Sicilian");
    assert_eq!(d.annotations(1), "!");
    assert_eq!(d.annotations(4), "$1");
    for (i, n) in d.nodes.iter().enumerate() {
        assert_eq!(d.node_for_offset(n.source.start), i);
        d.position(i, &AtomicBool::new(false)).unwrap();
    }
    assert_eq!(
        d.position(3, &AtomicBool::new(false))
            .unwrap()
            .piece(Square::C5),
        Some((Color::Black, Piece::Pawn))
    );
    assert_eq!(
        d.position(2, &AtomicBool::new(false))
            .unwrap()
            .piece(Square::C5),
        None
    );
}
#[test]
fn castling_en_passant_promotions_and_disambiguation() {
    let p = position("1.e4 e5 2.Nf3 Nc6 3.Bc4 Nf6 4.O-O *");
    assert_eq!(p.piece(Square::G1), Some((Color::White, Piece::King)));
    assert_eq!(p.piece(Square::F1), Some((Color::White, Piece::Rook)));
    assert_eq!(p.last_move, Some((Square::E1, Square::G1)));
    let p = position("1.e4 a6 2.e5 d5 3.exd6 e.p. *");
    assert_eq!(p.piece(Square::D5), None);
    assert_eq!(p.piece(Square::D6), Some((Color::White, Piece::Pawn)));
    let p = position("[SetUp \"1\"][FEN \"4k3/P7/8/8/8/8/8/4K3 w - - 0 1\"] 1.a8=N *");
    assert_eq!(p.piece(Square::A8), Some((Color::White, Piece::Knight)));
    let d = parse("[FEN \"4k3/8/8/8/8/8/8/1N2KN2 w - - 0 20\"] 20.Nd2 *");
    assert!(d.position(1, &AtomicBool::new(false)).is_err());
    let p = position("[FEN \"4k3/8/8/8/8/8/8/1N2KN2 w - - 0 20\"] 20.Nbd2 *");
    assert_eq!(p.piece(Square::B1), None);
    assert_eq!(p.piece(Square::F1), Some((Color::White, Piece::Knight)));
}
#[test]
fn fen_black_to_move_and_incremental_replay() {
    let d = parse("[FEN \"4k3/8/8/8/8/8/8/4K3 b - - 0 42\"] 42...Kd7 43.Kd2 *");
    assert_eq!(d.move_number(1), 42);
    assert_eq!(d.move_number(2), 43);
    assert_eq!(d.move_color(1), Some(Color::Black));
    let cancel = AtomicBool::new(false);
    let p = d
        .position(1, &cancel)
        .unwrap()
        .advance(&d, 2, &cancel)
        .unwrap();
    assert_eq!(p.fen(), d.position(2, &cancel).unwrap().fen());
    assert!(p.advance(&d, 1, &cancel).is_err());
    let fen = Collection::parse(
        Arc::from(&b"4k3/8/8/8/8/8/8/4K3 w - - 0 1\n4k3/8/8/8/8/8/8/4K3 b - - 0 2"[..]),
        Format::Fen,
        &cancel,
    )
    .unwrap();
    assert_eq!(fen.games.len(), 2);
}
#[test]
fn malformed_unsupported_and_resource_limits() {
    let cancel = AtomicBool::new(false);
    for s in [
        "",
        "(",
        "1.e4 (",
        "1.e4 ()",
        "1.e4 )",
        "{missing",
        "[Foo \"bar\"",
        "[Variant \"Atomic\"] 1.e4 *",
        "[SetUp \"1\"] *",
        "[FEN \"8/8/8/8/8/8/8/8 w - - 0 1\"] *",
    ] {
        assert!(
            Collection::parse(Arc::from(s.as_bytes()), Format::Pgn, &cancel).is_err(),
            "{s}"
        );
    }
    for s in ["1.e5 *", "1.Nf3 Nf6 2.Nxe5 *", "1.O-O *"] {
        let d = parse(s);
        assert!(d.position(d.nodes.len() - 1, &cancel).is_err(), "{s}");
    }
    assert!(
        Collection::parse(Arc::from(vec![b' '; MAX_SOURCE + 1]), Format::Pgn, &cancel).is_err()
    );
    let comment = format!("{{{}}}", "x".repeat(MAX_VALUE + 1));
    assert!(Collection::parse(Arc::from(comment.as_bytes()), Format::Pgn, &cancel).is_err());
    let deep = format!(
        "{}{}*",
        "1.e4 (".repeat(MAX_DEPTH + 1),
        ")".repeat(MAX_DEPTH + 1)
    );
    assert!(matches!(
        Collection::parse(Arc::from(deep.as_bytes()), Format::Pgn, &cancel),
        Err(Error::Limit(_))
    ));
    assert!(matches!(
        Collection::parse(Arc::from(&b"1.e4"[..]), Format::Pgn, &AtomicBool::new(true)),
        Err(Error::Cancelled)
    ));
}
#[test]
fn board_rasters_fit_the_budget_in_both_orientations() {
    let d = parse("1.e4 e5 2.Nf3 Nc6 3.Bb5 *");
    let cancel = AtomicBool::new(false);
    let p = d.position(5, &cancel).unwrap();
    for dark in [false, true] {
        for flipped in [false, true] {
            let raster = rasterize(
                &p,
                RenderOptions { dark, flipped },
                crate::RasterOptions::scaled(4.),
                &cancel,
            )
            .unwrap();
            assert_eq!(raster.width, raster.height);
            assert!(raster.rgba.len() <= crate::MAX_PIXELS * 4);
        }
    }
    assert!(matches!(
        to_svg(&p, RenderOptions::default(), &AtomicBool::new(true)),
        Err(Error::Cancelled)
    ));
}
#[test]
fn broad_trees_and_long_games_stay_bounded() {
    let d = parse(&format!("1.e4 {} *", "(1.d4) ".repeat(10_000)));
    assert_eq!(d.nodes.len(), 10_002);
    assert_eq!(d.nodes.last().unwrap().parent, Some(0));
    let long = format!("{}*", "Nf3 Nf6 Ng1 Ng8 ".repeat(4097));
    assert!(matches!(
        Collection::parse(
            Arc::from(long.as_bytes()),
            Format::Pgn,
            &AtomicBool::new(false)
        ),
        Err(Error::Limit(_))
    ));
}

#[test]
fn queenside_castling_and_pins_use_legal_moves() {
    let p = position("[FEN \"r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1\"] 1.O-O-O O-O *");
    assert_eq!(p.piece(Square::C1), Some((Color::White, Piece::King)));
    assert_eq!(p.piece(Square::D1), Some((Color::White, Piece::Rook)));
    assert_eq!(p.piece(Square::G8), Some((Color::Black, Piece::King)));
    // The e2 rook is pinned and cannot be used to disambiguate Rd2.
    let p = position("[FEN \"1k2r3/8/8/8/8/8/R3R3/4K3 w - - 0 1\"] 1.Rd2 *");
    assert_eq!(p.piece(Square::E2), Some((Color::White, Piece::Rook)));
    assert_eq!(p.piece(Square::D2), Some((Color::White, Piece::Rook)));
}
#[test]
fn arbitrary_bytes_and_truncated_records_do_not_panic() {
    let mut seed = 0x12345678u32;
    let cancel = AtomicBool::new(false);
    for length in 0..512 {
        let bytes: Vec<u8> = (0..length)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed as u8
            })
            .collect();
        if let Ok(doc) = Collection::parse(Arc::from(bytes), Format::Pgn, &cancel) {
            for node in 0..doc.nodes.len() {
                let _ = doc.position(node, &cancel);
            }
        }
    }
    let sample = b"[White \"One\"] 1.e4 e5 (1...c5 {A comment} 2.Nf3) 2.Nf3 *";
    for end in 0..sample.len() {
        if let Ok(doc) = Collection::parse(Arc::from(&sample[..end]), Format::Pgn, &cancel) {
            for node in 0..doc.nodes.len() {
                let _ = doc.position(node, &cancel);
            }
        }
    }
}
