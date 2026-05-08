use std::path::{Path, PathBuf};

use flexi_logger::{Cleanup, Criterion, Duplicate, FileSpec, Logger, Naming, WriteMode};

const ROTATE_BYTES: u64 = 5 * 1024 * 1024;
const KEEP_FILES: usize = 3;

pub fn log_dir(home: &Path) -> PathBuf {
    home.join("Library/Logs/Mnemonic")
}

pub fn log_file(home: &Path) -> PathBuf {
    // flexi_logger with Naming::Numbers writes the live file as
    // <basename>_rCURRENT.log; rotated files get _r00000.log etc.
    log_dir(home).join("mnemonic_rCURRENT.log")
}

pub fn init(home: &Path) -> Result<flexi_logger::LoggerHandle, String> {
    let level = std::env::var("MNEMONIC_LOG").unwrap_or_else(|_| "info".to_string());
    let dir = log_dir(home);
    Logger::try_with_str(&level)
        .map_err(|e| format!("level: {e}"))?
        .log_to_file(
            FileSpec::default()
                .directory(&dir)
                .basename("mnemonic"),
        )
        .rotate(
            Criterion::Size(ROTATE_BYTES),
            Naming::Numbers,
            Cleanup::KeepLogFiles(KEEP_FILES),
        )
        .duplicate_to_stderr(Duplicate::Info)
        .write_mode(WriteMode::Direct)
        .start()
        .map_err(|e| format!("start: {e}"))
}

/// Redact a path's basename so logs don't leak the slug (which is derived
/// from a transcribed title). Keep the parent directory and extension visible.
pub fn redact(path: &Path) -> String {
    let parent = path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    if parent.is_empty() {
        format!("<redacted>{ext}")
    } else {
        format!("{parent}/<redacted>{ext}")
    }
}
