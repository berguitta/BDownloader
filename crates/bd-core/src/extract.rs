//! Automatic extraction of finished packages: ZIP, 7z and RAR (including
//! multi-part RAR), with the package password and the user's password list.

use std::fs::File;
use std::path::{Path, PathBuf};

use crate::packages::{is_archive_part, is_first_volume};

#[derive(Clone, Debug, Default)]
pub struct ExtractOptions {
    /// Passwords tried in order after the package password.
    pub passwords: Vec<String>,
    /// Delete the archive files once everything extracted fine.
    pub delete_archives: bool,
    /// Also extract archives found inside the extracted files (one level).
    pub nested: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveResult {
    pub archive: PathBuf,
    /// Number of extracted entries, or why it failed.
    pub outcome: Result<usize, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtractReport {
    pub results: Vec<ArchiveResult>,
    pub deleted: usize,
}

impl ExtractReport {
    pub fn all_ok(&self) -> bool {
        self.results.iter().all(|r| r.outcome.is_ok())
    }
}

#[derive(Debug)]
enum Failure {
    WrongPassword,
    Other(String),
}

/// Extracts every archive of a finished package into `dir`. Blocking: run it
/// on a blocking thread.
pub fn extract_package(
    files: &[PathBuf],
    dir: &Path,
    package_password: Option<&str>,
    options: &ExtractOptions,
) -> ExtractReport {
    let mut candidates: Vec<Option<String>> = vec![None];
    if let Some(pw) = package_password.filter(|p| !p.is_empty()) {
        candidates.push(Some(pw.to_owned()));
    }
    candidates.extend(options.passwords.iter().filter(|p| !p.is_empty()).cloned().map(Some));

    let mut report = ExtractReport::default();
    let before: Vec<PathBuf> = list_files(dir);
    for archive in files.iter().filter(|f| f.file_name().and_then(|n| n.to_str()).is_some_and(is_first_volume)) {
        let mut outcome = Err("no password worked".to_owned());
        for password in &candidates {
            match extract_one(archive, dir, password.as_deref()) {
                Ok(count) => {
                    outcome = Ok(count);
                    break;
                }
                Err(Failure::WrongPassword) => continue,
                Err(Failure::Other(e)) => {
                    outcome = Err(e);
                    break;
                }
            }
        }
        report.results.push(ArchiveResult { archive: archive.clone(), outcome });
    }

    if options.nested && report.all_ok() {
        let inner: Vec<PathBuf> =
            list_files(dir).into_iter().filter(|f| !before.contains(f) && !files.contains(f)).collect();
        if inner.iter().any(|f| f.file_name().and_then(|n| n.to_str()).is_some_and(is_first_volume)) {
            let nested =
                extract_package(&inner, dir, package_password, &ExtractOptions { nested: false, ..options.clone() });
            report.results.extend(nested.results);
            report.deleted += nested.deleted;
        }
    }

    if options.delete_archives && !report.results.is_empty() && report.all_ok() {
        for file in files.iter().filter(|f| f.file_name().and_then(|n| n.to_str()).is_some_and(is_archive_part)) {
            if std::fs::remove_file(file).is_ok() {
                report.deleted += 1;
            }
        }
    }
    report
}

fn extract_one(archive: &Path, dir: &Path, password: Option<&str>) -> Result<usize, Failure> {
    let name = archive.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_ascii_lowercase();
    if name.ends_with(".zip") {
        extract_zip(archive, dir, password)
    } else if name.ends_with(".7z") {
        extract_7z(archive, dir, password)
    } else if name.ends_with(".rar") {
        extract_rar(archive, dir, password)
    } else {
        Err(Failure::Other(format!("{name}: split archives of this kind are not supported yet")))
    }
}

fn extract_zip(archive: &Path, dir: &Path, password: Option<&str>) -> Result<usize, Failure> {
    let file = File::open(archive).map_err(|e| Failure::Other(e.to_string()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| Failure::Other(e.to_string()))?;
    let mut count = 0;
    for i in 0..zip.len() {
        let mut entry = match password {
            Some(pw) => zip.by_index_decrypt(i, pw.as_bytes()),
            None => zip.by_index(i),
        }
        .map_err(|e| {
            let text = e.to_string();
            if text.contains("assword") {
                Failure::WrongPassword
            } else {
                Failure::Other(text)
            }
        })?;
        // Refuse entries escaping the destination folder.
        let Some(relative) = entry.enclosed_name() else { continue };
        let target = dir.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| Failure::Other(e.to_string()))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Failure::Other(e.to_string()))?;
        }
        let mut out = File::create(&target).map_err(|e| Failure::Other(e.to_string()))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| {
            let text = e.to_string();
            if text.contains("assword") {
                Failure::WrongPassword
            } else {
                Failure::Other(text)
            }
        })?;
        count += 1;
    }
    Ok(count)
}

fn extract_7z(archive: &Path, dir: &Path, password: Option<&str>) -> Result<usize, Failure> {
    let result = match password {
        Some(pw) => sevenz_rust2::decompress_file_with_password(archive, dir, pw.into()),
        None => sevenz_rust2::decompress_file(archive, dir),
    };
    match result {
        Ok(()) => Ok(count_files(dir)),
        Err(e) => {
            let text = e.to_string();
            if text.to_ascii_lowercase().contains("password") {
                Err(Failure::WrongPassword)
            } else {
                Err(Failure::Other(text))
            }
        }
    }
}

fn extract_rar(archive: &Path, dir: &Path, password: Option<&str>) -> Result<usize, Failure> {
    let opener = match password {
        Some(pw) => unrar::Archive::with_password(archive, pw),
        None => unrar::Archive::new(archive),
    };
    let mut open = opener.open_for_processing().map_err(rar_failure)?;
    let mut count = 0;
    while let Some(header) = open.read_header().map_err(rar_failure)? {
        let is_file = header.entry().is_file();
        open = if is_file {
            count += 1;
            header.extract_with_base(dir).map_err(rar_failure)?
        } else {
            header.skip().map_err(rar_failure)?
        };
    }
    Ok(count)
}

fn rar_failure(e: unrar::error::UnrarError) -> Failure {
    let text = e.to_string();
    let lower = text.to_ascii_lowercase();
    if lower.contains("password") || lower.contains("bad data") || lower.contains("crc") {
        Failure::WrongPassword
    } else {
        Failure::Other(text)
    }
}

fn list_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect()).unwrap_or_default()
}

fn count_files(dir: &Path) -> usize {
    std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bd-extract-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn extracts_zip_and_deletes_archive() {
        let dir = temp("zip");
        let archive = dir.join("pack.zip");
        {
            let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("hello.txt", options).unwrap();
            zip.write_all(b"hello").unwrap();
            zip.start_file("sub/world.txt", options).unwrap();
            zip.write_all(b"world").unwrap();
            zip.finish().unwrap();
        }
        let options = ExtractOptions { passwords: vec![], delete_archives: true, nested: false };
        let report = extract_package(std::slice::from_ref(&archive), &dir, None, &options);
        assert_eq!(report.results[0].outcome, Ok(2));
        assert_eq!(std::fs::read_to_string(dir.join("sub").join("world.txt")).unwrap(), "world");
        assert_eq!(report.deleted, 1);
        assert!(!archive.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ignores_non_archives_and_later_volumes() {
        let dir = temp("skip");
        let files = vec![dir.join("movie.mkv"), dir.join("a.part2.rar")];
        let report = extract_package(&files, &dir, None, &ExtractOptions::default());
        assert!(report.results.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
