//! Parallel filesystem walk into a shared [`Arena`], with live progress.

use crate::arena::{Arena, NodeId};
use anyhow::{Context, Result};
use jwalk::{Parallelism, WalkDir};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[derive(Debug, Clone)]
pub struct Options {
    /// `st_size` instead of `st_blocks * 512` (what baobab/GNOME shows).
    pub apparent_size: bool,
    /// Do not cross filesystem boundaries (`du -x`).
    pub one_file_system: bool,
    /// Glob patterns: matched against the basename, or path prefix if absolute.
    pub excludes: Vec<String>,
    pub jobs: Option<usize>,
    /// How many of the biggest files to remember for the "top files" view.
    pub top_n: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            apparent_size: false,
            one_file_system: true,
            excludes: Vec::new(),
            jobs: None,
            top_n: 500,
        }
    }
}

/// Pseudo filesystems that are never interesting and can be dangerous to walk.
const ALWAYS_SKIP: &[&str] = &["/proc", "/sys", "/dev", "/run"];

pub struct Progress {
    pub bytes: AtomicU64,
    pub entries: AtomicU64,
    pub dirs: AtomicU64,
    pub errors: AtomicU64,
    pub done: AtomicBool,
    pub elapsed_ms: AtomicU64,
    current: Mutex<String>,
}

impl Progress {
    fn new(start: &str) -> Self {
        Self {
            bytes: AtomicU64::new(0),
            entries: AtomicU64::new(0),
            dirs: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            done: AtomicBool::new(false),
            elapsed_ms: AtomicU64::new(0),
            current: Mutex::new(start.to_string()),
        }
    }

    pub fn current(&self) -> String {
        self.current.lock().map(|c| c.clone()).unwrap_or_default()
    }

    pub fn snapshot(&self) -> (u64, u64, u64, u64, bool) {
        (
            self.bytes.load(Ordering::Relaxed),
            self.entries.load(Ordering::Relaxed),
            self.dirs.load(Ordering::Relaxed),
            self.errors.load(Ordering::Relaxed),
            self.done.load(Ordering::Relaxed),
        )
    }
}

pub struct Scan {
    pub root: PathBuf,
    pub opts: Options,
    pub arena: Mutex<Arena>,
    pub prog: Progress,
    top: Mutex<BinaryHeap<Reverse<(u64, PathBuf)>>>,
    error: Mutex<Option<String>>,
    root_dev: u64,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl Scan {
    /// Create the scan state; the walk itself is started by [`Scan::start_thread`].
    pub fn new(root: &Path, opts: Options) -> Result<Arc<Self>> {
        let meta = std::fs::symlink_metadata(root)
            .with_context(|| format!("cannot read {}", root.display()))?;
        if !meta.is_dir() {
            anyhow::bail!("{} is not a directory", root.display());
        }
        let display = std::fs::canonicalize(root)
            .unwrap_or_else(|_| root.to_path_buf())
            .display()
            .to_string();
        Ok(Arc::new(Self {
            root: root.to_path_buf(),
            opts,
            arena: Mutex::new(Arena::new_root(display.clone())),
            prog: Progress::new(&display),
            top: Mutex::new(BinaryHeap::new()),
            error: Mutex::new(None),
            root_dev: dev_of(&meta),
            handle: Mutex::new(None),
        }))
    }

    pub fn start_thread(self: &Arc<Self>) {
        let me = Arc::clone(self);
        let handle = std::thread::Builder::new()
            .name("bloat-scan".into())
            .spawn(move || me.run())
            .expect("spawn scanner");
        *self.handle.lock().unwrap() = Some(handle);
    }

    /// Block until the walk finished (report mode).
    pub fn wait(&self) {
        let h = self.handle.lock().unwrap().take();
        if let Some(h) = h {
            let _ = h.join();
        }
    }

    pub fn arena_lock(&self) -> std::sync::MutexGuard<'_, Arena> {
        self.arena.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn first_error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|e| e.clone())
    }

    /// Biggest files seen so far, descending.
    pub fn top_files(&self, limit: usize) -> Vec<(u64, PathBuf)> {
        let heap = self.top.lock().unwrap_or_else(|e| e.into_inner());
        let mut v: Vec<(u64, PathBuf)> = heap.iter().map(|Reverse(x)| x.clone()).collect();
        v.sort_by(|a, b| b.0.cmp(&a.0));
        v.truncate(limit);
        v
    }

    fn offer_top(&self, size: u64, path: PathBuf) {
        let cap = self.opts.top_n;
        if cap == 0 {
            return;
        }
        let mut heap = self.top.lock().unwrap_or_else(|e| e.into_inner());
        let smallest = heap.peek().map(|Reverse((s, _))| *s).unwrap_or(0);
        if heap.len() >= cap && size <= smallest {
            return;
        }
        heap.push(Reverse((size, path)));
        while heap.len() > cap {
            heap.pop();
        }
    }

    fn run(&self) {
        let t0 = Instant::now();
        let one_fs = self.opts.one_file_system;
        let root_dev = self.root_dev;
        let excludes = self.opts.excludes.clone();

        let parallelism = match self.opts.jobs {
            None => Parallelism::RayonDefaultPool {
                busy_timeout: Duration::from_secs(2),
            },
            Some(n) => Parallelism::RayonNewPool(n.max(1)),
        };

        let walk = WalkDir::new(&self.root)
            .skip_hidden(false)
            .follow_links(false)
            .parallelism(parallelism)
            .process_read_dir(move |_depth, _dir, _state, children| {
                children.retain(|child| {
                    let Ok(entry) = child else { return true };
                    let path = entry.path();
                    let name = entry.file_name().to_string_lossy().to_string();
                    if is_excluded(&path, &name, &excludes) {
                        return false;
                    }
                    if entry.file_type().is_dir() {
                        if let Ok(md) = std::fs::symlink_metadata(&path) {
                            if not_one_fs(&md, one_fs, root_dev) {
                                return false;
                            }
                        }
                    }
                    true
                });
            });

        let mut stack: Vec<NodeId> = vec![0];
        let mut hardlinks: Option<HashSet<(u64, u64)>> = None;
        let mut seen: u64 = 0;

        for item in walk {
            let entry = match item {
                Ok(e) => e,
                Err(err) => {
                    self.prog.errors.fetch_add(1, Ordering::Relaxed);
                    let mut slot = self.error.lock().unwrap_or_else(|e| e.into_inner());
                    if slot.is_none() {
                        *slot = Some(err.to_string());
                    }
                    continue;
                }
            };
            let depth = entry.depth();
            let Ok(md) = entry.metadata() else {
                self.prog.errors.fetch_add(1, Ordering::Relaxed);
                continue;
            };
            let size = size_of(&md, self.opts.apparent_size);

            if depth == 0 {
                let mut arena = self.arena_lock();
                arena.add_root_own(size);
                continue;
            }
            let is_dir = entry.file_type().is_dir();
            if !is_dir && md.nlink() > 1 {
                // Count a hardlinked inode once, like du does.
                let key = (md.dev(), md.ino());
                let set = hardlinks.get_or_insert_with(HashSet::new);
                if !set.insert(key) {
                    continue;
                }
            }

            let name = entry.file_name().to_string_lossy().to_string();
            stack.truncate(depth);
            let parent = *stack.last().unwrap_or(&0);
            let id = {
                let mut arena = self.arena_lock();
                arena.add(parent, name, is_dir, size, mtime_of(&md))
            };
            if is_dir {
                stack.push(id);
                self.prog.dirs.fetch_add(1, Ordering::Relaxed);
            } else {
                self.offer_top(size, entry.path());
            }

            self.prog.bytes.fetch_add(size, Ordering::Relaxed);
            seen += 1;
            self.prog.entries.fetch_add(1, Ordering::Relaxed);
            if seen % 512 == 0 {
                if let Ok(mut cur) = self.prog.current.lock() {
                    *cur = clean(&entry.path().display().to_string());
                }
            }
        }

        self.prog.elapsed_ms.store(
            t0.elapsed().as_millis().min(u64::MAX as u128) as u64,
            Ordering::Relaxed,
        );
        self.prog.done.store(true, Ordering::Relaxed);
    }
}

fn not_one_fs(md: &std::fs::Metadata, one_fs: bool, root_dev: u64) -> bool {
    one_fs && dev_of(md) != root_dev
}

#[cfg(unix)]
fn dev_of(md: &std::fs::Metadata) -> u64 {
    md.dev()
}

#[cfg(not(unix))]
fn dev_of(_md: &std::fs::Metadata) -> u64 {
    0
}

#[cfg(unix)]
fn size_of(md: &std::fs::Metadata, apparent: bool) -> u64 {
    if apparent {
        md.size()
    } else {
        md.blocks().saturating_mul(512)
    }
}

#[cfg(not(unix))]
fn size_of(md: &std::fs::Metadata, _apparent: bool) -> u64 {
    md.len()
}

fn mtime_of(md: &std::fs::Metadata) -> i64 {
    #[cfg(unix)]
    {
        md.mtime()
    }
    #[cfg(not(unix))]
    {
        let _ = md;
        0
    }
}

fn clean(s: &str) -> String {
    const MAX: usize = 90;
    if s.chars().count() <= MAX {
        return s.to_string();
    }
    let tail: String = s
        .chars()
        .rev()
        .take(MAX - 3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("…{tail}")
}

/// `--exclude` matching: absolute patterns are path prefixes, relative ones glob
/// the basename.
pub fn is_excluded(path: &Path, name: &str, patterns: &[String]) -> bool {
    let full = path.to_string_lossy();
    for p in patterns {
        if p.starts_with('/') {
            let p = p.trim_end_matches('/');
            if full == p || full.starts_with(&format!("{p}/")) {
                return true;
            }
        } else if glob_match(p, name) {
            return true;
        }
    }
    if name.is_empty() {
        return false;
    }
    // Always prune pseudo filesystems when walking from a root that contains them.
    for skip in ALWAYS_SKIP {
        if full == *skip || full.starts_with(&format!("{skip}/")) {
            return true;
        }
    }
    false
}

/// Minimal glob: `*` and `?`.
pub fn glob_match(pat: &str, text: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = pi;
            mark = ti;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_match("*.iso", "ubuntu.iso"));
        assert!(glob_match("node_modules", "node_modules"));
        assert!(glob_match("ca?he", "cache"));
        assert!(!glob_match("*.iso", "ubuntu.zip"));
        assert!(!glob_match("node_modules", "node_modules2"));
    }

    #[test]
    fn excludes() {
        let p = vec!["*.iso".to_string(), "/var/cache".to_string()];
        assert!(is_excluded(Path::new("/home/x/Fedora.iso"), "Fedora.iso", &p));
        assert!(is_excluded(Path::new("/var/cache/apt"), "apt", &p));
        assert!(is_excluded(Path::new("/proc/1"), "1", &[]));
        assert!(!is_excluded(Path::new("/home/x/a.txt"), "a.txt", &p));
    }

    #[test]
    fn walks_a_temp_tree() {
        let base = std::env::temp_dir().join(format!("bloat-test-{}", std::process::id()));
        let sub = base.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(base.join("a.bin"), vec![0u8; 4096]).unwrap();
        std::fs::write(sub.join("b.bin"), vec![0u8; 8192]).unwrap();

        let scan = Scan::new(&base, Options::default()).unwrap();
        scan.start_thread();
        scan.wait();
        let arena = scan.arena_lock();
        assert!(arena.nodes[arena.root as usize].size >= 12288);
        assert!(arena.nodes[arena.root as usize].files >= 2);
        let tops = scan.top_files(10);
        assert_eq!(tops.len(), 2);
        assert!(tops[0].1.ends_with("b.bin"));
        std::fs::remove_dir_all(&base).unwrap();
    }
}