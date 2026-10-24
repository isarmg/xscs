//! Explicit product-neutral runtime directory layout.

use std::path::{Component, Path, PathBuf};

/// Names reserved for installed executables, definitions and private runtime
/// state. Product business data must not overlap these root namespaces.
pub const ROOT_RESERVED_DIRECTORIES: [&str; 7] = [
    "bin",
    "libexec",
    "share",
    "config",
    "logs",
    "run",
    "maintenance",
];

/// One explicit installation root; never infer it from the process directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    pub fn new(root: &Path) -> anyhow::Result<Self> {
        anyhow::ensure!(
            root.to_str()
                .is_some_and(|name| !name.chars().any(char::is_control)),
            "installation root must be UTF-8 without control characters"
        );
        anyhow::ensure!(root.is_absolute(), "installation root must be absolute");
        anyhow::ensure!(
            root != Path::new("/"),
            "installation root cannot be the filesystem root"
        );
        anyhow::ensure!(
            root.components()
                .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
                && root.components().collect::<PathBuf>().as_os_str() == root.as_os_str(),
            "installation root must be normalized"
        );
        Ok(Self {
            root: root.to_path_buf(),
        })
    }
    pub fn config(&self) -> PathBuf {
        self.root.join("config/config.json")
    }
    pub fn state(&self) -> PathBuf {
        self.root.join("state")
    }
    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }
    pub fn run(&self) -> PathBuf {
        self.root.join("run")
    }
}
