use std::time::SystemTime;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Cell, Clear, Paragraph, Row, Table},
};

use crate::{
    app::{App, Mode, Status},
    rules::Kind,
};

const ACCENT: Color = Color::Rgb(137, 180, 250);
const DIM: Color = Color::Rgb(108, 112, 134);
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn kind_color(k: Kind) -> Color {
    match k {
        Kind::Node => Color::Rgb(166, 227, 161),
        Kind::Rust => Color::Rgb(250, 179, 135),
        Kind::Python => Color::Rgb(249, 226, 175),
        Kind::PyCache => Color::Rgb(223, 200, 140),
        Kind::Web => Color::Rgb(148, 226, 213),
        Kind::Java => Color::Rgb(243, 139, 168),
        Kind::Gradle => Color::Rgb(116, 199, 236),
        Kind::Pods => Color::Rgb(203, 166, 247),
    }
}

pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

pub fn age(modified: Option<SystemTime>) -> String {
    let Some(secs) = modified
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .map(|d| d.as_secs())
    else {
        return "?".into();
    };
    let days = secs / 86_400;
    match days {
        0 => "today".into(),
        1..=59 => format!("{days}d"),
        60..=729 => format!("{}mo", days / 30),
        _ => format!("{}y", days / 365),
    }
}

fn bar(size: u64, max: u64, width: usize) -> String {
    if max == 0 {
        return "░".repeat(width);
    }
    let filled = ((size as f64 / max as f64) * width as f64).ceil() as usize;
    let filled = filled.min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(f.area());

    draw_header(f, app, header);
    draw_table(f, app, body);
    draw_footer(f, footer);

    if app.mode == Mode::Confirm {
        draw_confirm(f, app);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let status = if app.scanning {
        Span::styled(
            format!(" {} scanning ", SPINNER[app.tick / 2 % SPINNER.len()]),
            Style::new().fg(ACCENT),
        )
    } else {
        Span::styled(" ✓ done ", Style::new().fg(Color::Green))
    };
    let mut title = vec![
        Span::styled(
            " devsweep ",
            Style::new().fg(Color::Black).bg(ACCENT).bold(),
        ),
        Span::styled(format!(" {} ", app.root.display()), Style::new().fg(DIM)),
    ];
    if app.dry_run {
        title.push(Span::styled(
            " DRY RUN ",
            Style::new().fg(Color::Black).bg(Color::Yellow).bold(),
        ));
    }

    let targets = app.targets();
    let stats = Line::from(vec![
        Span::raw(" found "),
        Span::styled(app.items.len().to_string(), Style::new().bold()),
        Span::styled("  ·  ", Style::new().fg(DIM)),
        Span::raw("reclaimable "),
        Span::styled(
            human(app.reclaimable()),
            Style::new().fg(Color::Yellow).bold(),
        ),
        Span::styled("  ·  ", Style::new().fg(DIM)),
        Span::raw("selected "),
        Span::styled(
            format!("{} ({})", human(app.targets_size()), targets.len()),
            Style::new().fg(Color::Cyan).bold(),
        ),
        Span::styled("  ·  ", Style::new().fg(DIM)),
        Span::raw("freed "),
        Span::styled(human(app.freed), Style::new().fg(Color::Green).bold()),
    ]);

    let block = Block::bordered()
        .border_style(Style::new().fg(DIM))
        .title(Line::from(title))
        .title(Line::from(status).right_aligned());
    f.render_widget(Paragraph::new(stats).block(block), area);
}

fn draw_table(f: &mut Frame, app: &mut App, area: Rect) {
    let vis = app.visible();
    let max = app.items.iter().filter_map(|i| i.size).max().unwrap_or(0);

    let rows: Vec<Row> = vis
        .iter()
        .map(|&i| {
            let it = &app.items[i];
            let path = it
                .path
                .strip_prefix(&app.root)
                .unwrap_or(&it.path)
                .display()
                .to_string();
            let size = it.size.map(human).unwrap_or_else(|| "…".into());
            let bar_str = it.size.map(|s| bar(s, max, 10)).unwrap_or_default();

            let (mark, row_style, tail) = match &it.status {
                Status::Idle => (if it.selected { "[x]" } else { "[ ]" }, Style::new(), None),
                Status::Deleting => (
                    "[~]",
                    Style::new().fg(DIM),
                    Some(("deleting…".to_string(), DIM)),
                ),
                Status::Deleted => (
                    "[✓]",
                    Style::new().fg(DIM).add_modifier(Modifier::CROSSED_OUT),
                    None,
                ),
                Status::Failed(e) => (
                    "[!]",
                    Style::new().fg(Color::Red),
                    Some((e.clone(), Color::Red)),
                ),
            };
            let path_cell = match tail {
                Some((msg, c)) => Line::from(vec![
                    Span::raw(path),
                    Span::styled(format!("  {msg}"), Style::new().fg(c).not_crossed_out()),
                ]),
                None => Line::from(path),
            };
            Row::new(vec![
                Cell::from(mark).style(if it.selected {
                    Style::new().fg(Color::Cyan).bold()
                } else {
                    Style::new().fg(DIM)
                }),
                Cell::from(it.kind.label()).style(Style::new().fg(kind_color(it.kind))),
                Cell::from(Line::from(size).right_aligned()),
                Cell::from(bar_str).style(Style::new().fg(kind_color(it.kind))),
                Cell::from(Line::from(age(it.modified)).right_aligned())
                    .style(Style::new().fg(DIM)),
                Cell::from(path_cell),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(6),
        Constraint::Min(10),
    ];
    let header = Row::new(vec![
        Cell::from(""),
        Cell::from("KIND"),
        Cell::from(Line::from("SIZE").right_aligned()),
        Cell::from(""),
        Cell::from(Line::from("AGE").right_aligned()),
        Cell::from("PATH"),
    ])
    .style(Style::new().fg(DIM).bold());

    let filter = app.filter.map(|k| k.label()).unwrap_or("all");
    let block = Block::bordered()
        .border_style(Style::new().fg(DIM))
        .title(Line::from(vec![
            Span::styled(" sort: ", Style::new().fg(DIM)),
            Span::styled(app.sort.label(), Style::new().fg(ACCENT).bold()),
            Span::styled("  filter: ", Style::new().fg(DIM)),
            Span::styled(filter, Style::new().fg(ACCENT).bold()),
            Span::raw(" "),
        ]));

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1)
        .row_highlight_style(
            Style::new()
                .bg(Color::Rgb(49, 50, 68))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▌");

    app.table.select(vis.iter().position(|&i| i == app.cursor));
    f.render_stateful_widget(table, area, &mut app.table);

    if vis.is_empty() {
        let msg = if app.scanning {
            "looking for build directories…"
        } else {
            "nothing to sweep here — your disk is already clean"
        };
        let inner = Rect {
            x: area.x + 1,
            y: area.y + area.height / 2,
            width: area.width.saturating_sub(2),
            height: 1,
        };
        f.render_widget(
            Paragraph::new(msg).alignment(Alignment::Center).fg(DIM),
            inner,
        );
    }
}

fn draw_footer(f: &mut Frame, area: Rect) {
    let keys = [
        ("↑↓", "move"),
        ("space", "select"),
        ("a", "all"),
        ("n", "none"),
        ("d", "delete"),
        ("s", "sort"),
        ("tab", "filter"),
        ("q", "quit"),
    ];
    let mut spans = Vec::new();
    for (k, label) in keys {
        spans.push(Span::styled(
            format!(" {k} "),
            Style::new().fg(Color::Black).bg(DIM),
        ));
        spans.push(Span::styled(format!(" {label} "), Style::new().fg(DIM)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_confirm(f: &mut Frame, app: &App) {
    let n = app.targets().len();
    let size = human(app.targets_size());
    let verb = if app.dry_run {
        "Simulate deleting"
    } else {
        "Permanently delete"
    };
    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw(format!("{verb} ")),
            Span::styled(format!("{n}"), Style::new().bold()),
            Span::raw(if n == 1 {
                " directory ("
            } else {
                " directories ("
            }),
            Span::styled(size, Style::new().fg(Color::Yellow).bold()),
            Span::raw(")?"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(" y ", Style::new().fg(Color::Black).bg(Color::Green)),
            Span::raw(" yes    "),
            Span::styled(" n ", Style::new().fg(Color::Black).bg(Color::Red)),
            Span::raw(" no"),
        ]),
    ];
    let area = centered(f.area(), 54, 7);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(text).alignment(Alignment::Center).block(
            Block::bordered()
                .border_style(Style::new().fg(Color::Yellow))
                .title(" confirm "),
        ),
        area,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [v] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [h] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(v);
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::{fixture, wait_scan};
    use ratatui::{Terminal, backend::TestBackend};

    fn render(app: &mut App) -> String {
        let mut term = Terminal::new(TestBackend::new(90, 12)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_list_and_confirm_dialog() {
        let root = fixture("ui");
        let mut app = App::new(root.clone(), true, None);
        wait_scan(&mut app);
        let screen = render(&mut app);
        println!("{screen}");
        assert!(screen.contains("devsweep") && screen.contains("DRY RUN"));
        assert!(screen.contains("rust") && screen.contains("4.0 KB"));

        app.request_delete();
        let screen = render(&mut app);
        println!("{screen}");
        assert!(screen.contains("confirm"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human(0), "0 B");
        assert_eq!(human(1536), "1.5 KB");
        assert_eq!(human(3 * 1024 * 1024 * 1024), "3.0 GB");
    }
}
