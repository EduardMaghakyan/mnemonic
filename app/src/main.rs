#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod hotkey;
mod logging;
mod sounds;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use mnemonic_core::{
    health_check, is_silent, structure_audio, write_note, Config, HotkeyMode, NoteContent,
    NoteMetaOverrides, NoteStatus, StructureRequest, StructuringResult, WriteResult,
};
use log::{error, info, warn};
use mnemonic_core::permissions::{mic_status, MicStatus, PRIVACY_MIC_PANE};
use tauri::image::Image;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};
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
    Processing,
}

struct AppStateData {
    state: RecorderState,
    last_hotkey: Option<Instant>,
    capture: Option<AudioCapture>,
    recording_id: u64,
}

impl AppStateData {
    fn new() -> Self {
        Self {
            state: RecorderState::Idle,
            last_hotkey: None,
            capture: None,
            recording_id: 0,
        }
    }
}

struct ConfigState {
    config: Config,
    current_hotkey: Shortcut,
    config_path: PathBuf,
}

fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn make_icon(rgba: [u8; 4]) -> Image<'static> {
    const SIZE: u32 = 22;
    let mut data = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for _ in 0..(SIZE * SIZE) {
        data.extend_from_slice(&rgba);
    }
    Image::new_owned(data, SIZE, SIZE)
}

fn icon_for(state: RecorderState) -> Image<'static> {
    match state {
        RecorderState::Idle => make_icon([90, 90, 90, 255]),
        RecorderState::Recording => make_icon([220, 40, 40, 255]),
        RecorderState::Processing => make_icon([230, 170, 30, 255]),
    }
}

fn apply_state(app: &AppHandle, new_state: RecorderState) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_icon(Some(icon_for(new_state)));
    }
    info!("state -> {new_state:?}");
}

fn handle_hotkey_pressed(app: &AppHandle) {
    let mode = config_snapshot(app).hotkey.mode;
    let state_mutex = app.state::<Mutex<AppStateData>>();
    let mut s = state_mutex.lock().unwrap();

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
            try_start_recording(app);
        }
        (HotkeyMode::Toggle, RecorderState::Recording) => {
            drop(s);
            stop_recording(app);
        }
        (HotkeyMode::Hold, RecorderState::Recording) => {
            // Hold mode: a press while already recording is unexpected (key
            // repeat or rapid re-press); ignore silently.
        }
        (_, RecorderState::Processing) => {
            info!("hotkey ignored: still processing");
            sounds::play(sounds::IGNORED);
        }
    }
}

fn handle_hotkey_released(app: &AppHandle) {
    if config_snapshot(app).hotkey.mode == HotkeyMode::Hold {
        stop_recording(app);
    }
}

fn try_start_recording(app: &AppHandle) {
    if mic_status() == MicStatus::Denied {
        warn!("audio capture skipped: microphone permission denied");
        notify(
            app,
            "Mnemonic",
            "Microphone permission is denied. Tray menu → Grant Microphone Access… or System Settings → Privacy & Security → Microphone.",
        );
        return;
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
            s.state = RecorderState::Recording;
            s.recording_id = s.recording_id.wrapping_add(1);
            let recording_id = s.recording_id;
            drop(s);
            apply_state(app, RecorderState::Recording);
            arm_max_seconds_timer(app, recording_id);
        }
        Err(e) => {
            error!("audio capture failed to start: {e}");
        }
    }
}

fn stop_recording(app: &AppHandle) {
    let state_mutex = app.state::<Mutex<AppStateData>>();
    let cap = {
        let mut s = state_mutex.lock().unwrap();
        if s.state != RecorderState::Recording {
            return;
        }
        let Some(cap) = s.capture.take() else {
            warn!("recording without capture handle");
            s.state = RecorderState::Idle;
            return;
        };
        s.state = RecorderState::Processing;
        cap
    };
    sounds::play(sounds::STOP);
    apply_state(app, RecorderState::Processing);
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        run_processing(&app_handle, cap).await;
    });
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

async fn run_processing(app: &AppHandle, cap: AudioCapture) {
    let started = Instant::now();
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
        apply_idle(app);
        return;
    }

    let cfg = config_snapshot(app);
    let home = home_dir();
    let notes_dir = Config::expand_home(&cfg.paths.notes_dir, &home);
    let audio_dir = Config::expand_home(&cfg.paths.audio_dir, &home);

    let wav = match audio::encode_wav(&captured.samples, captured.sample_rate) {
        Ok(wav) => wav,
        Err(e) => {
            error!("wav encode failed: {e}");
            notify(app, "Mnemonic", &format!("Recording could not be encoded: {e}"));
            apply_idle(app);
            return;
        }
    };
    info!("wav: {} bytes; calling llama-server", wav.len());

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
    let outcome = structure_audio(&wav, &req).await;
    let elapsed = started.elapsed().as_secs_f32();

    if let StructuringResult::Ok(note) = &outcome {
        if is_silent(note) {
            info!("structured ({elapsed:.1}s) status=silent — skipped");
            notify(app, "Mnemonic", "Silent recording — nothing saved.");
            apply_idle(app);
            return;
        }
    }

    let overrides = NoteMetaOverrides {
        duration_sec,
        keep_raw: cfg.audio.keep_raw,
        model: cfg.model.name.clone(),
        mmproj: MMPROJ_ID.into(),
    };
    let now = chrono::Local::now();

    let write_result: Result<WriteResult, String> = match &outcome {
        StructuringResult::Ok(note) => {
            info!("structured ({elapsed:.1}s) status=ok");
            write_note(
                &notes_dir,
                &audio_dir,
                now,
                NoteContent::Ok(note),
                overrides,
                &wav,
            )
        }
        StructuringResult::Malformed { raw } => {
            info!("structured ({elapsed:.1}s) status=malformed");
            write_note(
                &notes_dir,
                &audio_dir,
                now,
                NoteContent::Malformed { raw },
                overrides,
                &wav,
            )
        }
        StructuringResult::Failed { error } => {
            warn!("structured ({elapsed:.1}s) status=failed: {error}");
            write_note(
                &notes_dir,
                &audio_dir,
                now,
                NoteContent::Failed { error },
                overrides,
                &wav,
            )
        }
    };

    match write_result {
        Ok(written) => {
            info!("saved: {}", logging::redact(&written.markdown_path));
            if let Some(audio_path) = &written.audio_path {
                info!("audio: {}", logging::redact(audio_path));
            }
            notify_for_write(app, &outcome, &written);
        }
        Err(e) => {
            error!("write failed: {e}");
            notify(app, "Mnemonic", &format!("Could not save note: {e}"));
        }
    }

    apply_idle(app);
}

fn apply_idle(app: &AppHandle) {
    let state_mutex = app.state::<Mutex<AppStateData>>();
    {
        let mut s = state_mutex.lock().unwrap();
        s.state = RecorderState::Idle;
    }
    apply_state(app, RecorderState::Idle);
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        warn!("notification failed: {e}");
    }
}

fn notify_for_write(app: &AppHandle, outcome: &StructuringResult, written: &WriteResult) {
    let body = match (&written.status, outcome) {
        (NoteStatus::Ok, StructuringResult::Ok(note)) => {
            notify(app, &note.title, "Saved to Mnemonic");
            return;
        }
        (NoteStatus::Malformed, _) => {
            "Model returned non-JSON twice — saved with status: malformed. Run `mnemonic doctor`."
                .to_string()
        }
        (NoteStatus::Failed, StructuringResult::Failed { error }) => {
            format!("Structuring failed: {error}. Audio preserved. Run `mnemonic doctor`.")
        }
        _ => "Saved to Mnemonic".to_string(),
    };
    notify(app, "Mnemonic", &body);
}

fn open_config_file(path: &Path) {
    if let Err(e) = std::process::Command::new("open").arg(path).spawn() {
        warn!("open config: {e}");
    }
}

fn open_url(url: &str) {
    if let Err(e) = std::process::Command::new("open").arg(url).spawn() {
        warn!("open {url}: {e}");
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
    let target = std::path::Path::new("/usr/local/bin/mnemonic");

    // Try the unprivileged symlink first; on EACCES fall through to osascript.
    let _ = std::fs::remove_file(target);
    match std::os::unix::fs::symlink(&cli_src, target) {
        Ok(()) => {
            notify(
                app,
                "Install CLI",
                "Symlinked /usr/local/bin/mnemonic. Try `mnemonic ls` in your terminal.",
            );
            return;
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied
            || e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            warn!("install_cli: symlink: {e}");
            notify(app, "Install CLI", &format!("Could not create symlink: {e}"));
            return;
        }
    }

    let script = format!(
        "do shell script \"mkdir -p /usr/local/bin && ln -sf '{}' '{}'\" with administrator privileges with prompt \"Mnemonic wants to install the `mnemonic` CLI to /usr/local/bin\"",
        cli_src.display(),
        target.display()
    );
    let result = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output();
    match result {
        Ok(out) if out.status.success() => notify(
            app,
            "Install CLI",
            "Symlinked /usr/local/bin/mnemonic. Try `mnemonic ls` in your terminal.",
        ),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            warn!("install_cli osascript: {stderr}");
            notify(
                app,
                "Install CLI",
                "Cancelled or failed. Run manually: sudo ln -sf <path> /usr/local/bin/mnemonic",
            );
        }
        Err(e) => {
            warn!("install_cli: osascript spawn: {e}");
            notify(app, "Install CLI", &format!("osascript failed: {e}"));
        }
    }
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
    cs.config = new_cfg;
    info!("config reloaded");
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

    let config_state = ConfigState {
        config: initial_config,
        current_hotkey: initial_hotkey,
        config_path: config_path.clone(),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, _shortcut, event| match event.state() {
                    ShortcutState::Pressed => handle_hotkey_pressed(app),
                    ShortcutState::Released => handle_hotkey_released(app),
                })
                .build(),
        )
        .manage(Mutex::new(AppStateData::new()))
        .manage(Mutex::new(config_state))
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let app_handle = app.handle().clone();
            let cfg_state = app_handle.state::<Mutex<ConfigState>>();
            let (current_hotkey, cfg_path) = {
                let cs = cfg_state.lock().unwrap();
                (cs.current_hotkey, cs.config_path.clone())
            };

            let open_cfg = MenuItemBuilder::with_id("open_config", "Open config…").build(app)?;
            let install_cli_item =
                MenuItemBuilder::with_id("install_cli", "Install CLI").build(app)?;
            let reveal_log =
                MenuItemBuilder::with_id("reveal_log", "Reveal log in Finder").build(app)?;
            let grant_mic =
                MenuItemBuilder::with_id("grant_mic", "Grant Microphone Access…").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit Mnemonic").build(app)?;
            let menu = MenuBuilder::new(app)
                .item(&open_cfg)
                .item(&install_cli_item)
                .item(&reveal_log)
                .separator()
                .item(&grant_mic)
                .separator()
                .item(&quit)
                .build()?;

            let menu_cfg_path = cfg_path.clone();
            let menu_log_path = logging::log_file(&home_dir());
            TrayIconBuilder::with_id(TRAY_ID)
                .icon(icon_for(RecorderState::Idle))
                .menu(&menu)
                .on_menu_event(move |app, event| match event.id.0.as_str() {
                    "open_config" => open_config_file(&menu_cfg_path),
                    "install_cli" => install_cli(app),
                    "reveal_log" => reveal_in_finder(&menu_log_path),
                    "grant_mic" => open_url(PRIVACY_MIC_PANE),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            app.global_shortcut().register(current_hotkey)?;
            info!(
                "mnemonic-app: tray armed; hotkey {current_hotkey:?}; config at {cfg_path:?}"
            );

            spawn_config_watcher(app_handle.clone(), cfg_path);
            spawn_startup_health_check(app_handle);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
