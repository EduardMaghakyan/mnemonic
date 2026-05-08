use std::fs;
use std::path::Path;
use std::time::Duration;

use mnemonic_core::{
    permissions::{accessibility_trusted, mic_status, MicStatus},
    Config,
};

struct Check {
    name: &'static str,
    status: Status,
}

enum Status {
    Pass,
    Fail { detail: String, hint: String },
    Skip { detail: String },
}

pub fn run() -> Result<(), String> {
    let home = dirs::home_dir().ok_or_else(|| "cannot determine home directory".to_string())?;
    let config_path = Config::default_path(&home);

    let config = match Config::load_from(&config_path) {
        Ok(c) => Some(c),
        Err(e) => {
            print_check(Check {
                name: "config readable",
                status: Status::Fail {
                    detail: e,
                    hint: format!("inspect {config_path:?} or delete it to regenerate"),
                },
            });
            None
        }
    };
    if let Some(_) = &config {
        print_check(Check {
            name: "config readable",
            status: Status::Pass,
        });
    }

    let cfg = config.unwrap_or_default();

    let notes_dir = Config::expand_home(&cfg.paths.notes_dir, &home);
    print_check(check_writable("notes dir writable", &notes_dir));
    let audio_dir = Config::expand_home(&cfg.paths.audio_dir, &home);
    print_check(check_writable("audio dir writable", &audio_dir));

    let endpoint = cfg.model.endpoint.trim_end_matches('/').to_string();
    let server_status = check_server_reachable(&endpoint);
    let server_pass = matches!(server_status.status, Status::Pass);
    print_check(server_status);

    if server_pass {
        print_check(check_model_loaded(&endpoint, &cfg.model.name));
        print_check(check_mmproj_loaded(&endpoint));
    } else {
        print_check(Check {
            name: "model loaded",
            status: Status::Skip {
                detail: "skipped: server unreachable".into(),
            },
        });
        print_check(Check {
            name: "mmproj (audio) loaded",
            status: Status::Skip {
                detail: "skipped: server unreachable".into(),
            },
        });
    }

    print_check(check_mic_permission());
    print_check(check_accessibility_permission());

    Ok(())
}

fn check_mic_permission() -> Check {
    match mic_status() {
        MicStatus::Authorized => Check {
            name: "microphone permission",
            status: Status::Pass,
        },
        MicStatus::Denied => Check {
            name: "microphone permission",
            status: Status::Fail {
                detail: "denied".into(),
                hint: "System Settings → Privacy & Security → Microphone, allow the binary that runs Mnemonic".into(),
            },
        },
        MicStatus::Restricted => Check {
            name: "microphone permission",
            status: Status::Fail {
                detail: "restricted (likely device management or parental controls)".into(),
                hint: "lift the restriction or run on an unmanaged user account".into(),
            },
        },
        MicStatus::NotDetermined => Check {
            name: "microphone permission",
            status: Status::Skip {
                detail: "not yet requested — macOS will prompt on the first recording".into(),
            },
        },
        MicStatus::Unknown => Check {
            name: "microphone permission",
            status: Status::Skip {
                detail: "unknown status returned by AVCaptureDevice".into(),
            },
        },
    }
}

fn check_accessibility_permission() -> Check {
    if accessibility_trusted() {
        Check {
            name: "accessibility (informational)",
            status: Status::Pass,
        }
    } else {
        Check {
            name: "accessibility (informational)",
            status: Status::Skip {
                detail: "not granted — typically not required; only matters if the global hotkey fails to fire (e.g., in apps using Secure Input). Grant via System Settings → Privacy & Security → Accessibility if needed.".into(),
            },
        }
    }
}

fn print_check(check: Check) {
    match &check.status {
        Status::Pass => println!("[ ok ] {}", check.name),
        Status::Fail { detail, hint } => {
            println!("[fail] {}: {detail}", check.name);
            println!("        hint: {hint}");
        }
        Status::Skip { detail } => {
            println!("[skip] {}: {detail}", check.name);
        }
    }
}

fn check_writable(name: &'static str, path: &Path) -> Check {
    if let Err(e) = fs::create_dir_all(path) {
        return Check {
            name,
            status: Status::Fail {
                detail: format!("cannot create {}: {e}", path.display()),
                hint: format!("ensure you have write permission to {}", path.display()),
            },
        };
    }
    let probe = path.join(format!(".mnemonic-doctor-{}", std::process::id()));
    match fs::write(&probe, b"") {
        Ok(()) => {
            let _ = fs::remove_file(&probe);
            Check { name, status: Status::Pass }
        }
        Err(e) => Check {
            name,
            status: Status::Fail {
                detail: format!("cannot write to {}: {e}", path.display()),
                hint: format!("ensure {} is writable", path.display()),
            },
        },
    }
}

fn check_server_reachable(endpoint: &str) -> Check {
    let url = format!("{endpoint}/health");
    let result = blocking_get(&url, Duration::from_secs(2));
    match result {
        Ok(body) => {
            if body.contains("\"status\":\"ok\"") || body.contains("\"status\": \"ok\"") {
                Check {
                    name: "llama-server reachable",
                    status: Status::Pass,
                }
            } else {
                Check {
                    name: "llama-server reachable",
                    status: Status::Fail {
                        detail: format!("/health returned: {body}"),
                        hint: "wait a few seconds for the model to finish loading".into(),
                    },
                }
            }
        }
        Err(e) => Check {
            name: "llama-server reachable",
            status: Status::Fail {
                detail: format!("{endpoint}/health: {e}"),
                hint: format!("start llama-server and confirm it listens at {endpoint}"),
            },
        },
    }
}

fn check_model_loaded(endpoint: &str, expected_name: &str) -> Check {
    let url = format!("{endpoint}/v1/models");
    match blocking_get(&url, Duration::from_secs(3)) {
        Ok(body) => {
            let needle = format!("\"id\":\"{expected_name}\"");
            let lower = body.to_lowercase();
            if lower.contains(&needle.to_lowercase())
                || lower.contains(&format!("\"id\":\"{}\"", expected_name.to_lowercase()))
                || lower.contains(&expected_name.to_lowercase())
            {
                Check {
                    name: "model loaded",
                    status: Status::Pass,
                }
            } else {
                Check {
                    name: "model loaded",
                    status: Status::Fail {
                        detail: format!("expected {expected_name:?} not found in /v1/models"),
                        hint: format!("start llama-server with -hf unsloth/gemma-4-E4B-it-GGUF:Q4_K_M (or update [model] name to match)"),
                    },
                }
            }
        }
        Err(e) => Check {
            name: "model loaded",
            status: Status::Fail {
                detail: format!("/v1/models: {e}"),
                hint: "ensure llama-server's API is responding".into(),
            },
        },
    }
}

fn check_mmproj_loaded(endpoint: &str) -> Check {
    let url = format!("{endpoint}/props");
    match blocking_get(&url, Duration::from_secs(3)) {
        Ok(body) => {
            // The /props endpoint exposes chat_template content; the audio
            // mmproj manifests as <|audio|> token usage in the template.
            if body.contains("audio") || body.contains("<|audio|>") {
                Check {
                    name: "mmproj (audio) loaded",
                    status: Status::Pass,
                }
            } else {
                Check {
                    name: "mmproj (audio) loaded",
                    status: Status::Fail {
                        detail: "no audio capability detected in /props".into(),
                        hint: "start llama-server with --mmproj-auto so the audio mmproj is loaded alongside the model".into(),
                    },
                }
            }
        }
        Err(e) => Check {
            name: "mmproj (audio) loaded",
            status: Status::Fail {
                detail: format!("/props: {e}"),
                hint: "ensure llama-server's API is responding".into(),
            },
        },
    }
}

fn blocking_get(url: &str, timeout: Duration) -> Result<String, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio: {e}"))?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| format!("client build: {e}"))?;
        let resp = client.get(url).send().await.map_err(|e| format!("send: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        resp.text().await.map_err(|e| format!("body: {e}"))
    })
}

