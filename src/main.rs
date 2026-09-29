mod app;
mod rules;
mod scan;
mod ui;

use std::{io, path::PathBuf, sync::mpsc, time::Duration};

use clap::Parser;
use ratatui::{
    DefaultTerminal,
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
};

use app::{App, Mode};

/// Find and sweep away node_modules, target/, venvs and other build junk.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Directory to scan (defaults to the current directory)
    path: Option<PathBuf>,

    /// Go through the motions without deleting anything
    #[arg(long)]
    dry_run: bool,

    /// Only show directories not modified in the last DAYS days
    #[arg(long, value_name = "DAYS")]
    older_than: Option<u64>,

    /// Print the results and exit instead of opening the TUI
    #[arg(long)]
    list: bool,
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();
    let root = cli
        .path
        .unwrap_or_else(|| PathBuf::from("."))
        .canonicalize()?;
    let root = strip_verbatim(root);

    if cli.list {
        return list(root, cli.older_than);
    }

    let app = App::new(root, cli.dry_run, cli.older_than);
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, app);
    ratatui::restore();
    result
}

/// `canonicalize` on Windows yields `\\?\C:\...`, which is ugly to display.
fn strip_verbatim(p: PathBuf) -> PathBuf {
    match p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) => PathBuf::from(rest),
        None => p,
    }
}

fn run(terminal: &mut DefaultTerminal, mut app: App) -> io::Result<()> {
    while !app.quit {
        app.poll();
        terminal.draw(|f| ui::draw(f, &mut app))?;

        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        // Windows also reports key releases; only act on presses.
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            break;
        }
        match app.mode {
            Mode::Normal => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => app.quit = true,
                KeyCode::Up | KeyCode::Char('k') => app.move_cursor(-1),
                KeyCode::Down | KeyCode::Char('j') => app.move_cursor(1),
                KeyCode::PageUp => app.move_cursor(-10),
                KeyCode::PageDown => app.move_cursor(10),
                KeyCode::Home | KeyCode::Char('g') => app.jump_top(),
                KeyCode::End | KeyCode::Char('G') => app.jump_bottom(),
                KeyCode::Char(' ') => app.toggle_current(),
                KeyCode::Char('a') => app.select_all(),
                KeyCode::Char('n') => app.clear_selection(),
                KeyCode::Char('s') => app.cycle_sort(),
                KeyCode::Tab => app.cycle_filter(),
                KeyCode::Char('d') | KeyCode::Delete | KeyCode::Enter => app.request_delete(),
                _ => {}
            },
            Mode::Confirm => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => app.confirm_delete(),
                KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q') => app.mode = Mode::Normal,
                _ => {}
            },
        }
    }
    Ok(())
}

/// Non-interactive mode: scan, print a table sorted by size, exit.
fn list(root: PathBuf, older_than: Option<u64>) -> io::Result<()> {
    let (tx, rx) = mpsc::channel();
    scan::spawn(root.clone(), tx);

    let mut items: Vec<(scan::Found, u64)> = Vec::new();
    let mut pending: Vec<scan::Found> = Vec::new();
    while let Ok(msg) = rx.recv() {
        match msg {
            scan::Msg::Found(f) => pending.push(f),
            scan::Msg::Size(path, bytes) => {
                if let Some(pos) = pending.iter().position(|f| f.path == path) {
                    items.push((pending.swap_remove(pos), bytes));
                }
            }
            scan::Msg::Done => break,
        }
    }

    if let Some(days) = older_than {
        let min_age = Duration::from_secs(days * 86_400);
        items.retain(|(f, _)| {
            f.modified
                .and_then(|m| std::time::SystemTime::now().duration_since(m).ok())
                .is_some_and(|age| age >= min_age)
        });
    }
    items.sort_by_key(|(_, size)| std::cmp::Reverse(*size));

    let total: u64 = items.iter().map(|(_, s)| s).sum();
    for (f, size) in &items {
        let rel = f.path.strip_prefix(&root).unwrap_or(&f.path);
        println!(
            "{:>10}  {:>6}  {:<8} {}",
            ui::human(*size),
            ui::age(f.modified),
            f.kind.label(),
            rel.display()
        );
    }
    println!(
        "\n{} directories, {} reclaimable",
        items.len(),
        ui::human(total)
    );
    Ok(())
}
