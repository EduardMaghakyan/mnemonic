use std::process::Command;

use log::warn;

pub const START: &str = "Pop";
pub const STOP: &str = "Bottle";
pub const IGNORED: &str = "Tink";

pub fn play(name: &str) {
    let path = format!("/System/Library/Sounds/{name}.aiff");
    if let Err(e) = Command::new("afplay").arg(&path).spawn() {
        warn!("afplay {name}: {e}");
    }
}
