use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use mnemonic_core::{
    find_by_id_prefix, is_silent, render_note, structure_audio, walk_notes, Config, FindByPrefix,
    LoadedNote, NoteContent, StructureRequest, StructuringResult,
};

const REQUEST_TIMEOUT_SECS: u64 = 180;
const MMPROJ_ID: &str = "mmproj-BF16.gguf";

pub fn run(prefix: &str) -> Result<(), String> {
    let home = dirs::home_dir().ok_or_else(|| "cannot determine home directory".to_string())?;
    let config_path = Config::default_path(&home);
    let cfg = Config::load_from(&config_path).unwrap_or_default();
    let notes_dir = Config::expand_home(&cfg.paths.notes_dir, &home);
    let notes = walk_notes(&notes_dir, |p, e| {
        eprintln!("warning: {}: {e}", p.display())
    });

    let note: &LoadedNote = match find_by_id_prefix(&notes, prefix) {
        FindByPrefix::One(n) => n,
        FindByPrefix::None => return Err(format!("no note matches {prefix:?}")),
        FindByPrefix::Many(ms) => {
            let ids: Vec<&str> = ms.iter().map(|n| n.meta.id.as_str()).collect();
            return Err(format!(
                "ambiguous prefix {prefix:?}; matches:\n  {}",
                ids.join("\n  ")
            ));
        }
    };

    let audio_rel = note
        .meta
        .audio
        .as_deref()
        .ok_or_else(|| {
            "this note has no saved audio (keep_raw was false when it was made)".to_string()
        })?;
    let md_dir = note.path.parent().ok_or("note path has no parent")?;
    let wav_path = resolve_audio_path(md_dir, audio_rel);
    let wav_bytes =
        fs::read(&wav_path).map_err(|e| format!("read {}: {e}", wav_path.display()))?;
    println!(
        "redo: id={} audio={} ({} bytes)",
        note.meta.id,
        wav_path.display(),
        wav_bytes.len()
    );

    let endpoint = format!(
        "{}/v1/chat/completions",
        cfg.model.endpoint.trim_end_matches('/')
    );
    let req = StructureRequest {
        endpoint: &endpoint,
        model_name: &cfg.model.name,
        timeout: Duration::from_secs(REQUEST_TIMEOUT_SECS),
        thinking: cfg.model.thinking,
    };

    let started = Instant::now();
    let outcome = run_async(structure_audio(&wav_bytes, &req));
    let elapsed = started.elapsed().as_secs_f32();

    let content: NoteContent<'_> = match &outcome {
        StructuringResult::Ok(n) if is_silent(n) => NoteContent::Silent { title: &n.title },
        StructuringResult::Ok(n) => NoteContent::Ok(n),
        StructuringResult::Malformed { raw } => NoteContent::Malformed { raw },
        StructuringResult::Failed { error } => NoteContent::Failed { error },
    };

    let new_md = render_note(
        note.meta.id.clone(),
        note.meta.created.clone(),
        note.meta.duration_sec,
        note.meta.audio.clone(),
        cfg.model.name.clone(),
        MMPROJ_ID.into(),
        content,
    )?;

    fs::write(&note.path, &new_md)
        .map_err(|e| format!("rewrite {}: {e}", note.path.display()))?;

    println!(
        "rewrote {} ({:.1}s)",
        note.path.display(),
        elapsed
    );
    Ok(())
}

fn resolve_audio_path(md_dir: &std::path::Path, audio_rel: &str) -> PathBuf {
    let p = std::path::Path::new(audio_rel);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        md_dir.join(p)
    }
}

fn run_async<F: std::future::Future>(fut: F) -> F::Output {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime.block_on(fut)
}
