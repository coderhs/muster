//! Builds the Tauri shell and records the git commit it is built from, for the About
//! section in Settings. Outside a git checkout the values are "unknown".

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    tauri_build::build();
    let mut commit = git(&["rev-parse", "--short=9", "HEAD"]).unwrap_or_else(|| "unknown".into());
    // Built with uncommitted changes to tracked files.
    if git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty()) {
        commit.push('+');
    }
    let date = git(&["show", "-s", "--format=%cI", "HEAD"]).unwrap_or_default();
    println!("cargo:rustc-env=MUSTER_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=MUSTER_BUILD_COMMIT_DATE={date}");
    // Run again when HEAD moves, not only when this crate's files change.
    if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        println!("cargo:rerun-if-changed={dir}/HEAD");
        println!("cargo:rerun-if-changed={dir}/packed-refs");
        if let Some(r) = git(&["symbolic-ref", "-q", "HEAD"]) {
            println!("cargo:rerun-if-changed={dir}/{r}");
        }
    }
}
