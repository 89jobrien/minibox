// dashbox/src/command.rs
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Instant;

pub struct InlineCommand {
    pub lines: Vec<String>,
    pub finished: bool,
    receiver: Receiver<Option<String>>,
    _child: Child,
}

impl InlineCommand {
    pub fn spawn(cmd: &str, args: &[String]) -> anyhow::Result<Self> {
        let mut child = Command::new(cmd)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (tx, rx) = mpsc::channel();

        // Drain stdout
        if let Some(out) = stdout {
            let tx2 = tx.clone();
            thread::spawn(move || {
                let reader = BufReader::new(out);
                for line in reader.lines().map_while(Result::ok) {
                    let _ = tx2.send(Some(line));
                }
            });
        }

        // Drain stderr
        if let Some(err) = stderr {
            let tx2 = tx.clone();
            thread::spawn(move || {
                let reader = BufReader::new(err);
                for line in reader.lines().map_while(Result::ok) {
                    let _ = tx2.send(Some(line));
                }
            });
        }

        // Drop the last tx clone here so the channel disconnects once both
        // stdout/stderr reader threads have finished and dropped their clones.
        drop(tx);

        Ok(Self {
            lines: Vec::new(),
            finished: false,
            receiver: rx,
            _child: child,
        })
    }

    /// Poll for new output lines. Call each tick.
    pub fn poll(&mut self) {
        loop {
            match self.receiver.try_recv() {
                Ok(Some(line)) => self.lines.push(line),
                Ok(None) => {
                    self.finished = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.finished = true;
                    break;
                }
            }
        }
    }
}

pub struct BackgroundCommand {
    pub label: String,
    pub started: Instant,
    pub finished: bool,
    pub exit_code: Option<i32>,
    /// Last line(s) of stderr captured from the child process.
    pub stderr_tail: Option<String>,
    receiver: Receiver<(i32, String)>,
}

impl BackgroundCommand {
    pub fn spawn(cmd: &str, args: &[String], label: String) -> anyhow::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let cmd = cmd.to_string();
        let args = args.to_vec();

        thread::spawn(move || {
            let mut child = match Command::new(&cmd)
                .args(&args)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send((-1, e.to_string()));
                    return;
                }
            };

            // Drain stderr in a separate thread to prevent the child from
            // blocking when the OS pipe buffer fills up.
            let stderr_handle = child.stderr.take().map(|stderr| {
                thread::spawn(move || {
                    use std::io::Read;
                    let mut buf = String::new();
                    let _ = BufReader::new(stderr).read_to_string(&mut buf);
                    buf
                })
            });

            let code = child.wait().map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);

            let stderr_output = stderr_handle
                .and_then(|h| h.join().ok())
                .unwrap_or_default();

            let _ = tx.send((code, stderr_output));
        });

        Ok(Self {
            label,
            started: Instant::now(),
            finished: false,
            exit_code: None,
            stderr_tail: None,
            receiver: rx,
        })
    }

    pub fn poll(&mut self) {
        if let Ok((code, stderr)) = self.receiver.try_recv() {
            self.finished = true;
            self.exit_code = Some(code);
            if !stderr.trim().is_empty() {
                self.stderr_tail = Some(stderr.trim().to_string());
            }
        }
    }

    pub fn elapsed_secs(&self) -> u64 {
        self.started.elapsed().as_secs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// BackgroundCommand captures stderr from a child process that writes to it.
    #[test]
    fn background_command_captures_stderr() {
        let mut cmd = BackgroundCommand::spawn(
            "sh",
            &[
                "-c".to_string(),
                "echo 'err output' >&2; exit 1".to_string(),
            ],
            "test".to_string(),
        )
        .expect("spawn failed");

        // Wait until finished (max 2 s)
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !cmd.finished && std::time::Instant::now() < deadline {
            cmd.poll();
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(cmd.finished, "command should have finished");
        assert_eq!(cmd.exit_code, Some(1), "exit code should be 1");
        let tail = cmd.stderr_tail.as_deref().unwrap_or("");
        assert!(
            tail.contains("err output"),
            "stderr_tail should contain 'err output', got: {tail:?}"
        );
    }

    /// BackgroundCommand reports exit code 0 on success and no stderr_tail.
    #[test]
    fn background_command_exit_zero_no_stderr() {
        let mut cmd = BackgroundCommand::spawn(
            "sh",
            &["-c".to_string(), "exit 0".to_string()],
            "test".to_string(),
        )
        .expect("spawn failed");

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !cmd.finished && std::time::Instant::now() < deadline {
            cmd.poll();
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(cmd.finished);
        assert_eq!(cmd.exit_code, Some(0));
        assert!(cmd.stderr_tail.is_none(), "no stderr expected");
    }

    /// BackgroundCommand returns exit_code -1 when the binary doesn't exist.
    #[test]
    fn background_command_spawn_failure_sets_exit_code() {
        let mut cmd =
            BackgroundCommand::spawn("this_binary_does_not_exist_ever", &[], "test".to_string())
                .expect("BackgroundCommand::spawn should return Ok even on missing binary");

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !cmd.finished && std::time::Instant::now() < deadline {
            cmd.poll();
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(cmd.finished);
        assert_eq!(cmd.exit_code, Some(-1));
        // stderr_tail carries the spawn error message
        assert!(
            cmd.stderr_tail.is_some(),
            "stderr_tail should contain spawn error"
        );
    }
}
