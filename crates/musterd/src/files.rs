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
    if !outside && !path.is_absolute() {
        if let Some(found) = search(&path, &roots)? {
            return Ok(found);
        }
    }
    if outside {
        return Err(invalid(format!(
            "{raw} is outside this card's folder and project (Settings can allow any file on the Mac)"
        )));
    }
    Err(not_found("file", raw))
}

/// Folders not looked in when searching for a file by name.
const SKIP: &[&str] =
    &["node_modules", "target", "vendor", "dist", "build", "tmp", "log", "coverage", "Pods", "DerivedData"];
/// At most this many entries are looked at in one search.
const MAX_SEARCH: usize = 200_000;

/// A relative path that is not at a root's top, such as `checkout_controller.rb` for
/// `app/controllers/checkout_controller.rb`: the roots are searched for files whose path
/// ends with it, the card's folder first (a worktree has the project's files too). One
/// match is the file; several are an error naming them.
fn search(rel: &Path, roots: &[PathBuf]) -> Result<Option<(PathBuf, String)>> {
    use std::path::Component;
    if !rel.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)) {
        return Ok(None);
    }
    let rel: PathBuf = rel.components().filter(|c| matches!(c, Component::Normal(_))).collect();
    let Some(name) = rel.file_name() else { return Ok(None) };
    let mut found: Vec<(PathBuf, String)> = Vec::new();
    let mut seen = 0usize;
    for root in roots {
        if !found.is_empty() {
            break;
        }
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                seen += 1;
                if seen > MAX_SEARCH {
                    break;
                }
                // Links are not followed, so a search stays inside the root.
                let Ok(kind) = e.file_type() else { continue };
                let fname = e.file_name();
                if kind.is_dir() {
                    let n = fname.to_string_lossy();
                    if !n.starts_with('.') && !SKIP.contains(&n.as_ref()) {
                        stack.push(e.path());
                    }
                } else if kind.is_file() && fname == name {
                    let p = e.path();
                    if p.ends_with(&rel) && !found.iter().any(|(f, _)| *f == p) {
                        let shown = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().into_owned();
                        found.push((p, shown));
                    }
                }
            }
        }
    }
    match found.len() {
        0 => Ok(None),
        1 => Ok(found.pop()),
        n => {
            found.sort_by(|a, b| a.1.cmp(&b.1));
            let list: Vec<&str> = found.iter().take(5).map(|(_, s)| s.as_str()).collect();
            let more = if n > 5 { format!(" and {} more", n - 5) } else { String::new() };
            Err(invalid(format!("{n} files match {}: {}{more}", rel.display(), list.join(", "))))
        }
    }
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
    fn a_name_not_at_the_top_is_searched_for() {
        let (_t, work, project, outside) = tree();
        std::fs::create_dir_all(project.join("app/controllers")).unwrap();
        std::fs::create_dir_all(project.join("node_modules/x")).unwrap();
        std::fs::write(project.join("app/controllers/checkout_controller.rb"), "class C\nend\n").unwrap();
        std::fs::write(project.join("node_modules/x/checkout_controller.rb"), "no\n").unwrap();
        std::fs::write(outside.join("only_outside.rb"), "no\n").unwrap();
        std::os::unix::fs::symlink(&outside, project.join("linked")).unwrap();
        let roots = [work.clone(), project.clone()];
        let (f, shown) = resolve("checkout_controller.rb", &roots, false).unwrap();
        assert_eq!(
            (f, shown.as_str()),
            (project.join("app/controllers/checkout_controller.rb"), "app/controllers/checkout_controller.rb")
        );
        let (_, shown) = resolve("controllers/checkout_controller.rb", &roots, false).unwrap();
        assert_eq!(shown, "app/controllers/checkout_controller.rb");
        // The card's folder wins over the project.
        let (f, _) = resolve("lib.rs", &roots, false).unwrap();
        assert_eq!(f, work.join("src/lib.rs"));
        // Two matches in one folder: say which, rather than guess.
        std::fs::create_dir_all(project.join("app/models")).unwrap();
        std::fs::write(project.join("app/models/user.rb"), "\n").unwrap();
        std::fs::write(project.join("src/user.rb"), "\n").unwrap();
        let e = resolve("user.rb", &roots, false).unwrap_err();
        assert!(e.msg.contains("2 files match user.rb: app/models/user.rb, src/user.rb"), "{}", e.msg);
        // Links out of the roots are not followed, and `..` is never searched.
        assert!(resolve("only_outside.rb", &roots, false).unwrap_err().msg.contains("not found"));
        assert!(resolve("../x/checkout_controller.rb", &roots, false).unwrap_err().msg.contains("not found"));
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
