pub mod rects;

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
        let renderer = TextRenderer::new(
            &mut atlas,
            device,
            wgpu::MultisampleState::default(),
            None,
        );
        let overlay_renderer = TextRenderer::new(
            &mut atlas,
            device,
            wgpu::MultisampleState::default(),
            None,
        );
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
    pub fn attrs<'a>(&'a self) -> Attrs<'a> {
        Attrs::new().family(grid_family(&self.font_system, &self.family_name))
    }

    pub fn remeasure(&mut self) {
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
    let named = fonts.db().faces().any(|f| {
        f.monospaced && f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(name))
    });
    if named {
        Family::Name(name)
    } else {
        Family::Monospace
    }
}

pub fn measure_cell(fonts: &mut FontSystem, family: Family<'_>, size: f32) -> f32 {
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


pub struct Layout {
    pub cols: u32,
    pub rows: u32,
    pub gutter_cols: u32,
}

pub fn tab_hit_ranges(ed: &crate::app::Editor, cw: f32) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    let mut x = 0.0f32;
    for (i, b) in ed.buffers.iter().enumerate() {
        let label = format!(
            " {} {}{} ",
            i + 1,
            b.display_name(),
            if b.dirty { "+" } else { "" }
        );
        let wid = label.chars().count() as f32 * cw;
        out.push((x, x + wid));
        x += wid;
    }
    out
}

impl Layout {
    pub fn new(gpu: &Gpu, ts: &TextSystem, ed: &crate::app::Editor) -> Self {
        let cw = ts.cell_w.max(1.0);
        let ch = ts.cell_h.max(1.0);
        let cols = (gpu.config.width as f32 / cw) as u32;
        let total = (gpu.config.height as f32 / ch) as u32;
        let rows = total.saturating_sub(2);
        let gutter_cols = if ed.config.line_numbers {
            (ed.buf().rope.len_lines().max(1).to_string().len() as u32) + 2
        } else {
            0
        };
        Self {
            cols,
            rows,
            gutter_cols,
        }
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
    let buf = ed.buf();
    let mut rects: Vec<rects::RectInstance> = Vec::new();
    let mut overlay: Vec<rects::RectInstance> = Vec::new();
    rects.push(rects::RectInstance {
        x: 0.0,
        y: 0.0,
        w,
        h,
        color: theme.bg,
    });
    let mut tab_text = String::new();
    for (i, b) in ed.buffers.iter().enumerate() {
        let label = format!(
            " {} {}{} ",
            i + 1,
            b.display_name(),
            if b.dirty { "+" } else { "" }
        );
        let start = tab_text.chars().count() as f32 * cw;
        tab_text.push_str(&label);
        rects.push(rects::RectInstance {
            x: start,
            y: 0.0,
            w: label.chars().count() as f32 * cw,
            h: ch,
            color: if i == ed.active {
                theme.tab_active
            } else {
                theme.tab_inactive
            },
        });
    }
    rects.push(rects::RectInstance {
        x: 0.0,
        y: h - ch,
        w,
        h: ch,
        color: theme.status_bg,
    });
    let cur_row = buf.cursor.line.saturating_sub(buf.view_top);
    if buf.cursor.line >= buf.view_top && cur_row < l.rows as usize {
        rects.push(rects::RectInstance {
            x: 0.0,
            y: (1 + cur_row) as f32 * ch,
            w,
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
    if let Some(term) = hi_term {
        let t: Vec<char> = term.chars().collect();
        let fold = crate::search::ignore_case(&term);
        if !t.is_empty() {
            for r in 0..(l.rows as usize) {
                let line = buf.view_top + r;
                if line >= buf.rope.len_lines() {
                    continue;
                }
                let raw = crate::buffer::line_text(&buf.rope, line);
                let chars: Vec<char> = raw.chars().collect();
                if chars.len() < t.len() {
                    continue;
                }
                for i in 0..=(chars.len() - t.len()) {
                    if crate::search::matches_at(&chars, &t, i, fold) {
                        let x = (l.gutter_cols as usize + ed.visual_x(&raw, i)) as f32 * cw;
                        rects.push(rects::RectInstance {
                            x,
                            y: (1 + r) as f32 * ch,
                            w: t.len() as f32 * cw,
                            h: ch,
                            color: theme.warn,
                        });
                    }
                }
            }
        }
    }
    if ed.mode == crate::mode::Mode::Visual {
        let ((sa, sb), lw) = ed.sel_range();
        for r in 0..(l.rows as usize) {
            let line = buf.view_top + r;
            if line < sa.line || line > sb.line {
                continue;
            }
            let raw = crate::buffer::line_text(&buf.rope, line);
            let gcol = l.gutter_cols as usize;
            let range: (usize, usize) = if lw {
                let last = if sb.col == 0 {
                    sb.line.saturating_sub(1)
                } else {
                    sb.line
                };
                if line > last {
                    continue;
                }
                (0, usize::MAX)
            } else if sa.line == sb.line {
                (
                    ed.visual_x(&raw, sa.col),
                    ed.visual_x(&raw, sb.col).max(ed.visual_x(&raw, sa.col) + 1),
                )
            } else if line == sa.line {
                (ed.visual_x(&raw, sa.col), usize::MAX)
            } else if line == sb.line {
                if sb.col == 0 {
                    continue;
                }
                (0, ed.visual_x(&raw, sb.col))
            } else {
                (0, usize::MAX)
            };
            let x = (gcol + range.0) as f32 * cw;
            let width = if range.1 == usize::MAX {
                w - x
            } else {
                (gcol + range.1) as f32 * cw - x
            };
            if width <= 0.0 {
                continue;
            }
            rects.push(rects::RectInstance {
                x,
                y: (1 + r) as f32 * ch,
                w: width,
                h: ch,
                color: theme.selection,
            });
        }
    }
    let vt = buf.view_top;
    let cur_line = crate::buffer::line_text(&buf.rope, buf.cursor.line);
    let cur_x = (l.gutter_cols as usize + ed.visual_x(&cur_line, buf.cursor.col)) as f32 * cw;
    let cur_y = (1 + buf.cursor.line.saturating_sub(vt)) as f32 * ch;
    if buf.cursor.line >= vt && buf.cursor.line < vt + l.rows as usize {
        rects.push(rects::RectInstance {
            x: cur_x,
            y: cur_y,
            w: if ed.mode == crate::mode::Mode::Insert {
                2.0
            } else {
                cw
            },
            h: ch,
            color: theme.accent,
        });
    }
    let visible = l.rows as usize;
    let body_cols = l.cols.saturating_sub(l.gutter_cols) as usize;
    let mut gutter_s = String::new();
    let mut body_s = String::new();
    for r in 0..visible {
        let line = vt + r;
        if line < buf.rope.len_lines() {
            if l.gutter_cols > 0 {
                gutter_s.push_str(&format!(
                    "{:>w$} ",
                    line + 1,
                    w = l.gutter_cols as usize - 1
                ));
            }
            let raw = crate::buffer::line_text(&buf.rope, line);
            let shown: String = ed
                .expand_tabs(&raw, usize::MAX)
                .chars()
                .take(body_cols)
                .collect();
            body_s.push_str(&shown);
        }
        if r + 1 < visible {
            gutter_s.push('\n');
            body_s.push('\n');
        }
    }
    let diags = ed.lsp.diags_for(buf.path.as_ref());
    let cursor_diag = diags
        .iter()
        .find(|d| d.line == buf.cursor.line)
        .map(|d| {
            let tag = if d.severity == crate::lsp::SEV_WARN {
                "WARN"
            } else {
                "ERR"
            };
            format!("{tag}: {}", d.message.replace('\n', " "))
        });
    let msg = ed.status.clone().or(cursor_diag).unwrap_or_default();
    let mode_label = match ed.mode {
        crate::mode::Mode::Normal => "NORMAL",
        crate::mode::Mode::Insert => "INSERT",
        crate::mode::Mode::Visual => "VISUAL",
        crate::mode::Mode::Command(k) => match k {
            crate::mode::CmdKind::Ex => "-- EX --",
            crate::mode::CmdKind::Search | crate::mode::CmdKind::SearchBack => "-- SEARCH --",
        },
    };
    let ruler = format!("LINE {}, COL {}", buf.cursor.line + 1, buf.cursor.col + 1);
    let right_x = (l.cols as usize)
        .saturating_sub(ruler.chars().count() + mode_label.chars().count() + 3)
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
    let left_s = fit(&left_s, room);
    let right_s = format!("{ruler}  {mode_label}");
    let left_color = if ed.cmd_kind.is_none() && !msg.is_empty() {
        theme.error
    } else {
        theme.fg
    };
    for d in diags {
        if d.line < ed.buf().view_top || d.line - ed.buf().view_top >= l.rows as usize {
            continue;
        }
        let y = (1 + d.line - ed.buf().view_top) as f32 * ch;
        rects.push(rects::RectInstance {
            x: 0.0,
            y,
            w: 4.0f32.max(cw * 0.35),
            h: ch,
            color: if d.severity == crate::lsp::SEV_WARN {
                theme.warn
            } else {
                theme.error
            },
        });
    }

    let hover_cols = ((w / cw) as usize).min(80);
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
    let hover_open = !hover_s.is_empty() && ed.panel == crate::fuzzy::Panel::None;
    let mut hover_pos = None;
    if hover_open {
        let nrows = hover_s.lines().count() as f32;
        let row = (1 + ed.buf().cursor.line.saturating_sub(ed.buf().view_top)) as f32;
        let hw = hover_cols as f32 * cw;
        let hx = (l.gutter_cols as f32 * cw + 10.0 * cw).min(w - hw);
        let hy = (row * ch + ch).min(h - (nrows + 1.0) * ch);
        hover_pos = Some((hx, hy));
        overlay.push(rects::RectInstance {
            x: hx,
            y: hy,
            w: hw,
            h: (nrows + 1.0) * ch,
            color: theme.popup_bg,
        });
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
        let row = (1 + cs.line.saturating_sub(ed.buf().view_top)) as f32;
        let cwid = 42.0f32 * cw;
        let cx = ((l.gutter_cols as f32 + cs.col as f32) * cw).min(w - cwid);
        let cy = ((row + 1.0) * ch).min(h - nrows * ch);
        comp_pos = Some((cx, cy, nrows));
        overlay.push(rects::RectInstance {
            x: cx,
            y: cy,
            w: cwid,
            h: nrows * ch,
            color: theme.popup_bg,
        });
        if n > 0 {
            overlay.push(rects::RectInstance {
                x: cx,
                y: cy + comp_sel_row as f32 * ch,
                w: cwid,
                h: ch,
                color: theme.popup_sel,
            });
        }
    }

    let mut popup_s = String::new();
    let popup_open = ed.panel != crate::fuzzy::Panel::None;
    if popup_open {
        let max_rows = 15usize;
        let shown = ed.panel_order.len().min(max_rows);
        let box_cols = (l.cols as f32 * 0.5).max(40.0).min(l.cols as f32) as usize;
        let pw = box_cols as f32 * cw;
        popup_s.push_str(&pad_line(&ed.panel_query, box_cols));
        for (row, &idx) in ed.panel_order.iter().take(max_rows).enumerate() {
            popup_s.push('\n');
            popup_s.push_str(&pad_line(
                &format!(
                    "{}{}",
                    if row == ed.panel_cursor { "> " } else { "  " },
                    ed.panel_items[idx]
                ),
                box_cols,
            ));
        }
        overlay.push(rects::RectInstance {
            x: 0.0,
            y: ch,
            w: pw,
            h: (1 + shown) as f32 * ch,
            color: theme.popup_bg,
        });
        if !ed.panel_order.is_empty() {
            overlay.push(rects::RectInstance {
                x: 0.0,
                y: (2 + ed.panel_cursor.min(shown.saturating_sub(1))) as f32 * ch,
                w: pw,
                h: ch,
                color: theme.popup_sel,
            });
        }
    }

    let mut tb_tab = mkbuf(ts, &tab_text, w, h);
    let mut tb_gutter = mkbuf(ts, &gutter_s, w, h);
    let mut tb_body = mkbuf(ts, &body_s, w, h);
    let mut tb_msg = mkbuf(ts, &left_s, w, h);
    let mut tb_ruler = mkbuf(ts, &right_s, w, h);
    let mut tb_popup = mkbuf(ts, &popup_s, w, h);
    let mut tb_hover = mkbuf(ts, &hover_s, w, h);
    let mut tb_comp = mkbuf(ts, &comp_s, w, h);

    let areas: Vec<(&mut Buffer, f32, f32, u32)> = vec![
        (
            &mut tb_tab as &mut Buffer,
            0.0f32,
            0.0f32,
            theme.fg,
        ),
        (&mut tb_gutter, 0.0, ch, theme.gutter_fg),
        (&mut tb_body, l.gutter_cols as f32 * cw, ch, theme.fg),
        (&mut tb_msg, 0.0, h - ch, left_color),
        (&mut tb_ruler, right_x as f32 * cw, h - ch, theme.fg),
    ];
    let mut oareas: Vec<(&mut Buffer, f32, f32, u32)> = Vec::new();
    if popup_open {
        oareas.push((&mut tb_popup, cw, ch, theme.popup_fg));
    }
    if let Some((hx, hy)) = hover_pos {
        oareas.push((&mut tb_hover, hx, hy, theme.popup_fg));
    }
    if let Some((cx, cy, _)) = comp_pos {
        oareas.push((&mut tb_comp, cx, cy, theme.popup_fg));
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
    let Layer { load, overlay, rects, areas } = layer;
    let TextSystem { renderer, overlay_renderer, font_system, atlas, viewport, swash_cache, .. } =
        ts;
    let renderer = if overlay { overlay_renderer } else { renderer };
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
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
    renderer
        .render(atlas, viewport, &mut pass)
        .unwrap();
}

fn pending_label(ed: &crate::app::Editor) -> String {
    use crate::mode::{Operator, Pending};
    let mut s = String::new();
    if ed.count > 0 {
        s.push_str(&ed.count.to_string());
    }
    match &ed.pending {
        Pending::None => {}
        Pending::Op(op) => {
            s.push_str(match op {
                Operator::Delete => "d",
                Operator::Change => "c",
                Operator::Yank => "y",
                Operator::Indent => ">",
                Operator::Outdent => "<",
            });
        }
        Pending::G => s.push('g'),
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
    }
    s
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

fn fit(s: &str, room: usize) -> String {
    s.chars().take(room).collect()
}

fn pad_line(s: &str, cols: usize) -> String {
    let trimmed: String = s.chars().take(cols).collect();
    format!("{trimmed:<cols$}")
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let cell = measure_cell(&mut fonts, family.clone(), 14.0);
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
}
