//! Per-document local version history: a full copy of a document is kept on
//! every save, no more often than `MIN_INTERVAL` apart, under the user's app
//! data (outside the library) so a lost or rolled-back vault still has its
//! past. Like Obsidian's File recovery; never committed to git — a purely
//! local safety net that the 版本历史 panel lists and rolls back to.

use crate::time::now_ms;
use std::path::{Path, PathBuf};

/// Snapshot a document at most this often (ms); matches Obsidian's ~5 min.
const MIN_INTERVAL_MS: u64 = 5 * 60 * 1000;
/// Keep at most this many snapshots per document.
const MAX_PER_DOC: usize = 100;
/// Drop snapshots older than this, as long as `KEEP_AT_LEAST` newer remain.
const MAX_AGE_MS: u64 = 30 * 24 * 60 * 60 * 1000;
/// Never prune below this many, however old.
const KEEP_AT_LEAST: usize = 10;

/// One stored version: the file is `<at_ms>.<xml|md>` in the document's dir.
pub struct Snapshot {
    /// When it was taken (unix ms), also its file stem.
    pub at_ms: u64,
    pub path: PathBuf,
    /// A Markdown (`.md`) snapshot rather than native DocxXML (`.xml`).
    pub markdown: bool,
}

/// A library's history store: `.../history/<library-hash>/<doc-id>/<ms>.<ext>`.
pub struct History { root: PathBuf }

impl History {
    pub fn new(root: PathBuf) -> Self { Self { root } }

    fn doc_dir(&self, id: &str) -> PathBuf { self.root.join(id) }

    /// Snapshots of `id`, newest first.
    pub fn list(&self, id: &str) -> Vec<Snapshot> {
        let mut out = vec![];
        let Ok(rd) = std::fs::read_dir(self.doc_dir(id)) else { return out };
        for e in rd.flatten() {
            let p = e.path();
            let markdown = match p.extension().and_then(|s| s.to_str()) {
                Some("md") => true, Some("xml") => false, _ => continue,
            };
            let Some(at_ms) = p.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse().ok()) else { continue };
            out.push(Snapshot { at_ms, path: p, markdown });
        }
        out.sort_by_key(|s| std::cmp::Reverse(s.at_ms));
        out
    }

    /// Record `bytes` as the newest version of `id` (`ext` is "xml" or "md").
    /// Skipped if a snapshot was taken within `MIN_INTERVAL` or the content is
    /// unchanged from the latest, unless `force` (a manual checkpoint). Returns
    /// whether a snapshot was written. Best-effort: callers ignore errors.
    pub fn snapshot(&self, id: &str, ext: &str, bytes: &[u8], force: bool) -> std::io::Result<bool> {
        self.snapshot_at(id, ext, bytes, force, now_ms())
    }

    fn snapshot_at(&self, id: &str, ext: &str, bytes: &[u8], force: bool, now: u64) -> std::io::Result<bool> {
        if !force && let Some(last) = self.list(id).into_iter().next() {
            if now.saturating_sub(last.at_ms) < MIN_INTERVAL_MS { return Ok(false); }
            if std::fs::read(&last.path).is_ok_and(|b| b == bytes) { return Ok(false); }
        }
        let dir = self.doc_dir(id);
        std::fs::create_dir_all(&dir)?;
        // Unique name (ms), so a plain write never clobbers an older version.
        std::fs::write(dir.join(format!("{now}.{ext}")), bytes)?;
        self.prune(id, now);
        Ok(true)
    }

    /// Drop a document's whole history (used when the document is destroyed).
    pub fn forget(&self, id: &str) { let _ = std::fs::remove_dir_all(self.doc_dir(id)); }

    fn prune(&self, id: &str, now: u64) {
        for (i, s) in self.list(id).iter().enumerate() {
            let too_many = i >= MAX_PER_DOC;
            let too_old = i >= KEEP_AT_LEAST && now.saturating_sub(s.at_ms) > MAX_AGE_MS;
            if too_many || too_old { let _ = std::fs::remove_file(&s.path); }
        }
    }
}

/// The bytes of a stored snapshot.
pub fn read(path: &Path) -> std::io::Result<String> { std::fs::read_to_string(path) }

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("qmd-hist-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn throttles_and_dedups() {
        let h = History::new(tmp());
        let t = 1_000_000_000_000;
        assert!(h.snapshot_at("a", "xml", b"v1", false, t).unwrap());        // first: kept
        assert!(!h.snapshot_at("a", "xml", b"v2", false, t + 1000).unwrap()); // within interval
        assert!(!h.snapshot_at("a", "xml", b"v1", false, t + MIN_INTERVAL_MS + 1).unwrap()); // unchanged
        assert!(h.snapshot_at("a", "xml", b"v2", false, t + MIN_INTERVAL_MS + 1).unwrap());  // later + changed
        assert!(h.snapshot_at("a", "xml", b"v3", true, t + 2000).unwrap());   // forced ignores both
        let list = h.list("a");
        assert_eq!(list.len(), 3);
        assert!(list[0].at_ms >= list[1].at_ms && list[1].at_ms >= list[2].at_ms); // newest first
    }

    #[test]
    fn prunes_by_count() {
        let h = History::new(tmp());
        for i in 0..(MAX_PER_DOC as u64 + 20) {
            h.snapshot_at("a", "xml", format!("v{i}").as_bytes(), true, 1_000_000 + i).unwrap();
        }
        assert_eq!(h.list("a").len(), MAX_PER_DOC);
    }
}
