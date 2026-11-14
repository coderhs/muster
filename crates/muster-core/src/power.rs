//! Keeping the Mac awake, and the battery level from `pmset`.
//!
//! The idle-sleep assertion is held by a `caffeinate -i -w <daemon pid>` child rather
//! than by linking IOKit: loading IOKit and CoreFoundation would add several MB of
//! resident pages to the daemon for its whole life, while the child exists only while
//! cards run and exits by itself if the daemon dies.

/// A held assertion; released on drop.
pub struct Assertion(std::process::Child);

impl Assertion {
    /// Prevents idle system sleep (the display may still sleep) until dropped.
    pub fn prevent_idle_sleep() -> Option<Assertion> {
        std::process::Command::new("/usr/bin/caffeinate")
            .args(["-i", "-w", &std::process::id().to_string()])
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
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// `(on_battery, percent)` from `pmset -g batt`; `None` when it cannot be read (or
/// the Mac has no battery and is on AC, reported as `(false, None)`).
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

    #[test]
    fn pmset_output() {
        let t = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=1)\t8%; discharging; 0:20 remaining present: true\n";
        assert_eq!(parse_pmset(t), (true, Some(8)));
        let t = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t100%; charged; 0:00 remaining\n";
        assert_eq!(parse_pmset(t), (false, Some(100)));
        assert_eq!(parse_pmset("Now drawing from 'AC Power'\n"), (false, None));
    }
}
