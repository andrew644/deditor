pub mod rects;

use crate::buffer::Pos;
use crate::wrap::{cell_of, cells};
use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache};
use glyphon::{Cache, TextAtlas, TextRenderer, Viewport};
use std::sync::Arc;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
}

impl Gpu {
    pub async fn new(window: Arc<Window>, event_loop: &ActiveEventLoop) -> crate::Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: if cfg!(windows) {
                wgpu::Backends::DX12 | wgpu::Backends::VULKAN
            } else {
                wgpu::Backends::VULKAN
            },
            ..wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
                event_loop.owned_display_handle(),
            ))
        });
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("create surface: {e}"))?;
        let adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        let mut chosen = None;
        let mut fallback = None;
        for a in adapters {
            if !a.is_surface_supported(&surface) {
                continue;
            }
            let hw = matches!(
                a.get_info().device_type,
                wgpu::DeviceType::IntegratedGpu
                    | wgpu::DeviceType::DiscreteGpu
                    | wgpu::DeviceType::VirtualGpu
            );
            if hw && chosen.is_none() {
                chosen = Some(a);
            } else if fallback.is_none() {
                fallback = Some(a);
            }
        }
        let adapter = chosen
            .or(fallback)
            .ok_or("no GPU adapter supports this surface")?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("deditor"),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("request device: {e}"))?;
        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);
        Ok(Self {
            instance,
            device,
            queue,
            surface,
            config,
        })
    }

    pub fn configure(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }
}

pub struct TextSystem {
    pub font_system: FontSystem,
    pub swash_cache: SwashCache,
    pub viewport: Viewport,
    pub atlas: TextAtlas,
    pub renderer: TextRenderer,
    /// Used by the overlay layer. `TextRenderer::render` destroys and replaces its own
    /// vertex buffer when it grows, which would invalidate a first layer's recorded
    /// draw, so each layer needs a renderer of its own.
    pub overlay_renderer: TextRenderer,
    pub family_name: String,
    pub font_size_px: f32,
    pub cell_w: f32,
    pub cell_h: f32,
}

impl TextSystem {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        family_name: String,
        font_size_px: f32,
    ) -> Self {
        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(device);
        let viewport = Viewport::new(device, &cache);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        let renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let overlay_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let mut ts = Self {
            font_system,
            swash_cache,
            viewport,
            atlas,
            renderer,
            overlay_renderer,
            family_name,
            font_size_px,
            cell_w: 8.0,
            cell_h: 16.0,
        };
        ts.remeasure();
        ts
    }

    /// The family used for both measuring and drawing. A name without a
    /// fixed-pitch face falls back to `Monospace`; the cell grid is only valid
    /// when every glyph advances by exactly one cell.
    /// Shape without a color so each `TextArea`'s `default_color` (the theme) wins:
    /// cosmic-text stamps `Attrs::color_opt` onto every glyph and glyphon prefers it.
    fn attrs<'a>(&'a self) -> Attrs<'a> {
        Attrs::new().family(grid_family(&self.font_system, &self.family_name))
    }

    fn remeasure(&mut self) {
        let name = self.family_name.clone();
        let font = grid_family(&self.font_system, &name);
        let line_height = self.font_size_px * 1.35;
        self.cell_w = measure_cell(&mut self.font_system, font, self.font_size_px);
        self.cell_h = line_height.round().max(1.0);
    }

    pub fn set_font(&mut self, family: String, size_px: f32) {
        self.family_name = family;
        self.font_size_px = size_px;
        self.remeasure();
    }
}

pub fn grid_family<'a>(fonts: &FontSystem, name: &'a str) -> Family<'a> {
    let named = fonts
        .db()
        .faces()
        .any(|f| f.monospaced && f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)));
    if named {
        Family::Name(name)
    } else {
        Family::Monospace
    }
}

fn measure_cell(fonts: &mut FontSystem, family: Family<'_>, size: f32) -> f32 {
    let line_height = size * 1.35;
    let attrs = Attrs::new().family(family);
    let mut buf = Buffer::new(fonts, Metrics::new(size, line_height));
    buf.set_size(Some(10000.0), Some(line_height * 4.0));
    buf.set_text("MMMMMMMMMM", &attrs, Shaping::Advanced, None);
    buf.shape_until_scroll(fonts, false);
    buf.layout_runs()
        .next()
        .map(|r| r.line_w / 10.0)
        .unwrap_or(size * 0.6)
        .max(1.0)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WinRect {
    pub win: usize,
    pub x: u32,
    pub y: u32,
    pub cols: u32,
    pub height: u32,
    pub gutter: u32,
    pub num_cols: u32,
    pub sign_cols: u32,
    pub blame_cols: u32,
}

impl WinRect {
    pub fn body_rows(&self) -> u32 {
        self.height.saturating_sub(1)
    }
}

pub struct Layout {
    pub cols: u32,
    pub rows: u32,
    pub wins: Vec<WinRect>,
}

pub fn tab_label(i: usize, b: &crate::buffer::Buffer) -> String {
    format!(
        " {} {}{} ",
        i + 1,
        b.display_name(),
        if b.dirty { "+" } else { "" }
    )
}

/// Width of the number field and of the sign field, in that order of priority
/// when the pane is too narrow to show both.
pub fn gutter_parts(b: &crate::buffer::Buffer, line_numbers: bool, signs: bool) -> (u32, u32) {
    let numbers = if line_numbers {
        (b.rope.len_lines().max(1).to_string().len() as u32) + 2
    } else {
        0
    };
    (numbers, if signs { 2 } else { 0 })
}

/// Width of the `date author` column that `F4` puts left of the line numbers.
pub const BLAME_W: usize = 22;

fn blame_text(e: Option<&crate::git::BlameLine>, w: usize) -> String {
    let Some(name) = w.checked_sub(12) else {
        return " ".repeat(w);
    };
    let Some(e) = e.filter(|e| !e.who.is_empty()) else {
        return " ".repeat(w);
    };
    let who = take_cells(&e.who, name);
    let mut out = format!("{:>10} {who}", e.when);
    out.push_str(&" ".repeat(name - cells(&who)));
    out.push(' ');
    out
}

/// Widths of the gutter fields in a pane `cols` columns wide, leaving the body
/// at least one column: line numbers win, then signs, then blame.
fn fit_gutter(cols: u32, num: u32, sign: u32, blame: u32) -> (u32, u32, u32) {
    let room = cols.saturating_sub(1);
    let num = num.min(room);
    let left = room - num;
    let sign = if sign <= left { sign } else { 0 };
    (num, sign, if blame <= left - sign { blame } else { 0 })
}

/// Origin of a box `size` wide that wants to start at `want`, kept inside the
/// span from `origin` to `origin + span`.
fn place_popup(want: f32, size: f32, origin: f32, span: f32) -> f32 {
    want.min(origin + span - size).max(origin)
}

/// Background of a popup plus the highlight of `sel_row`, when it has one.
fn popup_rects(
    overlay: &mut Vec<rects::RectInstance>,
    theme: &crate::config::RgbTheme,
    (x, y, w, h): (f32, f32, f32, f32),
    sel_row: Option<usize>,
    ch: f32,
) {
    overlay.push(rects::RectInstance {
        x,
        y,
        w,
        h,
        color: theme.popup_bg,
    });
    if let Some(row) = sel_row {
        overlay.push(rects::RectInstance {
            x,
            y: y + row as f32 * ch,
            w,
            h: ch,
            color: theme.popup_sel,
        });
    }
}

/// Text of the buffer line a screen-row loop last looked at. Wrapped lines
/// repeat the same buffer line over many screen rows.
struct LineCache {
    line: usize,
    text: String,
}

impl LineCache {
    fn get(&mut self, rope: &ropey::Rope, line: usize) -> &str {
        if self.line != line {
            self.line = line;
            self.text = crate::buffer::line_text(rope, line);
        }
        &self.text
    }
}

pub struct TabHit {
    pub x0: f32,
    pub x1: f32,
    pub y0: f32,
    pub y1: f32,
    pub win: usize,
    pub tab: usize,
}

pub fn tab_hit_ranges(ed: &crate::app::Editor, l: &Layout, cw: f32, ch: f32) -> Vec<TabHit> {
    let mut out = Vec::new();
    for wr in &l.wins {
        let Some(win) = ed.windows.get(wr.win) else {
            continue;
        };
        let mut x = wr.x as f32 * cw;
        let right = (wr.x + wr.cols) as f32 * cw;
        for (tab, &bi) in win.tabs.iter().enumerate() {
            if x >= right {
                break;
            }
            let wid = cells(&tab_label(tab, &ed.buffers[bi])) as f32 * cw;
            out.push(TabHit {
                x0: x,
                x1: x + wid,
                y0: wr.y as f32 * ch,
                y1: wr.y as f32 * ch + ch,
                win: wr.win,
                tab,
            });
            x += wid;
        }
    }
    out
}

impl Layout {
    pub fn new(gpu: &Gpu, ts: &TextSystem, ed: &crate::app::Editor) -> Self {
        let cw = ts.cell_w.max(1.0);
        let ch = ts.cell_h.max(1.0);
        let cols = (gpu.config.width as f32 / cw) as u32;
        let total = (gpu.config.height as f32 / ch) as u32;
        let mut cells = Vec::new();
        ed.tree.rects(
            crate::window::Rect {
                x: 0,
                y: 0,
                cols,
                rows: total.saturating_sub(1),
            },
            &mut cells,
        );
        let wins: Vec<WinRect> = cells
            .into_iter()
            .filter_map(|(win, r)| {
                let w = ed.windows.get(win)?;
                let (num, sign) = gutter_parts(
                    &ed.buffers[w.cur_buf()],
                    ed.config.line_numbers,
                    ed.has_signs(w.cur_buf()),
                );
                let blame = if ed.show_blame { BLAME_W as u32 } else { 0 };
                let (num, sign, blame) = fit_gutter(r.cols, num, sign, blame);
                Some(WinRect {
                    win,
                    x: r.x,
                    y: r.y,
                    cols: r.cols,
                    height: r.rows,
                    gutter: num + sign + blame,
                    num_cols: num,
                    sign_cols: sign,
                    blame_cols: blame,
                })
            })
            .collect();
        let focus = wins
            .iter()
            .find(|w| w.win == ed.active_win)
            .or_else(|| wins.first());
        let rows = match focus {
            Some(w) => w.body_rows(),
            None => total.saturating_sub(2),
        };
        Self { cols, rows, wins }
    }

    pub fn focus(&self, ed: &crate::app::Editor) -> Option<&WinRect> {
        self.wins.iter().find(|w| w.win == ed.active_win)
    }
}

pub fn draw_content(
    gpu: &mut Gpu,
    ts: &mut TextSystem,
    rp: &mut rects::RectPipeline,
    ed: &crate::app::Editor,
    view: &wgpu::TextureView,
    encoder: &mut wgpu::CommandEncoder,
) {
    let l = Layout::new(gpu, ts, ed);
    let cw = ts.cell_w;
    let ch = ts.cell_h;
    let w = gpu.config.width as f32;
    let h = gpu.config.height as f32;
    let theme = &ed.theme;
    let mut rects: Vec<rects::RectInstance> = Vec::new();
    let mut overlay: Vec<rects::RectInstance> = Vec::new();
    rects.push(rects::RectInstance {
        x: 0.0,
        y: 0.0,
        w,
        h,
        color: theme.bg,
    });
    struct Strip {
        tabs: String,
        gutter: String,
        sign: [String; 3],
        body: String,
        x: f32,
        y: f32,
        bw: f32,
        bh: f32,
        gutter_cols: u32,
        blame_cols: u32,
        focused: bool,
    }
    let mut strips: Vec<Strip> = Vec::new();
    for wr in &l.wins {
        let Some(win) = ed.windows.get(wr.win) else {
            continue;
        };
        let focused = wr.win == ed.active_win;
        let buf = &ed.buffers[win.cur_buf()];
        let ox = wr.x as f32 * cw;
        let oy = wr.y as f32 * ch;
        let wy = oy + ch;
        let wc = wr.cols as f32 * cw;
        let gutter = wr.gutter as usize;
        let wrap = ed.wrap_with((wr.cols as usize).saturating_sub(wr.gutter as usize).max(1));
        let rows = wrap.screen_rows(
            &buf.rope,
            buf.view_top,
            buf.view_sub,
            wr.body_rows() as usize,
        );
        let mut tab_text = String::new();
        for (i, &bi) in win.tabs.iter().enumerate() {
            let label = tab_label(i, &ed.buffers[bi]);
            let start = ox + cells(&tab_text) as f32 * cw;
            if start >= ox + wc {
                break;
            }
            tab_text.push_str(&label);
            rects.push(rects::RectInstance {
                x: start,
                y: oy,
                w: cells(&label) as f32 * cw,
                h: ch,
                color: if !focused {
                    theme.status_bg
                } else if bi == win.cur_buf() {
                    theme.tab_active
                } else {
                    theme.tab_inactive
                },
            });
        }
        let cursor_row = wrap.screen_row(&buf.rope, buf.view_top, buf.view_sub, buf.cursor);
        if focused
            && let Some(row) = cursor_row
            && row < rows.len()
        {
            rects.push(rects::RectInstance {
                x: ox,
                y: wy + row as f32 * ch,
                w: wc,
                h: ch,
                color: theme.cursorline,
            });
        }
        let hi_term: Option<String> = match ed.cmd_kind {
            Some(crate::mode::CmdKind::Search) | Some(crate::mode::CmdKind::SearchBack) => {
                if ed.cmdline.is_empty() {
                    None
                } else {
                    Some(ed.cmdline.clone())
                }
            }
            _ => ed.search.clone(),
        };
        let mut lines = LineCache {
            line: usize::MAX,
            text: String::new(),
        };
        if let Some(term) = hi_term {
            let t: Vec<char> = term.chars().collect();
            let fold = crate::search::ignore_case(&term);
            if !t.is_empty() {
                for (r, scr) in rows.iter().enumerate() {
                    if scr.line >= buf.rope.len_lines() || scr.start >= scr.end {
                        continue;
                    }
                    let raw = lines.get(&buf.rope, scr.line);
                    let chars: Vec<char> = raw.chars().collect();
                    for i in scr.start..scr.end {
                        if i + t.len() > chars.len() {
                            break;
                        }
                        if !crate::search::matches_at(&chars, &t, i, fold) {
                            continue;
                        }
                        rects.push(rects::RectInstance {
                            x: ox + (gutter + wrap.pos(raw, i).1) as f32 * cw,
                            y: wy + r as f32 * ch,
                            w: t.len().min(scr.end - i) as f32 * cw,
                            h: ch,
                            color: theme.warn,
                        });
                    }
                }
            }
        }
        if focused && ed.config.spellcheck {
            for (r, scr) in rows.iter().enumerate() {
                if scr.line >= buf.rope.len_lines() || scr.start >= scr.end {
                    continue;
                }
                let raw = lines.get(&buf.rope, scr.line);
                let Some((_, spans)) = ed.spell.spans.get(&scr.line) else {
                    continue;
                };
                for (col, len) in spans.iter().copied() {
                    if col < scr.start || col >= scr.end {
                        continue;
                    }
                    rects.push(rects::RectInstance {
                        x: ox + (gutter + wrap.pos(raw, col).1) as f32 * cw,
                        y: wy + r as f32 * ch + ch - 2.0,
                        w: len.min(scr.end - col) as f32 * cw,
                        h: 2.0,
                        color: theme.error,
                    });
                }
            }
        }
        if focused
            && ed.mode == crate::mode::Mode::Visual
            && let Some((r0, r1, c0, c1)) = ed.visual_block.then_some(ed.block_range()).flatten()
        {
            for (r, scr) in rows.iter().enumerate() {
                if scr.line < r0 || scr.line > r1 || scr.line >= buf.rope.len_lines() {
                    continue;
                }
                let raw = lines.get(&buf.rope, scr.line);
                let len = raw.chars().count();
                let lx = (gutter + wrap.pos(raw, c0.min(len)).1) as f32 * cw;
                let mut width = (gutter + wrap.pos(raw, (c1 + 1).min(len)).1) as f32 * cw - lx;
                if c1 + 1 > len {
                    width += (c1 + 1 - len) as f32 * cw;
                }
                if width <= 0.0 {
                    continue;
                }
                rects.push(rects::RectInstance {
                    x: ox + lx,
                    y: wy + r as f32 * ch,
                    w: width,
                    h: ch,
                    color: theme.selection,
                });
            }
        }
        if focused && ed.mode == crate::mode::Mode::Visual && !ed.visual_block {
            let ((sa, sb), lw) = ed.sel_range();
            for (r, scr) in rows.iter().enumerate() {
                let line = scr.line;
                if line >= buf.rope.len_lines() || scr.start >= scr.end {
                    continue;
                }
                let raw = lines.get(&buf.rope, scr.line);
                let (from, to) = match sel_row_span(sa, sb, lw, line) {
                    Some(f) => f,
                    None => continue,
                };
                let from = from.unwrap_or(scr.start);
                let a = from.max(scr.start);
                if a >= scr.end {
                    continue;
                }
                let lx = (gutter + wrap.pos(raw, a).1) as f32 * cw;
                let width = if to == usize::MAX {
                    wc - lx
                } else {
                    let b = to.min(scr.end).max(a + 1);
                    (gutter + wrap.pos(raw, b).1) as f32 * cw - lx
                };
                if width <= 0.0 {
                    continue;
                }
                rects.push(rects::RectInstance {
                    x: ox + lx,
                    y: wy + r as f32 * ch,
                    w: width,
                    h: ch,
                    color: theme.selection,
                });
            }
        }
        let cur_line = crate::buffer::line_text(&buf.rope, buf.cursor.line);
        let cur_x = ox + (gutter + wrap.pos(&cur_line, buf.cursor.col).1) as f32 * cw;
        if focused
            && ed.cmd_kind.is_none()
            && let Some(row) = cursor_row
            && row < rows.len()
        {
            rects.push(rects::RectInstance {
                x: cur_x,
                y: wy + row as f32 * ch,
                w: if ed.mode == crate::mode::Mode::Insert {
                    2.0
                } else {
                    cw
                },
                h: ch,
                color: theme.accent,
            });
        }

        let mut gutter_s = String::new();
        let mut body_s = String::new();
        let mut sign_s = [String::new(), String::new(), String::new()];
        let blame_w = wr.blame_cols as usize;
        let sign_w = wr.sign_cols as usize;
        let num_w = wr.num_cols as usize;
        for (r, row) in rows.iter().enumerate() {
            if row.line < buf.rope.len_lines() {
                if wr.gutter > 0 {
                    if row.first() {
                        gutter_s.push_str(&blame_text(buf.blame.get(row.line), blame_w));
                        gutter_s.push_str(&" ".repeat(sign_w));
                        if num_w > 0 && ed.config.line_numbers {
                            gutter_s.push_str(&format!("{:>w$} ", row.line + 1, w = num_w - 1));
                        }
                    } else {
                        gutter_s.push_str(&" ".repeat(gutter));
                    }
                }
                if sign_w > 0 {
                    let mark = match buf.signs.get(row.line) {
                        Some(crate::git::Sign::Added) => 0,
                        Some(crate::git::Sign::Modified) => 1,
                        Some(crate::git::Sign::Deleted) => 2,
                        _ => 3,
                    };
                    for (i, s) in sign_s.iter_mut().enumerate() {
                        s.push(if mark == i { ['+', '~', '-'][i] } else { ' ' });
                        s.push(' ');
                    }
                }
                let raw = lines.get(&buf.rope, row.line);
                body_s.push_str(&wrap.text(raw, row.start, row.end, ed.config.show_whitespace));
            }
            if r + 1 < rows.len() {
                gutter_s.push('\n');
                body_s.push('\n');
                for s in sign_s.iter_mut() {
                    s.push('\n');
                }
            }
        }
        for d in ed.lsp.diags_for(buf.path.as_ref()) {
            let Some(row) = rows.iter().position(|r| r.line == d.line) else {
                continue;
            };
            rects.push(rects::RectInstance {
                x: ox,
                y: wy + row as f32 * ch,
                w: 4.0f32.max(cw * 0.35),
                h: ch,
                color: match d.severity {
                    crate::lsp::Sev::Warn => theme.warn,
                    crate::lsp::Sev::Error => theme.error,
                },
            });
        }
        strips.push(Strip {
            tabs: tab_text,
            gutter: gutter_s,
            sign: sign_s,
            body: body_s,
            x: ox,
            y: oy,
            bw: wc,
            bh: wr.body_rows() as f32 * ch,
            gutter_cols: wr.gutter,
            blame_cols: u32::try_from(usize::from(ed.show_blame) * BLAME_W).unwrap(),
            focused,
        });
    }
    rects.push(rects::RectInstance {
        x: 0.0,
        y: h - ch,
        w,
        h: ch,
        color: theme.status_bg,
    });
    if ed.cmd_kind.is_some() {
        let col = ed.cmdline_col.min(ed.cmdline.chars().count());
        rects.push(rects::RectInstance {
            x: (2 + col) as f32 * cw,
            y: h - ch,
            w: 2.0,
            h: ch,
            color: theme.accent,
        });
    }
    for (i, wr) in l.wins.iter().enumerate() {
        if i + 1 >= l.wins.len() {
            continue;
        }
        let next = &l.wins[i + 1];
        let (sx, sy, sw, sh) = if wr.y == next.y {
            (
                next.x as f32 * cw,
                wr.y as f32 * ch,
                1.0f32.max(cw * 0.15),
                wr.height.max(next.height) as f32 * ch,
            )
        } else {
            (
                wr.x.min(next.x) as f32 * cw,
                next.y as f32 * ch,
                (wr.cols.max(next.cols)) as f32 * cw,
                1.0f32.max(ch * 0.15),
            )
        };
        rects.push(rects::RectInstance {
            x: sx,
            y: sy,
            w: sw,
            h: sh,
            color: theme.status_bg,
        });
    }
    let fallback = WinRect {
        win: ed.active_win,
        x: 0,
        y: 0,
        cols: l.cols,
        height: l.rows.saturating_add(1),
        gutter: 0,
        num_cols: 0,
        sign_cols: 0,
        blame_cols: 0,
    };
    let f = l.focus(ed).unwrap_or(&fallback);
    let ox = f.x as f32 * cw;
    let oy = f.y as f32 * ch;
    let wy = oy + ch;
    let wc = f.cols as f32 * cw;
    let wh = f.height as f32 * ch;
    let gutter = f.gutter as usize;
    let buf = ed.buf();
    let wrap = ed.wrap();
    let cursor_row = wrap.screen_row(&buf.rope, buf.view_top, buf.view_sub, buf.cursor);
    let cur_line = crate::buffer::line_text(&buf.rope, buf.cursor.line);
    let cur_x = ox + (gutter + wrap.pos(&cur_line, buf.cursor.col).1) as f32 * cw;
    let diags = ed.lsp.diags_for(buf.path.as_ref());
    let cursor_diag = diags.iter().find(|d| d.line == buf.cursor.line).map(|d| {
        let tag = match d.severity {
            crate::lsp::Sev::Warn => "WARN",
            crate::lsp::Sev::Error => "ERR",
        };
        format!("{tag}: {}", d.message.replace('\n', " "))
    });
    let msg = ed.status.clone().or(cursor_diag).unwrap_or_default();
    let mode_label = match ed.mode {
        crate::mode::Mode::Normal => "NORMAL",
        crate::mode::Mode::Insert => "INSERT",
        crate::mode::Mode::Visual => {
            if ed.visual_block {
                "BLOCK"
            } else {
                "VISUAL"
            }
        }
        crate::mode::Mode::Command(k) => match k {
            crate::mode::CmdKind::Ex => "-- EX --",
            crate::mode::CmdKind::Search | crate::mode::CmdKind::SearchBack => "-- SEARCH --",
        },
    };
    let ruler = format!("{}:{}", buf.cursor.line + 1, buf.cursor.col + 1);
    let right_x = (l.cols as usize)
        .saturating_sub(cells(&ruler) + cells(mode_label) + 3)
        .max(1);
    let room = right_x.saturating_sub(1);
    let left_s = match ed.cmd_kind {
        Some(kind) => {
            let p = match kind {
                crate::mode::CmdKind::Ex => ":",
                crate::mode::CmdKind::Search => "/",
                crate::mode::CmdKind::SearchBack => "?",
            };
            format!(" {p}{}", ed.cmdline)
        }
        None if !msg.is_empty() => format!(" {msg}"),
        None => {
            let pend = pending_label(ed);
            let mut s = format!(
                " {}{}",
                buf.display_name(),
                if buf.dirty { "[+]" } else { "" }
            );
            if !pend.is_empty() {
                s.push_str(&format!(" [{pend}]"));
            }
            s
        }
    };
    let left_s = take_cells(&left_s, room);
    let right_s = format!("{ruler}  {mode_label}");
    let left_color = if ed.cmd_kind.is_none() && ed.status_err && !msg.is_empty() {
        theme.error
    } else {
        theme.fg
    };

    let hover_cols = ((wc / cw) as usize).min(80);
    let hover_s = ed
        .lsp
        .hover
        .clone()
        .unwrap_or_default()
        .lines()
        .take(8)
        .map(|s| pad_line(&s.replace('\t', "    "), hover_cols))
        .collect::<Vec<_>>()
        .join("\n");
    let hover_open =
        !hover_s.is_empty() && ed.panel == crate::fuzzy::Panel::None && ed.context_menu.is_none();
    let mut hover_pos = None;
    if hover_open {
        let nrows = hover_s.lines().count() as f32;
        let row = 1.0 + cursor_row.unwrap_or(0) as f32;
        let hw = hover_cols as f32 * cw;
        let hx = place_popup(ox + gutter as f32 * cw + 10.0 * cw, hw, ox, wc);
        let hy = place_popup(oy + row * ch + ch, (nrows + 1.0) * ch, oy + ch, wh - ch);
        hover_pos = Some((hx, hy));
        popup_rects(
            &mut overlay,
            theme,
            (hx, hy, hw, (nrows + 1.0) * ch),
            None,
            ch,
        );
    }

    let menu_raw = ed.context_menu.clone().unwrap_or_default();
    let menu_lines: Vec<String> = menu_raw.lines().map(|s| format!(" {s} ")).collect();
    let menu_s = menu_lines.join("\n");
    let menu_open = !menu_s.is_empty() && ed.panel == crate::fuzzy::Panel::None;
    let mut menu_pos = None;
    if menu_open {
        let mcols = menu_lines.iter().map(|s| cells(s)).max().unwrap_or(1);
        let mrows = menu_lines.len() as f32;
        let mw = mcols as f32 * cw;
        let mx = place_popup(cur_x, mw, ox, wc);
        let my = place_popup(
            oy + (cursor_row.unwrap_or(0) as f32 + 2.0) * ch,
            (mrows + 1.0) * ch,
            oy + ch,
            wh - ch,
        );
        menu_pos = Some((mx, my));
        popup_rects(
            &mut overlay,
            theme,
            (mx, my, mw, (mrows + 1.0) * ch),
            None,
            ch,
        );
    }

    let mut comp_s = String::new();
    let mut comp_pos = None;
    if let Some(cs) = &ed.lsp.completion {
        let n = cs.items.len();
        let max_rows = 8usize;
        let start = if cs.selected < max_rows {
            0
        } else {
            cs.selected - max_rows + 1
        };
        let comp_sel_row = cs.selected - start;
        if n == 0 {
            comp_s.push_str("  ...");
        }
        for (row, it) in cs.items.iter().skip(start).take(max_rows).enumerate() {
            if row > 0 {
                comp_s.push('\n');
            }
            comp_s.push_str(if row == comp_sel_row { "> " } else { "  " });
            comp_s.push(it.kind);
            comp_s.push(' ');
            comp_s.push_str(&format!("{:<38.38}", it.text));
        }
        let nrows = comp_s.lines().count() as f32;
        let row = 1.0
            + wrap
                .screen_row(
                    &buf.rope,
                    buf.view_top,
                    buf.view_sub,
                    crate::buffer::Pos {
                        line: cs.line,
                        col: 0,
                    },
                )
                .unwrap_or(0) as f32;
        let cwid = 42.0f32 * cw;
        let cx = place_popup(ox + (gutter as f32 + cs.col as f32) * cw, cwid, ox, wc);
        let cy = place_popup(oy + (row + 1.0) * ch, nrows * ch, oy + ch, wh - ch);
        comp_pos = Some((cx, cy, nrows));
        popup_rects(
            &mut overlay,
            theme,
            (cx, cy, cwid, nrows * ch),
            (n > 0).then_some(comp_sel_row),
            ch,
        );
    }

    let mut popup_s = String::new();
    let popup_open = ed.panel != crate::fuzzy::Panel::None;
    if popup_open {
        let max_rows = 15usize;
        let shown = ed.panel_order.len().min(max_rows);
        let start = if ed.panel_cursor < max_rows {
            0
        } else {
            ed.panel_cursor - max_rows + 1
        };
        let sel_row = ed
            .panel_cursor
            .saturating_sub(start)
            .min(shown.saturating_sub(1));
        let box_cols = (f.cols as f32 * 0.5).max(40.0).min(f.cols as f32) as usize;
        let pw = box_cols as f32 * cw;
        popup_s.push_str(&pad_line(&ed.panel_query, box_cols));
        for (row, &idx) in ed.panel_order.iter().skip(start).take(max_rows).enumerate() {
            popup_s.push('\n');
            popup_s.push_str(&pad_line(
                &format!(
                    "{}{}",
                    if row == sel_row { "> " } else { "  " },
                    elide_path(&ed.panel_items[idx], box_cols - 2)
                ),
                box_cols,
            ));
        }
        popup_rects(
            &mut overlay,
            theme,
            (ox, wy, pw, (1 + shown) as f32 * ch),
            (!ed.panel_order.is_empty()).then_some(1 + sel_row),
            ch,
        );
    }

    let mut cpl_s = String::new();
    let mut cpl_pos = None;
    if let Some(cp) = &ed.completions {
        let max_rows = 8usize;
        let start = cp.cursor.saturating_sub(max_rows - 1);
        let sel_row = cp.cursor - start;
        let box_cols = ((l.cols as usize) / 2).clamp(24, (l.cols as usize).max(24));
        for (row, &idx) in cp.order.iter().skip(start).take(max_rows).enumerate() {
            if row > 0 {
                cpl_s.push('\n');
            }
            cpl_s.push_str(&pad_line(
                &format!(
                    "{}{}",
                    if row == sel_row { "> " } else { "  " },
                    cp.items[idx]
                ),
                box_cols,
            ));
        }
        let (cx, cy, box_w, box_h) = completion_box(
            cells(&ed.cmdline),
            room,
            cpl_s.lines().count(),
            box_cols,
            (w, h),
            (cw, ch),
        );
        cpl_pos = Some((cx, cy));
        popup_rects(
            &mut overlay,
            theme,
            (cx, cy, box_w, box_h),
            Some(sel_row),
            ch,
        );
    }

    let mut tb_msg = mkbuf(ts, &left_s, w, h);
    let mut tb_ruler = mkbuf(ts, &right_s, w, h);
    let mut tb_popup = mkbuf(ts, &popup_s, w, h);
    let mut tb_hover = mkbuf(ts, &hover_s, w, h);
    let mut tb_comp = mkbuf(ts, &comp_s, w, h);
    let mut tb_menu = mkbuf(ts, &menu_s, w, h);
    let mut tb_cpl = mkbuf(ts, &cpl_s, w, h);

    let mut strip_bufs: Vec<[Buffer; 6]> = strips
        .iter()
        .map(|s| {
            [
                mkbuf(ts, &s.tabs, s.bw.max(1.0), ch),
                mkbuf(ts, &s.gutter, s.bw.max(1.0), s.bh.max(1.0)),
                mkbuf(ts, &s.sign[0], s.bw.max(1.0), s.bh.max(1.0)),
                mkbuf(ts, &s.sign[1], s.bw.max(1.0), s.bh.max(1.0)),
                mkbuf(ts, &s.sign[2], s.bw.max(1.0), s.bh.max(1.0)),
                mkbuf(
                    ts,
                    &s.body,
                    (s.bw - s.gutter_cols as f32 * cw).max(1.0),
                    s.bh.max(1.0),
                ),
            ]
        })
        .collect();
    let mut areas: Vec<(&mut Buffer, f32, f32, u32)> = Vec::new();
    for (s, bs) in strips.iter().zip(strip_bufs.iter_mut()) {
        let [tab, gutter_b, add, change, delete, body] = bs;
        areas.push((
            tab,
            s.x,
            s.y,
            if s.focused { theme.fg } else { theme.gutter_fg },
        ));
        areas.push((gutter_b, s.x, s.y + ch, theme.gutter_fg));
        let sx = s.x + s.blame_cols as f32 * cw;
        areas.push((add, sx, s.y + ch, theme.sign_add));
        areas.push((change, sx, s.y + ch, theme.sign_change));
        areas.push((delete, sx, s.y + ch, theme.sign_delete));
        areas.push((body, s.x + s.gutter_cols as f32 * cw, s.y + ch, theme.fg));
    }
    areas.push((&mut tb_msg, 0.0, h - ch, left_color));
    areas.push((&mut tb_ruler, right_x as f32 * cw, h - ch, theme.fg));
    let mut oareas: Vec<(&mut Buffer, f32, f32, u32)> = Vec::new();
    if popup_open {
        oareas.push((&mut tb_popup, cw, ch, theme.popup_fg));
    }
    if let Some((px, py)) = cpl_pos {
        oareas.push((&mut tb_cpl, px, py, theme.popup_fg));
    }
    if let Some((hx, hy)) = hover_pos {
        oareas.push((&mut tb_hover, hx, hy, theme.popup_fg));
    }
    if let Some((cx, cy, _)) = comp_pos {
        oareas.push((&mut tb_comp, cx, cy, theme.popup_fg));
    }
    if let Some((mx, my)) = menu_pos {
        oareas.push((&mut tb_menu, mx, my, theme.popup_fg));
    }
    draw_layer(
        gpu,
        ts,
        rp,
        encoder,
        view,
        Layer {
            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            overlay: false,
            rects,
            areas,
        },
    );
    if !overlay.is_empty() || !oareas.is_empty() {
        draw_layer(
            gpu,
            ts,
            rp,
            encoder,
            view,
            Layer {
                load: wgpu::LoadOp::Load,
                overlay: true,
                rects: overlay,
                areas: oareas,
            },
        );
    }
    ts.atlas.trim();
}

/// One paint pass: its rects, then its text. Drawing a popup's opaque
/// background in a later layer is what hides the buffer text underneath it.
struct Layer<'a> {
    load: wgpu::LoadOp<wgpu::Color>,
    overlay: bool,
    rects: Vec<rects::RectInstance>,
    areas: Vec<(&'a mut Buffer, f32, f32, u32)>,
}

fn draw_layer(
    gpu: &Gpu,
    ts: &mut TextSystem,
    rp: &mut rects::RectPipeline,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    layer: Layer<'_>,
) {
    let Layer {
        load,
        overlay,
        rects,
        areas,
    } = layer;
    let TextSystem {
        renderer,
        overlay_renderer,
        font_system,
        atlas,
        viewport,
        swash_cache,
        ..
    } = ts;
    let renderer = if overlay { overlay_renderer } else { renderer };
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    rp.draw(gpu, &mut pass, &rects, overlay);
    if areas.is_empty() {
        return;
    }
    renderer
        .prepare(
            &gpu.device,
            &gpu.queue,
            font_system,
            atlas,
            viewport,
            areas.into_iter().map(|(b, x, y, color)| glyphon::TextArea {
                buffer: b,
                left: x,
                top: y,
                scale: 1.0,
                bounds: glyphon::TextBounds {
                    left: 0,
                    top: 0,
                    right: gpu.config.width as i32,
                    bottom: gpu.config.height as i32,
                },
                default_color: glyphon::Color(color),
                custom_glyphs: &[],
            }),
            swash_cache,
        )
        .unwrap();
    renderer.render(atlas, viewport, &mut pass).unwrap();
}

fn completion_box(
    typed: usize,
    room: usize,
    rows: usize,
    box_cols: usize,
    screen: (f32, f32),
    cell: (f32, f32),
) -> (f32, f32, f32, f32) {
    let (w, h) = screen;
    let (cw, ch) = cell;
    let box_w = box_cols as f32 * cw;
    let box_h = rows as f32 * ch;
    let cells = typed.min(room) + 3;
    let x = (cells as f32 * cw).min((w - box_w).max(0.0));
    let y = (h - box_h - ch).max(0.0);
    (x, y, box_w, box_h)
}

fn pending_label(ed: &crate::app::Editor) -> String {
    use crate::mode::Pending;
    let mut s = String::new();
    if ed.count > 0 {
        s.push_str(&ed.count.to_string());
    }
    match &ed.pending {
        Pending::None => {}
        Pending::Op(op) => s.push_str(op_letter(op)),
        Pending::G => s.push('g'),
        Pending::Z => s.push('z'),
        Pending::WinCmd => s.push('^'),
        Pending::F { forward, to } => {
            s.push(match (forward, to) {
                (true, true) => 'f',
                (true, false) => 't',
                (false, true) => 'F',
                (false, false) => 'T',
            });
        }
        Pending::R => s.push('r'),
        Pending::TextObj { inner, .. } => s.push(if *inner { 'i' } else { 'a' }),
        Pending::OpG { op } => {
            s.push_str(op_letter(op));
            s.push('g');
        }
    }
    s
}

fn op_letter(op: &crate::mode::Operator) -> &'static str {
    use crate::mode::Operator;
    match op {
        Operator::Delete => "d",
        Operator::Change => "c",
        Operator::Yank => "y",
        Operator::Indent => ">",
        Operator::Outdent => "<",
    }
}

fn mkbuf(ts: &mut TextSystem, s: &str, w: f32, h: f32) -> Buffer {
    let metrics = Metrics::new(ts.font_size_px, ts.cell_h);
    let mut b = Buffer::new(&mut ts.font_system, metrics);
    b.set_size(Some(w), Some(h));
    {
        let attrs = ts.attrs();
        b.set_text(s, &attrs, Shaping::Advanced, None);
    }
    b.shape_until_scroll(&mut ts.font_system, false);
    b
}

/// The longest prefix of `s` that fits in `room` cells.
fn take_cells(s: &str, room: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = cell_of(c);
        if used + w > room {
            break;
        }
        used += w;
        out.push(c);
    }
    out
}

/// Half-open cell span `[from, to)` the visual selection covers on `line`.
/// `sa` and `sb` are the inclusive anchor and exclusive end from
/// `Editor::sel_range`. `None` for from means the row's first cell, and
/// `usize::MAX` for to means the end of the screen row.
fn sel_row_span(sa: Pos, sb: Pos, lw: bool, line: usize) -> Option<(Option<usize>, usize)> {
    if lw {
        let last = if sb.col == 0 {
            sb.line.saturating_sub(1)
        } else {
            sb.line
        };
        return if line > last {
            None
        } else {
            Some((None, usize::MAX))
        };
    }
    if line < sa.line || line > sb.line {
        return None;
    }
    if sa.line == sb.line {
        Some((Some(sa.col), sb.col))
    } else if line == sa.line {
        Some((Some(sa.col), usize::MAX))
    } else if line == sb.line {
        if sb.col == 0 {
            None
        } else {
            Some((None, sb.col))
        }
    } else {
        Some((None, usize::MAX))
    }
}

fn elide_path(s: &str, cols: usize) -> String {
    if cells(s) <= cols {
        return s.to_string();
    }
    let mut kept = String::new();
    let mut used = 1;
    for c in s.chars().rev() {
        let w = cell_of(c);
        if used + w > cols {
            break;
        }
        used += w;
        kept.insert(0, c);
    }
    format!("…{kept}")
}

fn pad_line(s: &str, cols: usize) -> String {
    let mut out = take_cells(s, cols);
    out.push_str(&" ".repeat(cols - cells(&out)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elide_path_keeps_the_file_name_visible() {
        assert_eq!(elide_path("src/mod.rs", 20), "src/mod.rs");
        assert_eq!(elide_path("src/lsp/client.rs", 10), "…client.rs");
        assert_eq!(elide_path("deep/nested/path/file.txt", 1), "…");
    }

    #[test]
    fn sign_column_widens_the_gutter_only_when_used() {
        let mut b = crate::buffer::Buffer::empty();
        b.insert_text("one\ntwo\nthree\n", false);
        assert_eq!(gutter_parts(&b, true, false), (3, 0));
        assert_eq!(gutter_parts(&b, true, true), (3, 2));
        assert_eq!(gutter_parts(&b, false, false), (0, 0));
        assert_eq!(gutter_parts(&b, false, true), (0, 2));
    }

    #[test]
    fn blame_text_never_overflows_its_column() {
        let e = crate::git::BlameLine {
            when: "2026-11-20".into(),
            who: "andrew".into(),
        };
        for w in [0, 5, 11, 12, 21, BLAME_W] {
            assert_eq!(cells(&blame_text(Some(&e), w)), w, "width {w}");
        }
        let cjk = crate::git::BlameLine {
            when: "2026-11-20".into(),
            who: "佐藤一郎".into(),
        };
        assert_eq!(cells(&blame_text(Some(&cjk), BLAME_W)), BLAME_W);
        assert_eq!(blame_text(Some(&e), 12).trim_end(), "2026-11-20");
        assert_eq!(blame_text(None, BLAME_W), " ".repeat(BLAME_W));
    }

    #[test]
    fn a_narrow_pane_keeps_room_for_the_body() {
        assert_eq!(fit_gutter(80, 6, 2, 22), (6, 2, 22));
        assert_eq!(fit_gutter(8, 6, 2, 22), (6, 0, 0));
        assert_eq!(fit_gutter(1, 6, 2, 22), (0, 0, 0));
        assert_eq!(fit_gutter(20, 6, 2, 22), (6, 2, 0));
    }

    #[test]
    fn popups_stay_inside_their_window() {
        assert_eq!(place_popup(50.0, 10.0, 0.0, 100.0), 50.0);
        assert_eq!(place_popup(95.0, 10.0, 0.0, 100.0), 90.0);
        assert_eq!(place_popup(5.0, 140.0, 0.0, 100.0), 0.0);
    }

    #[test]
    fn tab_hits_stop_at_the_window_edge() {
        let mut ed = crate::app::Editor::test_new();
        for _ in 1..4 {
            ed.buffers.push(crate::buffer::Buffer::empty());
        }
        ed.windows[0].tabs = vec![0, 1, 2, 3];
        let layout = Layout {
            cols: 80,
            rows: 24,
            wins: vec![WinRect {
                win: 0,
                x: 0,
                y: 0,
                cols: 20,
                height: 24,
                gutter: 0,
                num_cols: 0,
                sign_cols: 0,
                blame_cols: 0,
            }],
        };
        let hits = tab_hit_ranges(&ed, &layout, 10.0, 20.0);
        assert!(hits.len() < 4, "tabs past the edge still claim clicks");
        assert!(hits.iter().all(|h| h.x0 < 200.0));
    }

    #[test]
    fn tab_hit_ranges_are_offset_per_window() {
        let mut ed = crate::app::Editor::test_new();
        ed.windows.push(crate::window::Window::new(vec![0]));
        let layout = Layout {
            cols: 80,
            rows: 24,
            wins: vec![
                WinRect {
                    win: 0,
                    x: 0,
                    y: 0,
                    cols: 40,
                    height: 24,
                    gutter: 2,
                    num_cols: 2,
                    sign_cols: 0,
                    blame_cols: 0,
                },
                WinRect {
                    win: 1,
                    x: 40,
                    y: 0,
                    cols: 40,
                    height: 24,
                    gutter: 2,
                    num_cols: 2,
                    sign_cols: 0,
                    blame_cols: 0,
                },
            ],
        };
        let label_w = tab_label(0, &ed.buffers[0]).chars().count() as f32 * 10.0;
        let hits = tab_hit_ranges(&ed, &layout, 10.0, 20.0);
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[0].win, hits[0].tab), (0, 0));
        assert_eq!(
            (hits[0].x0, hits[0].x1, hits[0].y0, hits[0].y1),
            (0.0, label_w, 0.0, 20.0)
        );
        assert_eq!((hits[1].win, hits[1].tab), (1, 0));
        assert_eq!((hits[1].x0, hits[1].y0, hits[1].y1), (400.0, 0.0, 20.0));
        assert_eq!(hits[1].x1, 400.0 + label_w);
    }

    #[test]
    fn completion_box_sits_beside_the_command_line_without_covering_it() {
        let cell = (8.0, 16.0);
        let screen = (800.0, 480.0);
        let (x, y, w, h) = completion_box(6, 78, 3, 40, screen, cell);
        assert_eq!((x, y, w, h), (9.0 * 8.0, 480.0 - 4.0 * 16.0, 320.0, 48.0));
        let (x, _, w, _) = completion_box(70, 78, 3, 40, screen, cell);
        assert!(x + w <= screen.0);
        for rows in 1..=8 {
            let (x, y, w, h) = completion_box(6, 78, rows, 40, screen, cell);
            assert!(x >= 9.0 * cell.0);
            assert!(y + h <= screen.1 - cell.1);
            assert_eq!(w, 40.0 * cell.0);
        }
    }

    fn glyph_xs(fonts: &mut FontSystem, family: Family<'_>, text: &str) -> Vec<f32> {
        let mut buf = Buffer::new(fonts, Metrics::new(14.0, 18.9));
        buf.set_size(Some(10000.0), Some(80.0));
        buf.set_text(text, &Attrs::new().family(family), Shaping::Advanced, None);
        buf.shape_until_scroll(fonts, false);
        buf.layout_runs()
            .next()
            .map(|r| r.glyphs.iter().map(|g| g.x).collect())
            .unwrap_or_default()
    }

    fn face_name(fonts: &FontSystem, monospaced: bool) -> Option<String> {
        fonts
            .db()
            .faces()
            .find(|f| f.monospaced == monospaced)
            .and_then(|f| f.families.first().map(|(n, _)| n.clone()))
    }

    #[test]
    fn drawn_glyphs_land_on_the_measured_cell_grid() {
        let mut fonts = FontSystem::new();
        let name = "deditor-no-such-family".to_string();
        let family = grid_family(&fonts, &name);
        let cell = measure_cell(&mut fonts, family, 14.0);
        let xs = glyph_xs(&mut fonts, family, "abcdefghijklmnopqrstuvwxyz");
        assert!(xs.len() > 20);
        for (i, pair) in xs.windows(2).enumerate() {
            let advance = pair[1] - pair[0];
            assert!(
                (advance - cell).abs() < 0.01,
                "column {i}: advance {advance} != cell {cell}"
            );
        }
        let far = xs[25] - xs[0];
        assert!(
            (far - 25.0 * cell).abs() < 0.01,
            "column 25 drawn at {far}, grid expects {}",
            25.0 * cell
        );
    }

    #[test]
    fn unknown_family_uses_the_monospace_fallback() {
        let fonts = FontSystem::new();
        assert!(matches!(
            grid_family(&fonts, "deditor-no-such-family"),
            Family::Monospace
        ));
    }

    #[test]
    fn fixed_pitch_names_are_kept_and_proportional_names_are_not() {
        let fonts = FontSystem::new();
        if let Some(name) = face_name(&fonts, true) {
            assert!(
                matches!(grid_family(&fonts, &name), Family::Name(_)),
                "{name} is fixed pitch and should be used"
            );
        }
        if let Some(name) = face_name(&fonts, false) {
            assert!(
                matches!(grid_family(&fonts, &name), Family::Monospace),
                "{name} is proportional and must not drive the cell grid"
            );
        }
    }

    #[test]
    fn pad_line_fits_text_to_the_popup_width() {
        assert_eq!(pad_line("0123456789", 6), "012345");
        assert_eq!(pad_line("ab", 5), "ab   ");
        assert_eq!(pad_line("", 3), "   ");
    }

    #[test]
    fn visual_span_covers_exactly_the_inclusive_selection() {
        let p = |line, col| Pos { line, col };
        assert_eq!(sel_row_span(p(0, 0), p(0, 3), false, 0), Some((Some(0), 3)));
        assert_eq!(
            sel_row_span(p(0, 2), p(1, 4), false, 0),
            Some((Some(2), usize::MAX))
        );
        assert_eq!(sel_row_span(p(0, 2), p(1, 4), false, 1), Some((None, 4)));
        assert_eq!(
            sel_row_span(p(0, 2), p(2, 4), false, 1),
            Some((None, usize::MAX))
        );
        assert_eq!(sel_row_span(p(0, 2), p(2, 0), false, 2), None);
        assert_eq!(
            sel_row_span(p(0, 0), p(2, 0), true, 1),
            Some((None, usize::MAX))
        );
        assert_eq!(sel_row_span(p(0, 0), p(2, 0), true, 2), None);
        assert_eq!(
            sel_row_span(p(0, 0), p(1, 3), true, 1),
            Some((None, usize::MAX))
        );
    }

    #[test]
    fn wide_glyphs_are_measured_in_cells() {
        assert_eq!(cells("abc"), 3);
        assert_eq!(cells("日本語"), 6);
        assert_eq!(cells("a😀b"), 4);
        assert_eq!(cells("e\u{301}"), 1);
        assert_eq!(take_cells("日本語", 4), "日本");
        assert_eq!(pad_line("日本", 6), "日本  ");
        assert_eq!(pad_line("日本語", 6), "日本語");
        assert_eq!(elide_path("src/日本語.rs", 7), "…語.rs");
    }
}
