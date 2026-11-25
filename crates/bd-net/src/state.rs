//! Resume data stored next to the partial file (`<name>.part.json`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Result;

/// One byte range of the file. `end` is inclusive; `None` means the size is
/// unknown and the segment runs until the server closes the stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Segment {
    pub start: u64,
    pub end: Option<u64>,
    pub done: u64,
}

impl Segment {
    pub fn len(&self) -> Option<u64> {
        self.end.map(|end| end - self.start + 1)
    }

    pub fn is_complete(&self) -> bool {
        self.len().is_some_and(|len| self.done >= len)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ResumeState {
    pub version: u32,
    pub size: Option<u64>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub segments: Vec<Segment>,
}

pub(crate) const STATE_VERSION: u32 = 1;

pub(crate) fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

pub(crate) fn state_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part.json");
    dest.with_file_name(name)
}

pub(crate) async fn load(path: &Path) -> Option<ResumeState> {
    let bytes = tokio::fs::read(path).await.ok()?;
    let state: ResumeState = serde_json::from_slice(&bytes).ok()?;
    (state.version == STATE_VERSION).then_some(state)
}

/// Writes atomically: a crash during the write never leaves a corrupt file.
pub(crate) async fn save(path: &Path, state: &ResumeState) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, serde_json::to_vec(state)?).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

/// Splits `size` bytes into at most `connections` segments of at least
/// `min_segment` bytes each.
pub(crate) fn plan(size: u64, connections: usize, min_segment: u64) -> Vec<Segment> {
    if size == 0 {
        return Vec::new();
    }
    let by_size = (size / min_segment.max(1)).max(1);
    let count = (connections.max(1) as u64).min(by_size);
    let base = size / count;
    let mut segments = Vec::with_capacity(count as usize);
    let mut start = 0;
    for i in 0..count {
        let len = if i == count - 1 { size - start } else { base };
        segments.push(Segment { start, end: Some(start + len - 1), done: 0 });
        start += len;
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_covers_the_whole_file() {
        for (size, conns) in [(1u64, 8usize), (10, 3), (10_000_000, 8), (5_000_001, 4)] {
            let segs = plan(size, conns, 1024);
            assert_eq!(segs.first().unwrap().start, 0);
            assert_eq!(segs.last().unwrap().end, Some(size - 1));
            for pair in segs.windows(2) {
                assert_eq!(pair[0].end.unwrap() + 1, pair[1].start);
            }
            assert!(segs.len() <= conns);
        }
    }

    #[test]
    fn small_files_use_one_segment() {
        assert_eq!(plan(500_000, 8, 1_048_576).len(), 1);
        assert!(plan(0, 8, 1).is_empty());
    }

    #[test]
    fn paths_are_derived_from_destination() {
        let dest = Path::new("C:/dl/file.rar");
        assert_eq!(part_path(dest), Path::new("C:/dl/file.rar.part"));
        assert_eq!(state_path(dest), Path::new("C:/dl/file.rar.part.json"));
    }
}
