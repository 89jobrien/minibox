//! Terminal raw-mode setup/restore and window-size query.

use std::io;
#[cfg(unix)]
use std::os::fd::AsRawFd;

/// Restores terminal settings on drop.
#[cfg(unix)]
pub struct RawModeGuard {
    fd: i32,
    saved: libc::termios,
}

#[cfg(unix)]
impl RawModeGuard {
    /// Put stdin into raw mode.
    pub fn enter() -> io::Result<Self> {
        let fd = io::stdin().as_raw_fd();
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: tcgetattr reads termios for fd; fd is always-open stdin.
        if unsafe { libc::tcgetattr(fd, &mut saved) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = saved;
        // SAFETY: cfmakeraw sets raw mode on the in-memory struct; no syscall yet.
        unsafe { libc::cfmakeraw(&mut raw) };
        // SAFETY: tcsetattr applies new settings immediately (TCSANOW).
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(RawModeGuard { fd, saved })
    }
}

#[cfg(unix)]
impl Drop for RawModeGuard {
    fn drop(&mut self) {
        // SAFETY: restoring saved termios; fd (stdin) is still valid.
        unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &self.saved) };
    }
}

/// Returns `(cols, rows)` from `TIOCGWINSZ`, or `(80, 24)` as fallback.
#[cfg(unix)]
pub fn terminal_size() -> (u16, u16) {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: TIOCGWINSZ ioctl reads window size from stdout fd.
    let ok = unsafe { libc::ioctl(io::stdout().as_raw_fd(), libc::TIOCGWINSZ as _, &mut ws) };
    if ok == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
        (ws.ws_col, ws.ws_row)
    } else {
        (80, 24)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn terminal_size_returns_nonzero() {
        // Not a TTY in CI — always returns fallback (80, 24).
        let (cols, rows) = super::terminal_size();
        assert!(cols > 0 && rows > 0);
    }
}
