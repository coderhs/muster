//! A project's own documentation, for the Docs tab: the instruction files agents read
//! (`CLAUDE.md`, `AGENTS.md`) and the Markdown files at the top of the project and
//! under `docs/`. Read from disk on request; nothing is watched or kept.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use muster_core::util::{err, invalid, not_found};
use muster_core::Result;
use muster_proto::{DocEntry, DocKind, ErrorCode};

/// At most this many files are listed.
pub const MAX_DOCS: usize = 300;
/// At most this much of a file is returned, and at most this much is written.
pub const MAX_BYTES: u64 = 512 * 1024;
/// Folders below `docs/` that are followed.
const MAX_DEPTH: usize = 5;
/// Folders of documentation, besides the top of the project.
const DOC_DIRS: &[&str] = &["docs", "doc"];
/// Where Claude Code also looks for its instructions.
const CLAUDE_DIR: &str = ".claude";

fn is_markdown(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown")
}

fn kind(path: &str) -> DocKind {
    let name = path.rsplit('/').next().unwrap_or(path);
    let top = !path.contains('/') || path.strip_suffix(name) == Some(".claude/");
    match name.to_ascii_lowercase().as_str() {
        "claude.md" | "claude.local.md" if top => DocKind::Claude,
        "agents.md" if top => DocKind::Agents,
        _ => DocKind::Doc,
    }
}

fn entry(file: &Path, path: String) -> Option<DocEntry> {
    // Follows a link, as agents do: `CLAUDE.md` is often a link to `AGENTS.md`.
    let meta = std::fs::metadata(file).ok().filter(|m| m.is_file())?;
    let modified_at = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Some(DocEntry { kind: kind(&path), path, size: meta.len(), modified_at })
}

/// The Markdown files directly in `dir`, and with `depth` left those in its folders.
/// Hidden entries and linked folders are skipped.
fn walk(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<DocEntry>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut names: Vec<(String, bool)> = read
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let is_dir = e.file_type().ok()?.is_dir();
            (!name.starts_with('.')).then_some((name, is_dir))
        })
        .collect();
    names.sort();
    for (name, is_dir) in names {
        // One more than the limit, so that the caller can tell there were more.
        if out.len() > MAX_DOCS {
            return;
        }
        let path = format!("{prefix}{name}");
        if is_dir {
            if depth > 0 {
                walk(&dir.join(&name), &format!("{path}/"), depth - 1, out);
            }
        } else if is_markdown(&name) {
            out.extend(entry(&dir.join(&name), path));
        }
    }
}

/// The project's documentation: instruction files first, then the README, then the
/// rest by path. The flag says the list was cut short.
pub fn list(root: &Path) -> (Vec<DocEntry>, bool) {
    let mut docs = Vec::new();
    walk(root, "", 0, &mut docs);
    walk(&root.join(CLAUDE_DIR), ".claude/", 0, &mut docs);
    docs.retain(|d| !d.path.starts_with(".claude/") || d.kind == DocKind::Claude);
    for dir in DOC_DIRS {
        // On a file system that ignores case `doc` and `docs` can be one folder.
        if docs.iter().any(|d| d.path.to_ascii_lowercase().starts_with(&format!("{dir}/"))) {
            continue;
        }
        if std::fs::symlink_metadata(root.join(dir)).is_ok_and(|m| m.is_dir()) {
            walk(&root.join(dir), &format!("{dir}/"), MAX_DEPTH, &mut docs);
        }
    }
    let readme = |d: &DocEntry| !d.path.contains('/') && d.path.to_ascii_lowercase().starts_with("readme.");
    docs.sort_by(|a, b| {
        (a.kind, !readme(a), a.path.contains('/'), &a.path).cmp(&(b.kind, !readme(b), b.path.contains('/'), &b.path))
    });
    let truncated = docs.len() > MAX_DOCS;
    docs.truncate(MAX_DOCS);
    (docs, truncated)
}

fn modified_ms(file: &Path) -> i64 {
    std::fs::metadata(file)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The file a path of the list names: inside the project's folder, following a link
/// only when it stays there.
fn existing(root: &Path, rel: &str) -> Result<PathBuf> {
    let path = Path::new(rel);
    if rel.is_empty() || !path.components().all(|c| matches!(c, Component::Normal(_))) || !is_markdown(rel) {
        return Err(invalid(format!("{rel} is not a documentation file of this project")));
    }
    let missing = || not_found("document", rel);
    let root = std::fs::canonicalize(root).map_err(|_| missing())?;
    let file = std::fs::canonicalize(root.join(path)).map_err(|_| missing())?;
    // A link may lead out of the project; that file is not the project's to show.
    if !file.starts_with(&root) || !file.is_file() {
        return Err(missing());
    }
    Ok(file)
}

/// The text of one documentation file, whether it was cut at [`MAX_BYTES`], and when
/// it was last changed. Only Markdown files inside the project's folder are read.
pub fn read(root: &Path, rel: &str) -> Result<(String, bool, i64)> {
    let file = existing(root, rel)?;
    let mut bytes = Vec::new();
    std::fs::File::open(&file)
        .and_then(|f| f.take(MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|e| invalid(format!("{rel}: {e}")))?;
    let truncated = bytes.len() as u64 > MAX_BYTES;
    bytes.truncate(MAX_BYTES as usize);
    Ok((String::from_utf8_lossy(&bytes).into_owned(), truncated, modified_ms(&file)))
}

/// Whether `rel` is a place the list shows, so that a file written there appears in it:
/// the top of the project, Claude's instructions in `.claude/`, or under `docs/`.
fn listed(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    let name = parts[parts.len() - 1];
    if rel.is_empty() || !is_markdown(name) || name.trim() != name || parts.iter().any(|p| p.is_empty()) {
        return false;
    }
    // The name is more than its extension.
    if name.rsplit_once('.').is_none_or(|(stem, _)| stem.is_empty()) {
        return false;
    }
    if parts.len() == 2 && parts[0] == CLAUDE_DIR {
        return kind(rel) == DocKind::Claude;
    }
    if parts.iter().any(|p| p.starts_with('.') || *p == ".." || p.contains('\\')) {
        return false;
    }
    parts.len() == 1 || (DOC_DIRS.iter().any(|d| parts[0].eq_ignore_ascii_case(d)) && parts.len() <= MAX_DEPTH + 2)
}

/// Saves a documentation file. With `create` the file must not exist yet (its folders
/// are made); without, it must exist, and when `expected` is given it must not have
/// changed since that time, so that an agent's edit is not overwritten unseen.
pub fn write(root: &Path, rel: &str, content: &str, create: bool, expected: Option<i64>) -> Result<DocEntry> {
    if !listed(rel) {
        return Err(invalid(format!("{rel}: documents are Markdown files at the top of the project or under docs/")));
    }
    if content.len() as u64 > MAX_BYTES {
        return Err(invalid(format!("{rel} is too long to save here")));
    }
    let io = |e: std::io::Error| invalid(format!("{rel}: {e}"));
    let target = if create {
        let root = std::fs::canonicalize(root).map_err(|_| not_found("project folder", &root.to_string_lossy()))?;
        let file = root.join(rel);
        let parent = file.parent().unwrap_or(&root).to_path_buf();
        // The folders that exist already must be the project's own, not a link out of it.
        let mut have = parent.clone();
        while !have.exists() {
            have.pop();
        }
        let inside = |p: &Path| std::fs::canonicalize(p).is_ok_and(|c| c.starts_with(&root));
        if !inside(&have) {
            return Err(invalid(format!("{rel} is outside the project")));
        }
        std::fs::create_dir_all(&parent).map_err(io)?;
        let mut f = match std::fs::OpenOptions::new().write(true).create_new(true).open(&file) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(err(ErrorCode::Conflict, format!("{rel} already exists")));
            }
            Err(e) => return Err(io(e)),
        };
        f.write_all(content.as_bytes()).map_err(io)?;
        file
    } else {
        let file = existing(root, rel)?;
        if expected.is_some_and(|at| at != modified_ms(&file)) {
            return Err(err(
                ErrorCode::Conflict,
                format!("{rel} was changed on disk after it was opened; refresh to see the new text"),
            ));
        }
        // Written beside the file and moved into place, so that a reader (an agent
        // starting up) never sees half of it.
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let tmp = file.with_file_name(format!(".{name}.muster-{}", std::process::id()));
        let res = std::fs::write(&tmp, content).and_then(|()| {
            if let Ok(meta) = std::fs::metadata(&file) {
                let _ = std::fs::set_permissions(&tmp, meta.permissions());
            }
            std::fs::rename(&tmp, &file)
        });
        if let Err(e) = res {
            let _ = std::fs::remove_file(&tmp);
            return Err(io(e));
        }
        file
    };
    entry(&target, rel.to_string()).ok_or_else(|| invalid(format!("{rel} could not be read back")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use muster_proto::ErrorCode;

    fn project(files: &[&str]) -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("muster-docs-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for f in files {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, format!("# {f}\n")).unwrap();
        }
        root
    }

    fn paths(root: &Path) -> Vec<String> {
        list(root).0.into_iter().map(|d| d.path).collect()
    }

    #[test]
    fn lists_instruction_files_then_readme_then_docs() {
        let root = project(&[
            "docs/wiki/Home.md",
            "docs/PLAN.md",
            "docs/diagram.png",
            "docs/.hidden/secret.md",
            "README.md",
            "CHANGELOG.md",
            "AGENTS.md",
            "CLAUDE.md",
            ".claude/CLAUDE.md",
            ".claude/commands/deploy.md",
            "src/notes.md",
            "node_modules/pkg/README.md",
        ]);
        assert_eq!(
            paths(&root),
            [
                "CLAUDE.md",
                ".claude/CLAUDE.md",
                "AGENTS.md",
                "README.md",
                "CHANGELOG.md",
                "docs/PLAN.md",
                "docs/wiki/Home.md"
            ]
        );
        let (docs, truncated) = list(&root);
        assert!(!truncated);
        assert_eq!(docs[0].kind, DocKind::Claude);
        assert_eq!(docs[2].kind, DocKind::Agents);
        assert_eq!(docs[3].kind, DocKind::Doc);
        assert_eq!(docs[3].size, "# README.md\n".len() as u64);
        assert!(docs[3].modified_at > 0);
        // A file named like an instruction file deeper down is an ordinary document.
        assert_eq!(kind("docs/AGENTS.md"), DocKind::Doc);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_project_without_documentation_lists_nothing() {
        let root = project(&["src/main.rs"]);
        assert!(paths(&root).is_empty());
        assert!(paths(&root.join("gone")).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reads_only_markdown_inside_the_project() {
        let root = project(&["proj/docs/a.md", "proj/Cargo.toml", "outside.md"]);
        let proj = root.join("proj");
        let (text, cut, at) = read(&proj, "docs/a.md").unwrap();
        assert_eq!((text.as_str(), cut), ("# proj/docs/a.md\n", false));
        assert_eq!(at, list(&proj).0[0].modified_at);
        for bad in ["", "../outside.md", "/etc/hosts", "docs/../../outside.md", "Cargo.toml", "./docs/a.md"] {
            assert_eq!(read(&proj, bad).unwrap_err().code, ErrorCode::Invalid, "{bad}");
        }
        assert_eq!(read(&proj, "docs/none.md").unwrap_err().code, ErrorCode::NotFound);
        std::os::unix::fs::symlink(root.join("outside.md"), proj.join("docs/link.md")).unwrap();
        assert_eq!(read(&proj, "docs/link.md").unwrap_err().code, ErrorCode::NotFound);
        // A link that stays inside the project is read.
        std::os::unix::fs::symlink(proj.join("docs/a.md"), proj.join("CLAUDE.md")).unwrap();
        assert_eq!(read(&proj, "CLAUDE.md").unwrap().0, "# proj/docs/a.md\n");
        assert_eq!(list(&proj).0[0].path, "CLAUDE.md");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn creates_files_where_the_list_looks() {
        let root = project(&["README.md"]);
        let made = write(&root, "docs/guides/Setup.md", "# Setup\n", true, None).unwrap();
        assert_eq!((made.path.as_str(), made.kind, made.size), ("docs/guides/Setup.md", DocKind::Doc, 8));
        assert_eq!(write(&root, "CLAUDE.md", "@AGENTS.md\n", true, None).unwrap().kind, DocKind::Claude);
        assert!(write(&root, ".claude/CLAUDE.local.md", "", true, None).is_ok());
        assert_eq!(paths(&root), ["CLAUDE.md", ".claude/CLAUDE.local.md", "README.md", "docs/guides/Setup.md"]);
        // Creating never replaces a file.
        assert_eq!(write(&root, "README.md", "x", true, None).unwrap_err().code, ErrorCode::Conflict);
        assert_eq!(read(&root, "README.md").unwrap().0, "# README.md\n");
        for bad in [
            "",
            "notes.txt",
            ".md",
            "src/notes.md",
            "../out.md",
            "/tmp/out.md",
            "docs/../../out.md",
            "docs//a.md",
            "docs/.hidden/a.md",
            ".claude/commands/x.md",
            ".github/README.md",
            ".secret.md",
            "docs/a/b/c/d/e/f/g.md",
        ] {
            assert_eq!(write(&root, bad, "x", true, None).unwrap_err().code, ErrorCode::Invalid, "{bad}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn saves_a_file_unless_it_changed_underneath() {
        let root = project(&["proj/docs/a.md", "outside/x.md"]);
        let proj = root.join("proj");
        let at = read(&proj, "docs/a.md").unwrap().2;
        let saved = write(&proj, "docs/a.md", "new text", false, Some(at)).unwrap();
        assert_eq!(read(&proj, "docs/a.md").unwrap().0, "new text");
        assert_eq!(saved.size, 8);
        // Nothing is left beside the file.
        assert_eq!(std::fs::read_dir(proj.join("docs")).unwrap().count(), 1);
        // Someone else wrote it since: the save is refused and their text stays.
        assert_eq!(
            write(&proj, "docs/a.md", "mine", false, Some(saved.modified_at - 5000)).unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(read(&proj, "docs/a.md").unwrap().0, "new text");
        assert_eq!(write(&proj, "docs/none.md", "x", false, None).unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(
            write(&proj, "docs/a.md", &"a".repeat(MAX_BYTES as usize + 1), false, None).unwrap_err().code,
            ErrorCode::Invalid
        );
        // A link to another file of the project is written through, and stays a link.
        std::os::unix::fs::symlink(proj.join("docs/a.md"), proj.join("CLAUDE.md")).unwrap();
        write(&proj, "CLAUDE.md", "shared", false, None).unwrap();
        assert_eq!(read(&proj, "docs/a.md").unwrap().0, "shared");
        assert!(std::fs::symlink_metadata(proj.join("CLAUDE.md")).unwrap().file_type().is_symlink());
        // Nothing is written through a link that leaves the project.
        std::os::unix::fs::symlink(root.join("outside/x.md"), proj.join("docs/out.md")).unwrap();
        assert_eq!(write(&proj, "docs/out.md", "x", false, None).unwrap_err().code, ErrorCode::NotFound);
        std::fs::remove_dir_all(proj.join("docs")).unwrap();
        std::os::unix::fs::symlink(root.join("outside"), proj.join("docs")).unwrap();
        assert_eq!(write(&proj, "docs/new.md", "x", true, None).unwrap_err().code, ErrorCode::Invalid);
        assert_eq!(write(&proj, "docs/sub/new.md", "x", true, None).unwrap_err().code, ErrorCode::Invalid);
        assert_eq!(std::fs::read_dir(root.join("outside")).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_long_file_is_cut() {
        let root = project(&[]);
        std::fs::write(root.join("big.md"), vec![b'a'; MAX_BYTES as usize + 10]).unwrap();
        let (text, truncated, _) = read(&root, "big.md").unwrap();
        assert!(truncated);
        assert_eq!(text.len() as u64, MAX_BYTES);
        std::fs::remove_dir_all(root).unwrap();
    }
}
