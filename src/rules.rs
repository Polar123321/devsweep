use std::path::Path;

/// The kinds of regenerable build/dependency directories devsweep knows about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Node,
    Rust,
    Python,
    PyCache,
    Web,
    Java,
    Gradle,
    Pods,
}

impl Kind {
    pub const ALL: [Kind; 8] = [
        Kind::Node,
        Kind::Rust,
        Kind::Python,
        Kind::PyCache,
        Kind::Web,
        Kind::Java,
        Kind::Gradle,
        Kind::Pods,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Node => "node",
            Kind::Rust => "rust",
            Kind::Python => "venv",
            Kind::PyCache => "pycache",
            Kind::Web => "web",
            Kind::Java => "maven",
            Kind::Gradle => "gradle",
            Kind::Pods => "pods",
        }
    }
}

/// Directories we never descend into: version control, OS/app data and tool
/// installs, where a `node_modules` or `target` is part of a working program.
pub const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "AppData",
    "Library",
    "Windows",
    "Program Files",
    "Program Files (x86)",
    "$RECYCLE.BIN",
    "System Volume Information",
    ".vscode",
    ".cursor",
    ".rustup",
    ".cargo",
    ".nvm",
    ".pyenv",
];

/// Decides whether `dir` is a regenerable artifact directory. A bare name like
/// `target` or `build` is only accepted when a sibling project file proves it
/// belongs to a build tool, so we never flag an unrelated folder.
pub fn classify(dir: &Path, name: &str) -> Option<Kind> {
    let parent = dir.parent()?;
    let sibling = |f: &str| parent.join(f).exists();
    match name {
        "node_modules" => Some(Kind::Node),
        "target" if sibling("Cargo.toml") => Some(Kind::Rust),
        "target" if sibling("pom.xml") => Some(Kind::Java),
        ".venv" | "venv" | "env" if dir.join("pyvenv.cfg").exists() => Some(Kind::Python),
        "__pycache__" => Some(Kind::PyCache),
        ".next" | ".nuxt" | ".svelte-kit" | ".turbo" | ".parcel-cache" | ".angular"
            if sibling("package.json") =>
        {
            Some(Kind::Web)
        }
        "build" | ".gradle"
            if sibling("build.gradle")
                || sibling("build.gradle.kts")
                || sibling("settings.gradle") =>
        {
            Some(Kind::Gradle)
        }
        "Pods" if sibling("Podfile") => Some(Kind::Pods),
        _ => None,
    }
}
