use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, FixedOffset};

use crate::NoteMeta;

#[derive(Debug, Clone)]
pub struct LoadedNote {
    pub path: PathBuf,
    pub meta: NoteMeta,
    pub body: String,
    pub created: DateTime<FixedOffset>,
}

pub fn load_note(path: &Path) -> Result<LoadedNote, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    parse_note(path, &raw)
}

fn parse_note(path: &Path, raw: &str) -> Result<LoadedNote, String> {
    let after_open = raw
        .strip_prefix("---\n")
        .ok_or_else(|| format!("{}: missing opening frontmatter", path.display()))?;
    let close_idx = after_open
        .find("\n---")
        .ok_or_else(|| format!("{}: missing closing frontmatter", path.display()))?;
    let frontmatter = &after_open[..close_idx];
    let after_close = &after_open[close_idx + 4..];
    let body = after_close.strip_prefix('\n').unwrap_or(after_close);
    let body = body.strip_prefix('\n').unwrap_or(body).to_string();

    let meta: NoteMeta = serde_yml::from_str(frontmatter)
        .map_err(|e| format!("{}: parse frontmatter: {e}", path.display()))?;
    let created = DateTime::parse_from_rfc3339(&meta.created)
        .map_err(|e| format!("{}: created not RFC3339: {e}", path.display()))?;

    Ok(LoadedNote { path: path.to_path_buf(), meta, body, created })
}

/// Walk `notes_dir/YYYY-MM-DD/*.md` and return all loaded notes, newest first.
/// Per-file parse failures are reported via `on_error` so the caller can decide
/// whether to log or propagate. The walker continues past any single failure.
pub fn walk_notes<F>(notes_dir: &Path, mut on_error: F) -> Vec<LoadedNote>
where
    F: FnMut(&Path, String),
{
    let mut notes = Vec::new();
    let Ok(day_entries) = fs::read_dir(notes_dir) else {
        return notes;
    };
    for day in day_entries.flatten() {
        let day_path = day.path();
        if !day_path.is_dir() {
            continue;
        }
        let Ok(file_entries) = fs::read_dir(&day_path) else {
            continue;
        };
        for entry in file_entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            match load_note(&p) {
                Ok(note) => notes.push(note),
                Err(e) => on_error(&p, e),
            }
        }
    }
    notes.sort_by(|a, b| b.created.cmp(&a.created));
    notes
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

#[derive(Debug)]
pub enum FindByPrefix<'a> {
    None,
    One(&'a LoadedNote),
    Many(Vec<&'a LoadedNote>),
}

pub fn find_by_id_prefix<'a>(notes: &'a [LoadedNote], prefix: &str) -> FindByPrefix<'a> {
    let matches: Vec<&LoadedNote> = notes
        .iter()
        .filter(|n| n.meta.id.starts_with(prefix))
        .collect();
    match matches.len() {
        0 => FindByPrefix::None,
        1 => FindByPrefix::One(matches[0]),
        _ => FindByPrefix::Many(matches),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(dir: &Path, day: &str, basename: &str, frontmatter: &str, body: &str) -> PathBuf {
        let day_dir = dir.join(day);
        fs::create_dir_all(&day_dir).unwrap();
        let p = day_dir.join(format!("{basename}.md"));
        let content = format!("---\n{frontmatter}---\n\n{body}");
        fs::write(&p, content).unwrap();
        p
    }

    fn meta_yaml(id: &str, created: &str, status: &str) -> String {
        format!(
            "id: '{id}'\ncreated: '{created}'\nduration_sec: 5\naudio: null\nmodel: gemma-4-e4b-it\nmmproj: mmproj-bf16\nstatus: {status}\n"
        )
    }

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
    fn walk_returns_newest_first_and_skips_non_md() {
        let dir = tempdir();
        fixture(
            &dir,
            "2026-05-08",
            "120000-old",
            &meta_yaml("a", "2026-05-08T12:00:00+00:00", "ok"),
            "# Old\n",
        );
        fixture(
            &dir,
            "2026-05-08",
            "180000-newer",
            &meta_yaml("b", "2026-05-08T18:00:00+00:00", "ok"),
            "# Newer\n",
        );
        // junk file that should be skipped
        fs::write(dir.join("2026-05-08").join("readme.txt"), "ignore me").unwrap();

        let mut errors = Vec::new();
        let notes = walk_notes(&dir, |p, e| errors.push((p.to_path_buf(), e)));
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].meta.id, "b");
        assert_eq!(notes[1].meta.id, "a");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_note_is_reported_but_does_not_abort_walk() {
        let dir = tempdir();
        fixture(
            &dir,
            "2026-05-08",
            "120000-good",
            &meta_yaml("good", "2026-05-08T12:00:00+00:00", "ok"),
            "# OK\n",
        );
        // missing frontmatter delimiters
        fs::write(dir.join("2026-05-08").join("999999-bad.md"), "no frontmatter here").unwrap();

        let mut errors = Vec::new();
        let notes = walk_notes(&dir, |p, e| errors.push((p.to_path_buf(), e)));
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].meta.id, "good");
        assert_eq!(errors.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_by_prefix_handles_unique_ambiguous_and_missing() {
        let dir = tempdir();
        fixture(
            &dir,
            "2026-05-08",
            "120000-foo",
            &meta_yaml("20260508T120000-foo", "2026-05-08T12:00:00+00:00", "ok"),
            "",
        );
        fixture(
            &dir,
            "2026-05-08",
            "120100-foobar",
            &meta_yaml("20260508T120100-foobar", "2026-05-08T12:01:00+00:00", "ok"),
            "",
        );
        let notes = walk_notes(&dir, |_, _| {});

        match find_by_id_prefix(&notes, "20260508T120000") {
            FindByPrefix::One(n) => assert_eq!(n.meta.id, "20260508T120000-foo"),
            other => panic!("expected One, got {other:?}"),
        }
        match find_by_id_prefix(&notes, "20260508T12") {
            FindByPrefix::Many(ms) => assert_eq!(ms.len(), 2),
            other => panic!("expected Many, got {other:?}"),
        }
        match find_by_id_prefix(&notes, "nope") {
            FindByPrefix::None => {}
            other => panic!("expected None, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
