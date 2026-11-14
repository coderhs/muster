//! launchd agent so the daemon starts at login.

use std::path::PathBuf;

pub const LABEL: &str = "com.muster.daemon";

fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn plist_path() -> PathBuf {
    home_dir().join("Library/LaunchAgents").join(format!("{LABEL}.plist"))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The plist for the currently running `musterd` binary. `MUSTER_HOME` and
/// `MUSTER_TMUX_SOCKET` are carried over when set, so a non-default setup
/// is preserved.
pub fn plist() -> String {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "musterd".into());
    let log_dir = home_dir().join("Library/Logs/Muster");
    let mut env = vec![(
        "PATH".to_string(),
        "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".to_string(),
    )];
    for key in ["MUSTER_HOME", "MUSTER_TMUX_SOCKET", "MUSTER_PORT", "MUSTER_TMUX"] {
        if let Ok(v) = std::env::var(key) {
            if !v.is_empty() {
                env.push((key.to_string(), v));
            }
        }
    }
    let env_xml: String = env
        .iter()
        .map(|(k, v)| format!("\t\t<key>{}</key>\n\t\t<string>{}</string>\n", xml_escape(k), xml_escape(v)))
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{label}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{exe}</string>
		<string>run</string>
	</array>
	<key>EnvironmentVariables</key>
	<dict>
{env_xml}	</dict>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<dict>
		<key>SuccessfulExit</key>
		<false/>
	</dict>
	<key>ProcessType</key>
	<string>Background</string>
	<key>StandardOutPath</key>
	<string>{log}</string>
	<key>StandardErrorPath</key>
	<string>{log}</string>
</dict>
</plist>
"#,
        label = LABEL,
        exe = xml_escape(&exe),
        log = xml_escape(&log_dir.join("musterd.log").display().to_string()),
    )
}

fn launchctl(args: &[&str]) -> Result<(), String> {
    let out = std::process::Command::new("/bin/launchctl").args(args).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("launchctl {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

fn domain() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}

pub fn install() -> Result<(), String> {
    let path = plist_path();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(home_dir().join("Library/Logs/Muster")).map_err(|e| e.to_string())?;
    // Replace any previous registration.
    let _ = launchctl(&["bootout", &format!("{}/{LABEL}", domain())]);
    std::fs::write(&path, plist()).map_err(|e| format!("{}: {e}", path.display()))?;
    launchctl(&["bootstrap", &domain(), &path.to_string_lossy()])?;
    println!("installed {} and loaded it into {}", path.display(), domain());
    Ok(())
}

pub fn uninstall() -> Result<(), String> {
    let path = plist_path();
    let _ = launchctl(&["bootout", &format!("{}/{LABEL}", domain())]);
    match std::fs::remove_file(&path) {
        Ok(()) => println!("removed {}", path.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => println!("{} was not installed", path.display()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    }
    Ok(())
}
