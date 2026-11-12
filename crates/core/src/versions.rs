//! Persistent document versions: one snapshot of the bytes written to disk
//! per document, kept in `history/<id>/` beside `docs/`.
//!
//! This is deliberately not the in-memory undo stack (`editor::core`'s
//! `Snapshot`, `file_history`'s records): those are lost when the app exits.
//! A version is recorded only after a successful document write, so a save
//! rejected by conflict detection never shows up as a version.

use super::{load_json, save_json, write_atomic, Store};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// How versions are kept. The UI sets this from preferences; a fresh library
/// uses these values until it does.
#[derive(Clone, Copy, Debug)]
pub struct VersionPolicy {
    pub enabled: bool,
    pub max: usize,
    pub min_interval_ms: u64,
}

impl Default for VersionPolicy {
    fn default() -> Self { Self { enabled: true, max: 50, min_interval_ms: 60_000 } }
}

/// One saved version. `id` is also the snapshot file's name stem.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VersionEntry {
    pub id: String,
    /// Unix seconds, for the UI's relative dates.
    pub saved: u64,
    pub size: u64,
    pub title: String,
}

/// `history/<id>/index.json`. Oldest first.
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
struct VersionIndex {
    #[serde(default)]
    versions: Vec<VersionEntry>,
    /// Unix milliseconds of the last snapshot, for throttling.
    #[serde(default)]
    last_ms: u64,
}

impl Store {
    fn history_dir(&self, id: &str) -> PathBuf {
        // ids are uuids, never user-supplied paths.
        self.root.join("history").join(id)
    }

    fn version_index(&self, id: &str) -> VersionIndex {
        let mut warnings = vec![];
        load_json(&self.history_dir(id).join("index.json"), "历史版本索引", &mut warnings)
    }

    /// Newest first. A damaged or missing index simply means "no versions".
    pub fn list_versions(&self, id: &str) -> Vec<VersionEntry> {
        let mut versions = self.version_index(id).versions;
        versions.reverse();
        versions
    }

    pub fn read_version(&self, id: &str, version_id: &str) -> Result<String> {
        // Resolve through the index: the caller's id never reaches the path.
        let entry = self.version_index(id).versions.into_iter()
            .find(|v| v.id == version_id)
            .ok_or_else(|| anyhow::anyhow!("找不到该历史版本"))?;
        let path = self.history_dir(id).join(format!("{}.snapshot", entry.id));
        std::fs::read_to_string(&path).with_context(|| format!("读取历史版本失败：{}", path.display()))
    }

    /// Plain text of a version for the preview pane. Best effort only.
    pub fn version_preview(&self, id: &str, version_id: &str) -> String {
        let Ok(text) = self.read_version(id, version_id) else { return String::new() };
        let shown = match self.parse_doc(id, &text) {
            Ok(doc) => format!("{}\n{}", doc.title.text, doc.plain_text()),
            Err(_) => text,
        };
        shown.lines().take(40).collect::<Vec<_>>().join("\n")
    }

    /// Keep `bytes` as a version. `force` skips throttling and de-duplication
    /// (used before a restore, so the replaced state is never lost), but an
    /// explicitly disabled history still records nothing.
    pub(super) fn record_version(&mut self, id: &str, bytes: &str, title: &str, force: bool) -> Result<()> {
        let policy = self.version_policy;
        if !policy.enabled { return Ok(()) }
        let dir = self.history_dir(id);
        let mut index = self.version_index(id);
        let ms = crate::time::now_ms();
        if !force {
            if ms.saturating_sub(index.last_ms) < policy.min_interval_ms { return Ok(()) }
            let duplicate = index.versions.last().is_some_and(|last| {
                std::fs::read(dir.join(format!("{}.snapshot", last.id))).is_ok_and(|b| b.as_slice() == bytes.as_bytes())
            });
            if duplicate { return Ok(()) }
        }
        std::fs::create_dir_all(&dir)?;
        let version_id = format!("{}-{}", ms, uuid::Uuid::new_v4());
        // The snapshot file first: a crash may leave an unreferenced file,
        // but never an index pointing at a file that was not written.
        write_atomic(&dir.join(format!("{version_id}.snapshot")), bytes.as_bytes())?;
        index.versions.push(VersionEntry {
            id: version_id,
            saved: crate::time::now(),
            size: bytes.len() as u64,
            title: title.to_string(),
        });
        index.last_ms = ms;
        let keep = policy.max.max(1);
        while index.versions.len() > keep {
            let old = index.versions.remove(0);
            let _ = std::fs::remove_file(dir.join(format!("{}.snapshot", old.id)));
        }
        save_json(&dir.join("index.json"), &index)
    }

    /// Replace the document with an older version. Callers must have flushed
    /// the editor and closed other windows editing this document.
    pub fn restore_version(&mut self, id: &str, version_id: &str) -> Result<()> {
        let _lock = self.begin_write()?;
        let bytes = self.read_version(id, version_id)?;
        // Keep what we are about to replace, so a restore has an undo point.
        if let Ok(current) = std::fs::read_to_string(self.path(id)) {
            let _ = self.record_version(id, &current, "恢复前", true);
        }
        write_atomic(&self.path(id), bytes.as_bytes())?;
        // Without the new baseline the next save would report a conflict with
        // another instance; `parsed` caches the old title otherwise.
        self.loaded.borrow_mut().insert(id.to_string(), bytes);
        self.parsed.remove(id);
        let t = crate::time::now();
        if let Some(m) = self.index.get_mut(id) { m.modified = t; }
        self.save_index()?;
        Ok(())
    }

    pub fn set_version_policy(&mut self, policy: VersionPolicy) { self.version_policy = policy; }
}
