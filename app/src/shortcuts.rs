use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Hard timeout for `shortcuts run`. Apple Shortcuts can hang indefinitely
/// if a Shortcut prompts for user input or permission; we'd rather kill the
/// child than let the worker stall.
const TIMEOUT: Duration = Duration::from_secs(5);

/// Run a registered macOS Shortcut by name, piping `input` to its stdin
/// (never the command line — no interpolation, no shell-escape risk).
/// Returns Ok on a zero-exit child, Err otherwise.
pub fn run_shortcut(name: &str, input: &str) -> Result<(), String> {
    let mut child = Command::new("shortcuts")
        .args(["run", name])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn shortcuts: {e}"))?;

    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "stdin unavailable".to_string())?;
        stdin
            .write_all(input.as_bytes())
            .map_err(|e| format!("write stdin: {e}"))?;
    }
    drop(child.stdin.take());

    let start = Instant::now();
    loop {
        match child
            .try_wait()
            .map_err(|e| format!("wait shortcuts: {e}"))?
        {
            Some(status) if status.success() => return Ok(()),
            Some(status) => {
                let stderr = child
                    .stderr
                    .take()
                    .and_then(|mut s| {
                        use std::io::Read;
                        let mut buf = String::new();
                        s.read_to_string(&mut buf).ok().map(|_| buf)
                    })
                    .unwrap_or_default();
                return Err(format!(
                    "shortcuts run {name} exited {status}: {}",
                    stderr.trim()
                ));
            }
            None => {
                if start.elapsed() >= TIMEOUT {
                    let _ = child.kill();
                    return Err(format!("shortcuts run {name} timed out after 5s"));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}
