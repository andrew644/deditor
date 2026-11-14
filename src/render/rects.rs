use crate::render::Gpu;
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct RectInstance {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: u32,
}

const SHADER: &str = r#"
struct Screen {
    size: vec2<f32>,
};

struct RectIn {
    @location(0) xy: vec2<f32>,
    @location(1) wh: vec2<f32>,
    @location(2) color: u32,
};

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) color: u32,
};

@group(0) @binding(0)
var<uniform> screen: Screen;

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        return c / 12.92;
    } else {
        return pow((c + 0.055) / 1.055, 2.4);
    }
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, r: RectIn) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
    );
    let p = r.xy + corners[vi] * r.wh;
    let x = (p.x / screen.size.x) * 2.0 - 1.0;
    let y = 1.0 - (p.y / screen.size.y) * 2.0;
    var o: VOut;
    o.pos = vec4(x, y, 0.0, 1.0);
    o.color = r.color;
    return o;
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let c = in.color;
    let a = f32((c >> 24u) & 0xffu) / 255.0;
    let r = srgb_to_linear(f32((c >> 16u) & 0xffu) / 255.0);
    let g = srgb_to_linear(f32((c >> 8u) & 0xffu) / 255.0);
    let b = srgb_to_linear(f32(c & 0xffu) / 255.0);
    return vec4(r, g, b, a);
}
"#;

pub struct RectPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    screen_uniform: wgpu::Buffer,
    /// One slot per layer: a layer's draw reads its instances from the buffer at submit
    /// time, so the overlay layer must not rewrite the buffer the base layer recorded.
    slots: [Instances; 2],
}

struct Instances {
    buf: wgpu::Buffer,
    cap: usize,
}

impl Instances {
    fn new(device: &wgpu::Device, cap: usize) -> Self {
        Self {
            buf: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rect-instances"),
                size: (cap * std::mem::size_of::<RectInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            cap,
        }
    }

    fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, rects: &[RectInstance]) {
        if self.cap < rects.len() {
            let cap = rects.len().next_power_of_two();
            *self = Self::new(device, cap);
        }
        queue.write_buffer(&self.buf, 0, bytemuck::cast_slice(rects));
    }
}

impl RectPipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rects"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let screen_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rect-screen"),
            size: 8,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rect-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(8),
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rects"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<RectInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Uint32],
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rects"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(instance_layout)],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            bind_layout,
            screen_uniform,
            slots: [Instances::new(device, 256), Instances::new(device, 64)],
        }
    }

    pub fn draw(
        &mut self,
        gpu: &Gpu,
        pass: &mut wgpu::RenderPass<'_>,
        rects: &[RectInstance],
        overlay: bool,
    ) {
        if rects.is_empty() {
            return;
        }
        let screen = [gpu.config.width as f32, gpu.config.height as f32];
        gpu.queue.write_buffer(&self.screen_uniform, 0, bytemuck::bytes_of(&screen));
        let slot = usize::from(overlay);
        self.slots[slot].write(&gpu.device, &gpu.queue, rects);
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rect-bg"),
            layout: &self.bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.screen_uniform.as_entire_binding(),
            }],
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.set_vertex_buffer(0, self.slots[slot].buf.slice(..));
        pass.draw(0..6, 0..rects.len() as u32);
    }
}
