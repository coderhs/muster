//! Per-process memory and CPU from the macOS kernel (libproc calls, no crates).
//!
//! Memory is the physical footprint (what Activity Monitor shows as "Memory"), not
//! summed RSS, so shared pages are not counted once per process.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    pub footprint: u64,
    /// User + system CPU time in nanoseconds.
    pub cpu_ns: u64,
}

#[repr(C)]
struct MachTimebase {
    numer: u32,
    denom: u32,
}

extern "C" {
    fn mach_timebase_info(info: *mut MachTimebase) -> libc::c_int;
}

/// Converts rusage CPU ticks to nanoseconds (ticks are not nanoseconds on Apple silicon).
fn timebase() -> (u64, u64) {
    let mut tb = MachTimebase { numer: 0, denom: 0 };
    unsafe { mach_timebase_info(&mut tb) };
    if tb.denom == 0 {
        (1, 1)
    } else {
        (tb.numer as u64, tb.denom as u64)
    }
}

/// Memory and CPU time of one process (`None` if it is gone or not ours).
pub fn usage(pid: i32) -> Option<Usage> {
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    let r =
        unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, &mut info as *mut _ as *mut libc::rusage_info_t) };
    if r != 0 {
        return None;
    }
    let (n, d) = timebase();
    let ticks = info.ri_user_time + info.ri_system_time;
    Some(Usage { footprint: info.ri_phys_footprint, cpu_ns: (ticks as u128 * n as u128 / d as u128) as u64 })
}

/// Every pid on the system.
pub fn all_pids() -> Vec<i32> {
    let n = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if n <= 0 {
        return vec![];
    }
    let mut buf = vec![0i32; n as usize + 64];
    let bytes = (buf.len() * std::mem::size_of::<i32>()) as libc::c_int;
    let got = unsafe { libc::proc_listallpids(buf.as_mut_ptr().cast(), bytes) };
    buf.truncate(got.max(0) as usize);
    buf.retain(|&p| p > 0);
    buf
}

fn parent(pid: i32) -> Option<i32> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let r = unsafe {
        libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size)
    };
    (r == size).then_some(info.pbi_ppid as i32)
}

/// A snapshot of the process table: parent and session of each pid.
pub struct ProcTable {
    parent: HashMap<i32, i32>,
    children: HashMap<i32, Vec<i32>>,
    session: HashMap<i32, i32>,
}

impl ProcTable {
    pub fn read() -> ProcTable {
        let mut parent = HashMap::new();
        let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
        let mut session = HashMap::new();
        for pid in all_pids() {
            if let Some(pp) = parent_of(pid) {
                parent.insert(pid, pp);
                children.entry(pp).or_default().push(pid);
            }
            let sid = unsafe { libc::getsid(pid) };
            if sid > 0 {
                session.insert(pid, sid);
            }
        }
        ProcTable { parent, children, session }
    }

    /// `root` and all its descendants, plus processes in `root`'s session (tmux makes
    /// each pane process a session leader, so this catches re-parented children).
    pub fn tree(&self, root: i32) -> Vec<i32> {
        let mut seen: HashSet<i32> = HashSet::new();
        let mut stack = vec![root];
        while let Some(p) = stack.pop() {
            if seen.insert(p) {
                if let Some(c) = self.children.get(&p) {
                    stack.extend(c);
                }
            }
        }
        let leader = self.session.get(&root).copied() == Some(root);
        if leader {
            for (&pid, &sid) in &self.session {
                if sid == root && seen.insert(pid) {
                    if let Some(c) = self.children.get(&pid) {
                        stack.extend(c);
                    }
                }
            }
            while let Some(p) = stack.pop() {
                if seen.insert(p) {
                    if let Some(c) = self.children.get(&p) {
                        stack.extend(c);
                    }
                }
            }
        }
        let mut v: Vec<i32> = seen.into_iter().filter(|p| self.parent.contains_key(p) || *p == root).collect();
        v.sort_unstable();
        v
    }
}

fn parent_of(pid: i32) -> Option<i32> {
    parent(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_process_and_children() {
        let me = std::process::id() as i32;
        let u = usage(me).unwrap();
        assert!(u.footprint > 0);
        let mut child = std::process::Command::new("/bin/sleep").arg("5").spawn().unwrap();
        let t = ProcTable::read();
        let tree = t.tree(me);
        assert!(tree.contains(&me));
        assert!(tree.contains(&(child.id() as i32)), "{tree:?}");
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(usage(-5).is_none());
    }
}
