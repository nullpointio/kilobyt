//! Output naming and the on-disk commit step (PRD §5.6, §5.2.11-12).
//!
//! `{stem}_fit.jpg`, then `-2` … `-999`. Never overwrites: the `.part` file is
//! linked/created into its final name with a create-new primitive, so an existing
//! file is never truncated even if another process races us to the name.

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::fit::{msg_access, msg_name, msg_unstable};
use crate::{E_ACCESS, E_UNSTABLE};

pub(crate) const MAX_SUFFIX: u32 = 999;

/// Windows rule: a file name may not end in dots or spaces.
pub(crate) fn sanitise_stem(stem: &str) -> String {
    let trimmed = stem.trim_end_matches(['.', ' ']);
    if trimmed.is_empty() {
        "image".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Path keys are compared case-insensitively on Windows, exactly as in the shell.
fn key(path: &Path) -> String {
    let s = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s
    }
}

fn taken(candidate: &Path, reserved: &HashSet<String>) -> bool {
    if fs::symlink_metadata(candidate).is_ok() {
        return true;
    }
    reserved.contains(&key(candidate))
        || reserved.contains(&candidate.to_string_lossy().into_owned())
}

/// Write `jpeg` next to `dir` as `{stem}_fit.jpg`, resolving collisions against the
/// disk and `reserved`. Returns the committed path, which is added to `reserved`.
///
/// `Err` is `"<CODE>: <user message>"`.
pub(crate) fn commit(
    jpeg: &[u8],
    dir: &Path,
    stem: &str,
    reserved: &mut HashSet<String>,
) -> Result<PathBuf, String> {
    if jpeg.is_empty() {
        return Err(format!("{E_UNSTABLE}: {}", msg_unstable()));
    }
    if !dir.is_dir() {
        return Err(format!("{E_ACCESS}: {}", msg_access(dir)));
    }

    let clean_stem = sanitise_stem(stem);
    let base = format!("{clean_stem}_fit");

    let mut chosen: Option<PathBuf> = None;
    for i in 1..=MAX_SUFFIX {
        let name = if i == 1 {
            format!("{base}.jpg")
        } else {
            format!("{base}-{i}.jpg")
        };
        let candidate = dir.join(name);
        if taken(&candidate, reserved) {
            continue;
        }
        chosen = Some(candidate);
        break;
    }
    let Some(final_path) = chosen else {
        return Err(format!("{E_NAME_CODE}: {}", msg_name(&clean_stem)));
    };

    // Claim the name before touching the disk so a batch can never hand it out twice.
    reserved.insert(key(&final_path));

    let part_path = with_suffix(&final_path, ".part");
    let written = write_and_verify(&part_path, jpeg, &final_path, dir);
    if let Err(message) = written {
        let _ = fs::remove_file(&part_path);
        return Err(message);
    }

    match link_into_place(&part_path, &final_path, jpeg) {
        Ok(()) => {
            let _ = fs::remove_file(&part_path);
            Ok(final_path)
        }
        Err(message) => {
            let _ = fs::remove_file(&part_path);
            Err(message)
        }
    }
}

const E_NAME_CODE: &str = crate::E_NAME;

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

/// Write `<final>.part`, flush it, and re-read its length from disk (§5.2.11).
fn write_and_verify(part: &Path, jpeg: &[u8], final_path: &Path, dir: &Path) -> Result<(), String> {
    let failed = || format!("{E_ACCESS}: {}", msg_access(dir));

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(part)
        .map_err(|_| failed())?;
    file.write_all(jpeg).map_err(|_| failed())?;
    file.flush().map_err(|_| failed())?;
    // Best effort: a filesystem that cannot fsync must not fail the file.
    let _ = file.sync_all();
    drop(file);

    let on_disk = fs::metadata(part).map(|m| m.len()).map_err(|_| failed())?;
    if on_disk != jpeg.len() as u64 {
        return Err(format!("{E_UNSTABLE}: {}", msg_unstable()));
    }
    // The name must still be free; a directory squatting on it is E_ACCESS.
    if final_path.exists() {
        return Err(format!(
            "{E_NAME_CODE}: {}",
            msg_name(
                &final_path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default()
            )
        ));
    }
    Ok(())
}

/// Move the verified `.part` to its final name without ever overwriting.
/// `hard_link` is atomic and fails with `AlreadyExists`; on filesystems without hard
/// links (FAT32/exFAT, some network shares) fall back to an exclusive create+copy.
fn link_into_place(part: &Path, final_path: &Path, jpeg: &[u8]) -> Result<(), String> {
    match fs::hard_link(part, final_path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(format!(
            "{E_NAME_CODE}: {}",
            msg_name(&stem_label(final_path))
        )),
        Err(_) => {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(final_path)
                .map_err(|_| format!("{E_ACCESS}: {}", msg_access(final_path)))?;
            file.write_all(jpeg)
                .map_err(|_| format!("{E_ACCESS}: {}", msg_access(final_path)))?;
            file.flush()
                .map_err(|_| format!("{E_ACCESS}: {}", msg_access(final_path)))?;
            let _ = file.sync_all();
            Ok(())
        }
    }
}

fn stem_label(path: &Path) -> String {
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.strip_suffix("_fit").unwrap_or(&name).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::temp_dir;
    use crate::{E_NAME, E_UNSTABLE};

    fn bytes(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 % 251) as u8).collect()
    }

    #[test]
    fn first_name_is_stem_fit_jpg() {
        let dir = temp_dir("commit-basic");
        let mut reserved = HashSet::new();
        let path = commit(&bytes(120), &dir, "IMG_2031", &mut reserved).unwrap();
        assert_eq!(path.file_name().unwrap(), "IMG_2031_fit.jpg");
        assert_eq!(fs::read(&path).unwrap(), bytes(120));
        assert!(reserved.contains(&key(&path)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn existing_file_forces_dash_two() {
        let dir = temp_dir("commit-2");
        fs::write(dir.join("a_fit.jpg"), b"old").unwrap();
        let mut reserved = HashSet::new();
        let path = commit(&bytes(50), &dir, "a", &mut reserved).unwrap();
        assert_eq!(path.file_name().unwrap(), "a_fit-2.jpg");
        // The original is untouched.
        assert_eq!(fs::read(dir.join("a_fit.jpg")).unwrap(), b"old");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reserved_names_are_skipped_without_touching_disk() {
        let dir = temp_dir("commit-reserved");
        let mut reserved = HashSet::new();
        reserved.insert(key(&dir.join("b_fit.jpg")));
        let path = commit(&bytes(10), &dir, "b", &mut reserved).unwrap();
        assert_eq!(path.file_name().unwrap(), "b_fit-2.jpg");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn batch_hands_out_distinct_names() {
        let dir = temp_dir("commit-batch");
        let mut reserved = HashSet::new();
        let first = commit(&bytes(10), &dir, "same", &mut reserved).unwrap();
        let second = commit(&bytes(11), &dir, "same", &mut reserved).unwrap();
        assert_eq!(first.file_name().unwrap(), "same_fit.jpg");
        assert_eq!(second.file_name().unwrap(), "same_fit-2.jpg");
        assert_ne!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn exhausted_suffixes_report_e_name() {
        let dir = temp_dir("commit-999");
        let mut reserved = HashSet::new();
        reserved.insert(key(&dir.join("x_fit.jpg")));
        for i in 2..=999u32 {
            reserved.insert(key(&dir.join(format!("x_fit-{i}.jpg"))));
        }
        let err = commit(&bytes(10), &dir, "x", &mut reserved).unwrap_err();
        assert!(err.starts_with(E_NAME), "{err}");
        assert!(err.contains("Too many files named x_fit"), "{err}");
        // Nothing was written for the failed attempt.
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_directory_is_e_access() {
        let dir = temp_dir("commit-missing");
        let missing = dir.join("nope");
        let mut reserved = HashSet::new();
        let err = commit(&bytes(10), &missing, "a", &mut reserved).unwrap_err();
        assert!(err.starts_with(E_ACCESS), "{err}");
        assert!(err.contains("read-only or missing"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_part_file_is_left_behind() {
        let dir = temp_dir("commit-part");
        let mut reserved = HashSet::new();
        let path = commit(&bytes(64), &dir, "clean", &mut reserved).unwrap();
        assert!(path.exists());
        let parts: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
            .collect();
        assert!(parts.is_empty(), "leftover .part files: {parts:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_payload_is_rejected_as_unstable() {
        let dir = temp_dir("commit-empty");
        let mut reserved = HashSet::new();
        let err = commit(&[], &dir, "a", &mut reserved).unwrap_err();
        assert!(err.starts_with(E_UNSTABLE), "{err}");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitise_trims_trailing_dots_and_spaces() {
        assert_eq!(sanitise_stem("photo.jpg "), "photo.jpg");
        assert_eq!(sanitise_stem("holiday..."), "holiday");
        assert_eq!(sanitise_stem("name . ."), "name");
        assert_eq!(sanitise_stem("..."), "image");
        assert_eq!(sanitise_stem(""), "image");
    }

    #[test]
    fn unicode_stem_is_preserved() {
        let dir = temp_dir("commit-unicode");
        let mut reserved = HashSet::new();
        let path = commit(&bytes(5), &dir, "Ferien – Zürich 📷", &mut reserved).unwrap();
        assert_eq!(path.file_name().unwrap(), "Ferien – Zürich 📷_fit.jpg");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn disk_failure_message_names_the_folder() {
        let dir = temp_dir("commit-msg");
        let missing = dir.join("gone");
        let err = commit(&bytes(5), &missing, "z", &mut HashSet::new()).unwrap_err();
        assert!(err.contains(&missing.display().to_string()), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }
}
