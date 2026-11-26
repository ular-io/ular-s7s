//! Dev aid, not an assertion: renders one sample in every built-in theme to an
//! HTML page and dumps the `md_*` colors as JSON, so derived palettes can be
//! reviewed by eye and by script after a theme or derivation change.
//!
//! `cargo test markdown_theme_gallery -- --ignored --nocapture`

use super::render_markdown;
use crate::theme::{builtin_themes, color_rgb};
use ratatui::style::{Color, Modifier, Style};

const WIDTH: usize = 56;
const SAMPLE: &str = "\
# Heading one
## Heading two
Body with **bold**, *italic*, ~~strike~~, `inline code` and a [link](https://example.com).

- bullet item
  - nested item
1. ordered item
- [x] done task

| Name | Value |
| :--- | ---: |
| alpha | 12 |

> plain quote

> [!NOTE]
> note body

> [!TIP]
> tip body

> [!IMPORTANT]
> important body

> [!WARNING]
> warning body

> [!CAUTION]
> caution body

```rust
let answer = 42;
```
";

fn hex(c: Color) -> String {
    let (r, g, b) = color_rgb(c);
    format!("#{r:02X}{g:02X}{b:02X}")
}

fn css(st: Style) -> String {
    let mut out = String::new();
    for (prop, color) in [("color", st.fg), ("background", st.bg)] {
        if let Some(c) = color.filter(|c| *c != Color::Reset) {
            out += &format!("{prop}:{};", hex(c));
        }
    }
    for (modifier, rule) in [
        (Modifier::BOLD, "font-weight:bold;"),
        (Modifier::ITALIC, "font-style:italic;"),
        (Modifier::UNDERLINED, "text-decoration:underline;"),
        (Modifier::CROSSED_OUT, "text-decoration:line-through;"),
    ] {
        if st.add_modifier.contains(modifier) {
            out += rule;
        }
    }
    out
}

#[test]
#[ignore = "writes an HTML gallery and a color dump for manual review"]
fn markdown_theme_gallery() {
    let dir = std::env::temp_dir().join("s7s-markdown-gallery");
    std::fs::create_dir_all(&dir).expect("gallery dir");
    let mut html = String::from(
        "<!doctype html><meta charset=utf-8><style>body{margin:0;display:flex;flex-wrap:wrap}\
         .t{font:12px/14px Menlo,monospace;white-space:pre;padding:6px;width:56ch}\
         .n{font-weight:bold;text-decoration:underline}</style>",
    );
    let mut colors = serde_json::Map::new();
    for th in builtin_themes() {
        html += &format!(
            "<div class=t style='background:{};color:{}'><div class=n>{}</div>",
            hex(th.bg),
            hex(th.fg),
            th.key
        );
        for line in render_markdown(SAMPLE, WIDTH, &th) {
            for span in &line.spans {
                let text = span.content.replace('&', "&amp;").replace('<', "&lt;");
                html += &format!("<span style='{}'>{text}</span>", css(span.style));
            }
            html.push('\n');
        }
        html += "</div>";
        let md = th.md;
        let roles = [
            ("bg", th.bg),
            ("fg", th.fg),
            ("heading_bg", md.heading_bg),
            ("heading_fg", md.heading_fg),
            ("code_bg", md.code_bg),
            ("code_fg", md.code_fg),
            ("code_block_fg", md.code_block_fg),
            ("table_head", md.table_head),
            ("marker", md.marker),
            ("link", md.link),
            ("quote", md.quote),
            ("border", md.border),
            ("alert_fg", md.alert_fg),
            ("alert_note", md.alert_note),
            ("alert_tip", md.alert_tip),
            ("alert_important", md.alert_important),
            ("alert_warning", md.alert_warning),
            ("alert_caution", md.alert_caution),
        ];
        let mut entry: serde_json::Map<String, serde_json::Value> = roles
            .into_iter()
            .map(|(role, color)| (role.to_string(), hex(color).into()))
            .collect();
        entry.insert("dark".into(), th.dark.into());
        colors.insert(th.key.clone(), entry.into());
    }
    std::fs::write(dir.join("gallery.html"), html).expect("write gallery");
    let dump = serde_json::to_string_pretty(&colors).expect("serialize colors");
    std::fs::write(dir.join("colors.json"), dump).expect("write colors");
    println!("wrote {}", dir.display());
}
