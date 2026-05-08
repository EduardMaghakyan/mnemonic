use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use serde::Serialize;

use serde::Deserialize;

use crate::{NoteStatus, StructuredNote, title_to_slug};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteMeta {
    pub id: String,
    pub created: String,
    pub duration_sec: u32,
    pub audio: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub people: Vec<String>,
    #[serde(default)]
    pub projects: Vec<String>,
    #[serde(default)]
    pub places: Vec<String>,
    pub model: String,
    pub mmproj: String,
    pub status: NoteStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

pub struct NoteMetaOverrides {
    pub duration_sec: u32,
    pub keep_raw: bool,
    pub model: String,
    pub mmproj: String,
}

/// Compose just the markdown contents (frontmatter + body) for an existing
/// note that's being rewritten in place — used by `mnemonic redo`. Reuses the
/// original id, created timestamp, and audio path so external references to
/// the note stay stable. Returns the new file contents; the caller writes.
pub fn render_note(
    id: String,
    created_rfc3339: String,
    duration_sec: u32,
    audio: Option<String>,
    model: String,
    mmproj: String,
    content: NoteContent<'_>,
) -> Result<String, String> {
    let status = status_for(&content);
    let (tags, people, projects, places) = match &content {
        NoteContent::Ok(note) => (
            note.tags.clone(),
            note.entities.people.clone(),
            note.entities.projects.clone(),
            note.entities.places.clone(),
        ),
        _ => (vec![], vec![], vec![], vec![]),
    };
    let error = match &content {
        NoteContent::Failed { error } => Some((*error).to_string()),
        NoteContent::Malformed { .. } => Some("model_returned_invalid_json".into()),
        _ => None,
    };
    let meta = NoteMeta {
        id,
        created: created_rfc3339,
        duration_sec,
        audio,
        tags,
        people,
        projects,
        places,
        model,
        mmproj,
        status,
        error,
    };
    let frontmatter = serde_yml::to_string(&meta).map_err(|e| format!("yaml: {e}"))?;
    let body = compose_body(&content);
    Ok(format!("---\n{frontmatter}---\n\n{body}"))
}

pub struct WriteResult {
    pub markdown_path: PathBuf,
    pub audio_path: Option<PathBuf>,
    pub status: NoteStatus,
}

pub enum NoteContent<'a> {
    Ok(&'a StructuredNote),
    Silent { title: &'a str },
    Malformed { raw: &'a str },
    Failed { error: &'a str },
}

pub fn write_note(
    notes_dir: &Path,
    audio_dir: &Path,
    timestamp: DateTime<Local>,
    content: NoteContent<'_>,
    overrides: NoteMetaOverrides,
    wav_bytes: &[u8],
) -> Result<WriteResult, String> {
    let fallback_ts = timestamp.format("%Y%m%dT%H%M%S").to_string();
    let slug = slug_for(&content, &fallback_ts);
    let status = status_for(&content);

    let date_dir = timestamp.format("%Y-%m-%d").to_string();
    let time_part = timestamp.format("%H%M%S").to_string();
    let basename = format!("{time_part}-{slug}");

    let day_notes_dir = notes_dir.join(&date_dir);
    let md_path = day_notes_dir.join(format!("{basename}.md"));

    let audio_path = if overrides.keep_raw {
        let day_audio_dir = audio_dir.join(&date_dir);
        Some(day_audio_dir.join(format!("{basename}.wav")))
    } else {
        None
    };

    let id = format!("{}-{slug}", timestamp.format("%Y%m%dT%H%M%S"));
    let audio_rel = audio_path.as_ref().and_then(|p| {
        pathdiff::diff_paths(p, &day_notes_dir).map(|p| p.to_string_lossy().into_owned())
    });

    let (tags, people, projects, places) = match &content {
        NoteContent::Ok(note) => (
            note.tags.clone(),
            note.entities.people.clone(),
            note.entities.projects.clone(),
            note.entities.places.clone(),
        ),
        _ => (vec![], vec![], vec![], vec![]),
    };

    let error = match &content {
        NoteContent::Failed { error } => Some((*error).to_string()),
        NoteContent::Malformed { .. } => Some("model_returned_invalid_json".into()),
        _ => None,
    };

    let meta = NoteMeta {
        id,
        created: timestamp.to_rfc3339(),
        duration_sec: overrides.duration_sec,
        audio: audio_rel,
        tags,
        people,
        projects,
        places,
        model: overrides.model,
        mmproj: overrides.mmproj,
        status,
        error,
    };

    let frontmatter = serde_yml::to_string(&meta).map_err(|e| format!("yaml: {e}"))?;
    let body = compose_body(&content);
    let md_content = format!("---\n{frontmatter}---\n\n{body}");

    std::fs::create_dir_all(&day_notes_dir).map_err(|e| format!("mkdir notes: {e}"))?;
    std::fs::write(&md_path, md_content).map_err(|e| format!("write md: {e}"))?;

    if let Some(audio_path) = &audio_path {
        if let Some(parent) = audio_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir audio: {e}"))?;
        }
        std::fs::write(audio_path, wav_bytes).map_err(|e| format!("write audio: {e}"))?;
    }

    Ok(WriteResult { markdown_path: md_path, audio_path, status })
}

fn status_for(content: &NoteContent<'_>) -> NoteStatus {
    match content {
        NoteContent::Ok(_) => NoteStatus::Ok,
        NoteContent::Silent { .. } => NoteStatus::Silent,
        NoteContent::Malformed { .. } => NoteStatus::Malformed,
        NoteContent::Failed { .. } => NoteStatus::Failed,
    }
}

fn slug_for(content: &NoteContent<'_>, fallback_ts: &str) -> String {
    match content {
        NoteContent::Ok(note) => title_to_slug(&note.title, fallback_ts),
        NoteContent::Silent { title } => title_to_slug(title, fallback_ts),
        NoteContent::Malformed { .. } | NoteContent::Failed { .. } => format!("note-{fallback_ts}"),
    }
}

fn compose_body(content: &NoteContent<'_>) -> String {
    match content {
        NoteContent::Ok(note) => compose_ok_body(note),
        NoteContent::Silent { title } => format!(
            "# {title}\n\n> Audio was silent or unintelligible; no transcription was written.\n"
        ),
        NoteContent::Malformed { raw } => format!(
            "# Malformed structuring output\n\n> The model returned non-JSON twice. The raw second attempt is preserved below.\n\n## Raw Output\n\n```\n{raw}\n```\n"
        ),
        NoteContent::Failed { error } => format!(
            "# Recording failed\n\n> The structuring call did not complete; the audio is preserved alongside this note.\n\n## Error\n\n```\n{error}\n```\n"
        ),
    }
}

fn compose_ok_body(note: &StructuredNote) -> String {
    let mut out = String::new();
    if !note.title.is_empty() {
        out.push_str(&format!("# {}\n\n", note.title));
    }
    if !note.summary.is_empty() {
        out.push_str(&format!("> {}\n\n", note.summary));
    }
    if !note.cleaned.is_empty() {
        out.push_str("## Note\n\n");
        out.push_str(note.cleaned.trim_end());
        out.push_str("\n\n");
    }
    if !note.actions.is_empty() {
        out.push_str("## Actions\n\n");
        for action in &note.actions {
            out.push_str(&format!("- [ ] {action}\n"));
        }
        out.push('\n');
    }
    if !note.questions.is_empty() {
        out.push_str("## Questions\n\n");
        for question in &note.questions {
            out.push_str(&format!("- {question}\n"));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Entities;
    use chrono::TimeZone;

    fn sample_note() -> StructuredNote {
        StructuredNote {
            title: "Email Sarah about migration".into(),
            tags: vec!["email".into(), "migration".into()],
            summary: "Need to email Sarah tomorrow.".into(),
            cleaned: "Remind me to email Sarah about the migration plan tomorrow morning.".into(),
            actions: vec!["Email Sarah about migration plan".into()],
            questions: vec![],
            entities: Entities {
                people: vec!["Sarah".into()],
                projects: vec!["migration".into()],
                places: vec![],
            },
        }
    }

    fn ts() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 5, 8, 12, 34, 56).unwrap()
    }

    fn overrides() -> NoteMetaOverrides {
        NoteMetaOverrides {
            duration_sec: 8,
            keep_raw: true,
            model: "gemma-4-e4b-it".into(),
            mmproj: "mmproj-bf16".into(),
        }
    }

    #[test]
    fn writes_ok_note_with_full_frontmatter_and_body() {
        let tmp = tempdir();
        let result = write_note(
            &tmp.join("notes"),
            &tmp.join("audio"),
            ts(),
            NoteContent::Ok(&sample_note()),
            overrides(),
            b"fake wav",
        )
        .unwrap();

        assert_eq!(result.status, NoteStatus::Ok);
        assert!(result.audio_path.is_some());

        let md = std::fs::read_to_string(&result.markdown_path).unwrap();
        assert!(md.contains("status: ok"));
        assert!(!md.contains("error:"));
        assert!(md.contains("# Email Sarah about migration"));
        assert!(md.contains("- [ ] Email Sarah about migration plan"));
        assert!(!md.contains("## Questions"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn writes_silent_note_with_status_silent_and_no_actions() {
        let tmp = tempdir();
        let result = write_note(
            &tmp.join("notes"),
            &tmp.join("audio"),
            ts(),
            NoteContent::Silent { title: "untranscribable" },
            overrides(),
            b"fake wav",
        )
        .unwrap();

        assert_eq!(result.status, NoteStatus::Silent);
        let md = std::fs::read_to_string(&result.markdown_path).unwrap();
        assert!(md.contains("status: silent"));
        assert!(md.contains("# untranscribable"));
        assert!(!md.contains("## Note"));
        assert!(!md.contains("## Actions"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn writes_malformed_note_with_raw_output_block() {
        let tmp = tempdir();
        let result = write_note(
            &tmp.join("notes"),
            &tmp.join("audio"),
            ts(),
            NoteContent::Malformed { raw: "not json {nope" },
            overrides(),
            b"fake wav",
        )
        .unwrap();

        assert_eq!(result.status, NoteStatus::Malformed);
        let md = std::fs::read_to_string(&result.markdown_path).unwrap();
        assert!(md.contains("status: malformed"));
        assert!(md.contains("error: model_returned_invalid_json"));
        assert!(md.contains("## Raw Output"));
        assert!(md.contains("not json {nope"));
        assert!(result.markdown_path.to_string_lossy().contains("note-"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn writes_failed_note_with_error_in_frontmatter() {
        let tmp = tempdir();
        let err = "send: error sending request: connection refused";
        let result = write_note(
            &tmp.join("notes"),
            &tmp.join("audio"),
            ts(),
            NoteContent::Failed { error: err },
            overrides(),
            b"fake wav",
        )
        .unwrap();

        assert_eq!(result.status, NoteStatus::Failed);
        let md = std::fs::read_to_string(&result.markdown_path).unwrap();
        assert!(md.contains("status: failed"));
        assert!(md.contains("connection refused"));
        assert!(md.contains("# Recording failed"));
        assert!(md.contains("## Error"));
        std::fs::remove_dir_all(&tmp).ok();
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
