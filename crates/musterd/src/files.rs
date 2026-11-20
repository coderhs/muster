//! Files named in a card's terminal or chat, for the file viewer. A path is looked up
//! in the card's folder, then the project's; only files inside those are shown unless
//! the user allowed any file (and the caller is on the Mac, not a paired device).

use std::io::Read;
use std::path::{Path, PathBuf};

use muster_core::util::{invalid, not_found};
use muster_core::Result;
use muster_proto::CardFileResult;

/// At most this much of a file is returned.
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;
/// A NUL byte in this much of the start marks a file as binary.
const SNIFF: usize = 8000;

/// The file `raw` names, with the path to show for it. `roots` are tried in order
/// for a relative path; a file outside every root is refused unless `any` is set.
pub fn resolve(raw: &str, roots: &[PathBuf], any: bool) -> Result<(PathBuf, String)> {
    let raw = raw.trim();
    let raw = raw.strip_prefix("file://").unwrap_or(raw);
    if raw.is_empty() || raw.contains('\0') {
        return Err(invalid("no file path given"));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let path = match (raw.strip_prefix("~/"), &home) {
        (Some(rest), Some(h)) => h.join(rest),
        _ => PathBuf::from(raw),
    };
    let roots: Vec<PathBuf> = roots.iter().filter_map(|r| std::fs::canonicalize(r).ok()).collect();
    let candidates: Vec<PathBuf> =
        if path.is_absolute() { vec![path.clone()] } else { roots.iter().map(|r| r.join(&path)).collect() };
    let mut outside = false;
    for c in candidates {
        // Links are followed, then the real place is what is checked.
        let Ok(file) = std::fs::canonicalize(&c) else { continue };
        if !file.is_file() {
            continue;
        }
        if let Some(root) = roots.iter().find(|r| file.starts_with(r)) {
            let shown = file.strip_prefix(root).unwrap_or(&file).to_string_lossy().into_owned();
            return Ok((file, shown));
        }
        if any {
            let shown = file.to_string_lossy().into_owned();
            return Ok((file, shown));
        }
        outside = true;
    }
    if outside {
        return Err(invalid(format!(
            "{raw} is outside this card's folder and project (Settings can allow any file on the Mac)"
        )));
    }
    Err(not_found("file", raw))
}

/// Reads up to [`MAX_BYTES`] of `file`.
pub fn read(file: &Path, shown: String) -> Result<CardFileResult> {
    let io = |e: std::io::Error| invalid(format!("{shown}: {e}"));
    let size = std::fs::metadata(file).map_err(io)?.len();
    let mut bytes = Vec::new();
    std::fs::File::open(file).and_then(|f| f.take(MAX_BYTES).read_to_end(&mut bytes)).map_err(io)?;
    let binary = bytes[..bytes.len().min(SNIFF)].contains(&0);
    let content = if binary { String::new() } else { String::from_utf8_lossy(&bytes).into_owned() };
    Ok(CardFileResult {
        path: shown,
        abs_path: file.to_string_lossy().into_owned(),
        content,
        size,
        truncated: !binary && size > MAX_BYTES,
        binary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Removes the folder when dropped.
    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tree() -> (Dir, PathBuf, PathBuf, PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!("muster-files-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let t = Dir(base.clone());
        let root = std::fs::canonicalize(&base).unwrap();
        let project = root.join("project");
        let work = root.join("work");
        let outside = root.join("outside");
        for d in [&project, &work, &outside] {
            std::fs::create_dir_all(d.join("src")).unwrap();
        }
        std::fs::write(project.join("src/lib.rs"), "fn project() {}\n").unwrap();
        std::fs::write(work.join("src/lib.rs"), "fn work() {}\n").unwrap();
        std::fs::write(project.join("README.md"), "# hi\n").unwrap();
        std::fs::write(outside.join("secret.txt"), "secret\n").unwrap();
        (t, work, project, outside)
    }

    #[test]
    fn relative_paths_try_the_card_folder_then_the_project() {
        let (_t, work, project, _) = tree();
        let roots = [work.clone(), project.clone()];
        let (f, shown) = resolve("src/lib.rs", &roots, false).unwrap();
        assert_eq!((f, shown.as_str()), (work.join("src/lib.rs"), "src/lib.rs"));
        let (f, shown) = resolve("./README.md", &roots, false).unwrap();
        assert_eq!((f, shown.as_str()), (project.join("README.md"), "README.md"));
        let abs = project.join("src/lib.rs");
        let (f, shown) = resolve(&format!("file://{}", abs.display()), &roots, false).unwrap();
        assert_eq!((f, shown.as_str()), (abs, "src/lib.rs"));
        assert!(resolve("src/missing.rs", &roots, false).unwrap_err().msg.contains("not found"));
    }

    #[test]
    fn files_outside_need_the_setting() {
        let (_t, work, project, outside) = tree();
        std::os::unix::fs::symlink(outside.join("secret.txt"), work.join("link.txt")).unwrap();
        let roots = [work.clone(), project.clone()];
        let abs = outside.join("secret.txt").to_string_lossy().into_owned();
        for p in [abs.as_str(), "../outside/secret.txt", "link.txt", "src/../../outside/secret.txt"] {
            let e = resolve(p, &roots, false).unwrap_err();
            assert!(e.msg.contains("outside"), "{p}: {}", e.msg);
            let (f, shown) = resolve(p, &roots, true).unwrap();
            assert_eq!(f, outside.join("secret.txt"));
            assert_eq!(shown, abs);
        }
    }

    #[test]
    fn text_binary_and_long_files() {
        let (_t, work, ..) = tree();
        let r = read(&work.join("src/lib.rs"), "src/lib.rs".into()).unwrap();
        assert_eq!((r.content.as_str(), r.binary, r.truncated, r.size), ("fn work() {}\n", false, false, 13));
        std::fs::write(work.join("a.bin"), [0x89u8, b'P', 0, 1, 2]).unwrap();
        let r = read(&work.join("a.bin"), "a.bin".into()).unwrap();
        assert!(r.binary && r.content.is_empty());
        std::fs::write(work.join("big.txt"), "x".repeat(MAX_BYTES as usize + 10)).unwrap();
        let r = read(&work.join("big.txt"), "big.txt".into()).unwrap();
        assert!(r.truncated && r.content.len() == MAX_BYTES as usize);
    }
}
