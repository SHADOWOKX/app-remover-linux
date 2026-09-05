//! Fixed executable paths, argument arrays, bounded nonblocking pipes and process-group cancellation.
use cleanly_core::{Cancellation, Result};
use std::{
    io::Read,
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Clone, Debug)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}
impl Output {
    pub fn checked(self) -> Result<String> {
        if self.success {
            Ok(self.stdout)
        } else {
            Err(format!("Command failed: {}", self.stderr.trim()))
        }
    }
}
pub trait Runner: Send + Sync {
    fn run(&self, program: &str, args: &[&str], cancel: &Cancellation) -> Result<Output>;
}
#[derive(Clone)]
pub struct CommandRunner {
    pub timeout: Duration,
    pub limit: usize,
}
impl Default for CommandRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            limit: 8 * 1024 * 1024,
        }
    }
}
fn drain(pipe: &mut impl Read, buffer: &mut Vec<u8>, limit: usize) -> Result<bool> {
    let mut chunk = [0; 8192];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => return Ok(true),
            Ok(n) => {
                if buffer.len() + n > limit {
                    return Err("Subprocess output limit exceeded".into());
                }
                buffer.extend_from_slice(&chunk[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
}
impl Runner for CommandRunner {
    fn run(&self, program: &str, args: &[&str], cancel: &Cancellation) -> Result<Output> {
        cancel.check()?;
        if !program.starts_with('/') {
            return Err("Executable must be absolute".into());
        }
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .env_remove("DPKG_FORCE")
            .env_remove("DPKG_ROOT")
            .env_remove("DPKG_ADMINDIR")
            .env_remove("APT_CONFIG")
            .env_remove("LD_PRELOAD")
            .env_remove("LD_LIBRARY_PATH")
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LC_ALL", "C")
            .env("LANG", "C");
        let mut child = command.spawn().map_err(|e| format!("{program}: {e}"))?;
        let mut stdout = child.stdout.take().ok_or("Missing stdout")?;
        let mut stderr = child.stderr.take().ok_or("Missing stderr")?;
        for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
            // SAFETY: descriptors belong to live pipe handles, F_GETFL/F_SETFL do not take pointer arguments.
            unsafe {
                let flags = libc::fcntl(fd, libc::F_GETFL);
                if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                    let _ = child.wait();
                    return Err("Could not make subprocess pipes nonblocking".into());
                }
            }
        }
        let started = Instant::now();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let result = (|| {
            loop {
                cancel.check()?;
                if started.elapsed() > self.timeout {
                    return Err(format!(
                        "{program}: timed out after {} seconds; verify package-manager state before retrying",
                        self.timeout.as_secs()
                    ));
                }
                let done_out = drain(&mut stdout, &mut out, self.limit)?;
                let done_err = drain(&mut stderr, &mut err, self.limit)?;
                if let Some(status) = child.try_wait().map_err(|e| e.to_string())?
                    && done_out
                    && done_err
                {
                    return Ok(Output {
                        stdout: String::from_utf8(out).map_err(|_| "Non-UTF8 command output")?,
                        stderr: String::from_utf8(err).map_err(|_| "Non-UTF8 error output")?,
                        success: status.success(),
                    });
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })();
        if result.is_err() {
            // SAFETY: child created a new process group whose id is the child's pid.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.wait();
        }
        result
    }
}
