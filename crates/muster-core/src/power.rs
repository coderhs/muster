//! Keeping the Mac awake, and the battery level from `pmset`.
//!
//! The idle-sleep assertion is held by a `caffeinate -i -w <daemon pid>` child rather
//! than by linking IOKit: loading IOKit and CoreFoundation would add several MB of
//! resident pages to the daemon for its whole life, while the child exists only while
//! cards run and exits by itself if the daemon dies.

#[cfg(not(target_os = "linux"))]
fn inhibitor() -> std::process::Command {
    let mut c = std::process::Command::new("/usr/bin/caffeinate");
    c.args(["-i", "-w", &std::process::id().to_string()]);
    c
}

/// systemd's inhibitor lock, held while a `tail` that watches the daemon runs.
#[cfg(target_os = "linux")]
fn inhibitor() -> std::process::Command {
    let mut c = std::process::Command::new("systemd-inhibit");
    c.args(["--what=idle:sleep", "--who=Muster", "--why=Agents are running", "--mode=block"]);
    c.args(["tail", "-f", "/dev/null", &format!("--pid={}", std::process::id())]);
    // Killing systemd-inhibit alone would leave `tail`: they get a group of their own.
    std::os::unix::process::CommandExt::process_group(&mut c, 0);
    c
}

/// A held assertion; released on drop.
pub struct Assertion(std::process::Child);

impl Assertion {
    /// Prevents idle system sleep (the display may still sleep) until dropped.
    pub fn prevent_idle_sleep() -> Option<Assertion> {
        inhibitor()
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()
            .map(Assertion)
    }

    pub fn pid(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for Assertion {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if let Ok(group) = i32::try_from(self.0.id()) {
            unsafe { libc::kill(-group, libc::SIGTERM) };
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// `(on_battery, percent)` from the kernel's power supplies.
#[cfg(target_os = "linux")]
pub async fn battery() -> Option<(bool, Option<u32>)> {
    Some(power_supplies(std::path::Path::new("/sys/class/power_supply")))
}

#[cfg(target_os = "linux")]
fn power_supplies(dir: &std::path::Path) -> (bool, Option<u32>) {
    let (mut discharging, mut percent) = (false, None);
    for supply in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let read = |name: &str| std::fs::read_to_string(supply.path().join(name)).unwrap_or_default();
        // A mouse or keyboard battery has scope "Device"; only the computer's counts.
        if read("type").trim() != "Battery" || read("scope").trim() == "Device" {
            continue;
        }
        discharging |= read("status").trim() == "Discharging";
        percent = percent.or_else(|| read("capacity").trim().parse().ok());
    }
    (discharging, percent)
}

/// `(on_battery, percent)` from `pmset -g batt`; `None` when it cannot be read (or
/// the Mac has no battery and is on AC, reported as `(false, None)`).
#[cfg(not(target_os = "linux"))]
pub async fn battery() -> Option<(bool, Option<u32>)> {
    let out = tokio::process::Command::new("/usr/bin/pmset").args(["-g", "batt"]).output().await.ok()?;
    Some(parse_pmset(&String::from_utf8_lossy(&out.stdout)))
}

pub fn parse_pmset(text: &str) -> (bool, Option<u32>) {
    let on_battery = text.contains("'Battery Power'");
    let percent = text.lines().find_map(|l| {
        let i = l.find('%')?;
        let digits: String = l[..i].chars().rev().take_while(|c| c.is_ascii_digit()).collect();
        digits.chars().rev().collect::<String>().parse().ok()
    });
    (on_battery, percent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_power_supplies() {
        let dir = std::env::temp_dir().join(format!("muster-power-{}", std::process::id()));
        for (name, files) in [
            ("AC", vec![("type", "Mains")]),
            ("BAT0", vec![("type", "Battery"), ("status", "Discharging"), ("capacity", "42\n")]),
            ("mouse", vec![("type", "Battery"), ("scope", "Device"), ("capacity", "5")]),
        ] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            for (f, v) in files {
                std::fs::write(dir.join(name).join(f), v).unwrap();
            }
        }
        assert_eq!(power_supplies(&dir), (true, Some(42)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pmset_output() {
        let t = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=1)\t8%; discharging; 0:20 remaining present: true\n";
        assert_eq!(parse_pmset(t), (true, Some(8)));
        let t = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t100%; charged; 0:00 remaining\n";
        assert_eq!(parse_pmset(t), (false, Some(100)));
        assert_eq!(parse_pmset("Now drawing from 'AC Power'\n"), (false, None));
    }
}
