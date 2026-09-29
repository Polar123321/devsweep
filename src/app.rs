use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    time::{Duration, SystemTime},
};

use ratatui::widgets::TableState;
use rayon::prelude::*;

use crate::{
    rules::{self, Kind},
    scan::{self, Msg},
};

#[derive(Clone, PartialEq)]
pub enum Status {
    Idle,
    Deleting,
    Deleted,
    Failed(String),
}

pub struct Item {
    pub path: PathBuf,
    pub kind: Kind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub selected: bool,
    pub status: Status,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Sort {
    Size,
    Age,
    Path,
}

impl Sort {
    pub fn next(self) -> Self {
        match self {
            Sort::Size => Sort::Age,
            Sort::Age => Sort::Path,
            Sort::Path => Sort::Size,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Sort::Size => "size",
            Sort::Age => "age",
            Sort::Path => "path",
        }
    }
}

#[derive(PartialEq)]
pub enum Mode {
    Normal,
    Confirm,
}

struct Deleted(PathBuf, Result<(), String>);

pub struct App {
    pub root: PathBuf,
    pub items: Vec<Item>,
    /// Index into `items` (not into the visible list), so the cursor stays on
    /// the same row while results stream in and the view reorders.
    pub cursor: usize,
    pub table: TableState,
    pub sort: Sort,
    pub filter: Option<Kind>,
    pub mode: Mode,
    pub scanning: bool,
    pub freed: u64,
    pub dry_run: bool,
    pub tick: usize,
    pub quit: bool,
    older_than: Option<Duration>,
    scan_rx: Receiver<Msg>,
    del_tx: Sender<Deleted>,
    del_rx: Receiver<Deleted>,
}

impl App {
    pub fn new(root: PathBuf, dry_run: bool, older_than_days: Option<u64>) -> Self {
        let (scan_tx, scan_rx) = mpsc::channel();
        scan::spawn(root.clone(), scan_tx);
        let (del_tx, del_rx) = mpsc::channel();
        Self {
            root,
            items: Vec::new(),
            cursor: 0,
            table: TableState::default(),
            sort: Sort::Size,
            filter: None,
            mode: Mode::Normal,
            scanning: true,
            freed: 0,
            dry_run,
            tick: 0,
            quit: false,
            older_than: older_than_days.map(|d| Duration::from_secs(d * 86_400)),
            scan_rx,
            del_tx,
            del_rx,
        }
    }

    /// Drains pending scan and delete messages. Call once per frame.
    pub fn poll(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        loop {
            match self.scan_rx.try_recv() {
                Ok(Msg::Found(f)) => {
                    if self.too_recent(f.modified) {
                        continue;
                    }
                    self.items.push(Item {
                        path: f.path,
                        kind: f.kind,
                        size: None,
                        modified: f.modified,
                        selected: false,
                        status: Status::Idle,
                    });
                }
                Ok(Msg::Size(path, bytes)) => {
                    if let Some(it) = self.items.iter_mut().find(|i| i.path == path) {
                        it.size = Some(bytes);
                    }
                }
                Ok(Msg::Done) => self.scanning = false,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.scanning = false;
                    break;
                }
            }
        }
        while let Ok(Deleted(path, result)) = self.del_rx.try_recv() {
            let Some(it) = self.items.iter_mut().find(|i| i.path == path) else {
                continue;
            };
            it.selected = false;
            match result {
                Ok(()) => {
                    self.freed += it.size.unwrap_or(0);
                    it.status = Status::Deleted;
                }
                Err(e) => it.status = Status::Failed(e),
            }
        }
    }

    fn too_recent(&self, modified: Option<SystemTime>) -> bool {
        let (Some(min_age), Some(modified)) = (self.older_than, modified) else {
            return false;
        };
        SystemTime::now()
            .duration_since(modified)
            .map(|age| age < min_age)
            .unwrap_or(true)
    }

    /// Indices into `items` that pass the filter, in display order.
    pub fn visible(&self) -> Vec<usize> {
        let mut v: Vec<usize> = (0..self.items.len())
            .filter(|&i| self.filter.is_none_or(|k| self.items[i].kind == k))
            .collect();
        match self.sort {
            Sort::Size => v.sort_by_key(|&i| std::cmp::Reverse(self.items[i].size.unwrap_or(0))),
            Sort::Age => {
                v.sort_by_key(|&i| (self.items[i].modified.is_none(), self.items[i].modified))
            }
            Sort::Path => v.sort_by(|&a, &b| self.items[a].path.cmp(&self.items[b].path)),
        }
        v
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let vis = self.visible();
        if vis.is_empty() {
            return;
        }
        let pos = vis.iter().position(|&i| i == self.cursor).unwrap_or(0) as isize;
        let next = (pos + delta).clamp(0, vis.len() as isize - 1) as usize;
        self.cursor = vis[next];
    }

    pub fn jump_top(&mut self) {
        if let Some(&i) = self.visible().first() {
            self.cursor = i;
        }
    }

    pub fn jump_bottom(&mut self) {
        if let Some(&i) = self.visible().last() {
            self.cursor = i;
        }
    }

    pub fn toggle_current(&mut self) {
        if let Some(it) = self.items.get_mut(self.cursor)
            && it.status == Status::Idle
        {
            it.selected = !it.selected;
        }
        self.move_cursor(1);
    }

    pub fn select_all(&mut self) {
        for i in self.visible() {
            if self.items[i].status == Status::Idle {
                self.items[i].selected = true;
            }
        }
    }

    pub fn clear_selection(&mut self) {
        for it in &mut self.items {
            it.selected = false;
        }
    }

    pub fn cycle_sort(&mut self) {
        self.sort = self.sort.next();
    }

    pub fn cycle_filter(&mut self) {
        self.filter = match self.filter {
            None => Some(Kind::ALL[0]),
            Some(k) => {
                let idx = Kind::ALL.iter().position(|&x| x == k).unwrap_or(0);
                Kind::ALL.get(idx + 1).copied()
            }
        };
        let vis = self.visible();
        if !vis.contains(&self.cursor) {
            self.cursor = vis.first().copied().unwrap_or(0);
        }
    }

    /// Selected, still-deletable, currently visible items.
    pub fn targets(&self) -> Vec<usize> {
        self.visible()
            .into_iter()
            .filter(|&i| self.items[i].selected && self.items[i].status == Status::Idle)
            .collect()
    }

    pub fn targets_size(&self) -> u64 {
        self.targets()
            .iter()
            .map(|&i| self.items[i].size.unwrap_or(0))
            .sum()
    }

    pub fn reclaimable(&self) -> u64 {
        self.items
            .iter()
            .filter(|i| i.status == Status::Idle)
            .map(|i| i.size.unwrap_or(0))
            .sum()
    }

    /// Opens the confirmation dialog. With nothing selected, the row under the
    /// cursor is used so a quick `d` still does something sensible.
    pub fn request_delete(&mut self) {
        if self.targets().is_empty()
            && let Some(it) = self.items.get_mut(self.cursor)
            && it.status == Status::Idle
        {
            it.selected = true;
        }
        if !self.targets().is_empty() {
            self.mode = Mode::Confirm;
        }
    }

    pub fn confirm_delete(&mut self) {
        self.mode = Mode::Normal;
        let idxs = self.targets();
        let paths: Vec<PathBuf> = idxs
            .iter()
            .map(|&i| {
                self.items[i].status = Status::Deleting;
                self.items[i].path.clone()
            })
            .collect();
        let tx = self.del_tx.clone();
        let dry = self.dry_run;
        std::thread::spawn(move || {
            paths.par_iter().for_each_with(tx, |tx, p| {
                let _ = tx.send(Deleted(p.clone(), delete(p, dry)));
            });
        });
    }
}

/// Re-validates the path right before removal, so a directory that changed
/// since the scan (renamed, replaced by a symlink) is never deleted blindly.
fn delete(path: &Path, dry_run: bool) -> Result<(), String> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if rules::classify(path, name).is_none() {
        return Err("no longer looks like a build directory".into());
    }
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() {
        return Err("is a symlink, skipped".into());
    }
    if dry_run {
        return Ok(());
    }
    fs::remove_dir_all(path).map_err(|e| e.to_string())
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::{thread::sleep, time::Instant};

    pub fn fixture(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("devsweep-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let proj = root.join("proj");
        fs::create_dir_all(proj.join("target/debug")).unwrap();
        fs::write(proj.join("Cargo.toml"), "[package]").unwrap();
        fs::write(proj.join("target/debug/blob"), vec![0u8; 4096]).unwrap();
        fs::create_dir_all(root.join("plain/target")).unwrap(); // no Cargo.toml: must be ignored
        root
    }

    pub fn wait_scan(app: &mut App) {
        let t = Instant::now();
        while app.scanning || app.items.iter().any(|i| i.size.is_none()) {
            app.poll();
            assert!(t.elapsed() < Duration::from_secs(10), "scan timed out");
            sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn scans_and_deletes() {
        let root = fixture("delete");
        let mut app = App::new(root.clone(), false, None);
        wait_scan(&mut app);
        assert_eq!(
            app.items.len(),
            1,
            "only the real cargo target should match"
        );
        assert_eq!(app.items[0].size, Some(4096));

        app.request_delete(); // nothing selected: falls back to the cursor row
        assert!(app.mode == Mode::Confirm);
        app.confirm_delete();
        let t = Instant::now();
        while app.items[0].status != Status::Deleted {
            app.poll();
            assert!(t.elapsed() < Duration::from_secs(10), "delete timed out");
        }
        assert_eq!(app.freed, 4096);
        assert!(!root.join("proj/target").exists());
        assert!(root.join("plain/target").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn dry_run_keeps_files() {
        let root = fixture("dry");
        let mut app = App::new(root.clone(), true, None);
        wait_scan(&mut app);
        app.select_all();
        app.request_delete();
        app.confirm_delete();
        while app.items[0].status != Status::Deleted {
            app.poll();
        }
        assert!(root.join("proj/target/debug/blob").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn older_than_hides_fresh_dirs() {
        let root = fixture("age");
        let mut app = App::new(root.clone(), false, Some(30));
        wait_scan(&mut app);
        assert!(app.items.is_empty());
        let _ = fs::remove_dir_all(root);
    }
}
