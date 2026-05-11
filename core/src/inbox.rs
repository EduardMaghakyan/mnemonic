use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

const MANIFEST_FILENAME: &str = "manifest.json";
const AUDIO_FILENAME: &str = "audio.wav";
const IMAGE_FILENAME: &str = "image.png";
const PARTIAL_PREFIX: &str = ".partial-";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    schema_version: u32,
    /// RFC3339 timestamp in the user's local timezone.
    recorded_at: String,
}

#[derive(Debug, Clone)]
pub struct InboxJob {
    pub dir: PathBuf,
    pub recorded_at: DateTime<Local>,
}

impl InboxJob {
    fn audio_path(&self) -> PathBuf {
        self.dir.join(AUDIO_FILENAME)
    }
    fn image_path(&self) -> PathBuf {
        self.dir.join(IMAGE_FILENAME)
    }
}

/// Write a new job into `inbox_dir`. The job is staged in a `.partial-*` dir
/// then atomically renamed to its final name once all files are flushed, so a
/// torn write is never visible to [`scan`].
pub fn enqueue(
    inbox_dir: &Path,
    recorded_at: DateTime<Local>,
    wav_bytes: &[u8],
    image_png: Option<&[u8]>,
) -> Result<InboxJob, String> {
    fs::create_dir_all(inbox_dir).map_err(|e| format!("mkdir inbox: {e}"))?;

    let job_id = job_id(recorded_at);
    let partial_dir = inbox_dir.join(format!("{PARTIAL_PREFIX}{job_id}"));
    let final_dir = inbox_dir.join(&job_id);

    if final_dir.exists() {
        return Err(format!("inbox job already exists: {final_dir:?}"));
    }
    let _ = fs::remove_dir_all(&partial_dir);
    fs::create_dir_all(&partial_dir).map_err(|e| format!("mkdir partial: {e}"))?;

    let manifest = Manifest {
        schema_version: SCHEMA_VERSION,
        recorded_at: recorded_at.to_rfc3339_opts(SecondsFormat::Secs, false),
    };
    let manifest_json =
        serde_json::to_string(&manifest).map_err(|e| format!("encode manifest: {e}"))?;
    fs::write(partial_dir.join(MANIFEST_FILENAME), manifest_json)
        .map_err(|e| format!("write manifest: {e}"))?;
    fs::write(partial_dir.join(AUDIO_FILENAME), wav_bytes)
        .map_err(|e| format!("write audio.wav: {e}"))?;
    if let Some(png) = image_png {
        fs::write(partial_dir.join(IMAGE_FILENAME), png)
            .map_err(|e| format!("write image.png: {e}"))?;
    }

    fs::rename(&partial_dir, &final_dir).map_err(|e| format!("commit job: {e}"))?;

    Ok(InboxJob {
        dir: final_dir,
        recorded_at,
    })
}

/// List jobs in `inbox_dir`, oldest first. Skips `.partial-*` dirs and any
/// entry whose manifest is missing or malformed (caller can log + recover).
pub fn scan(inbox_dir: &Path) -> Vec<InboxJob> {
    let entries = match fs::read_dir(inbox_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut jobs: Vec<InboxJob> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            if !path.is_dir() {
                return None;
            }
            let name = path.file_name()?.to_str()?;
            if name.starts_with(PARTIAL_PREFIX) {
                return None;
            }
            read_manifest(&path).ok().map(|m| InboxJob {
                dir: path,
                recorded_at: m,
            })
        })
        .collect();
    jobs.sort_by(|a, b| a.recorded_at.cmp(&b.recorded_at));
    jobs
}

pub fn read_wav(job: &InboxJob) -> Result<Vec<u8>, String> {
    fs::read(job.audio_path()).map_err(|e| format!("read audio.wav: {e}"))
}

pub fn read_image(job: &InboxJob) -> Option<Vec<u8>> {
    fs::read(job.image_path()).ok()
}

pub fn complete(job: &InboxJob) -> Result<(), String> {
    fs::remove_dir_all(&job.dir).map_err(|e| format!("remove job dir: {e}"))
}

fn job_id(recorded_at: DateTime<Local>) -> String {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let utc = recorded_at.with_timezone(&Utc);
    // Compact RFC3339 (no `:` or `-` so it's path-safe and sortable).
    let stamp = utc.format("%Y%m%dT%H%M%SZ").to_string();
    format!("{stamp}-{pid}-{nanos:09}")
}

fn read_manifest(dir: &Path) -> Result<DateTime<Local>, String> {
    let raw = fs::read_to_string(dir.join(MANIFEST_FILENAME))
        .map_err(|e| format!("read manifest: {e}"))?;
    let m: Manifest =
        serde_json::from_str(&raw).map_err(|e| format!("parse manifest: {e}"))?;
    let parsed = DateTime::parse_from_rfc3339(&m.recorded_at)
        .map_err(|e| format!("parse recorded_at: {e}"))?;
    Ok(Local.from_utc_datetime(&parsed.naive_utc()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn tempdir() -> PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("mnemonic-inbox-test-{pid}-{nanos}"));
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn ts(h: u32, m: u32, s: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 5, 11, h, m, s).unwrap()
    }

    #[test]
    fn enqueue_writes_manifest_and_audio_and_optional_image() {
        let inbox = tempdir();
        let job = enqueue(&inbox, ts(15, 23, 45), b"WAV_BYTES", Some(b"PNG_BYTES")).unwrap();

        assert!(job.dir.join(MANIFEST_FILENAME).exists());
        assert_eq!(fs::read(job.dir.join(AUDIO_FILENAME)).unwrap(), b"WAV_BYTES");
        assert_eq!(fs::read(job.dir.join(IMAGE_FILENAME)).unwrap(), b"PNG_BYTES");

        let name = job.dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            !name.starts_with(PARTIAL_PREFIX),
            "final dir leaks partial prefix: {name}"
        );
        fs::remove_dir_all(&inbox).ok();
    }

    #[test]
    fn enqueue_without_image_omits_image_file() {
        let inbox = tempdir();
        let job = enqueue(&inbox, ts(15, 23, 45), b"WAV", None).unwrap();
        assert!(!job.dir.join(IMAGE_FILENAME).exists());
        fs::remove_dir_all(&inbox).ok();
    }

    #[test]
    fn scan_returns_jobs_oldest_first_and_skips_partial() {
        let inbox = tempdir();
        let _newer = enqueue(&inbox, ts(15, 24, 0), b"second", None).unwrap();
        let _older = enqueue(&inbox, ts(15, 23, 0), b"first", None).unwrap();

        // Drop a `.partial-` dir to ensure it's ignored.
        fs::create_dir_all(inbox.join(".partial-bogus")).unwrap();
        fs::write(inbox.join(".partial-bogus").join(MANIFEST_FILENAME), "{}").unwrap();

        let jobs = scan(&inbox);
        assert_eq!(jobs.len(), 2, "scan returned: {jobs:?}");
        assert!(jobs[0].recorded_at < jobs[1].recorded_at);
        fs::remove_dir_all(&inbox).ok();
    }

    #[test]
    fn scan_skips_dirs_with_malformed_manifest() {
        let inbox = tempdir();
        let bad = inbox.join("20260511T152345Z-bad");
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join(MANIFEST_FILENAME), "{not json").unwrap();

        let jobs = scan(&inbox);
        assert!(jobs.is_empty(), "expected zero jobs, got {jobs:?}");
        // The bad dir is left in place for manual recovery.
        assert!(bad.exists());
        fs::remove_dir_all(&inbox).ok();
    }

    #[test]
    fn complete_removes_job_dir() {
        let inbox = tempdir();
        let job = enqueue(&inbox, ts(15, 23, 45), b"WAV", None).unwrap();
        let dir = job.dir.clone();
        complete(&job).unwrap();
        assert!(!dir.exists());
        fs::remove_dir_all(&inbox).ok();
    }

    #[test]
    fn read_wav_and_image_round_trip() {
        let inbox = tempdir();
        let job = enqueue(&inbox, ts(15, 23, 45), b"WAV_BYTES", Some(b"PNG_BYTES")).unwrap();
        assert_eq!(read_wav(&job).unwrap(), b"WAV_BYTES");
        assert_eq!(read_image(&job).as_deref(), Some(b"PNG_BYTES" as &[u8]));
        fs::remove_dir_all(&inbox).ok();
    }
}
