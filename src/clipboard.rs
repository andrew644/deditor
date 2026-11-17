use std::io::Write;
use std::process::{Command, Stdio};

#[cfg(unix)]
static CLIP: std::sync::Mutex<Option<smithay_clipboard::Clipboard>> = std::sync::Mutex::new(None);

pub fn init(window: &winit::window::Window) {
    #[cfg(unix)]
    {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return;
        }
        use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
        let display = window.display_handle().ok().and_then(|h| match h.as_raw() {
            RawDisplayHandle::Wayland(w) => Some(w.display.as_ptr()),
            _ => None,
        });
        if let Some(d) = display {
            let clip = unsafe { smithay_clipboard::Clipboard::new(d) };
            if let Ok(mut slot) = CLIP.lock() {
                *slot = Some(clip);
            }
        }
    }
}

pub fn shutdown() {
    #[cfg(unix)]
    if let Ok(mut slot) = CLIP.lock() {
        let _ = slot.take();
    }
}

pub fn set(text: &str) -> crate::Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    #[cfg(unix)]
    if let Ok(slot) = CLIP.lock()
        && let Some(clip) = slot.as_ref()
    {
        clip.store(text);
        return Ok(());
    }
    #[cfg(unix)]
    {
        wl_copy(text)
    }
    #[cfg(not(unix))]
    {
        win_set(text)
    }
}

pub fn get() -> crate::Result<String> {
    #[cfg(unix)]
    if let Ok(slot) = CLIP.lock()
        && let Some(clip) = slot.as_ref()
    {
        return clip.load().map_err(|e| format!("wayland clipboard: {e}").into());
    }
    #[cfg(unix)]
    {
        wl_paste()
    }
    #[cfg(not(unix))]
    {
        win_get()
    }
}

#[cfg(unix)]
fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn copy_via_command(bin: &str, args: &[&str], text: &str) -> crate::Result<()> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{bin}: {e}"))?;
    let mut stdin = child.stdin.take().ok_or_else(|| format!("{bin}: no stdin"))?;
    let written = stdin.write_all(text.as_bytes());
    drop(stdin);
    written.map_err(|e| format!("{bin}: {e}"))?;
    let status = child.wait().map_err(|e| format!("{bin}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{bin}: failed to set the clipboard").into())
    }
}

fn paste_via_command(bin: &str, args: &[&str]) -> crate::Result<String> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| format!("{bin}: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("{bin}: {}", err.trim()).into());
    }
    String::from_utf8(out.stdout).map_err(|_| format!("{bin}: clipboard is not text").into())
}

#[cfg_attr(not(windows), allow(dead_code))]
fn win_set(text: &str) -> crate::Result<()> {
    copy_via_command(
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Console]::InputEncoding=[Text.Encoding]::UTF8; Set-Clipboard -Value ([Console]::In.ReadToEnd())",
        ],
        text,
    )
}

#[cfg_attr(not(windows), allow(dead_code))]
fn win_get() -> crate::Result<String> {
    let raw = paste_via_command(
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Get-Clipboard -Raw",
        ],
    )?;
    Ok(normalize_win(raw))
}

fn normalize_win(raw: String) -> String {
    let mut text = raw.replace("\r\n", "\n");
    if text.ends_with('\n') {
        text.pop();
    }
    text
}

#[cfg(unix)]
fn wl_copy(text: &str) -> crate::Result<()> {
    if !wayland() {
        return Err("no Wayland display".into());
    }
    copy_via_command("wl-copy", &[], text)
}

#[cfg(unix)]
fn wl_paste() -> crate::Result<String> {
    if !wayland() {
        return Err("no Wayland display".into());
    }
    paste_via_command("wl-paste", &["-n"])
}

#[cfg(all(test, unix))]
mod e2e_tests {
    use std::sync::Arc;
    use std::time::Duration;
    use winit::application::ApplicationHandler;
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
    use winit::window::{WindowAttributes, WindowId};

    #[derive(Default)]
    struct Harness {
        result: Option<(String, String)>,
    }

    impl ApplicationHandler for Harness {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            let window = Arc::new(el.create_window(WindowAttributes::default()).unwrap());
            super::init(&window);
            let mut gpu = crate::block_on(crate::render::Gpu::new(window.clone(), el)).expect("gpu");
            gpu.configure(window.inner_size().width, window.inner_size().height);
            let frame = match gpu.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
                other => panic!("no surface: {other:?}"),
            };
            let enc = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            gpu.queue.submit(Some(enc.finish()));
            gpu.queue.present(frame);
            window.focus_window();
            std::thread::sleep(Duration::from_millis(1500));
            let _ = super::set("smithay store works");
            std::thread::sleep(Duration::from_millis(500));
            let pasted = std::process::Command::new("timeout")
                .args(["5", "wl-paste", "-n"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default();
            let _ = std::process::Command::new("pkill").arg("-x").arg("wl-copy").status();
            let mut child = std::process::Command::new("wl-copy")
                .stdin(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            if let Some(mut si) = child.stdin.take() {
                use std::io::Write;
                let _ = si.write_all(b"external copy");
            }
            let _ = child.wait();
            std::thread::sleep(Duration::from_millis(500));
            let loaded = super::get().unwrap_or_default();
            self.result = Some((pasted, loaded));
            super::shutdown();
            el.exit();
        }

        fn window_event(&mut self, _el: &ActiveEventLoop, _id: WindowId, _e: winit::event::WindowEvent) {}
    }

    #[test]
    #[ignore]
    fn wayland_clipboard_round_trip() {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            eprintln!("skipping: no Wayland display");
            return;
        }
        use winit::platform::wayland::EventLoopBuilderExtWayland;
        let el = EventLoop::builder().with_any_thread(true).build().unwrap();
        el.set_control_flow(ControlFlow::Wait);
        let mut h = Harness::default();
        el.run_app(&mut h).unwrap();
        super::shutdown();
        let _ = std::process::Command::new("pkill").arg("-x").arg("wl-copy").status();
        let (pasted, loaded) = h.result.expect("harness never ran");
        assert_eq!(pasted, "smithay store works");
        assert_eq!(loaded, "external copy");
    }
}

#[cfg(test)]
mod command_tests {
    use super::*;

    #[test]
    fn normalize_win_converts_crlf_and_one_trailing_newline() {
        assert_eq!(normalize_win("abc\r\n".into()), "abc");
        assert_eq!(normalize_win("abc\r\n\r\n".into()), "abc\n");
        assert_eq!(normalize_win("a\r\nb\n".into()), "a\nb");
        assert_eq!(normalize_win(String::new()), "");
        assert_eq!(normalize_win("héllo".into()), "héllo");
    }

    #[cfg(unix)]
    #[test]
    fn command_helpers_round_trip_text_through_stdin_and_stdout() {
        let f = std::env::temp_dir().join(format!("deditor_clip_{}", std::process::id()));
        let write = format!("cat > {}", f.display());
        let read = format!("cat {}", f.display());
        copy_via_command("sh", &["-c", &write], "one\ntwo\n").unwrap();
        assert_eq!(paste_via_command("sh", &["-c", &read]).unwrap(), "one\ntwo\n");
        let err = copy_via_command("false", &[], "x").unwrap_err().to_string();
        assert!(err.starts_with("false:"));
        let err = paste_via_command("sh", &["-c", "echo bad 1>&2; exit 3"])
            .unwrap_err()
            .to_string();
        assert!(err.contains("bad"));
        std::fs::remove_file(&f).ok();
    }
}

