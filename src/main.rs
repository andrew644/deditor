#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::Window;

mod app;
mod buffer;
mod config;
mod fuzzy;
mod git;
mod keys;
mod lsp;
mod mode;
mod motions;
mod render;
mod search;
mod spell;
mod textobjects;
mod window;
mod wrap;

use app::Editor;
use render::rects::RectPipeline;
use render::{Gpu, TextSystem};

pub type Result<T, E = Box<dyn std::error::Error + Send + Sync>> = std::result::Result<T, E>;

struct ThreadWake(std::thread::Thread);

impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<F: Future>(mut fut: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut fut = unsafe { Pin::new_unchecked(&mut fut) };
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::park(),
        }
    }
}

fn main() -> Result<()> {
    let path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App {
        gpu: None,
        text: None,
        rects: None,
        window: None,
        editor: Some(Editor::new(path.as_deref())),
        ime_allowed: false,
        mods: Default::default(),
        tab_ranges: Vec::new(),
        pointer_tab: false,
        last_mouse: None,
        win_rects: Vec::new(),
        text_sig: (String::new(), 0.0),
        proxy: Some(event_loop.create_proxy()),
        pump_armed: false,
        close_warned: false,
        paint_warned: false,
    };
    let res = event_loop.run_app(&mut app);
    app::clipboard_shutdown();
    res?;
    Ok(())
}

struct App {
    gpu: Option<Gpu>,
    text: Option<TextSystem>,
    rects: Option<RectPipeline>,
    window: Option<Arc<Window>>,
    editor: Option<Editor>,
    ime_allowed: bool,
    mods: winit::keyboard::ModifiersState,
    tab_ranges: Vec<render::TabHit>,
    pointer_tab: bool,
    last_mouse: Option<(f64, f64)>,
    win_rects: Vec<render::WinRect>,
    text_sig: (String, f32),
    proxy: Option<winit::event_loop::EventLoopProxy<()>>,
    pump_armed: bool,
    close_warned: bool,
    paint_warned: bool,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_inner_size(LogicalSize::new(1280.0, 800.0))
            .with_title("deditor");
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("deditor: {e}");
                app::clipboard_shutdown();
                event_loop.exit();
                return;
            }
        };
        let mut gpu = match block_on(Gpu::new(window.clone(), event_loop)) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("deditor: {e}");
                app::clipboard_shutdown();
                event_loop.exit();
                return;
            }
        };
        let format = gpu.config.format;
        let default_font = config::FontCfg::default();
        let (font_family, font_size) = self
            .editor
            .as_ref()
            .map(|e| (e.config.font.family.clone(), e.config.font.size))
            .unwrap_or((default_font.family, default_font.size));
        let text = TextSystem::new(
            &gpu.device,
            &gpu.queue,
            format,
            font_family.clone(),
            font_size * window.scale_factor() as f32,
        );
        self.text_sig = (font_family, font_size);
        let rects = RectPipeline::new(&gpu.device, format);
        gpu.configure(window.inner_size().width, window.inner_size().height);
        self.window = Some(window);
        self.text = Some(text);
        self.rects = Some(rects);
        self.gpu = Some(gpu);
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn user_event(&mut self, _el: &ActiveEventLoop, _: ()) {
        self.pump_armed = false;
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu
                    && size.width > 0
                    && size.height > 0
                {
                    gpu.configure(size.width, size.height);
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let (Some(ts), Some(ed)) = (&mut self.text, &self.editor) {
                    let px = ed.config.font.size * scale_factor as f32;
                    ts.set_font(ed.config.font.family.clone(), px);
                }
                if let (Some(gpu), Some(w)) = (&mut self.gpu, &self.window) {
                    let s = w.inner_size();
                    gpu.configure(s.width, s.height);
                    w.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                self.draw();
            }
            WindowEvent::ModifiersChanged(m) => {
                self.mods = m.state();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if self.mods.alt_key()
                    && event.state == ElementState::Pressed
                    && let winit::keyboard::Key::Character(s) = &event.logical_key
                    && let Some(d) = s.chars().next().and_then(|c| c.to_digit(10))
                    && (1..=9).contains(&d)
                {
                    if let Some(ed) = &mut self.editor {
                        let i = (d - 1) as usize;
                        if let Some(&buf) = ed.win().tabs.get(i) {
                            ed.activate(buf);
                            ed.ensure_lsp();
                        }
                    }
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                    return;
                }
                if event.state == ElementState::Released {
                    return;
                }
                if let Some(ed) = &mut self.editor {
                    keys::on_key(ed, &event, self.mods);
                    if ed.quit {
                        app::clipboard_shutdown();
                        event_loop.exit();
                        return;
                    }
                }
                self.sync_ime();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::Ime(Ime::Commit(s)) => {
                if let Some(ed) = &mut self.editor
                    && self.ime_allowed
                {
                    ed.buf_mut().insert_text(&s, true);
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::Ime(_) => {}
            WindowEvent::MouseWheel { delta, .. } => {
                let over = self.last_mouse.and_then(|(x, y)| self.win_under(x, y));
                if let (Some(ed), Some(ts)) = (&mut self.editor, &self.text) {
                    let ch = ts.cell_h.max(1.0);
                    let notches = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y as f64,
                        MouseScrollDelta::PixelDelta(pos) => pos.y / ch as f64,
                    };
                    let win = over.unwrap_or(ed.active_win);
                    let (top, sub) = {
                        let w = &ed.windows[win];
                        let b = &ed.buffers[w.cur_buf()];
                        let cols = w.view_cols.max(1);
                        let notches = (notches * 3.0).round() as i64;
                        ed.wrap_with(cols)
                            .scroll(&b.rope, w.view_top, w.view_sub, -notches)
                    };
                    let w = &mut ed.windows[win];
                    w.view_top = top;
                    w.view_sub = sub;
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.last_mouse = Some((position.x, position.y));
                let over_tab = self.hit_tab(position.x, position.y).is_some();
                if over_tab != self.pointer_tab {
                    self.pointer_tab = over_tab;
                    if let Some(w) = &self.window {
                        w.set_cursor(if over_tab {
                            winit::window::CursorIcon::Pointer
                        } else {
                            winit::window::CursorIcon::Text
                        });
                    }
                }
            }
            WindowEvent::MouseInput {
                button: winit::event::MouseButton::Left,
                state: ElementState::Pressed,
                ..
            } => {
                let Some((mx, my)) = self.last_mouse else {
                    return;
                };
                let hit = self.hit_tab(mx, my);
                if let Some(ed) = &mut self.editor {
                    ed.context_menu = None;
                    if let Some((win, tab)) = hit {
                        ed.focus_win(win);
                        if let Some(&buf) = ed.windows[win].tabs.get(tab) {
                            ed.activate(buf);
                            ed.ensure_lsp();
                        }
                    }
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::MouseInput {
                button: winit::event::MouseButton::Right,
                state: ElementState::Pressed,
                ..
            } => {
                let Some(ed) = &mut self.editor else {
                    return;
                };
                if ed.context_menu.take().is_some() {
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                    return;
                }
                let Some((mx, my)) = self.last_mouse else {
                    return;
                };
                let Some(ts) = &self.text else {
                    return;
                };
                let cw = ts.cell_w.max(1.0) as f64;
                let ch = ts.cell_h.max(1.0) as f64;
                let cx = (mx / cw) as u32;
                let cy = (my / ch) as u32;
                let Some(r) = self
                    .win_rects
                    .iter()
                    .find(|r| cx >= r.x && cx < r.x + r.cols && cy >= r.y && cy < r.y + r.height)
                else {
                    return;
                };
                if cy <= r.y {
                    return;
                }
                let hit = r.win;
                let drow = cy.saturating_sub(r.y + 1) as usize;
                let xcell = cx.saturating_sub(r.x + r.gutter) as usize;
                ed.focus_win(hit);
                let pos = ed.pos_at_window(hit, drow, xcell);
                ed.buf_mut().cursor = pos;
                {
                    let b = ed.buf_mut();
                    b.clamp();
                    let len = crate::buffer::line_chars(&b.rope, b.cursor.line);
                    if b.cursor.col > len.saturating_sub(1) {
                        b.cursor.col = len.saturating_sub(1);
                    }
                }
                ed.comp_cancel();
                let menu = ed.context_menu_text();
                ed.context_menu = Some(menu);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::CloseRequested => {
                if let Some(ed) = self.editor.as_mut()
                    && ed.buffers.iter().any(|b| b.dirty)
                    && !self.close_warned
                {
                    self.close_warned = true;
                    ed.set_status(
                        "E37: No write since last change; close again to discard".to_string(),
                        true,
                    );
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                    return;
                }
                app::clipboard_shutdown();
                event_loop.exit();
            }
            _ => {}
        }
    }
}

impl App {
    /// The hit rectangles are only refreshed when a frame is painted, so a
    /// window closed since the last one still has a rect: treat that as no hit
    /// rather than indexing past the window list.
    fn hit_tab(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        let windows = self.editor.as_ref()?.windows.len();
        self.tab_ranges
            .iter()
            .filter(|r| r.win < windows)
            .find(|r| x >= r.x0 as f64 && x < r.x1 as f64 && y >= r.y0 as f64 && y < r.y1 as f64)
            .map(|r| (r.win, r.tab))
    }

    fn win_under(&self, x: f64, y: f64) -> Option<usize> {
        let ts = self.text.as_ref()?;
        let windows = self.editor.as_ref()?.windows.len();
        let cx = (x / ts.cell_w.max(1.0) as f64) as u32;
        let cy = (y / ts.cell_h.max(1.0) as f64) as u32;
        self.win_rects
            .iter()
            .filter(|r| r.win < windows)
            .find(|r| cx >= r.x && cx < r.x + r.cols && cy >= r.y && cy < r.y + r.height)
            .map(|r| r.win)
    }

    fn sync_ime(&mut self) {
        let want = matches!(
            self.editor.as_ref().map(|e| e.mode),
            Some(mode::Mode::Insert)
        );
        if want != self.ime_allowed {
            if let Some(w) = &self.window {
                w.set_ime_allowed(want);
            }
            self.ime_allowed = want;
        }
        if want {
            self.update_ime_area();
        }
    }

    fn update_ime_area(&mut self) {
        let (Some(w), Some(ts), Some(gpu), Some(ed)) =
            (&self.window, &self.text, &self.gpu, &self.editor)
        else {
            return;
        };
        let l = render::Layout::new(gpu, ts, ed);
        let Some(r) = l.wins.iter().find(|r| r.win == ed.active_win) else {
            return;
        };
        let b = ed.buf();
        let win = ed.win();
        let wrap = ed.wrap_with(win.view_cols.max(1));
        let row = wrap
            .screen_row(&b.rope, win.view_top, win.view_sub, b.cursor)
            .unwrap_or(0);
        let x = (r.x
            + r.gutter
            + wrap
                .rope_pos(
                    &b.rope,
                    b.cursor.line.min(b.rope.len_lines() - 1),
                    b.cursor.col,
                )
                .1 as u32) as f64
            * ts.cell_w as f64;
        let y = (r.y + 1 + row as u32) as f64 * ts.cell_h as f64;
        w.set_ime_cursor_area(
            PhysicalPosition::new(x, y),
            PhysicalSize::new(ts.cell_w as u32 * 4, ts.cell_h as u32),
        );
    }

    fn draw(&mut self) {
        if let Some(ed) = self.editor.as_ref() {
            let sig = (ed.config.font.family.clone(), ed.config.font.size);
            if sig != self.text_sig
                && let (Some(ts), Some(w)) = (&mut self.text, &self.window)
            {
                ts.set_font(sig.0.clone(), sig.1 * w.scale_factor() as f32);
                self.text_sig = sig;
            }
            if self.close_warned && !ed.buffers.iter().any(|b| b.dirty) {
                self.close_warned = false;
            }
        }
        let (Some(gpu), Some(text), Some(rects), Some(ed)) = (
            &mut self.gpu,
            &mut self.text,
            &mut self.rects,
            &mut self.editor,
        ) else {
            return;
        };
        text.viewport.update(
            &gpu.queue,
            glyphon::Resolution {
                width: gpu.config.width,
                height: gpu.config.height,
            },
        );
        let got = gpu.surface.get_current_texture();
        let frame = match got {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            other => {
                let lost = matches!(other, wgpu::CurrentSurfaceTexture::Lost);
                let reconfigure = matches!(other, wgpu::CurrentSurfaceTexture::Outdated);
                drop(other);
                if lost {
                    let Some(w) = &self.window else { return };
                    let Ok(s) = gpu.instance.create_surface(w.clone()) else {
                        return;
                    };
                    gpu.surface = s;
                    gpu.configure(gpu.config.width, gpu.config.height);
                    w.request_redraw();
                    return;
                }
                if reconfigure {
                    gpu.configure(gpu.config.width, gpu.config.height);
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let grid = render::Layout::new(gpu, text, ed);
        for w in &grid.wins {
            if let Some(win) = ed.windows.get_mut(w.win) {
                win.view_rows = w.body_rows() as usize;
                win.view_cols = (w.cols as usize).saturating_sub(w.gutter as usize).max(1);
            }
        }
        ed.clamp_views();
        ed.poll_lsp();
        let rows = grid.focus(ed).map_or(0, |w| w.body_rows() as usize);
        ed.scroll_to_cursor(rows);
        let lsp_busy = ed.lsp.busy();
        self.tab_ranges =
            render::tab_hit_ranges(ed, &grid, text.cell_w.max(1.0), text.cell_h.max(1.0));
        self.win_rects = grid.wins.clone();
        if lsp_busy
            && !self.pump_armed
            && let Some(p) = self.proxy.clone()
        {
            self.pump_armed = true;
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(200));
                let _ = p.send_event(());
            });
        }
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        ed.clamp_cursors();
        ed.refresh_spell_spans();
        ed.refresh_marks();
        if let Err(e) = render::draw_content(gpu, text, rects, ed, &view, &mut encoder) {
            if !self.paint_warned {
                self.paint_warned = true;
                eprintln!("deditor: {e}");
            }
            // A cache that changed underneath the frame is rebuilt by the next
            // one. A full atlas is not, so the frame is dropped and the next
            // real event repaints instead of the editor spinning on it.
            if matches!(e, render::PaintError::Retry)
                && let Some(w) = &self.window
            {
                w.request_redraw();
            }
            return;
        }
        self.paint_warned = false;
        gpu.queue.submit(Some(encoder.finish()));
        gpu.queue.present(frame);
    }
}
