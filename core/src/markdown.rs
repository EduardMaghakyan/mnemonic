use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};

use crate::{NoteStatus, StructuredNote};

pub struct EntryOverrides {
    pub keep_raw: bool,
    pub model: String,
    pub mmproj: String,
}

pub struct AppendResult {
    pub daily_path: PathBuf,
    pub audio_path: Option<PathBuf>,
    pub status: NoteStatus,
}

pub enum NoteContent<'a> {
    Ok(&'a StructuredNote),
    Malformed { raw: &'a str },
    Failed { error: &'a str },
}

/// Append a single recording's bullet entry to the daily file at
/// `notes_dir/YYYY-MM-DD.md`, creating the file if it doesn't exist. Audio is
/// kept per-recording at `audio_dir/YYYY-MM-DD/HHMMSS.wav`.
pub fn append_entry(
    notes_dir: &Path,
    audio_dir: &Path,
    timestamp: DateTime<Local>,
    content: NoteContent<'_>,
    overrides: EntryOverrides,
    wav_bytes: &[u8],
) -> Result<AppendResult, String> {
    let date = timestamp.format("%Y-%m-%d").to_string();
    let time_part = timestamp.format("%H%M%S").to_string();
    let status = status_for(&content);

    let daily_path = notes_dir.join(format!("{date}.md"));

    let audio_path = if overrides.keep_raw {
        let day_audio_dir = audio_dir.join(&date);
        Some(day_audio_dir.join(format!("{time_part}.wav")))
    } else {
        None
    };

    let audio_rel = audio_path.as_ref().and_then(|p| {
        pathdiff::diff_paths(p, notes_dir).map(|p| p.to_string_lossy().into_owned())
    });

    let line = compose_bullet(timestamp, &content, audio_rel.as_deref());

    std::fs::create_dir_all(notes_dir).map_err(|e| format!("mkdir notes: {e}"))?;
    let existing = std::fs::read_to_string(&daily_path).unwrap_or_default();
    let separator = if existing.is_empty() || existing.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let new_content = format!("{existing}{separator}{line}");
    std::fs::write(&daily_path, new_content).map_err(|e| format!("write daily: {e}"))?;

    if let Some(audio_path) = &audio_path {
        if let Some(parent) = audio_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir audio: {e}"))?;
        }
        std::fs::write(audio_path, wav_bytes).map_err(|e| format!("write audio: {e}"))?;
    }

    Ok(AppendResult { daily_path, audio_path, status })
}

fn status_for(content: &NoteContent<'_>) -> NoteStatus {
    match content {
        NoteContent::Ok(_) => NoteStatus::Ok,
        NoteContent::Malformed { .. } => NoteStatus::Malformed,
        NoteContent::Failed { .. } => NoteStatus::Failed,
    }
}

fn compose_bullet(
    timestamp: DateTime<Local>,
    content: &NoteContent<'_>,
    audio_rel: Option<&str>,
) -> String {
    let hhmm = timestamp.format("%H:%M").to_string();
    let mut bullet = format!("- {hhmm} ");
    match content {
        NoteContent::Ok(note) => {
            let text = collapse_whitespace(&note.cleaned);
            bullet.push_str(&text);
        }
        NoteContent::Failed { error } => {
            let safe = collapse_whitespace(error);
            bullet.push_str(&format!("_recording failed: {safe}_"));
        }
        NoteContent::Malformed { .. } => {
            bullet.push_str("_structuring failed: model returned non-JSON twice_");
        }
    }
    if let Some(rel) = audio_rel {
        bullet.push_str(&format!(" [audio]({rel})"));
    }
    bullet.push('\n');
    bullet
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<&str>>().join(" ")
}

/// Count entries (lines starting with "- HH:MM ") in a daily-file body.
pub fn count_entries(body: &str) -> usize {
    body.lines().filter(|l| is_entry_line(l)).count()
}

fn is_entry_line(line: &str) -> bool {
    let bytes = line.as_bytes();
    bytes.len() >= 8
        && &bytes[..2] == b"- "
        && bytes[2].is_ascii_digit()
        && bytes[3].is_ascii_digit()
        && bytes[4] == b':'
        && bytes[5].is_ascii_digit()
        && bytes[6].is_ascii_digit()
        && bytes[7] == b' '
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample_note() -> StructuredNote {
        StructuredNote {
            cleaned: "I want to email Sarah about the migration plan tomorrow morning.".into(),
        }
    }

    fn ts(h: u32, m: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 5, 8, h, m, 0).unwrap()
    }

    fn overrides() -> EntryOverrides {
        EntryOverrides {
            keep_raw: true,
            model: "gemma-4-e4b-it".into(),
            mmproj: "mmproj-bf16".into(),
        }
    }

    #[test]
    fn appends_ok_bullet_creating_daily_file() {
        let tmp = tempdir();
        let result = append_entry(
            &tmp.join("notes"),
            &tmp.join("audio"),
            ts(14, 35),
            NoteContent::Ok(&sample_note()),
            overrides(),
            b"fake wav",
        )
        .unwrap();

        assert_eq!(result.status, NoteStatus::Ok);
        assert!(result.audio_path.is_some());
        assert_eq!(result.daily_path, tmp.join("notes").join("2026-05-08.md"));

        let md = std::fs::read_to_string(&result.daily_path).unwrap();
        assert_eq!(
            md,
            "- 14:35 I want to email Sarah about the migration plan tomorrow morning. [audio](../audio/2026-05-08/143500.wav)\n"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn second_bullet_lands_on_its_own_line() {
        let tmp = tempdir();
        let _ = append_entry(
            &tmp.join("notes"),
            &tmp.join("audio"),
            ts(14, 35),
            NoteContent::Ok(&sample_note()),
            overrides(),
            b"a",
        )
        .unwrap();
        let result = append_entry(
            &tmp.join("notes"),
            &tmp.join("audio"),
            ts(15, 12),
            NoteContent::Ok(&StructuredNote {
                cleaned: "This is a new node.".into(),
            }),
            overrides(),
            b"b",
        )
        .unwrap();

        let md = std::fs::read_to_string(&result.daily_path).unwrap();
        let expected = "- 14:35 I want to email Sarah about the migration plan tomorrow morning. [audio](../audio/2026-05-08/143500.wav)\n- 15:12 This is a new node. [audio](../audio/2026-05-08/151200.wav)\n";
        assert_eq!(md, expected);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn collapses_multiline_cleaned_to_single_bullet_line() {
        let tmp = tempdir();
        let result = append_entry(
            &tmp.join("notes"),
            &tmp.join("audio"),
            ts(14, 35),
            NoteContent::Ok(&StructuredNote {
                cleaned: "First sentence.\n\nSecond sentence.".into(),
            }),
            overrides(),
            b"a",
        )
        .unwrap();
        let md = std::fs::read_to_string(&result.daily_path).unwrap();
        assert!(md.contains("- 14:35 First sentence. Second sentence. ["));
        assert!(md.lines().count() == 1, "expected single line, got: {md:?}");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn appends_failed_bullet() {
        let tmp = tempdir();
        let result = append_entry(
            &tmp.join("notes"),
            &tmp.join("audio"),
            ts(14, 35),
            NoteContent::Failed { error: "connection refused" },
            overrides(),
            b"fake wav",
        )
        .unwrap();
        assert_eq!(result.status, NoteStatus::Failed);
        let md = std::fs::read_to_string(&result.daily_path).unwrap();
        assert!(md.starts_with("- 14:35 _recording failed: connection refused_ [audio]("));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn appends_malformed_bullet() {
        let tmp = tempdir();
        let result = append_entry(
            &tmp.join("notes"),
            &tmp.join("audio"),
            ts(14, 35),
            NoteContent::Malformed { raw: "{nope" },
            overrides(),
            b"fake wav",
        )
        .unwrap();
        assert_eq!(result.status, NoteStatus::Malformed);
        let md = std::fs::read_to_string(&result.daily_path).unwrap();
        assert!(md.starts_with("- 14:35 _structuring failed:"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn count_entries_counts_bullet_lines_with_time() {
        let body = "- 14:35 first.\n- 15:12 second.\n- bullet without time.\n## a heading\n- 16:00 third.\n";
        assert_eq!(count_entries(body), 3);
    }

    fn tempdir() -> PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("mnemonic-test-{pid}-{nanos}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
