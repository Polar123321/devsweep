use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::Sender,
    time::SystemTime,
};

use crate::rules::{self, Kind};

pub struct Found {
    pub path: PathBuf,
    pub kind: Kind,
    pub modified: Option<SystemTime>,
}

pub enum Msg {
    /// A directory matched a rule; its size is still being measured.
    Found(Found),
    /// Size measurement for a previously found directory finished.
    Size(PathBuf, u64),
    /// Discovery and all size measurements are complete.
    Done,
}

/// Scans `root` on a background thread pool, streaming results over `tx`.
pub fn spawn(root: PathBuf, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        rayon::scope(|s| walk(root, s, tx.clone()));
        let _ = tx.send(Msg::Done);
    });
}

fn walk<'s>(dir: PathBuf, s: &rayon::Scope<'s>, tx: Sender<Msg>) {
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        // `DirEntry::file_type` does not follow symlinks, so links are ignored.
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let path = entry.path();

        if let Some(kind) = rules::classify(&path, name) {
            let modified = entry.metadata().ok().and_then(|m| m.modified().ok());
            let _ = tx.send(Msg::Found(Found {
                path: path.clone(),
                kind,
                modified,
            }));
            let tx = tx.clone();
            s.spawn(move |_| {
                let bytes = dir_size(&path);
                let _ = tx.send(Msg::Size(path, bytes));
            });
            continue; // never descend into an artifact dir
        }
        if rules::SKIP_DIRS.contains(&name) {
            continue;
        }
        let tx = tx.clone();
        s.spawn(move |s| walk(path, s, tx));
    }
}

fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                total += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    total
}
