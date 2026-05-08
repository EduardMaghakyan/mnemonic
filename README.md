# Mnemonic

A macOS menu-bar app that records short voice notes, transcribes and structures them with **Gemma 4 E4B** (native audio via `llama-server`), and writes Markdown files to disk. Comes with a `mnemonic` CLI for retrieval.

The product is fully local. No network call ever leaves the loopback interface.

## Demo

<!-- Drop a 10s screencap here. -->
![demo placeholder](docs/demo.gif)

## Privacy guarantees

These are user-facing claims. They remain true in every release:

- **No network call ever leaves the loopback interface.** The release build is compiled with no telemetry crates.
- **No analytics, no crash reporting service, no auto-updater that phones home in v1.**
- **Audio and notes live on disk in user-owned directories with default permissions.** The app does not read other applications' data.

The release DMG is signed with a Developer ID Application certificate and notarized by Apple. SHA256 is published with each GitHub release.

## Install

### 1. Install Mnemonic

Download the latest `Mnemonic_0.1.0_aarch64.dmg` from the [GitHub releases](https://github.com/EduardMaghakyan/mnemonic/releases/latest) page (Apple Silicon only — Intel Macs are not a v1 target). Drag `Mnemonic.app` to `/Applications`.

On first launch:

- Click the gray dot in your menu bar → **Install CLI** → authenticate when prompted. This symlinks `/usr/local/bin/mnemonic` so you can call the CLI from anywhere.
- macOS will ask for **Microphone** permission the first time you record. Grant it.

### 2. Install and start `llama-server`

Mnemonic doesn't bundle the model; you run `llama-server` yourself. This keeps the app small, lets you swap models, and keeps the privacy story strict — there's literally nothing to talk to other than your own loopback process.

```bash
brew install llama.cpp
llama-server \
  -hf unsloth/gemma-4-E4B-it-GGUF:Q4_K_M \
  --port 5809 \
  --mmproj-auto \
  -ngl 99 -c 8192
```

The first run downloads ~5 GB (Q4_K_M GGUF) + ~1 GB (audio mmproj) into `~/.cache/huggingface/`. Subsequent runs reuse the cache.

Requires `llama.cpp >= 9050` (the version that introduced the `gemma4` architecture). Older Homebrew bottles will silently fail to load the model — `brew upgrade llama.cpp` if needed.

### 3. Verify

```bash
mnemonic doctor
```

All eight checks should pass:

```
[ ok ] config readable
[ ok ] notes dir writable
[ ok ] audio dir writable
[ ok ] llama-server reachable
[ ok ] model loaded
[ ok ] mmproj (audio) loaded
[ ok ] microphone permission
[ ok ] accessibility (informational)
```

## Usage

### Recording

The default hotkey is `Ctrl+Option+Space` in **hold-to-record** mode.

1. **Hold** `Ctrl+Option+Space`
2. Speak
3. **Release**

A markdown file appears at `~/Mnemonic/notes/YYYY-MM-DD/HHMMSS-{slug}.md` with full YAML frontmatter (id, created, duration, tags, people, projects, places, model, status) and body sections (title, summary, note, actions, questions). The audio is preserved at `~/Mnemonic/audio/YYYY-MM-DD/`.

The tray icon reflects state:

- **gray** — idle
- **red** — recording
- **yellow** — processing (model is structuring)

### Tray menu

- **Open config…** — opens `~/.config/mnemonic/config.toml` in your default editor for `.toml`
- **Install CLI** — symlinks `/usr/local/bin/mnemonic`
- **Reveal log in Finder** — points at `~/Library/Logs/Mnemonic/mnemonic_rCURRENT.log`
- **Grant Microphone Access…** — opens System Settings → Privacy & Security → Microphone
- **Quit Mnemonic**

### CLI

| Command | What it does |
|---|---|
| `mnemonic ls [--since 7d] [--tag TAG] [--limit N]` | List notes newest-first |
| `mnemonic find QUERY [--open]` | Case-insensitive substring search across body and frontmatter; ±1 line of context. `--open` opens the first hit. |
| `mnemonic show ID` | Print a note by id or unambiguous prefix; uses `bat` if available, else plain print. |
| `mnemonic doctor` | Health check: config, paths, llama-server, model, mmproj, mic permission. |
| `mnemonic redo ID` | Re-run structuring against the saved audio for a note; rewrites the markdown in place (preserves id and created timestamp). Requires `keep_raw=true` at the time of the original recording. |

`--since` accepts shorthand like `30s`, `5m`, `2h`, `7d`, `1w`.

## Configuration

`~/.config/mnemonic/config.toml` is created on first launch with these defaults:

```toml
[hotkey]
combo = "ctrl+alt+space"
mode = "hold"             # "hold" (push-to-talk) or "toggle"

[audio]
max_seconds = 300         # auto-stop cap
keep_raw = true           # save the WAV alongside the note

[paths]
notes_dir = "~/Mnemonic/notes"
audio_dir = "~/Mnemonic/audio"

[model]
endpoint = "http://127.0.0.1:5809"
name = "gemma-4-e4b-it"
thinking = true           # Gemma 4's chain-of-thought; set false for ~5x faster but less accurate structuring
```

Edits hot-reload — the config file watcher re-registers the hotkey and applies path/model changes on save without a restart.

## Troubleshooting

Run `mnemonic doctor` first. It surfaces the most common issues with actionable hints.

| Symptom | Likely cause |
|---|---|
| Hotkey does nothing | Confirm the binary has Accessibility access (rare — only matters in apps with Secure Input). Default Carbon hotkey path doesn't require it. |
| Notification "Microphone permission is denied" | Tray menu → Grant Microphone Access… and toggle Mnemonic on |
| Notification "llama-server isn't reachable" | Start `llama-server` per the install steps above |
| Notes save with `status: failed` | llama-server unreachable; the audio is preserved next to the failed note. Run `mnemonic redo <id>` once the server is back. |
| Notes save with `status: malformed` | Model returned non-JSON twice. The raw second attempt is preserved in the note body under `## Raw Output`. |
| `mnemonic doctor` reports the model isn't loaded | The `llama-server` is reachable but doesn't have `gemma-4-e4b-it` loaded. Check the `-hf` flag in the server command. |

Logs at `~/Library/Logs/Mnemonic/mnemonic_rCURRENT.log` (rotated at 5 MB, 3 files retained). No log line ever contains transcribed text or model output — paths to saved notes are redacted in logs because the slug is derived from the title.

## Why Gemma 4 E4B

Gemma 4 ships in four sizes (E2B, E4B, 26B A4B, 31B). E4B is the **only one with native audio that fits a 16 GB Mac**:

- E2B and E4B include both vision (~150M params) and audio (~300M params) encoders. The 26B and 31B variants are vision-only.
- E4B's text quality (MMLU Pro 69.4%, BigBench Extra Hard 33.1%) is the higher of the two audio-capable models.
- Audio benchmarks: CoVoST 35.54, FLEURS 0.08 — strong enough that ASR + structured-output + entity extraction all happen in a single forward pass.
- ~5 GB at Q4_K_M plus ~1 GB mmproj fits comfortably in 16 GB unified memory alongside the rest of the user's working set.

A two-model pipeline (Whisper for ASR, then a text LLM for structuring) was considered and rejected: more moving parts, more latency, more memory, and no quality win for short voice memos.

## Build from source

```bash
git clone <this-repo>
cd mnemonic

# Dev build (debug)
cargo build

# Bundle a signed + notarized DMG
cp .env.example .env       # then fill in APPLE_PASSWORD with an app-specific password
./scripts/bundle.sh
```

Requires:

- Rust 1.91+ (`rustup install stable`)
- `llama.cpp` 9050+ (`brew install llama.cpp`)
- `cargo install tauri-cli@^2 --locked`
- For signing: an Apple Developer ID Application certificate in your keychain and an app-specific password from appleid.apple.com

Without `.env`, `bundle.sh` produces an unsigned `.app` and `.dmg` that work for personal use. Without signing the DMG won't pass Gatekeeper from a web download.

## License

Apache-2.0. See `Cargo.toml` for full attribution.
