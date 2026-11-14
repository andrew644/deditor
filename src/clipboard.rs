use std::io::Write;
use std::process::{Command, Stdio};

fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

pub fn set(text: &str) -> crate::Result<()> {
    if !wayland() {
        return Err("no Wayland display".into());
    }
    if text.is_empty() {
        return Ok(());
    }
    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("wl-copy: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("wl-copy: no stdin")?;
    let written = stdin.write_all(text.as_bytes());
    drop(stdin);
    written.map_err(|e| format!("wl-copy: {e}"))?;
    let status = child.wait().map_err(|e| format!("wl-copy: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("wl-copy: failed to own the clipboard".into())
    }
}

pub fn get() -> crate::Result<String> {
    if !wayland() {
        return Err("no Wayland display".into());
    }
    let out = Command::new("wl-paste")
        .arg("-n")
        .output()
        .map_err(|e| format!("wl-paste: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("wl-paste: {}", err.trim()).into());
    }
    String::from_utf8(out.stdout).map_err(|_| "wl-paste: clipboard is not text".into())
}
