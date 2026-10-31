use super::*;
fn parse(s: &str) -> Collection {
    Collection::parse(Arc::from(s.as_bytes()), &AtomicBool::new(false)).unwrap()
}
fn pos(doc: &Collection, n: NodeId) -> Position {
    doc.position(n, &AtomicBool::new(false)).unwrap()
}
fn p(x: u8, y: u8) -> Point {
    Point { x, y }
}

#[test]
fn parses_collections_variations_escapes_and_keeps_source_ranges() {
    let input = "(;FF[4]GM[1]SZ[9]C[line\\\r\n joined\\]\\\\\r\nnext]XX[private];B[aa](;W[bb]N[A])(;W[cc];B[]))(;SZ[13:9];W[dd])";
    let doc = parse(input);
    assert_eq!(doc.games().len(), 2);
    assert_eq!(doc.nodes().len(), 7);
    assert_eq!(doc.children(1).collect::<Vec<_>>(), vec![2, 3]);
    assert_eq!(doc.nodes()[4].parent, Some(3));
    assert_eq!(doc.text(0, "C").unwrap(), "line joined]\\\nnext");
    assert_eq!(doc.text(0, "XX").unwrap(), "private");
    for (id, node) in doc.nodes().iter().enumerate() {
        assert_eq!(input.as_bytes()[node.source.start], b';');
        assert_eq!(doc.node_for_offset(node.source.start), id);
    }
    assert_eq!(doc.games()[1].height, 9);
    assert_eq!(pos(&doc, 4).last_move.unwrap().point, None);
    assert_eq!(pos(&doc, 2).stone(p(2, 2)), None);
}
#[test]
fn replays_captures_suicide_setup_ranges_passes_and_move_numbers() {
    let doc = parse("(;SZ[5]AB[ba][ab][cb]AW[bb];B[bc];W[];B[tt]MN[20])");
    let root = pos(&doc, 0);
    let next = root.advance(&doc, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(next.stone(p(1, 1)), None);
    assert_eq!(next.captures, [1, 0]);
    assert_eq!(next.stone(p(1, 2)), Some(Color::Black));
    assert_eq!(pos(&doc, 3).move_number, 20);
    let suicide = parse("(;SZ[3]AB[ab][ba];W[aa])");
    assert_eq!(pos(&suicide, 1).stone(p(0, 0)), None);
    assert_eq!(pos(&suicide, 1).captures, [1, 0]);
    let setup = parse("(;SZ[9]HA[2]AB[aa:bb];AE[aa]AW[cc]PL[B])");
    assert_eq!(pos(&setup, 0).to_play, Color::White);
    assert_eq!(pos(&setup, 1).stone(p(0, 0)), None);
    assert_eq!(pos(&setup, 1).stone(p(2, 2)), Some(Color::White));
    assert_eq!(pos(&setup, 1).captures, [0, 0]);
}
#[test]
fn rejects_invalid_or_excessive_input_without_recursion() {
    for s in [
        "",
        "()",
        "(;B[aa]",
        "(;C[unfinished)",
        "(;B[aa]B[bb])",
        "(;SZ[9](;B[aa]);W[bb])",
        ";B[aa]",
        "(C[x];B[aa])",
        "(;GM[2])",
        "(;SZ[53])",
    ] {
        assert!(
            Collection::parse(Arc::from(s.as_bytes()), &AtomicBool::new(false)).is_err(),
            "{s}"
        );
    }
    for s in [
        "(;SZ[9];B[zz])",
        "(;SZ[9];B[aa]W[bb])",
        "(;SZ[9];B[aa]AB[bb])",
    ] {
        let d = parse(s);
        assert!(d.position(1, &AtomicBool::new(false)).is_err());
    }
    let deep = format!("{}{}", "(;".repeat(300), ")".repeat(300));
    assert!(matches!(
        Collection::parse(Arc::from(deep.as_bytes()), &AtomicBool::new(false)),
        Err(Error::Limit(_))
    ));
    assert!(matches!(
        Collection::parse(Arc::from(&b"(;)"[..]), &AtomicBool::new(true)),
        Err(Error::Cancelled)
    ));
    let d = parse("(;SZ[9])");
    assert!(matches!(
        d.position(0, &AtomicBool::new(true)),
        Err(Error::Cancelled)
    ));
}
#[test]
fn markup_is_local_and_rendering_is_bounded_and_escaped() {
    let d = parse("(;SZ[9]AB[dd]C[<script>]TR[dd]LB[aa:A<&]AR[aa:hh]DD[bb];W[ee];B[ff]DD[])");
    assert_eq!(pos(&d, 1).marks.len(), 1);
    assert_eq!(pos(&d, 2).marks.len(), 0);
    let first = pos(&d, 0);
    let cancel = AtomicBool::new(false);
    let svg = to_svg(&first, RenderOptions::default(), &cancel).unwrap();
    assert!(svg.contains("A&lt;&amp;"));
    assert!(!svg.contains("<script>"));
    for dark in [false, true] {
        let raster = rasterize(
            &first,
            RenderOptions {
                dark,
                move_numbers: false,
            },
            crate::RasterOptions::fit((800, 600)),
            &cancel,
        )
        .unwrap();
        assert!(raster.width <= 800 && raster.height <= 600);
        assert_eq!(
            raster.rgba.len(),
            raster.width as usize * raster.height as usize * 4
        );
    }
}
#[test]
fn accepts_legacy_encoding_and_large_linear_records() {
    let d = Collection::parse(
        Arc::from(&b"(;CA[ISO-8859-1]PB[Andr\xe9]SZ[19])"[..]),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(d.text(0, "PB").unwrap(), "André");
    let text = format!("(;SZ[19]{})", ";B[];W[]".repeat(5000));
    let d = parse(&text);
    assert_eq!(d.nodes().len(), 10001);
    assert_eq!(pos(&d, 10000).move_number, 10000);
}

#[test]
fn collection_encodings_are_independent_and_public_diagram_input_is_checked() {
    let doc = Collection::parse(
        Arc::from(&b"(;CA[UTF-8]PB[A])(;CA[ISO-8859-1]PB[Andr\xe9])"[..]),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(doc.text(1, "PB").unwrap(), "André");
    let mut p = pos(&doc, 0);
    p.width = 0;
    assert!(to_svg(&p, RenderOptions::default(), &AtomicBool::new(false)).is_err());
}

#[test]
fn source_value_and_node_limits_are_enforced() {
    let cancel = AtomicBool::new(false);
    assert!(matches!(
        Collection::parse(vec![b' '; MAX_SOURCE + 1].into(), &cancel),
        Err(Error::Limit(_))
    ));
    let long = format!("(;C[{}])", "x".repeat(MAX_VALUE + 1));
    assert!(matches!(
        Collection::parse(Arc::from(long.as_bytes()), &cancel),
        Err(Error::Limit(_))
    ));
    let many = format!("({})", ";".repeat(MAX_NODES + 1));
    assert!(matches!(
        Collection::parse(Arc::from(many.as_bytes()), &cancel),
        Err(Error::Limit(_))
    ));
    let record = parse("(;SZ[9];B[dd];W[ee])");
    for dark in [false, true] {
        let raster = rasterize(
            &pos(&record, 2),
            RenderOptions {
                dark,
                move_numbers: true,
            },
            crate::RasterOptions::scaled(4.),
            &cancel,
        )
        .unwrap();
        assert!(raster.width as usize * raster.height as usize <= crate::MAX_PIXELS);
    }
}
