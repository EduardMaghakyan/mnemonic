use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::NaiveDate;

#[derive(Debug, Clone)]
pub struct DailyFile {
    pub path: PathBuf,
    pub date: NaiveDate,
    pub body: String,
}

pub fn load_day(path: &Path) -> Result<DailyFile, String> {
    let body = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("{}: invalid filename", path.display()))?;
    let date = NaiveDate::parse_from_str(stem, "%Y-%m-%d")
        .map_err(|e| format!("{}: filename not YYYY-MM-DD: {e}", path.display()))?;
    Ok(DailyFile { path: path.to_path_buf(), date, body })
}

/// Walk `notes_dir` for `YYYY-MM-DD.md` files, newest first. Per-file failures
/// go through `on_error`; the walker continues. Files whose names don't match
/// the date pattern are skipped silently (they're someone else's markdown).
pub fn walk_days<F>(notes_dir: &Path, mut on_error: F) -> Vec<DailyFile>
where
    F: FnMut(&Path, String),
{
    let mut days = Vec::new();
    let Ok(entries) = fs::read_dir(notes_dir) else {
        return days;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_file() || p.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let stem = match p.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s,
            None => continue,
        };
        if NaiveDate::parse_from_str(stem, "%Y-%m-%d").is_err() {
            continue;
        }
        match load_day(&p) {
            Ok(d) => days.push(d),
            Err(e) => on_error(&p, e),
        }
    }
    days.sort_by(|a, b| b.date.cmp(&a.date));
    days
}

/// Parse a duration shorthand like "7d", "24h", "1w", "30m", "45s" into a Duration.
pub fn parse_since(input: &str) -> Result<Duration, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("empty duration".into());
    }
    let split_at = trimmed
        .find(|c: char| c.is_alphabetic())
        .ok_or_else(|| format!("missing unit in {trimmed:?}"))?;
    let (num_str, unit) = trimmed.split_at(split_at);
    let n: u64 = num_str
        .parse()
        .map_err(|e| format!("bad number in {trimmed:?}: {e}"))?;
    let secs = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 60 * 60,
        "d" => n * 24 * 60 * 60,
        "w" => n * 7 * 24 * 60 * 60,
        other => return Err(format!("unknown unit {other:?}; want s/m/h/d/w")),
    };
    Ok(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("mnemonic-notes-test-{pid}-{nanos}"));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn parse_since_supports_common_units() {
        assert_eq!(parse_since("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_since("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_since("2h").unwrap(), Duration::from_secs(7200));
        assert_eq!(parse_since("3d").unwrap(), Duration::from_secs(259200));
        assert_eq!(parse_since("1w").unwrap(), Duration::from_secs(604800));
        assert!(parse_since("").is_err());
        assert!(parse_since("7").is_err());
        assert!(parse_since("7y").is_err());
    }

    #[test]
    fn walk_returns_newest_first_and_skips_non_dated_files() {
        let dir = tempdir();
        fs::write(dir.join("2026-05-08.md"), "## 12:00 — A\n").unwrap();
        fs::write(dir.join("2026-05-09.md"), "## 09:00 — B\n").unwrap();
        fs::write(dir.join("README.md"), "ignore me").unwrap();
        fs::write(dir.join("2026-05-08.txt"), "wrong ext").unwrap();

        let mut errors = Vec::new();
        let days = walk_days(&dir, |p, e| errors.push((p.to_path_buf(), e)));
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].date.format("%Y-%m-%d").to_string(), "2026-05-09");
        assert_eq!(days[1].date.format("%Y-%m-%d").to_string(), "2026-05-08");
        let _ = fs::remove_dir_all(&dir);
    }
}
