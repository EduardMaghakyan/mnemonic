#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod hotkey;
mod logging;
mod shortcuts;
mod sounds;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use mnemonic_core::{
    append_entry, extract_intent, health_check, inbox, is_silent, structure_audio, AppendResult,
    Config, EntryOverrides, ExecutedIntent, HotkeyMode, Intent, IntentRequest, IntentResult,
    NoteContent, NoteStatus, StructureRequest, StructuringResult,
};
use log::{error, info, warn};
use mnemonic_core::permissions::{mic_status, MicStatus};
use tauri::image::Image;
use tauri::menu::{MenuBuilder, MenuItem, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};
use notify::Watcher;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_notification::NotificationExt;

use audio::{AudioCapture, CapturedAudio};

const TRAY_ID: &str = "main";
const HOTKEY_DEBOUNCE_MS: u64 = 250;
const REQUEST_TIMEOUT_SECS: u64 = 180;
const MMPROJ_ID: &str = "mmproj-BF16.gguf";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecorderState {
    Idle,
    Recording,
}

struct AppStateData {
    state: RecorderState,
    last_hotkey: Option<Instant>,
    capture: Option<AudioCapture>,
    recording_id: u64,
    pending_image_png: Option<Vec<u8>>,
    /// True while `screencapture -i` is blocking on the user. Guards against
    /// rapid double-press of the screenshot hotkey spawning two processes.
    screencap_in_flight: bool,
}

impl AppStateData {
    fn new() -> Self {
        Self {
            state: RecorderState::Idle,
            last_hotkey: None,
            capture: None,
            recording_id: 0,
            pending_image_png: None,
            screencap_in_flight: false,
        }
    }
}

const MAX_IMAGE_PNG_BYTES: usize = 4 * 1024 * 1024;

/// If the system clipboard currently holds an image, encode it as a PNG and
/// return the bytes. Returns `None` for: no image present, encoding failure,
/// or images that exceed `MAX_IMAGE_PNG_BYTES` after encoding.
fn read_clipboard_image_png() -> Option<Vec<u8>> {
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            warn!("clipboard init failed: {e}");
            return None;
        }
    };
    let img = match clipboard.get_image() {
        Ok(i) => i,
        Err(_) => return None,
    };
    let width = u32::try_from(img.width).ok()?;
    let height = u32::try_from(img.height).ok()?;
    let buf = image::RgbaImage::from_raw(width, height, img.bytes.into_owned())?;
    let mut out = Vec::with_capacity(width as usize * height as usize);
    if let Err(e) = image::DynamicImage::ImageRgba8(buf)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
    {
        warn!("clipboard png encode failed: {e}");
        return None;
    }
    if out.len() > MAX_IMAGE_PNG_BYTES {
        warn!("clipboard image too large: {} bytes", out.len());
        return None;
    }
    Some(out)
}

struct ConfigState {
    config: Config,
    current_hotkey: Shortcut,
    current_screenshot_hotkey: Option<Shortcut>,
    config_path: PathBuf,
}

/// Managed state shared between the recording-stop path, the inbox worker, and
/// the tray menu. The worker pulls jobs from disk; recording-stop nudges it via
/// `tx` after every successful enqueue. `queue_depth` is the visible counter.
/// `undo_state` carries the most recent fired intent so the tray-menu Undo
/// item can revert it within `undo_window_ms`.
struct WorkerHandle {
    tx: tokio::sync::mpsc::UnboundedSender<()>,
    queue_depth: AtomicUsize,
    queue_item: OnceLock<MenuItem<Wry>>,
    undo_state: Mutex<Option<UndoState>>,
    undo_item: OnceLock<MenuItem<Wry>>,
    /// Monotonically incremented each time a new intent fires. The sleep task
    /// that disables the Undo item only does so if the seq it captured at
    /// arm-time still matches — prevents an old sleep from clearing a fresh
    /// fire's state.
    undo_seq: AtomicU64,
}

struct UndoState {
    shortcut: String,
    input: String,
    seq: u64,
}

impl WorkerHandle {
    fn refresh_menu_label(&self) {
        let Some(item) = self.queue_item.get() else {
            return;
        };
        let n = self.queue_depth.load(Ordering::SeqCst);
        let label = if n == 0 {
            "Queue: idle".to_string()
        } else {
            format!("Queue: {n} waiting")
        };
        let _ = item.set_text(&label);
    }
}

fn queue_inc(app: &AppHandle) {
    let wh = app.state::<WorkerHandle>();
    wh.queue_depth.fetch_add(1, Ordering::SeqCst);
    wh.refresh_menu_label();
}

fn queue_dec(app: &AppHandle) {
    let wh = app.state::<WorkerHandle>();
    // Saturating-sub via compare_exchange to never underflow.
    let _ = wh
        .queue_depth
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
            if n == 0 { None } else { Some(n - 1) }
        });
    wh.refresh_menu_label();
}

fn nudge_worker(app: &AppHandle) {
    let wh = app.state::<WorkerHandle>();
    let _ = wh.tx.send(());
}

/// Record a freshly-fired intent so the tray-menu Undo button can revert it,
/// and arm a sleep task that clears the state after `undo_window_ms`.
fn arm_undo(app: &AppHandle, shortcut: String, input: String, window: Duration) {
    let wh = app.state::<WorkerHandle>();
    let seq = wh.undo_seq.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
    {
        let mut state = wh.undo_state.lock().unwrap();
        *state = Some(UndoState {
            shortcut: shortcut.clone(),
            input,
            seq,
        });
    }
    if let Some(item) = wh.undo_item.get() {
        let _ = item.set_text(format!("Undo: {shortcut}"));
        let _ = item.set_enabled(true);
    }

    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(window).await;
        let wh = app_handle.state::<WorkerHandle>();
        let mut state = wh.undo_state.lock().unwrap();
        if state.as_ref().map(|s| s.seq) == Some(seq) {
            *state = None;
            if let Some(item) = wh.undo_item.get() {
                let _ = item.set_text("Undo last action");
                let _ = item.set_enabled(false);
            }
        }
    });
}

/// Invoked when the user clicks the tray-menu Undo item. If the window hasn't
/// expired, run `undo-<shortcut>` via the Shortcuts CLI and clear the state.
fn invoke_undo(app: &AppHandle) {
    let taken = {
        let wh = app.state::<WorkerHandle>();
        let mut state = wh.undo_state.lock().unwrap();
        state.take()
    };
    let Some(undo) = taken else {
        info!("undo clicked but no state armed");
        return;
    };
    if let Some(item) = app.state::<WorkerHandle>().undo_item.get() {
        let _ = item.set_text("Undo last action");
        let _ = item.set_enabled(false);
    }
    let undo_name = format!("undo-{}", undo.shortcut);
    info!("undo: running shortcut {undo_name:?}");
    match shortcuts::run_shortcut(&undo_name, &undo.input) {
        Ok(()) => notify(
            app,
            "Mnemonic",
            &format!("Undid \"{}\".", undo.shortcut),
        ),
        Err(e) => {
            warn!("undo shortcut failed: {e}");
            notify(
                app,
                "Mnemonic",
                &format!(
                    "Couldn't run \"{undo_name}\". Define this Shortcut in Shortcuts.app to enable undo."
                ),
            );
        }
    }
}

fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Render an antialiased ring with a filled dot in the centre, sized to look
/// proportional in the macOS menu bar (~22pt tall).
fn render_icon(rgb: [u8; 3]) -> Image<'static> {
    const SIZE: u32 = 44; // 22pt @2x for retina sharpness
    const OUTER_R: f32 = 13.0;
    const RING_THICKNESS: f32 = 2.5;
    const DOT_R: f32 = 4.5;

    let inner_ring_r = OUTER_R - RING_THICKNESS;
    let cx = (SIZE as f32 - 1.0) * 0.5;
    let cy = (SIZE as f32 - 1.0) * 0.5;
    let mut data = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            let outer_edge = (OUTER_R + 0.5 - d).clamp(0.0, 1.0);
            let inner_edge = (d - (inner_ring_r - 0.5)).clamp(0.0, 1.0);
            let ring = outer_edge.min(inner_edge);
            let dot = (DOT_R + 0.5 - d).clamp(0.0, 1.0);
            let coverage = ring.max(dot);
            data.push(rgb[0]);
            data.push(rgb[1]);
            data.push(rgb[2]);
            data.push((coverage * 255.0).round() as u8);
        }
    }
    Image::new_owned(data, SIZE, SIZE)
}

fn icon_for(state: RecorderState) -> Image<'static> {
    match state {
        // Idle is rendered as a template image (see is_template); only the
        // alpha mask is used by macOS, so the RGB here is incidental.
        RecorderState::Idle => render_icon([0, 0, 0]),
        RecorderState::Recording => render_icon([220, 40, 40]),
    }
}

fn is_template(state: RecorderState) -> bool {
    matches!(state, RecorderState::Idle)
}

fn apply_state(app: &AppHandle, new_state: RecorderState) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_icon(Some(icon_for(new_state)));
        let _ = tray.set_icon_as_template(is_template(new_state));
    }
    info!("state -> {new_state:?}");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HotkeyKind {
    Voice,
    Screenshot,
}

fn classify_shortcut(app: &AppHandle, shortcut: &Shortcut) -> Option<HotkeyKind> {
    let cs = app.state::<Mutex<ConfigState>>();
    let cs = cs.lock().unwrap();
    if &cs.current_hotkey == shortcut {
        return Some(HotkeyKind::Voice);
    }
    if cs.current_screenshot_hotkey.as_ref() == Some(shortcut) {
        return Some(HotkeyKind::Screenshot);
    }
    None
}

fn handle_hotkey_pressed(app: &AppHandle, shortcut: &Shortcut) {
    let Some(kind) = classify_shortcut(app, shortcut) else {
        return;
    };
    match kind {
        HotkeyKind::Voice => handle_voice_hotkey_pressed(app),
        HotkeyKind::Screenshot => handle_screenshot_hotkey_pressed(app),
    }
}

fn handle_voice_hotkey_pressed(app: &AppHandle) {
    let mode = config_snapshot(app).hotkey.mode;
    let state_mutex = app.state::<Mutex<AppStateData>>();
    let mut s = state_mutex.lock().unwrap();

    if s.screencap_in_flight {
        info!("voice hotkey ignored: screencap in flight");
        return;
    }

    if mode == HotkeyMode::Toggle {
        let now = Instant::now();
        if let Some(last) = s.last_hotkey {
            if now.duration_since(last) < Duration::from_millis(HOTKEY_DEBOUNCE_MS) {
                info!("hotkey debounced");
                return;
            }
        }
        s.last_hotkey = Some(now);
    }

    match (mode, s.state) {
        (_, RecorderState::Idle) => {
            drop(s);
            try_start_recording(app, None);
        }
        (HotkeyMode::Toggle, RecorderState::Recording) => {
            drop(s);
            stop_recording(app);
        }
        (HotkeyMode::Hold, RecorderState::Recording) => {
            // Hold mode: a press while already recording is unexpected (key
            // repeat or rapid re-press); ignore silently.
        }
    }
}

fn handle_screenshot_hotkey_pressed(app: &AppHandle) {
    let state_mutex = app.state::<Mutex<AppStateData>>();
    let mut s = state_mutex.lock().unwrap();

    if s.screencap_in_flight {
        info!("screenshot hotkey ignored: screencap already in flight");
        return;
    }

    match s.state {
        RecorderState::Idle => {
            s.screencap_in_flight = true;
            drop(s);
            spawn_screencap_then_record(app.clone());
        }
        RecorderState::Recording => {
            // Toggle stop, regardless of voice-hotkey mode.
            drop(s);
            stop_recording(app);
        }
    }
}

fn handle_hotkey_released(app: &AppHandle, shortcut: &Shortcut) {
    if classify_shortcut(app, shortcut) != Some(HotkeyKind::Voice) {
        return;
    }
    if config_snapshot(app).hotkey.mode == HotkeyMode::Hold {
        stop_recording(app);
    }
}

fn spawn_screencap_then_record(app: AppHandle) {
    std::thread::spawn(move || {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let temp_path = std::env::temp_dir().join(format!(
            "mnemonic-screencap-{pid}-{nanos}.png"
        ));

        info!("screencap: spawning screencapture -i {temp_path:?}");
        // `screencapture -i <file>`: interactive region select, write PNG.
        // No `-c` flag — leave the user's clipboard untouched. The process
        // blocks until the user finishes the drag or hits Escape.
        let status = std::process::Command::new("screencapture")
            .arg("-i")
            .arg(&temp_path)
            .status();

        // Always clear the in-flight guard first, regardless of outcome.
        {
            let state_mutex = app.state::<Mutex<AppStateData>>();
            let mut s = state_mutex.lock().unwrap();
            s.screencap_in_flight = false;
        }

        let bytes = match status {
            Ok(_) => match std::fs::read(&temp_path) {
                Ok(b) if !b.is_empty() => {
                    let _ = std::fs::remove_file(&temp_path);
                    Some(b)
                }
                _ => {
                    // File missing or empty: user hit Escape.
                    let _ = std::fs::remove_file(&temp_path);
                    None
                }
            },
            Err(e) => {
                error!("screencap: failed to spawn screencapture: {e}");
                let _ = std::fs::remove_file(&temp_path);
                notify(
                    &app,
                    "Mnemonic",
                    "Couldn't launch screencapture. Grant Screen Recording permission in System Settings → Privacy & Security → Screen Recording, then try again.",
                );
                return;
            }
        };

        match bytes {
            Some(bytes) if bytes.len() > MAX_IMAGE_PNG_BYTES => {
                warn!("screencap image too large: {} bytes, dropping", bytes.len());
                notify(
                    &app,
                    "Mnemonic",
                    "Screenshot too large; recording cancelled.",
                );
            }
            Some(bytes) => {
                info!("screencap: captured {} bytes; starting recording", bytes.len());
                // Hop back to the Tauri main thread to start the recording.
                let app_handle = app.clone();
                if let Err(e) = app.run_on_main_thread(move || {
                    try_start_recording(&app_handle, Some(bytes));
                }) {
                    error!("screencap: run_on_main_thread failed: {e}");
                }
            }
            None => {
                info!("screencap: cancelled by user");
                notify(&app, "Mnemonic", "Screenshot cancelled.");
            }
        }
    });
}

fn try_start_recording(app: &AppHandle, image_override: Option<Vec<u8>>) {
    if mic_status() == MicStatus::Denied {
        warn!("audio capture skipped: microphone permission denied");
        notify(
            app,
            "Mnemonic",
            "Microphone permission is denied. Enable it in System Settings → Privacy & Security → Microphone, then try again.",
        );
        return;
    }
    let image_png = image_override.or_else(read_clipboard_image_png);
    let has_image = image_png.is_some();
    if let Some(bytes) = &image_png {
        info!("image attached to recording: {} bytes", bytes.len());
    }

    let state_mutex = app.state::<Mutex<AppStateData>>();
    let mut s = state_mutex.lock().unwrap();
    if s.state != RecorderState::Idle {
        return;
    }
    sounds::play(sounds::START);
    match AudioCapture::start() {
        Ok(cap) => {
            s.capture = Some(cap);
            s.pending_image_png = image_png;
            s.state = RecorderState::Recording;
            s.recording_id = s.recording_id.wrapping_add(1);
            let recording_id = s.recording_id;
            drop(s);
            apply_state(app, RecorderState::Recording);
            arm_max_seconds_timer(app, recording_id);
            if has_image {
                notify(app, "Mnemonic", "Recording with screenshot attached.");
            }
        }
        Err(e) => {
            error!("audio capture failed to start: {e}");
        }
    }
}

fn stop_recording(app: &AppHandle) {
    let state_mutex = app.state::<Mutex<AppStateData>>();
    let (cap, image_png) = {
        let mut s = state_mutex.lock().unwrap();
        if s.state != RecorderState::Recording {
            return;
        }
        let Some(cap) = s.capture.take() else {
            warn!("recording without capture handle");
            s.state = RecorderState::Idle;
            s.pending_image_png = None;
            return;
        };
        let img = s.pending_image_png.take();
        // Flip to Idle now so the user can fire off the next recording while
        // the previous one is still being drained + enqueued on a worker task.
        s.state = RecorderState::Idle;
        (cap, img)
    };
    sounds::play(sounds::STOP);
    apply_state(app, RecorderState::Idle);
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        drain_and_enqueue(&app_handle, cap, image_png).await;
    });
}

/// Drain the audio capture, encode the WAV, and enqueue a job to the inbox.
/// This is the bridge between "user stopped speaking" and "worker structures
/// the recording at its own pace."
async fn drain_and_enqueue(app: &AppHandle, cap: AudioCapture, image_png: Option<Vec<u8>>) {
    let captured = tokio::task::spawn_blocking(move || cap.stop())
        .await
        .unwrap_or_else(|_| CapturedAudio {
            samples: Vec::new(),
            sample_rate: audio::TARGET_SAMPLE_RATE,
        });
    let duration_sec =
        ((captured.samples.len() as f64 / captured.sample_rate as f64).round()) as u32;
    info!(
        "captured {} samples @ {} Hz ({}s)",
        captured.samples.len(),
        captured.sample_rate,
        duration_sec
    );

    if captured.samples.is_empty() {
        info!("captured 0 samples — skipped");
        notify(app, "Mnemonic", "Silent recording — nothing saved.");
        return;
    }

    let wav = match audio::encode_wav(&captured.samples, captured.sample_rate) {
        Ok(wav) => wav,
        Err(e) => {
            error!("wav encode failed: {e}");
            notify(app, "Mnemonic", &format!("Recording could not be encoded: {e}"));
            return;
        }
    };

    let cfg = config_snapshot(app);
    let inbox_dir = Config::expand_home(&cfg.paths.inbox_dir, &home_dir());
    let recorded_at = chrono::Local::now();
    match inbox::enqueue(&inbox_dir, recorded_at, &wav, image_png.as_deref()) {
        Ok(job) => {
            info!("enqueued job: {}", logging::redact(&job.dir));
            queue_inc(app);
            nudge_worker(app);
        }
        Err(e) => {
            error!("inbox enqueue failed: {e}");
            notify(
                app,
                "Mnemonic",
                &format!("Could not queue recording: {e}"),
            );
        }
    }
}

fn arm_max_seconds_timer(app: &AppHandle, recording_id: u64) {
    let max_seconds = config_snapshot(app).audio.max_seconds.max(1);
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(max_seconds as u64)).await;
        let still_this_recording = {
            let state_mutex = app_handle.state::<Mutex<AppStateData>>();
            let s = state_mutex.lock().unwrap();
            s.state == RecorderState::Recording && s.recording_id == recording_id
        };
        if still_this_recording {
            info!("auto-stop: {max_seconds}s cap reached");
            stop_recording(&app_handle);
        }
    });
}

fn config_snapshot(app: &AppHandle) -> Config {
    app.state::<Mutex<ConfigState>>().lock().unwrap().config.clone()
}

/// Process one queued job: call llama-server, write the daily bullet (or stub
/// on failure), then remove the inbox dir. Same observable outcome as the
/// previous synchronous `run_processing`, just one extra hop through disk.
async fn process_job(app: &AppHandle, job: inbox::InboxJob) {
    let started = Instant::now();
    let wav = match inbox::read_wav(&job) {
        Ok(b) => b,
        Err(e) => {
            error!("inbox read wav failed for {:?}: {e}", job.dir);
            // Drop the job — re-running it would just fail again. Leave the
            // dir for manual recovery only if we genuinely can't proceed.
            let _ = inbox::complete(&job);
            queue_dec(app);
            return;
        }
    };
    let image_png = inbox::read_image(&job);
    let recorded_at = job.recorded_at;

    let cfg = config_snapshot(app);
    let home = home_dir();
    let notes_dir = Config::expand_home(&cfg.paths.notes_dir, &home);
    let audio_dir = Config::expand_home(&cfg.paths.audio_dir, &home);

    info!(
        "process_job: {} wav={}B image={}B",
        logging::redact(&job.dir),
        wav.len(),
        image_png.as_ref().map(|b| b.len()).unwrap_or(0)
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
    let outcome = structure_audio(&wav, image_png.as_deref(), &req).await;
    let elapsed = started.elapsed().as_secs_f32();

    if let StructuringResult::Ok(note) = &outcome {
        if is_silent(note) {
            info!("structured ({elapsed:.1}s) status=silent — skipped");
            notify(app, "Mnemonic", "Silent recording — nothing saved.");
            let _ = inbox::complete(&job);
            queue_dec(app);
            return;
        }
    }

    // Fork to intent extraction on Ok structuring, gated by config. Holds
    // owned strings so we can borrow them into both the bullet render and any
    // future undo plumbing.
    let fired_intent = match &outcome {
        StructuringResult::Ok(note)
            if cfg.intents.enabled && !cfg.intents.allowed_shortcuts.is_empty() =>
        {
            try_fire_intent(
                app,
                &note.cleaned,
                &cfg.intents.allowed_shortcuts,
                &endpoint,
                &cfg.model.name,
            )
            .await
        }
        _ => None,
    };
    let executed_intent = fired_intent.as_ref().map(|f| ExecutedIntent {
        shortcut: &f.shortcut,
        input: &f.input,
    });

    let overrides = EntryOverrides {
        keep_raw: cfg.audio.keep_raw,
        model: cfg.model.name.clone(),
        mmproj: MMPROJ_ID.into(),
    };

    let image_for_disk = image_png.as_deref();
    let intent_for_disk = executed_intent.as_ref();
    let write_result: Result<AppendResult, String> = match &outcome {
        StructuringResult::Ok(note) => {
            info!("structured ({elapsed:.1}s) status=ok");
            append_entry(
                &notes_dir,
                &audio_dir,
                recorded_at,
                NoteContent::Ok(note),
                overrides,
                &wav,
                image_for_disk,
                intent_for_disk,
            )
        }
        StructuringResult::Malformed { raw } => {
            info!("structured ({elapsed:.1}s) status=malformed");
            append_entry(
                &notes_dir,
                &audio_dir,
                recorded_at,
                NoteContent::Malformed { raw },
                overrides,
                &wav,
                image_for_disk,
                None,
            )
        }
        StructuringResult::Failed { error } => {
            warn!("structured ({elapsed:.1}s) status=failed: {error}");
            append_entry(
                &notes_dir,
                &audio_dir,
                recorded_at,
                NoteContent::Failed { error },
                overrides,
                &wav,
                image_for_disk,
                None,
            )
        }
    };

    match write_result {
        Ok(written) => {
            info!("appended to: {}", logging::redact(&written.daily_path));
            if let Some(audio_path) = &written.audio_path {
                info!("audio: {}", logging::redact(audio_path));
            }
            notify_for_write(app, &outcome, &written);
            let _ = inbox::complete(&job);
        }
        Err(e) => {
            error!("write failed: {e}");
            notify(app, "Mnemonic", &format!("Could not save note: {e}"));
            // Leave the job in inbox so it can be retried next run.
        }
    }
    queue_dec(app);
}

/// Side effect that successfully fired for a recording. Owned so the bullet
/// render and (later) the tray-menu undo state can both refer to the same
/// data without sharing a lifetime to a temporary.
struct FiredIntent {
    shortcut: String,
    input: String,
}

const INTENT_TIMEOUT_SECS: u64 = 30;

/// Attempt to extract and fire an intent from the cleaned transcript. Returns
/// `Some(FiredIntent)` only when the second LLM call returned a `run_shortcut`
/// pointing at an allowlisted name AND `shortcuts run` succeeded. All other
/// outcomes (NoIntent, Malformed, Failed, whitelist miss, executor failure)
/// log and return None — the bullet still gets written, just without a `↳`
/// continuation.
async fn try_fire_intent(
    app: &AppHandle,
    cleaned: &str,
    allowed_shortcuts: &[String],
    endpoint: &str,
    model_name: &str,
) -> Option<FiredIntent> {
    let req = IntentRequest {
        endpoint,
        model_name,
        timeout: Duration::from_secs(INTENT_TIMEOUT_SECS),
    };
    let started = Instant::now();
    let result = extract_intent(cleaned, allowed_shortcuts, &req).await;
    let elapsed = started.elapsed().as_secs_f32();
    match result {
        IntentResult::Matched(Intent::RunShortcut { shortcut, input }) => {
            if !allowed_shortcuts.iter().any(|s| s == &shortcut) {
                warn!(
                    "intent rejected ({elapsed:.1}s): shortcut {shortcut:?} not in allowlist"
                );
                return None;
            }
            info!("intent ({elapsed:.1}s) shortcut={shortcut:?}");
            match shortcuts::run_shortcut(&shortcut, &input) {
                Ok(()) => {
                    let undo_window = Duration::from_millis(
                        config_snapshot(app).intents.undo_window_ms.max(500),
                    );
                    arm_undo(app, shortcut.clone(), input.clone(), undo_window);
                    notify(
                        app,
                        "Mnemonic",
                        &format!(
                            "Ran shortcut \"{shortcut}\": {input}. Click \"Undo: {shortcut}\" in the tray within {}s to revert.",
                            undo_window.as_secs()
                        ),
                    );
                    Some(FiredIntent { shortcut, input })
                }
                Err(e) => {
                    warn!("shortcut run failed: {e}");
                    notify(
                        app,
                        "Mnemonic",
                        &format!("Couldn't run shortcut \"{shortcut}\": {e}"),
                    );
                    None
                }
            }
        }
        // extract_intent collapses Intent::None to IntentResult::NoIntent, so
        // this branch is unreachable in practice but the match must be total.
        IntentResult::Matched(Intent::None) => None,
        IntentResult::NoIntent => {
            info!("intent ({elapsed:.1}s) status=none");
            None
        }
        IntentResult::Malformed { raw } => {
            warn!(
                "intent ({elapsed:.1}s) malformed: {}",
                raw.chars().take(120).collect::<String>()
            );
            None
        }
        IntentResult::Failed { error } => {
            warn!("intent ({elapsed:.1}s) failed: {error}");
            None
        }
    }
}

/// Long-lived task spawned at startup. Drains the inbox serially, blocking on
/// the `rx` channel when the queue is empty.
async fn worker_loop(app: AppHandle, mut rx: tokio::sync::mpsc::UnboundedReceiver<()>) {
    let cfg = config_snapshot(&app);
    let inbox_dir = Config::expand_home(&cfg.paths.inbox_dir, &home_dir());

    // Crash recovery: prime queue depth with whatever's already on disk.
    let recovered = inbox::scan(&inbox_dir);
    if !recovered.is_empty() {
        info!("crash recovery: {} job(s) in inbox", recovered.len());
        let wh = app.state::<WorkerHandle>();
        wh.queue_depth
            .store(recovered.len(), Ordering::SeqCst);
        wh.refresh_menu_label();
    }

    loop {
        // Process everything currently on disk.
        loop {
            let next = inbox::scan(&inbox_dir).into_iter().next();
            match next {
                Some(job) => process_job(&app, job).await,
                None => break,
            }
        }
        // Idle: block until somebody nudges us.
        match rx.recv().await {
            Some(()) => {
                // Coalesce: drain any other pending signals before rescanning.
                while rx.try_recv().is_ok() {}
            }
            None => {
                info!("worker channel closed; exiting loop");
                return;
            }
        }
    }
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        warn!("notification failed: {e}");
    }
}

fn notify_for_write(app: &AppHandle, outcome: &StructuringResult, written: &AppendResult) {
    let body = match (&written.status, outcome) {
        (NoteStatus::Ok, StructuringResult::Ok(note)) => {
            let chars: Vec<char> = note.cleaned.chars().collect();
            if chars.len() > 80 {
                let snippet: String = chars.iter().take(80).collect();
                format!("{snippet}…")
            } else {
                chars.iter().collect()
            }
        }
        (NoteStatus::Malformed, _) => {
            "Model returned non-JSON twice — stub entry written. Run `mnemonic doctor`.".to_string()
        }
        (NoteStatus::Failed, StructuringResult::Failed { error }) => {
            format!("Structuring failed: {error}. Audio preserved. Run `mnemonic doctor`.")
        }
        _ => "Appended to today's note".to_string(),
    };
    notify(app, "Mnemonic", &body);
}

fn open_config_file(path: &Path) {
    if let Err(e) = std::process::Command::new("open").arg(path).spawn() {
        warn!("open config: {e}");
    }
}

fn reveal_in_finder(path: &Path) {
    if let Err(e) = std::process::Command::new("open")
        .args(["-R"])
        .arg(path)
        .spawn()
    {
        warn!("reveal {}: {e}", path.display());
    }
}

fn install_cli(app: &AppHandle) {
    let app_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            warn!("install_cli: current_exe: {e}");
            notify(app, "Install CLI", "Could not locate the running app binary.");
            return;
        }
    };
    let cli_src = match app_exe.parent().map(|p| p.join("mnemonic")) {
        Some(p) if p.exists() => p,
        _ => {
            notify(
                app,
                "Install CLI",
                "The bundled mnemonic CLI was not found alongside the app. Run a fresh `cargo tauri build`.",
            );
            return;
        }
    };
    let home = match std::env::var_os("HOME") {
        Some(h) => std::path::PathBuf::from(h),
        None => {
            notify(app, "Install CLI", "Could not locate $HOME.");
            return;
        }
    };
    let target = home.join(".mnemonic/bin/mnemonic");
    let bin_dir = target.parent().unwrap();
    if let Err(e) = std::fs::create_dir_all(bin_dir) {
        warn!("install_cli: create_dir_all {}: {e}", bin_dir.display());
        notify(
            app,
            "Install CLI",
            &format!("Could not create {}: {e}", bin_dir.display()),
        );
        return;
    }

    let _ = std::fs::remove_file(&target);
    if let Err(e) = std::os::unix::fs::symlink(&cli_src, &target) {
        warn!("install_cli: symlink: {e}");
        notify(app, "Install CLI", &format!("Could not create symlink: {e}"));
        return;
    }

    notify(
        app,
        "Install CLI",
        "Installed to ~/.mnemonic/bin/mnemonic. Add `export PATH=\"$HOME/.mnemonic/bin:$PATH\"` to ~/.zshrc, then restart your terminal.",
    );
}

fn apply_config_change(app: &AppHandle, new_cfg: Config) {
    let cfg_state = app.state::<Mutex<ConfigState>>();
    let mut cs = cfg_state.lock().unwrap();
    if cs.config == new_cfg {
        return;
    }
    if cs.config.hotkey.combo != new_cfg.hotkey.combo {
        match hotkey::parse_hotkey(&new_cfg.hotkey.combo) {
            Ok(new_hk) => {
                let _ = app.global_shortcut().unregister(cs.current_hotkey);
                match app.global_shortcut().register(new_hk) {
                    Ok(()) => {
                        info!("hotkey -> {}", new_cfg.hotkey.combo);
                        cs.current_hotkey = new_hk;
                    }
                    Err(e) => {
                        error!(
                            "hotkey re-register failed: {e}; reverting to {:?}",
                            cs.current_hotkey
                        );
                        let _ = app.global_shortcut().register(cs.current_hotkey);
                    }
                }
            }
            Err(e) => warn!(
                "invalid hotkey {:?}: {e}; keeping previous",
                new_cfg.hotkey.combo
            ),
        }
    }
    if cs.config.hotkey.screenshot_combo != new_cfg.hotkey.screenshot_combo {
        if let Some(old) = cs.current_screenshot_hotkey.take() {
            let _ = app.global_shortcut().unregister(old);
        }
        match parse_screenshot_combo(&new_cfg.hotkey.screenshot_combo) {
            Ok(Some(new_hk)) => match app.global_shortcut().register(new_hk) {
                Ok(()) => {
                    info!("screenshot hotkey -> {}", new_cfg.hotkey.screenshot_combo);
                    cs.current_screenshot_hotkey = Some(new_hk);
                }
                Err(e) => {
                    error!("screenshot hotkey register failed: {e}");
                }
            },
            Ok(None) => info!("screenshot hotkey disabled"),
            Err(e) => warn!(
                "invalid screenshot hotkey {:?}: {e}; keeping disabled",
                new_cfg.hotkey.screenshot_combo
            ),
        }
    }
    cs.config = new_cfg;
    info!("config reloaded");
}

/// Parse the optional screenshot combo. Empty string → `Ok(None)` (disabled).
fn parse_screenshot_combo(combo: &str) -> Result<Option<Shortcut>, String> {
    if combo.trim().is_empty() {
        Ok(None)
    } else {
        hotkey::parse_hotkey(combo).map(Some)
    }
}

/// Fire one throwaway `extract_intent` call at startup so the first real
/// intent doesn't pay the llama-server cold-prime tax. Per the Phase 0 spike,
/// warm calls are ~1.7s but the very first one can be 6–7s; this amortises it
/// to a moment when nobody is waiting. No-op when intents are disabled.
fn spawn_intent_warmer(app: AppHandle) {
    let cfg = config_snapshot(&app);
    if !cfg.intents.enabled || cfg.intents.allowed_shortcuts.is_empty() {
        return;
    }
    let endpoint = format!(
        "{}/v1/chat/completions",
        cfg.model.endpoint.trim_end_matches('/')
    );
    let model_name = cfg.model.name.clone();
    let allowlist = cfg.intents.allowed_shortcuts.clone();
    tauri::async_runtime::spawn(async move {
        let started = Instant::now();
        let req = IntentRequest {
            endpoint: &endpoint,
            model_name: &model_name,
            timeout: Duration::from_secs(20),
        };
        let _ = extract_intent("ping", &allowlist, &req).await;
        info!(
            "intent warmer: primed in {:.1}s",
            started.elapsed().as_secs_f32()
        );
    });
}

fn spawn_startup_health_check(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let endpoint = config_snapshot(&app).model.endpoint.clone();
        match health_check(&endpoint, Duration::from_secs(2)).await {
            Ok(()) => info!("llama-server reachable at {endpoint}"),
            Err(e) => {
                warn!("llama-server unreachable at {endpoint}: {e}");
                notify(
                    &app,
                    "Mnemonic",
                    &format!(
                        "llama-server isn't reachable at {endpoint}. Recordings will fail until it's running. Run `mnemonic doctor` for help."
                    ),
                );
            }
        }
    });
}

fn spawn_config_watcher(app: AppHandle, path: PathBuf) {
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel::<Result<notify::Event, notify::Error>>();
        let mut watcher: notify::RecommendedWatcher = match notify::Watcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            notify::Config::default(),
        ) {
            Ok(w) => w,
            Err(e) => {
                error!("config watcher: init: {e}");
                return;
            }
        };
        let watch_target: &Path = path.parent().unwrap_or(path.as_path());
        if let Err(e) = watcher.watch(watch_target, notify::RecursiveMode::NonRecursive) {
            error!("config watcher: watch {watch_target:?}: {e}");
            return;
        }
        let mut last_apply = Instant::now() - Duration::from_secs(60);
        for res in rx {
            let Ok(evt) = res else {
                continue;
            };
            if !evt.paths.iter().any(|p| p == &path) {
                continue;
            }
            let now = Instant::now();
            if now.duration_since(last_apply) < Duration::from_millis(200) {
                continue;
            }
            // editors often write+rename; let the dust settle
            std::thread::sleep(Duration::from_millis(80));
            last_apply = Instant::now();
            match Config::load_from(&path) {
                Ok(cfg) => apply_config_change(&app, cfg),
                Err(e) => warn!("config reload: {e}"),
            }
        }
    });
}

fn main() {
    let home = home_dir();
    let logger_handle = match logging::init(&home) {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("could not init logger: {e}");
            None
        }
    };
    let _ = logger_handle;

    let config_path = Config::default_path(&home);
    if let Err(e) = Config::ensure_exists(&config_path) {
        warn!("could not write default config to {config_path:?}: {e}");
    }
    let initial_config = Config::load_from(&config_path).unwrap_or_else(|e| {
        warn!("could not parse {config_path:?}: {e}; using defaults");
        Config::default()
    });
    let initial_hotkey = hotkey::parse_hotkey(&initial_config.hotkey.combo).unwrap_or_else(|e| {
        warn!(
            "invalid hotkey {:?}: {e}; falling back to ctrl+alt+space",
            initial_config.hotkey.combo
        );
        hotkey::parse_hotkey("ctrl+alt+space").unwrap()
    });
    let initial_screenshot_hotkey =
        match parse_screenshot_combo(&initial_config.hotkey.screenshot_combo) {
            Ok(opt) => opt,
            Err(e) => {
                warn!(
                    "invalid screenshot hotkey {:?}: {e}; disabling",
                    initial_config.hotkey.screenshot_combo
                );
                None
            }
        };

    let config_state = ConfigState {
        config: initial_config,
        current_hotkey: initial_hotkey,
        current_screenshot_hotkey: initial_screenshot_hotkey,
        config_path: config_path.clone(),
    };

    let (worker_tx, worker_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let worker_handle = WorkerHandle {
        tx: worker_tx,
        queue_depth: AtomicUsize::new(0),
        queue_item: OnceLock::new(),
        undo_state: Mutex::new(None),
        undo_item: OnceLock::new(),
        undo_seq: AtomicU64::new(0),
    };
    let worker_rx_cell: std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<()>>> =
        std::sync::Mutex::new(Some(worker_rx));

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| match event.state() {
                    ShortcutState::Pressed => handle_hotkey_pressed(app, shortcut),
                    ShortcutState::Released => handle_hotkey_released(app, shortcut),
                })
                .build(),
        )
        .manage(Mutex::new(AppStateData::new()))
        .manage(Mutex::new(config_state))
        .manage(worker_handle)
        .manage(worker_rx_cell)
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let app_handle = app.handle().clone();
            let cfg_state = app_handle.state::<Mutex<ConfigState>>();
            let (current_hotkey, screenshot_hotkey, cfg_path) = {
                let cs = cfg_state.lock().unwrap();
                (
                    cs.current_hotkey,
                    cs.current_screenshot_hotkey,
                    cs.config_path.clone(),
                )
            };

            let queue_status = MenuItemBuilder::with_id("queue_status", "Queue: idle")
                .enabled(false)
                .build(app)?;
            let undo_last = MenuItemBuilder::with_id("undo_last", "Undo last action")
                .enabled(false)
                .build(app)?;
            let open_cfg = MenuItemBuilder::with_id("open_config", "Open config…").build(app)?;
            let install_cli_item =
                MenuItemBuilder::with_id("install_cli", "Install CLI").build(app)?;
            let reveal_log =
                MenuItemBuilder::with_id("reveal_log", "Reveal log in Finder").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit Mnemonic").build(app)?;
            let menu = MenuBuilder::new(app)
                .item(&queue_status)
                .item(&undo_last)
                .separator()
                .item(&open_cfg)
                .item(&install_cli_item)
                .item(&reveal_log)
                .separator()
                .item(&quit)
                .build()?;

            // Park the queue-status and undo menu items on the WorkerHandle so
            // background tasks can update them as queue depth and intent
            // firings change.
            {
                let wh = app_handle.state::<WorkerHandle>();
                let _ = wh.queue_item.set(queue_status.clone());
                let _ = wh.undo_item.set(undo_last.clone());
            }

            let menu_cfg_path = cfg_path.clone();
            let menu_log_path = logging::log_file(&home_dir());
            TrayIconBuilder::with_id(TRAY_ID)
                .icon(icon_for(RecorderState::Idle))
                .icon_as_template(is_template(RecorderState::Idle))
                .menu(&menu)
                .on_menu_event(move |app, event| match event.id.0.as_str() {
                    "open_config" => open_config_file(&menu_cfg_path),
                    "install_cli" => install_cli(app),
                    "reveal_log" => reveal_in_finder(&menu_log_path),
                    "undo_last" => invoke_undo(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            app.global_shortcut().register(current_hotkey)?;
            if let Some(hk) = screenshot_hotkey {
                if let Err(e) = app.global_shortcut().register(hk) {
                    error!("screenshot hotkey register failed at startup: {e}");
                }
            }
            info!(
                "mnemonic-app: tray armed; hotkey {current_hotkey:?}; screenshot hotkey {screenshot_hotkey:?}; config at {cfg_path:?}"
            );

            spawn_config_watcher(app_handle.clone(), cfg_path);
            spawn_startup_health_check(app_handle.clone());
            spawn_intent_warmer(app_handle.clone());

            // Spin up the inbox worker. The receiver is parked in a Mutex<Option<_>>
            // during build so we can take ownership here exactly once.
            let rx_cell = app_handle
                .state::<std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<()>>>>();
            if let Some(rx) = rx_cell.lock().unwrap().take() {
                let worker_app = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    worker_loop(worker_app, rx).await;
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
