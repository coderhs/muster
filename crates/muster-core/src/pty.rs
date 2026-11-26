//! Minimal PTY support: open a pseudo-terminal, run a child on its slave side
//! as a session leader, and read/write the master side asynchronously.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::process::Stdio;

use tokio::io::unix::AsyncFd;
use tokio::process::{Child, Command};

pub struct Pty {
    master: AsyncFd<OwnedFd>,
}

fn winsize(cols: u16, rows: u16) -> libc::winsize {
    libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 }
}

fn cvt(r: libc::c_int) -> io::Result<libc::c_int> {
    if r == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(r)
    }
}

impl Pty {
    /// Spawns `cmd` attached to a new PTY of the given size. The child gets the
    /// slave as stdin/stdout/stderr and as its controlling terminal.
    pub fn spawn(mut cmd: Command, cols: u16, rows: u16) -> io::Result<(Pty, Child)> {
        let mut master: RawFd = -1;
        let mut slave: RawFd = -1;
        let mut ws = winsize(cols, rows);
        unsafe {
            cvt(libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::addr_of_mut!(ws),
            ))?;
        }
        let master = unsafe { OwnedFd::from_raw_fd(master) };
        let slave = unsafe { OwnedFd::from_raw_fd(slave) };
        for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
            unsafe {
                let flags = cvt(libc::fcntl(fd, libc::F_GETFD))?;
                cvt(libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC))?;
            }
        }
        cmd.stdin(Stdio::from(slave.try_clone()?));
        cmd.stdout(Stdio::from(slave.try_clone()?));
        cmd.stderr(Stdio::from(slave));
        cmd.kill_on_drop(true);
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd.spawn()?;
        // `cmd` still holds the slave copies; drop them so EOF reaches us.
        drop(cmd);
        unsafe {
            let flags = cvt(libc::fcntl(master.as_raw_fd(), libc::F_GETFL))?;
            cvt(libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK))?;
        }
        // SAFETY: `master` is an owned, valid descriptor that is neither replaced
        // nor closed while registered.
        let master = unsafe { AsyncFd::register(master)? };
        Ok((Pty { master }, child))
    }

    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        let ws = winsize(cols, rows);
        unsafe { cvt(libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &ws)) }.map(|_| ())
    }

    /// Reads available output. `Ok(0)` means the child side has closed.
    pub async fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut guard = self.master.readable().await?;
            match guard.try_io(|fd| {
                let n = unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
                if n < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(n as usize)
                }
            }) {
                Ok(Ok(n)) => return Ok(n),
                // macOS reports EIO once the slave side is closed.
                Ok(Err(e)) if e.raw_os_error() == Some(libc::EIO) => return Ok(0),
                Ok(Err(e)) => return Err(e),
                Err(_would_block) => continue,
            }
        }
    }

    /// Writes all of `data`, waiting for the PTY to drain as needed.
    pub async fn write_all(&self, mut data: &[u8]) -> io::Result<()> {
        while !data.is_empty() {
            let mut guard = self.master.writable().await?;
            match guard.try_io(|fd| {
                let n = unsafe { libc::write(fd.as_raw_fd(), data.as_ptr().cast(), data.len()) };
                if n < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(n as usize)
                }
            }) {
                Ok(Ok(n)) => data = &data[n..],
                Ok(Err(e)) => return Err(e),
                Err(_would_block) => continue,
            }
        }
        Ok(())
    }
}
