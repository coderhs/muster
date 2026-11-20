//! Opening a file from the file viewer in the user's editor, at a line. The editor is
//! chosen in Settings and kept in the app's config folder (`editor.json`).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/// How an editor is told to open a file at a line.
#[derive(Clone, Copy)]
enum Open {
    /// `<scheme>://file<path>:<line>:<col>`, opened with `open`. Works without the
    /// editor's command-line tool installed.
    Url(&'static str),
    /// Sublime Text's `subl`, inside the app: `subl <path>:<line>:<col>`.
    Subl,
    /// Xcode's `xed -l <line> <path>`.
    Xed,
    /// A JetBrains IDE's own executable: `<exe> --line <line> --column <col> <path>`. It
    /// hands the file to the running IDE.
    JetBrains,
    /// `open -a <app> <path>`, without a line.
    App,
}

struct Editor {
    id: &'static str,
    label: &'static str,
    /// App bundle names, any of which means the editor is installed.
    apps: &'static [&'static str],
    open: Open,
}

const EDITORS: &[Editor] = &[
    Editor { id: "cursor", label: "Cursor", apps: &["Cursor.app"], open: Open::Url("cursor") },
    Editor { id: "vscode", label: "Visual Studio Code", apps: &["Visual Studio Code.app"], open: Open::Url("vscode") },
    Editor {
        id: "vscode-insiders",
        label: "VS Code Insiders",
        apps: &["Visual Studio Code - Insiders.app"],
        open: Open::Url("vscode-insiders"),
    },
    Editor { id: "windsurf", label: "Windsurf", apps: &["Windsurf.app"], open: Open::Url("windsurf") },
    Editor { id: "zed", label: "Zed", apps: &["Zed.app", "Zed Preview.app"], open: Open::Url("zed") },
    Editor { id: "sublime", label: "Sublime Text", apps: &["Sublime Text.app"], open: Open::Subl },
    Editor { id: "xcode", label: "Xcode", apps: &["Xcode.app"], open: Open::Xed },
    Editor {
        id: "intellij",
        label: "IntelliJ IDEA",
        apps: &["IntelliJ IDEA.app", "IntelliJ IDEA CE.app", "IntelliJ IDEA Ultimate.app"],
        open: Open::JetBrains,
    },
    Editor { id: "webstorm", label: "WebStorm", apps: &["WebStorm.app"], open: Open::JetBrains },
    Editor { id: "rubymine", label: "RubyMine", apps: &["RubyMine.app"], open: Open::JetBrains },
    Editor {
        id: "pycharm",
        label: "PyCharm",
        apps: &["PyCharm.app", "PyCharm CE.app", "PyCharm Community Edition.app", "PyCharm Professional Edition.app"],
        open: Open::JetBrains,
    },
    Editor { id: "goland", label: "GoLand", apps: &["GoLand.app"], open: Open::JetBrains },
    Editor { id: "rustrover", label: "RustRover", apps: &["RustRover.app"], open: Open::JetBrains },
    Editor { id: "phpstorm", label: "PhpStorm", apps: &["PhpStorm.app"], open: Open::JetBrains },
    Editor { id: "nova", label: "Nova", apps: &["Nova.app"], open: Open::App },
    Editor { id: "bbedit", label: "BBEdit", apps: &["BBEdit.app"], open: Open::App },
];

/// The default app for the file's type (`open <path>`).
const SYSTEM: &str = "system";
/// The user's own command, with `{file}`, `{line}` and `{col}`.
const CUSTOM: &str = "custom";

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct EditorPref {
    /// An editor id, `system` or `custom`. Empty: not chosen yet.
    #[serde(default)]
    pub editor: String,
    /// The command for `custom`.
    #[serde(default)]
    pub command: String,
}

#[derive(Serialize)]
pub struct EditorChoice {
    pub id: String,
    pub label: String,
}

fn app_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/Applications"), PathBuf::from("/System/Applications")];
    if let Some(h) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(&h).join("Applications"));
        // JetBrains Toolbox keeps its IDEs here.
        dirs.push(PathBuf::from(h).join("Applications/JetBrains Toolbox"));
    }
    dirs
}

fn find_app(e: &Editor) -> Option<PathBuf> {
    app_dirs().into_iter().flat_map(|d| e.apps.iter().map(move |a| d.join(a))).find(|p| p.is_dir())
}

/// The installed editors, then the system default and a custom command.
pub fn installed() -> Vec<EditorChoice> {
    let mut out: Vec<EditorChoice> = EDITORS
        .iter()
        .filter(|e| find_app(e).is_some())
        .map(|e| EditorChoice { id: e.id.into(), label: e.label.into() })
        .collect();
    out.push(EditorChoice { id: SYSTEM.into(), label: "Default app for the file".into() });
    out.push(EditorChoice { id: CUSTOM.into(), label: "Custom command".into() });
    out
}

/// The saved choice; when none is saved, Cursor if installed, else the first installed
/// editor, else the system default.
pub fn resolved(saved: EditorPref) -> EditorPref {
    if !saved.editor.is_empty() {
        return saved;
    }
    let first = EDITORS.iter().find(|e| find_app(e).is_some()).map(|e| e.id).unwrap_or(SYSTEM);
    EditorPref { editor: first.into(), command: saved.command }
}

pub fn read(path: Option<PathBuf>) -> EditorPref {
    path.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn write(path: Option<PathBuf>, pref: &EditorPref) -> Result<(), String> {
    let path = path.ok_or("no config directory")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, serde_json::to_string(pref).unwrap()).map_err(|e| e.to_string())
}

fn percent_path(p: &str) -> String {
    let mut out = String::new();
    for b in p.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The custom command with its placeholders filled in (each value shell-quoted).
fn fill(template: &str, file: &str, line: u32, col: u32) -> String {
    template
        .replace("{file}", &shell_quote(file))
        .replace("{line}", &line.to_string())
        .replace("{col}", &col.to_string())
}

fn jetbrains_exe(app: &Path) -> Option<PathBuf> {
    let dir = app.join("Contents/MacOS");
    let mut files: Vec<PathBuf> =
        std::fs::read_dir(&dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
    files.sort();
    files.into_iter().next()
}

/// The command that opens `file` at `line`:`col` with `pref`.
fn command(pref: &EditorPref, file: &str, line: u32, col: u32) -> Result<Command, String> {
    let open = |args: &[&str]| {
        let mut c = Command::new("/usr/bin/open");
        c.args(args);
        c
    };
    match pref.editor.as_str() {
        SYSTEM => return Ok(open(&[file])),
        CUSTOM => {
            if !pref.command.contains("{file}") {
                return Err("the custom editor command needs {file} in it (Settings)".into());
            }
            let mut c = Command::new("/bin/sh");
            c.args(["-c", &fill(&pref.command, file, line, col)]);
            return Ok(c);
        }
        _ => {}
    }
    let e = EDITORS.iter().find(|e| e.id == pref.editor).ok_or_else(|| format!("unknown editor {}", pref.editor))?;
    let app = find_app(e).ok_or_else(|| format!("{} is not installed", e.label))?;
    Ok(match e.open {
        Open::Url(scheme) => open(&[&format!("{scheme}://file{}:{line}:{col}", percent_path(file))]),
        Open::Subl => {
            let mut c = Command::new(app.join("Contents/SharedSupport/bin/subl"));
            c.arg(format!("{file}:{line}:{col}"));
            c
        }
        Open::Xed => {
            let mut c = Command::new("/usr/bin/xed");
            c.args(["-l", &line.to_string(), file]);
            c
        }
        Open::JetBrains => {
            let exe = jetbrains_exe(&app).ok_or_else(|| format!("{} could not be started", e.label))?;
            let mut c = Command::new(exe);
            c.args(["--line", &line.to_string(), "--column", &col.to_string(), file]);
            c
        }
        Open::App => open(&["-a", &app.to_string_lossy(), file]),
    })
}

pub fn open_file(pref: &EditorPref, file: &str, line: Option<u32>, col: Option<u32>) -> Result<(), String> {
    if !Path::new(file).is_absolute() || !Path::new(file).exists() {
        return Err(format!("{file} is not a file on this Mac"));
    }
    let mut c = command(pref, file, line.unwrap_or(1).max(1), col.unwrap_or(1).max(1))?;
    c.spawn().map(|_| ()).map_err(|e| e.to_string())
}

pub fn reveal(file: &str) -> Result<(), String> {
    if !Path::new(file).is_absolute() || !Path::new(file).exists() {
        return Err(format!("{file} is not a file on this Mac"));
    }
    Command::new("/usr/bin/open").args(["-R", file]).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_custom_commands_carry_the_line() {
        assert_eq!(percent_path("/Users/me/My Proj/a#b.rs"), "/Users/me/My%20Proj/a%23b.rs");
        assert_eq!(fill("nvim +{line} {file}", "/tmp/it's here.rs", 12, 3), r"nvim +12 '/tmp/it'\''s here.rs'");
        let pref = EditorPref { editor: CUSTOM.into(), command: "nvim".into() };
        assert!(command(&pref, "/tmp/x", 1, 1).is_err());
    }
}
