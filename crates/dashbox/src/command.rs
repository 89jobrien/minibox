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
    receiver: Receiver<i32>,
}

impl BackgroundCommand {
    pub fn spawn(cmd: &str, args: &[String], label: String) -> anyhow::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let cmd = cmd.to_string();
        let args = args.to_vec();

        thread::spawn(move || {
            let status = Command::new(&cmd)
                .args(&args)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .status();
            let code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
            let _ = tx.send(code);
        });

        Ok(Self {
            label,
            started: Instant::now(),
            finished: false,
            exit_code: None,
            receiver: rx,
        })
    }

    pub fn poll(&mut self) {
        if let Ok(code) = self.receiver.try_recv() {
            self.finished = true;
            self.exit_code = Some(code);
        }
    }

    pub fn elapsed_secs(&self) -> u64 {
        self.started.elapsed().as_secs()
    }
}
