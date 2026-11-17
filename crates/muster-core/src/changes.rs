//! What a card changed in its folder, for the card's Changes tab: the files that differ
//! from the branch the card started from, committed or not, and the diff of one file.
//! Read from git on request; nothing is watched or kept.

use std::io::Read;
use std::path::Path;

use muster_proto::{CardChangesResult, ChangeKind, ChangeScope, ChangedFile, ErrorCode};
use tokio::process::Command;

use crate::util::{err, invalid, Result};

/// At most this many files are listed.
pub const MAX_FILES: usize = 500;
/// At most this much of one file's diff is returned.
pub const MAX_PATCH: usize = 512 * 1024;
/// Branches tried as the base when the project folder does not name one.
const DEFAULT_BASES: &[&str] = &["main", "master", "trunk", "develop"];
/// At most this many branches are offered to compare against.
const MAX_BRANCHES: usize = 200;
/// Git's empty tree, the base of a repository without commits.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// What to compare: `base` overrides the automatic base branch; `scope` picks committed
/// work, uncommitted work or both.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options<'a> {
    pub base: Option<&'a str>,
    pub scope: ChangeScope,
}

/// Runs git in `dir` and returns its exit code and output. Settings in the repository
/// that would run other programs (external diff, text conversion, fsmonitor) are off.
async fn run(dir: &Path, args: &[&str]) -> Result<(i32, Vec<u8>, String)> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "core.quotepath=off", "-c", "core.fsmonitor=false", "-c", "diff.noprefix=false"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| err(ErrorCode::Unavailable, format!("git: {e}")))?;
    Ok((out.status.code().unwrap_or(-1), out.stdout, String::from_utf8_lossy(&out.stderr).trim().to_string()))
}

async fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let (code, out, stderr) = run(dir, args).await?;
    if code == 0 {
        Ok(String::from_utf8_lossy(&out).trim_end_matches('\n').to_string())
    } else {
        Err(invalid(format!("git {}: {stderr}", args.join(" "))))
    }
}

async fn git_ok(dir: &Path, args: &[&str]) -> Option<String> {
    git(dir, args).await.ok().filter(|s| !s.is_empty())
}

/// The repository a folder is in: its top folder and its shared git folder.
async fn repo(dir: &Path) -> Result<(String, String)> {
    let not_repo = || invalid(format!("{} is not in a git repository", dir.display()));
    if !dir.is_dir() {
        return Err(invalid(format!("{} does not exist", dir.display())));
    }
    let top = git_ok(dir, &["rev-parse", "--show-toplevel"]).await.ok_or_else(not_repo)?;
    let common =
        git_ok(dir, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).await.ok_or_else(not_repo)?;
    Ok((top, common))
}

/// The branch the card's work is measured against: the one checked out in the project
/// folder when that is the same repository, else the remote's default, else a usual
/// name that exists.
async fn base_branch(root: &Path, common: &str, project: &Path) -> Option<String> {
    if let Ok((_, pc)) = repo(project).await {
        if pc == common {
            if let Some(b) = git_ok(project, &["symbolic-ref", "--short", "-q", "HEAD"]).await {
                return Some(b);
            }
        }
    }
    if let Some(r) = git_ok(root, &["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"]).await {
        return Some(r);
    }
    for b in DEFAULT_BASES {
        if git_ok(root, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{b}")]).await.is_some() {
            return Some(b.to_string());
        }
    }
    None
}

/// Where the comparison starts: the base name and the commit.
struct Base {
    name: String,
    commit: String,
}

/// The commit checked out, or the empty tree in a repository without commits.
async fn head_commit(root: &Path) -> String {
    git_ok(root, &["rev-parse", "--verify", "--quiet", "HEAD"]).await.unwrap_or_else(|| EMPTY_TREE.to_string())
}

/// Whether `name` is a local or remote branch of the repository.
async fn is_branch(root: &Path, name: &str) -> bool {
    if name.is_empty() || name.starts_with('-') {
        return false;
    }
    for prefix in ["refs/heads/", "refs/remotes/"] {
        if git_ok(root, &["rev-parse", "--verify", "--quiet", &format!("{prefix}{name}^{{commit}}")]).await.is_some() {
            return true;
        }
    }
    false
}

/// The base to compare against: the chosen branch, or the automatic one (`HEAD` when
/// the card works on that branch itself or none is found).
async fn base(root: &Path, auto: Option<&str>, chosen: Option<&str>, branch: Option<&str>) -> Result<Base> {
    let head = || async { Base { name: "HEAD".into(), commit: head_commit(root).await } };
    let name = match chosen {
        Some(c) if !is_branch(root, c).await => return Err(invalid(format!("no branch named {c:?}"))),
        Some(c) => c.to_string(),
        None => match auto {
            Some(a) if Some(a) != branch => a.to_string(),
            _ => return Ok(head().await),
        },
    };
    let full = if name.starts_with("refs/") { name.clone() } else { format!("{name}^{{commit}}") };
    match git_ok(root, &["merge-base", &full, "HEAD"]).await {
        Some(commit) => Ok(Base { name, commit }),
        None => Err(invalid(format!("{name} has no history in common with this branch"))),
    }
}

/// The local branches, and the automatic base when it is a remote one.
async fn branches(root: &Path, auto: Option<&str>) -> Vec<String> {
    let out = git_ok(root, &["for-each-ref", "--format=%(refname:short)", "--sort=-committerdate", "refs/heads"])
        .await
        .unwrap_or_default();
    let mut v: Vec<String> = out.lines().take(MAX_BRANCHES).map(str::to_string).collect();
    if let Some(a) = auto {
        if !v.iter().any(|b| b == a) {
            v.insert(0, a.to_string());
        }
    }
    v
}

/// Splits git's `-z` output into its fields.
fn fields(out: &str) -> Vec<&str> {
    let mut v: Vec<&str> = out.split('\0').collect();
    if v.last() == Some(&"") {
        v.pop();
    }
    v
}

/// Parses `git diff --name-status -z` and `git diff --numstat -z` into files.
fn parse(name_status: &str, numstat: &str) -> Vec<ChangedFile> {
    let mut files = Vec::new();
    let f = fields(name_status);
    let mut i = 0;
    while i < f.len() {
        let code = f[i];
        let kind = match code.chars().next() {
            Some('A') => ChangeKind::Added,
            Some('D') => ChangeKind::Deleted,
            Some('R') | Some('C') => ChangeKind::Renamed,
            _ => ChangeKind::Modified,
        };
        if kind == ChangeKind::Renamed {
            let (Some(old), Some(new)) = (f.get(i + 1), f.get(i + 2)) else { break };
            files.push(ChangedFile {
                path: new.to_string(),
                old_path: Some(old.to_string()),
                kind,
                additions: 0,
                deletions: 0,
                binary: false,
            });
            i += 3;
        } else {
            let Some(path) = f.get(i + 1) else { break };
            files.push(ChangedFile {
                path: path.to_string(),
                old_path: None,
                kind,
                additions: 0,
                deletions: 0,
                binary: false,
            });
            i += 2;
        }
    }
    // numstat: "add\tdel\tpath" or, for a rename, "add\tdel\t" followed by old and new.
    let f = fields(numstat);
    let mut i = 0;
    while i < f.len() {
        let mut parts = f[i].splitn(3, '\t');
        let (Some(a), Some(d), Some(p)) = (parts.next(), parts.next(), parts.next()) else { break };
        let path = if p.is_empty() {
            i += 2;
            f.get(i).copied().unwrap_or_default()
        } else {
            p
        };
        i += 1;
        if let Some(file) = files.iter_mut().find(|x| x.path == path) {
            file.binary = a == "-" && d == "-";
            file.additions = a.parse().unwrap_or(0);
            file.deletions = d.parse().unwrap_or(0);
        }
    }
    files
}

/// Lines in a file git does not track, and whether it looks binary (a NUL byte near
/// the start, as git decides).
fn count_lines(file: &Path) -> (u32, bool) {
    let Ok(mut f) = std::fs::File::open(file) else { return (0, false) };
    let mut buf = vec![0u8; 64 * 1024];
    let (mut lines, mut last, mut first) = (0u32, b'\n', true);
    loop {
        let n = match f.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if first && buf[..n.min(8000)].contains(&0) {
            return (0, true);
        }
        first = false;
        lines = lines.saturating_add(buf[..n].iter().filter(|&&b| b == b'\n').count() as u32);
        last = buf[n - 1];
    }
    if last != b'\n' {
        lines = lines.saturating_add(1);
    }
    (lines, false)
}

/// The files git does not track and does not ignore.
async fn untracked(root: &Path) -> Result<Vec<String>> {
    let out = git(root, &["ls-files", "--others", "--exclude-standard", "-z"]).await?;
    Ok(fields(&out).into_iter().map(str::to_string).collect())
}

/// The changes of the folder `dir` against its base. `project` is the project's folder,
/// whose checked-out branch is the usual base.
pub async fn list(dir: &Path, project: &Path, opts: Options<'_>) -> Result<CardChangesResult> {
    Ok(listing(dir, project, opts).await?.0)
}

/// The commits a scope's `git diff` takes: from the base to the working tree, from the
/// base to `HEAD`, or from `HEAD` to the working tree.
fn range(scope: ChangeScope, base: &str, head: &str) -> Vec<String> {
    match scope {
        ChangeScope::All => vec![base.to_string()],
        ChangeScope::Committed => vec![base.to_string(), head.to_string()],
        ChangeScope::Uncommitted => vec![head.to_string()],
    }
}

/// The listing and the full commits of the range it was made from.
async fn listing(dir: &Path, project: &Path, opts: Options<'_>) -> Result<(CardChangesResult, Vec<String>)> {
    let (top, common) = repo(dir).await?;
    let root = Path::new(&top);
    let branch = git_ok(root, &["symbolic-ref", "--short", "-q", "HEAD"]).await;
    let auto = base_branch(root, &common, project).await;
    let base = base(root, auto.as_deref(), opts.base.filter(|b| !b.is_empty()), branch.as_deref()).await?;
    let head = head_commit(root).await;
    let range = range(opts.scope, &base.commit, &head);
    let diff: Vec<&str> = ["diff", "--no-ext-diff", "--no-textconv", "-M", "-z"]
        .into_iter()
        .chain(range.iter().map(String::as_str))
        .collect();
    let name_status = git(root, &[&diff[..], &["--name-status"]].concat()).await?;
    let numstat = git(root, &[&diff[..], &["--numstat"]].concat()).await?;
    let mut files = parse(&name_status, &numstat);
    let with_untracked = opts.scope != ChangeScope::Committed;
    for path in if with_untracked { untracked(root).await? } else { Vec::new() } {
        if files.len() > MAX_FILES {
            break;
        }
        let (additions, binary) = count_lines(&root.join(&path));
        files.push(ChangedFile { path, old_path: None, kind: ChangeKind::Untracked, additions, deletions: 0, binary });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let truncated = files.len() > MAX_FILES;
    files.truncate(MAX_FILES);
    let default_base = match auto.as_deref() {
        Some(a) if Some(a) != branch.as_deref() => a.to_string(),
        _ => "HEAD".to_string(),
    };
    let branches = branches(root, auto.as_deref()).await;
    let commits = if base.name == "HEAD" {
        0
    } else {
        git_ok(root, &["rev-list", "--count", &format!("{}..HEAD", base.commit)])
            .await
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    };
    let uncommitted = git_ok(root, &["status", "--porcelain", "-z", "--untracked-files=normal"]).await.is_some();
    let short = git_ok(root, &["rev-parse", "--short", &base.commit]).await.unwrap_or(base.commit.clone());
    let result = CardChangesResult {
        root: top,
        branch,
        base: base.name,
        default_base,
        branches,
        scope: opts.scope,
        base_commit: short,
        commits,
        uncommitted,
        files,
        truncated,
    };
    Ok((result, range))
}

/// Cuts a patch at the last whole line within the limit.
fn cut(patch: Vec<u8>) -> (String, bool) {
    if patch.len() <= MAX_PATCH {
        return (String::from_utf8_lossy(&patch).into_owned(), false);
    }
    let end = patch[..MAX_PATCH].iter().rposition(|&b| b == b'\n').map(|i| i + 1).unwrap_or(MAX_PATCH);
    (String::from_utf8_lossy(&patch[..end]).into_owned(), true)
}

/// The unified diff of one changed file of the folder `dir`. Only a path that
/// `list` reports is accepted.
pub async fn diff(dir: &Path, project: &Path, opts: Options<'_>, path: &str) -> Result<(String, bool)> {
    let (changes, range) = listing(dir, project, opts).await?;
    let file = changes
        .files
        .iter()
        .find(|f| f.path == path)
        .ok_or_else(|| err(ErrorCode::NotFound, format!("{path} has no changes")))?;
    let root = Path::new(&changes.root);
    let (code, out, stderr) = if file.kind == ChangeKind::Untracked {
        run(root, &["diff", "--no-ext-diff", "--no-textconv", "--no-index", "--", "/dev/null", path]).await?
    } else {
        let new = format!(":(literal){path}");
        let mut args = vec!["diff", "--no-ext-diff", "--no-textconv", "-M"];
        args.extend(range.iter().map(String::as_str));
        args.extend(["--", new.as_str()]);
        let old = file.old_path.as_ref().map(|o| format!(":(literal){o}"));
        if let Some(o) = &old {
            args.push(o);
        }
        run(root, &args).await?
    };
    // `--no-index` exits with 1 when the files differ.
    if code != 0 && !(code == 1 && file.kind == ChangeKind::Untracked) {
        return Err(invalid(format!("git diff: {stderr}")));
    }
    Ok(cut(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: Options<'static> = Options { base: None, scope: ChangeScope::All };

    #[test]
    fn parses_name_status_and_numstat() {
        let ns = "M\0src/a.rs\0R087\0old.txt\0new.txt\0A\0bin.png\0D\0gone.md\0";
        let num = "3\t1\tsrc/a.rs\0".to_string() + "2\t2\t\0old.txt\0new.txt\0-\t-\tbin.png\0" + "0\t9\tgone.md\0";
        let f = parse(ns, &num);
        assert_eq!(f.len(), 4);
        assert_eq!((f[0].kind, f[0].additions, f[0].deletions), (ChangeKind::Modified, 3, 1));
        assert_eq!(
            (f[1].kind, f[1].old_path.as_deref(), f[1].path.as_str()),
            (ChangeKind::Renamed, Some("old.txt"), "new.txt")
        );
        assert_eq!((f[1].additions, f[1].deletions), (2, 2));
        assert!(f[2].binary && f[2].kind == ChangeKind::Added);
        assert_eq!((f[3].kind, f[3].deletions), (ChangeKind::Deleted, 9));
    }

    #[test]
    fn cuts_long_patches_at_a_line() {
        let long = "x".repeat(100) + "\n";
        let (p, t) = cut(long.repeat(MAX_PATCH / 101 + 10).into_bytes());
        assert!(t && p.len() <= MAX_PATCH && p.ends_with('\n'));
        assert_eq!(cut(b"a\n".to_vec()), ("a\n".to_string(), false));
    }

    async fn g(dir: &Path, args: &[&str]) {
        let mut all = vec!["-c", "user.email=t@t", "-c", "user.name=t"];
        all.extend_from_slice(args);
        git(dir, &all).await.unwrap();
    }

    #[tokio::test]
    async fn lists_and_diffs_a_worktree_against_its_base() {
        let root = std::env::temp_dir().join(format!("muster-ch-{}", crate::util::new_id()));
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        g(&repo, &["init", "-q", "-b", "main"]).await;
        std::fs::write(repo.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(repo.join("old name.txt"), "keep\nthese\nlines\nfor\nrename\n").unwrap();
        g(&repo, &["add", "."]).await;
        g(&repo, &["commit", "-q", "-m", "init"]).await;
        let wt = crate::worktree::create(&repo, "feat/x").await.unwrap();

        // Nothing changed yet.
        let c = list(&wt, &repo, D).await.unwrap();
        assert_eq!((c.base.as_str(), c.branch.as_deref(), c.files.len()), ("main", Some("feat/x"), 0));
        assert!(!c.uncommitted);

        // A commit on the branch, an uncommitted edit, a rename and a new file.
        std::fs::write(wt.join("a.txt"), "one\nTWO\nthree\nfour\n").unwrap();
        g(&wt, &["commit", "-q", "-am", "edit"]).await;
        g(&wt, &["mv", "old name.txt", "new name.txt"]).await;
        std::fs::write(wt.join("fresh.txt"), "x\ny").unwrap();
        // The base moving on does not show as the card's change.
        std::fs::write(repo.join("main-only.txt"), "m\n").unwrap();
        g(&repo, &["add", "."]).await;
        g(&repo, &["commit", "-q", "-m", "main moves"]).await;

        let c = list(&wt, &repo, D).await.unwrap();
        assert_eq!((c.commits, c.uncommitted), (1, true));
        let paths: Vec<_> = c.files.iter().map(|f| (f.path.as_str(), f.kind)).collect();
        assert_eq!(
            paths,
            vec![
                ("a.txt", ChangeKind::Modified),
                ("fresh.txt", ChangeKind::Untracked),
                ("new name.txt", ChangeKind::Renamed)
            ]
        );
        assert_eq!((c.files[0].additions, c.files[0].deletions), (2, 1));
        assert_eq!(c.files[1].additions, 2);

        let (p, t) = diff(&wt, &repo, D, "a.txt").await.unwrap();
        assert!(!t && p.contains("-two\n+TWO\n") && p.contains("+four"));
        let (p, _) = diff(&wt, &repo, D, "fresh.txt").await.unwrap();
        assert!(p.contains("+x\n+y"));
        let (p, _) = diff(&wt, &repo, D, "new name.txt").await.unwrap();
        assert!(p.contains("rename from old name.txt"));
        // Committed only: the edit, not the rename or the new file.
        let only = |scope| Options { base: None, scope };
        let c = list(&wt, &repo, only(ChangeScope::Committed)).await.unwrap();
        assert_eq!(c.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), vec!["a.txt"]);
        assert_eq!(c.scope, ChangeScope::Committed);
        // Uncommitted only: the rename and the new file.
        let c = list(&wt, &repo, only(ChangeScope::Uncommitted)).await.unwrap();
        assert_eq!(c.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), vec!["fresh.txt", "new name.txt"]);
        let (p, _) = diff(&wt, &repo, only(ChangeScope::Uncommitted), "new name.txt").await.unwrap();
        assert!(p.contains("rename from old name.txt"));
        assert_eq!(
            diff(&wt, &repo, only(ChangeScope::Uncommitted), "a.txt").await.unwrap_err().code,
            ErrorCode::NotFound
        );

        // Another base: against main's tip, main's own new file shows as removed.
        assert_eq!((c.default_base.as_str(), c.branches.contains(&"main".to_string())), ("main", true));
        g(&repo, &["branch", "side", "HEAD~1"]).await;
        let c = list(&wt, &repo, Options { base: Some("side"), scope: ChangeScope::All }).await.unwrap();
        assert_eq!((c.base.as_str(), c.commits), ("side", 1));
        let c = list(&wt, &repo, Options { base: Some("feat/x"), scope: ChangeScope::All }).await.unwrap();
        assert_eq!((c.commits, c.files.len()), (0, 2));
        for bad in ["nope", "--output=/tmp/x", "HEAD~1"] {
            let e = list(&wt, &repo, Options { base: Some(bad), scope: ChangeScope::All }).await.unwrap_err();
            assert_eq!(e.code, ErrorCode::Invalid, "{bad}");
        }

        // Only changed files, never anything outside.
        assert_eq!(diff(&wt, &repo, D, "../repo/a.txt").await.unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(diff(&wt, &repo, D, "main-only.txt").await.unwrap_err().code, ErrorCode::NotFound);

        // The project folder on main shows only its own uncommitted changes.
        std::fs::write(repo.join("a.txt"), "changed\n").unwrap();
        let c = list(&repo, &repo, D).await.unwrap();
        assert_eq!((c.base.as_str(), c.commits, c.files.len()), ("HEAD", 0, 1));

        let not_repo = root.join("plain");
        std::fs::create_dir_all(&not_repo).unwrap();
        assert_eq!(list(&not_repo, &repo, D).await.unwrap_err().code, ErrorCode::Invalid);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
