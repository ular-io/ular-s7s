use super::render_markdown;
use crate::theme::{resolve, Theme};
use ratatui::style::Modifier;
use ratatui::text::Line;

fn ular_light() -> Theme {
    resolve("ular-light")
}

fn render(md: &str, width: usize) -> (Vec<Line<'static>>, Vec<String>) {
    let lines = render_markdown(md, width, &ular_light());
    let text = lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
    (lines, text)
}

#[test]
fn markdown_markers_are_removed() {
    let (_, text) = render(
        "## Title\n\n**bold** `code` ~~gone~~ [link](https://example.com)\n\n- [x] done\n",
        80,
    );
    let all = text.join("\n");
    for marker in ["#", "**", "`", "~~", "](", "https://", "[x]"] {
        assert!(!all.contains(marker), "{marker:?} left in {all:?}");
    }
    assert!(all.contains("[✓] done"));
}

#[test]
fn headings_fill_the_width_on_a_bar_indented_by_level() {
    let th = ular_light();
    let (lines, text) = render("# One\n\n## Two\n\n### Three\n", 20);
    let headings: Vec<_> = lines
        .iter()
        .zip(&text)
        .filter(|(l, _)| l.width() > 0)
        .collect();
    let indents: Vec<usize> = headings
        .iter()
        .map(|(_, t)| t.len() - t.trim_start().len())
        .collect();
    assert_eq!(indents, [1, 3, 5]);
    for (line, _) in headings {
        assert_eq!(line.width(), 20);
        for span in &line.spans {
            assert_eq!(span.style.bg, Some(th.md.heading_bg));
            assert_eq!(span.style.fg, Some(th.md.heading_fg));
        }
    }
}

#[test]
fn inline_code_is_a_tinted_chip_padded_with_nbsp() {
    let th = ular_light();
    let (lines, text) = render("run `ls` now", 40);
    assert_eq!(text, ["run \u{a0}ls\u{a0} now"]);
    let chip = lines[0]
        .spans
        .iter()
        .find(|s| s.content.contains("ls"))
        .expect("code span");
    assert_eq!(chip.style.fg, Some(th.md.code_fg));
    assert_eq!(chip.style.bg, Some(th.md.code_bg));
}

#[test]
fn lists_use_dots_and_hang_wrapped_lines_under_the_text() {
    let (_, text) = render("- alpha beta gamma\n  - nested\n", 10);
    assert_eq!(text, ["• alpha", "  beta", "  gamma", "  ◦ nested"]);
}

#[test]
fn ordered_and_task_lists_keep_numbers_and_boxes() {
    let (_, ordered) = render("3. x\n4. y\n", 20);
    assert_eq!(ordered, ["3. x", "4. y"]);
    let (_, tasks) = render("- [x] done\n- [ ] todo\n", 20);
    assert_eq!(tasks, ["[✓] done", "[ ] todo"]);
}

#[test]
fn tables_are_boxed_and_honor_alignment() {
    let th = ular_light();
    let (lines, text) = render("| a | b |\n|:--|--:|\n| x | 12 |\n", 40);
    assert_eq!(
        text,
        [
            "┌───┬────┐",
            "│ a │  b │",
            "├───┼────┤",
            "│ x │ 12 │",
            "└───┴────┘"
        ]
    );
    let head = lines[1]
        .spans
        .iter()
        .find(|s| s.content == "a")
        .expect("head");
    assert_eq!(head.style.fg, Some(th.md.table_head));
    assert!(head.style.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn wide_tables_shrink_columns_and_wrap_cells_within_the_width() {
    let md = "| 항목 | 설명 |\n| --- | --- |\n| 한글 폭 계산 | unicode-width 0.2 라서 같은 결과가 나온다 |\n";
    for width in [24, 30, 40] {
        let (lines, _) = render(md, width);
        assert!(lines.len() > 5, "cells should wrap at {width}");
        assert!(
            lines.iter().all(|l| l.width() <= width),
            "overflow at {width}"
        );
    }
}

#[test]
fn code_blocks_are_framed_and_wrap_under_the_gutter() {
    let th = ular_light();
    let (lines, text) = render("```rust\nlet x = 1;\n    indented\n```\n", 40);
    assert_eq!(text, ["╭─ rust", "│ let x = 1;", "│     indented", "╰─"]);
    assert_eq!(lines[1].spans[1].style.fg, Some(th.md.code_block_fg));
    let (_, wrapped) = render("```\nlet value = compute(alpha, beta);\n```\n", 14);
    assert!(wrapped.len() > 3);
    assert!(wrapped[1..wrapped.len() - 1]
        .iter()
        .all(|l| l.starts_with("│ ")));
}

#[test]
fn quotes_rules_and_links_render_without_markup() {
    let th = ular_light();
    let (_, quote) = render("> quoted line\n", 20);
    assert_eq!(quote, ["▎ quoted line"]);
    let (_, rule) = render("---\n", 6);
    assert_eq!(rule, ["──────"]);
    let (lines, link) = render("see [s7s](https://example.com)", 40);
    assert_eq!(link, ["see s7s"]);
    let span = lines[0]
        .spans
        .iter()
        .find(|s| s.content == "s7s")
        .expect("link");
    assert_eq!(span.style.fg, Some(th.md.link));
    assert!(span.style.add_modifier.contains(Modifier::UNDERLINED));
}

#[test]
fn lines_never_exceed_the_width_or_split_graphemes() {
    let family = "👨‍👩‍👧‍👦";
    let md = format!(
        "# 결론 제목이 길어서 줄이 넘어간다\n\n한글과 English words가 섞인 긴 문장이다 {family}{family}{family}\n\n- 목록 항목 `inline code` 와 **굵게**\n\n> 인용문도 접힌다\n"
    );
    for width in 6..60 {
        let (lines, text) = render(&md, width);
        assert!(
            lines.iter().all(|l| l.width() <= width),
            "overflow at {width}"
        );
        assert!(
            lines.last().is_some_and(|l| l.width() > 0),
            "trailing blank at {width}"
        );
        assert_eq!(
            text.join("").matches(family).count(),
            3,
            "emoji split at {width}"
        );
    }
}

#[test]
fn br_breaks_the_line_inside_table_cells_and_paragraphs() {
    let (_, table) = render("| a | b |\n|---|---|\n| one<br>three | x<BR/>y |\n", 40);
    assert_eq!(
        table,
        [
            "┌───────┬───┐",
            "│ a     │ b │",
            "├───────┼───┤",
            "│ one   │ x │",
            "│ three │ y │",
            "└───────┴───┘"
        ]
    );
    let (_, paragraph) = render("first<br>second", 40);
    assert_eq!(paragraph, ["first", "second"]);
}

#[test]
fn gfm_alerts_get_a_label_chip_and_a_bar_by_weight() {
    let th = ular_light();
    let (lines, text) = render("> [!NOTE]\n> read this\n\n> [!WARNING]\n> careful\n", 40);
    assert_eq!(
        text,
        [
            "▎ \u{a0}NOTE\u{a0}",
            "▎ read this",
            "",
            "█ \u{a0}WARNING\u{a0}",
            "█ careful"
        ]
    );
    let (bar, chip) = (&lines[3].spans[0], &lines[3].spans[1]);
    assert_eq!(bar.style.fg, Some(th.md.alert_warning));
    assert_eq!(chip.style.bg, Some(th.md.alert_warning));
    assert_eq!(chip.style.fg, Some(th.md.alert_fg));
    assert!(chip.style.add_modifier.contains(Modifier::BOLD));
    // Alert bodies use the default text color, unlike plain quotes.
    assert_eq!(lines[4].spans[1].style.fg, None);
}
