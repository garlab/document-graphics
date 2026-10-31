//! Bounded native Mermaid subset. Parsing and layout belong in the worker process.

use crate::{Raster, RasterOptions, svg};
pub const MAX_SOURCE: usize = 16 * 1024;

/// Reject excessive input and configuration before starting a helper or parser.
pub fn admissible(source: &str) -> bool {
    if source.is_empty()
        || source.len() > MAX_SOURCE
        || source.lines().count() > 256
        || source.bytes().filter(|b| *b == b';').count() > 256
        || source.bytes().filter(|b| *b == b'&').count() > 64
        || source.contains("%%{")
        || source.trim_start().starts_with("---")
        || source
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return false;
    }
    let header = source
        .lines()
        .map(str::trim)
        .find(|s| !s.is_empty() && !s.starts_with("%%"))
        .and_then(|s| s.split(|c: char| c.is_whitespace() || c == ';').next())
        .unwrap_or_default();
    if !matches!(
        header,
        "flowchart"
            | "graph"
            | "sequenceDiagram"
            | "classDiagram"
            | "stateDiagram"
            | "stateDiagram-v2"
            | "erDiagram"
            | "pie"
            | "mindmap"
            | "timeline"
            | "journey"
            | "quadrantChart"
    ) {
        return false;
    }
    // A conservative lexical nesting guard bounds work before native parsing.
    let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
    for character in source.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if character == '"' {
            quoted = !quoted;
            continue;
        }
        if quoted {
            continue;
        }
        if matches!(character, '[' | '(' | '{') {
            depth += 1;
            if depth > 32 {
                return false;
            }
        } else if matches!(character, ']' | ')' | '}') {
            depth = depth.saturating_sub(1);
        }
    }
    !matches!(header, "flowchart" | "graph") || (depth == 0 && !quoted)
}

pub(crate) fn render(source: &str, light: bool, options: RasterOptions) -> Option<Raster> {
    let source = diagram_svg(source, light)?;
    svg::diagram(&source, options).ok()
}

fn diagram_svg(source: &str, light: bool) -> Option<String> {
    use mermaid_rs_renderer::{
        DiagramKind, LayoutConfig, Theme, compute_layout, parse_mermaid_strict, render_svg,
    };
    if !admissible(source) {
        return None;
    }
    let parsed = parse_mermaid_strict(source).ok()?;
    let graph = &parsed.graph;
    if parsed.init_config.is_some()
        || !matches!(
            graph.kind,
            DiagramKind::Flowchart
                | DiagramKind::Sequence
                | DiagramKind::Class
                | DiagramKind::State
                | DiagramKind::Er
                | DiagramKind::Pie
                | DiagramKind::Mindmap
                | DiagramKind::Timeline
                | DiagramKind::Journey
                | DiagramKind::Quadrant
        )
        || graph.nodes.len() > 128
        || graph.edges.len() > 256
        || graph.subgraphs.len() > 16
        || graph.subgraphs.iter().map(|g| g.nodes.len()).sum::<usize>() > 512
        || graph.sequence_participants.len() > 32
        || graph.sequence_frames.len() > 16
        || graph.sequence_notes.len() > 64
        || graph.sequence_activations.len() > 128
        || graph.sequence_boxes.len() > 16
        || graph.state_notes.len() > 64
        || !graph.node_links.is_empty()
        || graph
            .nodes
            .values()
            .any(|node| node.icon.is_some() || node.label.len() > 1024)
    {
        return None;
    }
    if !additional_structure_is_bounded(graph) {
        return None;
    }
    static FONTS_READY: std::sync::Once = std::sync::Once::new();
    FONTS_READY.call_once(|| mermaid_rs_renderer::set_font_database((**svg::fonts(true)).clone()));
    let mut theme = if light {
        Theme::modern()
    } else {
        Theme::dark()
    };
    if !light {
        theme.background = "#252a33".into();
        theme.primary_color = "#243447".into();
        theme.primary_border_color = "#63cce3".into();
        theme.primary_text_color = "#e3eaf2".into();
        theme.text_color = "#e3eaf2".into();
        theme.line_color = "#b1c3d4".into();
        theme.edge_label_background = theme.background.clone();
        theme.sequence_actor_fill = theme.primary_color.clone();
        theme.sequence_actor_border = theme.primary_border_color.clone();
    }
    theme.font_family = if source.is_ascii() {
        "KaTeX_SansSerif"
    } else {
        "Noto Sans"
    }
    .into();
    theme.font_size = 16.;
    if graph.kind == DiagramKind::Pie {
        theme.pie_colors = [
            "#67c9de", "#b3a1e8", "#f5bc73", "#9bd59a", "#ed9eac", "#91b8ee", "#e5cb82", "#d0a5ce",
            "#88cebb", "#c2cb89", "#ecb399", "#9fbac8",
        ]
        .map(str::to_owned);
        theme.pie_opacity = 1.;
        theme.pie_section_text_color = "#172033".into();
        theme.pie_title_text_color = theme.text_color.clone();
        theme.pie_legend_text_color = theme.text_color.clone();
        theme.pie_outer_stroke_color = theme.line_color.clone();
        theme.pie_stroke_color = theme.background.clone();
    }
    let mut config = LayoutConfig {
        fast_text_metrics: true,
        ..Default::default()
    };
    if graph.kind == DiagramKind::Er && graph.nodes.values().any(|n| n.label.contains('\n')) {
        // Attribute rows are records, not paragraphs: wrapping can change their
        // meaning when the backend splits type/name/key fields during drawing.
        config.max_label_width_chars = 1024;
        config.node_padding_x = 48.;
    }
    let layout = compute_layout(graph, &theme, &config);
    let svg = render_svg(&layout, &theme, &config);
    if graph.kind == DiagramKind::Er && !er_content_is_complete(graph, &svg) {
        return None;
    }
    Some(svg)
}

// The backend may hide a type column if it cannot fit. Never display an ER
// table that silently drops a field or key: count the parsed record tokens in
// the generated text, including repeated types and quoted comments.
fn er_content_is_complete(graph: &mermaid_rs_renderer::Graph, svg: &str) -> bool {
    let Ok(xml) = roxmltree::Document::parse(svg) else {
        return false;
    };
    let mut actual = std::collections::HashMap::<&str, usize>::new();
    for node in xml.descendants().filter(|n| n.is_text()) {
        for token in node
            .text()
            .unwrap_or_default()
            .split_whitespace()
            .flat_map(|s| s.split(','))
            .map(|s| s.trim_matches(';'))
        {
            *actual.entry(token).or_default() += 1;
        }
    }
    for node in graph.nodes.values() {
        for token in node
            .label
            .lines()
            .filter(|s| *s != "---")
            .flat_map(str::split_whitespace)
            .flat_map(|s| s.split(','))
        {
            let token = token.trim_matches(';');
            if token.is_empty() {
                continue;
            }
            let Some(count) = actual.get_mut(token) else {
                return false;
            };
            if *count == 0 {
                return false;
            }
            *count -= 1;
        }
    }
    true
}

fn additional_structure_is_bounded(graph: &mermaid_rs_renderer::Graph) -> bool {
    if graph.kind == mermaid_rs_renderer::DiagramKind::Journey {
        let mut actors = std::collections::HashSet::new();
        for node in graph.nodes.values() {
            if !node.value.is_some_and(|score| (1. ..=5.).contains(&score)) {
                return false;
            }
            actors.extend(
                node.label
                    .lines()
                    .skip(1)
                    .flat_map(|line| line.split(','))
                    .map(str::trim),
            );
        }
        if actors.len() > 32 {
            return false;
        }
    }
    let label = |s: &str| s.len() <= 1024;
    graph.pie_slices.len() <= 64
        && graph
            .pie_slices
            .iter()
            .all(|s| label(&s.label) && s.value.is_finite() && s.value >= 0.)
        && (graph.pie_slices.is_empty() || {
            let sum: f32 = graph.pie_slices.iter().map(|s| s.value).sum();
            sum.is_finite() && sum > 0.
        })
        && graph.mindmap.nodes.len() <= 128
        && graph
            .mindmap
            .nodes
            .iter()
            .all(|n| n.level <= 32 && n.icon.is_none() && label(&n.label))
        && graph.timeline.sections.is_empty()
        && graph.timeline.events.len() <= 64
        && graph
            .timeline
            .events
            .iter()
            .map(|e| e.events.len())
            .sum::<usize>()
            <= 128
        && graph
            .timeline
            .events
            .iter()
            .all(|e| label(&e.time) && e.events.iter().all(|s| label(s)))
        && graph.quadrant.points.len() <= 128
        && graph
            .quadrant
            .points
            .iter()
            .all(|p| label(&p.label) && (0. ..=1.).contains(&p.x) && (0. ..=1.).contains(&p.y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_PIXELS;
    fn render(source: &str, light: bool) -> Option<Raster> {
        super::render(source, light, RasterOptions::scaled(2.))
    }
    #[test]
    fn supported_diagrams_have_bounded_retina_rasters_in_both_themes() {
        for source in [
            "flowchart LR\n A[Start] --> B{Ready?}\n B -->|Yes| C[Done]\n",
            "sequenceDiagram\n Alice->>Bob: Hello\n Bob-->>Alice: Reply\n",
            "classDiagram\n class Note {\n +String title\n }\n Note --> Reader\n",
            "stateDiagram-v2\n [*] --> Ready\n Ready --> [*]\n",
            "erDiagram\n BOOK ||--o{ PAGE : contains\n",
            "flowchart TD\n subgraph Input\n A-->B\n end\n B-->C\n",
        ] {
            for light in [false, true] {
                let p = render(source, light).unwrap_or_else(|| panic!("Failed: {source}"));
                assert!(p.width > 0 && p.height > 0);
                assert!(p.width as usize * p.height as usize <= MAX_PIXELS);
                assert_eq!(p.rgba.len(), (p.width * p.height * 4) as usize);
                assert!(p.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
                assert!(p.width <= p.original_width * 2 && p.height <= p.original_height * 2);
            }
        }
    }
    #[test]
    fn unicode_attributes_and_more_diagrams_render_without_system_fonts() {
        for source in [
            "flowchart LR\n A[Café déjà vu] --> B[Ελληνικά Русский]",
            "flowchart LR\n A[Cafe\u{301}] --> B[naïve façade]",
            "sequenceDiagram\n participant A as André\n participant B as Zoë\n A->>B: prêt ?",
            "erDiagram\n BOOK {\n string title\n int id PK,FK,UK\n string résumé \"détail, précis\"\n }",
            "pie showData\n title Café\n \"Thé\": 3\n \"Café\": 7",
            "mindmap\n root((Reading))\n   Notes\n     Ideas\n   Diagrams",
            "timeline\n title History\n 2020 : Launch : Learn\n 2021 : Grow",
            "journey\n title Reading\n section Start\n Open file: 5: Reader\n Read: 4: Reader, Editor",
            "quadrantChart\n title Work\n x-axis Low effort --> High effort\n y-axis Low value --> High value\n Task A: [0.2, 0.8]\n Task B: [0.8, 0.3]",
        ] {
            for light in [false, true] {
                assert!(render(source, light).is_some(), "Failed: {source}");
            }
        }
    }

    #[test]
    fn er_tables_preserve_types_keys_and_comments() {
        let source = "erDiagram\n BOOK {\n int identifier PK,FK,UK\n varchar title \"display name\"\n decimal(10,2) price\n }\n BOOK ||--o{ PAGE : contains\n PAGE {\n int identifier PK\n int book_id FK\n }";
        let svg = diagram_svg(source, false).expect("complete attribute table");
        let xml = roxmltree::Document::parse(&svg).unwrap();
        let text: Vec<_> = xml
            .descendants()
            .filter(|n| n.is_text())
            .filter_map(|n| n.text())
            .collect();
        for field in [
            "int",
            "varchar",
            "decimal(10,2)",
            "PK",
            "FK",
            "UK",
            "identifier",
            "price",
            "book_id",
        ] {
            assert!(text.contains(&field), "Missing: {field}");
        }
        assert!(text.iter().any(|s| s.contains("display name")));
        // A deliberately incomplete renderer result must never be accepted.
        let graph = mermaid_rs_renderer::parse_mermaid_strict(source)
            .unwrap()
            .graph;
        assert!(!er_content_is_complete(
            &graph,
            &svg.replace(">varchar<", "><")
        ));
    }

    #[test]
    fn additional_diagram_collections_and_numbers_are_bounded() {
        for source in [
            format!(
                "pie\n{}",
                (0..65)
                    .map(|n| format!("\"Item {n}\": 1\n"))
                    .collect::<String>()
            ),
            format!(
                "timeline\n{}",
                (0..65)
                    .map(|n| format!("{n} : Event\n"))
                    .collect::<String>()
            ),
            format!(
                "quadrantChart\n{}",
                (0..129)
                    .map(|n| format!("P{n}: [0.2, 0.5]\n"))
                    .collect::<String>()
            ),
            format!(
                "mindmap\n{}",
                (0..34)
                    .map(|n| format!("{}N{n}\n", "  ".repeat(n)))
                    .collect::<String>()
            ),
            "pie\n\"Empty\": 0".into(),
            "quadrantChart\nOut: [2, 0]".into(),
            "timeline\nsection Hidden\n2026 : Event".into(),
            "journey\nTask: 6: Reader".into(),
            "journey\nTask: NaN: Reader".into(),
            "pie\n\"Infinite\": 1e39".into(),
        ] {
            assert!(render(&source, false).is_none(), "Accepted: {source}");
        }
    }

    #[test]
    fn unsupported_excessive_or_external_content_keeps_source() {
        for source in [
            "gantt\nsection Plan\nTask: 2025-01-01, 1d\n",
            "%%{init: {}}%%\nflowchart LR\nA-->B\n",
            "---\ntheme: dark\n---\nflowchart LR; A-->B",
            "flowchart LR\nA\x1b[2J-->B",
            "flowchart LR\nA[中文]-->B",
            "flowchart LR\nA[Broken",
            "flowchart LR\nA-->B\nclick A \"https://example.com\"\n",
        ] {
            assert!(render(source, false).is_none(), "Accepted: {source}");
        }
        assert!(!admissible(&format!(
            "flowchart LR\n{}",
            "A".repeat(MAX_SOURCE)
        )));
        assert!(!admissible(&format!("flowchart LR\n{}", "A;".repeat(257))));
        assert!(!admissible(&format!(
            "flowchart LR\nA{}B{}",
            "[".repeat(33),
            "]".repeat(33)
        )));
        let large = format!(
            "flowchart LR\n{}",
            (0..129)
                .map(|i| format!("N{i}[Node {i}]\n"))
                .collect::<String>()
        );
        assert!(render(&large, false).is_none());
        assert!(render("flowchart LR\nA-->B", false).is_some());
    }
}
