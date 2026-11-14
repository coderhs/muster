//! Git worktrees for cards.

use std::path::{Path, PathBuf};

use muster_proto::ErrorCode;
use tokio::process::Command;

use crate::util::{err, invalid, Result};

fn valid_branch(branch: &str) -> bool {
    !branch.is_empty()
        && !branch.starts_with('-')
        && !branch.starts_with('/')
        && !branch.ends_with('/')
        && !branch.ends_with(".lock")
        && !branch.contains("..")
        && !branch.contains("//")
        && branch.chars().all(|c| !c.is_control() && !" ~^:?*[\\".contains(c))
}

/// Where the worktree for `branch` goes: beside the project, named
/// `<project-dir>-<branch with / replaced by ->`.
pub fn worktree_path(project: &Path, branch: &str) -> PathBuf {
    let base = project.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "project".into());
    let slug: String = branch.chars().map(|c| if c == '/' { '-' } else { c }).collect();
    project.with_file_name(format!("{base}-{slug}"))
}

async fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .map_err(|e| err(ErrorCode::Unavailable, format!("git: {e}")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(err(ErrorCode::Invalid, format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim())))
    }
}

/// Creates a worktree for `branch` beside the project and returns its path.
/// An existing branch is checked out; otherwise it is created from HEAD.
pub async fn create(project: &Path, branch: &str) -> Result<PathBuf> {
    if !valid_branch(branch) {
        return Err(invalid(format!("invalid branch name {branch:?}")));
    }
    git(project, &["rev-parse", "--git-dir"]).await.map_err(|_| invalid(format!("{} is not a git repository", project.display())))?;
    let path = worktree_path(project, branch);
    if path.exists() {
        return Err(err(ErrorCode::Conflict, format!("{} already exists", path.display())));
    }
    let path_s = path.to_string_lossy().into_owned();
    let exists = git(project, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).await.is_ok();
    if exists {
        git(project, &["worktree", "add", &path_s, branch]).await?;
    } else {
        git(project, &["worktree", "add", "-b", branch, &path_s]).await?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_names() {
        assert_eq!(worktree_path(Path::new("/a/proj"), "feat/x"), PathBuf::from("/a/proj-feat-x"));
        assert!(valid_branch("feat/x-1"));
        assert!(!valid_branch("-x"));
        assert!(!valid_branch("a..b"));
        assert!(!valid_branch("a b"));
    }

    #[tokio::test]
    async fn creates_worktree() {
        let root = std::env::temp_dir().join(format!("muster-wt-{}", crate::util::new_id()));
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "--allow-empty", "-m", "init"],
        ] {
            git(&repo, &args).await.unwrap();
        }
        let wt = create(&repo, "feat/one").await.unwrap();
        assert_eq!(wt, root.join("repo-feat-one"));
        assert!(wt.join(".git").exists());
        assert_eq!(create(&repo, "feat/one").await.unwrap_err().code, ErrorCode::Conflict);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
