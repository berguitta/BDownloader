//! Checksums: compute MD5, SHA-1, SHA-256 and CRC32 of a file, and verify a
//! package against the checksum files that come with it (.sfv, .md5,
//! .sha1, .sha256).

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use md5::Digest as _;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hashes {
    pub crc32: String,
    pub md5: String,
    pub sha1: String,
    pub sha256: String,
}

/// Reads the file once and computes every hash. Blocking.
pub fn compute(path: &Path) -> std::io::Result<Hashes> {
    let mut file = std::fs::File::open(path)?;
    let mut crc = crc32fast::Hasher::new();
    let mut md5 = md5::Md5::new();
    let mut sha1 = sha1::Sha1::new();
    let mut sha256 = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        crc.update(&buf[..n]);
        md5.update(&buf[..n]);
        sha1.update(&buf[..n]);
        sha256.update(&buf[..n]);
    }
    Ok(Hashes {
        crc32: format!("{:08x}", crc.finalize()),
        md5: hex::encode(md5.finalize()),
        sha1: hex::encode(sha1.finalize()),
        sha256: hex::encode(sha256.finalize()),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Algo {
    Crc32,
    Md5,
    Sha1,
    Sha256,
}

/// Expected hashes read from checksum files: file name → (algorithm, hash).
fn expected_in(dir: &Path) -> HashMap<String, (Algo, String)> {
    let mut map = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return map };
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
        let algo = match ext.as_str() {
            "sfv" => Algo::Crc32,
            "md5" => Algo::Md5,
            "sha1" => Algo::Sha1,
            "sha256" => Algo::Sha256,
            _ => continue,
        };
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with(';')) {
            let (name, hash) = if algo == Algo::Crc32 {
                // "name.ext CRC32"
                match line.rsplit_once(char::is_whitespace) {
                    Some((name, hash)) => (name.trim().to_owned(), hash.trim().to_ascii_lowercase()),
                    None => continue,
                }
            } else {
                // "hash *name" or "hash  name"
                match line.split_once(char::is_whitespace) {
                    Some((hash, name)) => {
                        (name.trim().trim_start_matches('*').to_owned(), hash.trim().to_ascii_lowercase())
                    }
                    None => continue,
                }
            };
            map.insert(name.to_ascii_lowercase(), (algo, hash));
        }
    }
    map
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckResult {
    pub file: PathBuf,
    /// `Some(true)` matches, `Some(false)` corrupted, `None` no checksum known.
    pub ok: Option<bool>,
}

/// Verifies files against the checksum files found in their folder. Blocking.
pub fn verify(files: &[PathBuf]) -> Vec<CheckResult> {
    let mut cache: HashMap<PathBuf, HashMap<String, (Algo, String)>> = HashMap::new();
    files
        .iter()
        .map(|file| {
            let dir = file.parent().unwrap_or(Path::new(".")).to_path_buf();
            let expected = cache.entry(dir.clone()).or_insert_with(|| expected_in(&dir));
            let name = file.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            let ok = expected.get(&name).and_then(|(algo, hash)| {
                let hashes = compute(file).ok()?;
                Some(match algo {
                    Algo::Crc32 => &hashes.crc32 == hash,
                    Algo::Md5 => &hashes.md5 == hash,
                    Algo::Sha1 => &hashes.sha1 == hash,
                    Algo::Sha256 => &hashes.sha256 == hash,
                })
            });
            CheckResult { file: file.clone(), ok }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_known_hashes() {
        let dir = std::env::temp_dir().join(format!("bd-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("abc.txt");
        std::fs::write(&file, b"abc").unwrap();
        let h = compute(&file).unwrap();
        assert_eq!(h.crc32, "352441c2");
        assert_eq!(h.md5, "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(h.sha1, "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(h.sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");

        std::fs::write(dir.join("pack.sfv"), "; made by test\nabc.txt 352441C2\n").unwrap();
        std::fs::write(dir.join("other.bin"), b"x").unwrap();
        std::fs::write(dir.join("pack.md5"), "00000000000000000000000000000000 *other.bin\n").unwrap();
        let results = verify(&[file.clone(), dir.join("other.bin"), dir.join("pack.sfv")]);
        assert_eq!(results[0].ok, Some(true));
        assert_eq!(results[1].ok, Some(false));
        assert_eq!(results[2].ok, None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
