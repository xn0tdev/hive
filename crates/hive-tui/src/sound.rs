//! Short UI click for the logo bonk. Uses desktop theme samples (not the
//! terminal BEL), played asynchronously so the TUI never stalls.

use std::path::Path;
use std::process::{Command, Stdio};

/// Play a short "bonk" without blocking the UI.
pub fn play_bonk() {
    std::thread::spawn(|| {
        let _ = play_system_click();
    });
}

fn play_system_click() -> bool {
    // Prefer a soft UI button click over the classic terminal/X11 bell.
    const SAMPLES: &[&str] = &[
        "/usr/share/sounds/ocean/stereo/button-pressed.oga",
        "/usr/share/sounds/ocean/stereo/button-pressed-modifier.oga",
        "/usr/share/sounds/gnome/default/alerts/click.ogg",
        "/usr/share/sounds/freedesktop/stereo/message-new-instant.oga",
        "/usr/share/sounds/freedesktop/stereo/camera-shutter.oga",
    ];

    for sample in SAMPLES {
        if !Path::new(sample).is_file() {
            continue;
        }
        if spawn_wait("pw-play", &["--volume=0.45", sample]) {
            return true;
        }
        if spawn_wait("paplay", &[sample]) {
            return true;
        }
    }

    false
}

fn spawn_wait(bin: &str, args: &[&str]) -> bool {
    Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
