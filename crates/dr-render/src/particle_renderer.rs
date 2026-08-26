use blade_graphics as gpu;
use bytemuck::{Pod, Zeroable};
use dr_core::particles::ParticleGpu;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ParticleGlobals {
    pub screen_size: [f32; 2],
    pub bh_center_uv: [f32; 2],
    pub time: f32,
    pub _pad0: f32,
    pub _pad1: f32,
    pub _pad2: f32,
}

#[derive(blade_macros::ShaderData)]
struct ParticleShaderData {
    globals: ParticleGlobals,
}

pub struct ParticleRenderer {
    pipeline: gpu::RenderPipeline,
    instance_buffer: gpu::Buffer,
    capacity: usize,
}

impl ParticleRenderer {
    pub fn new(
        context: &gpu::Context,
        surface_format: gpu::TextureFormat,
        capacity: usize,
    ) -> Self {
        let source = include_str!("shaders/particles.wgsl");
        let shader = context.create_shader(gpu::ShaderDesc {
            source,
            naga_module: None,
        });

        let data_layout = <ParticleShaderData as gpu::ShaderData>::layout();

        let pipeline = context.create_render_pipeline(gpu::RenderPipelineDesc {
            name: "particle_pipeline",
            data_layouts: &[&data_layout],
            vertex: shader.at("vs_main"),
            vertex_fetches: &[gpu::VertexFetchState {
                layout: &gpu::VertexLayout {
                    stride: std::mem::size_of::<ParticleGpu>() as u32,
                    attributes: vec![
                        ("pos", gpu::VertexAttribute {
                            offset: 0,
                            format: gpu::VertexFormat::F32Vec2,
                        }),
                        ("vel", gpu::VertexAttribute {
                            offset: 8,
                            format: gpu::VertexFormat::F32Vec2,
                        }),
                        ("color", gpu::VertexAttribute {
                            offset: 16,
                            format: gpu::VertexFormat::F32Vec4,
                        }),
                        ("size", gpu::VertexAttribute {
                            offset: 32,
                            format: gpu::VertexFormat::F32,
                        }),
                        ("spaghettification", gpu::VertexAttribute {
                            offset: 36,
                            format: gpu::VertexFormat::F32,
                        }),
                        ("tier", gpu::VertexAttribute {
                            offset: 40,
                            format: gpu::VertexFormat::U32,
                        }),
                        ("_pad", gpu::VertexAttribute {
                            offset: 44,
                            format: gpu::VertexFormat::F32,
                        }),
                    ],
                },
                instanced: true,
            }],
            primitive: gpu::PrimitiveState {
                topology: gpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            fragment: Some(shader.at("fs_main")),
            color_targets: &[gpu::ColorTargetState {
                format: surface_format,
                blend: Some(gpu::BlendState::ADDITIVE),
                write_mask: gpu::ColorWrites::default(),
            }],
            multisample_state: gpu::MultisampleState::default(),
        });

        let buffer_size = (capacity * std::mem::size_of::<ParticleGpu>()) as u64;
        let instance_buffer = context.create_buffer(gpu::BufferDesc {
            name: "particle_instances",
            size: buffer_size,
            memory: gpu::Memory::Shared,
        });

        Self {
            pipeline,
            instance_buffer,
            capacity,
        }
    }

    pub fn draw(
        &mut self,
        encoder: &mut gpu::CommandEncoder,
        context: &gpu::Context,
        target_view: gpu::TextureView,
        globals: ParticleGlobals,
        particles: &[ParticleGpu],
        active_count: u32,
    ) {
        let count = (active_count as usize).min(self.capacity).min(particles.len());
        if count == 0 {
            return;
        }

        let byte_size = (count * std::mem::size_of::<ParticleGpu>()) as u64;
        let slice = bytemuck::cast_slice(&particles[..count]);
        unsafe {
            let ptr = self.instance_buffer.data();
            std::ptr::copy_nonoverlapping(slice.as_ptr(), ptr, byte_size as usize);
        }
        context.sync_buffer(self.instance_buffer, 0, byte_size);

        let mut pass = encoder.render(
            "particles_pass",
            gpu::RenderTargetSet {
                colors: &[gpu::RenderTarget {
                    view: target_view,
                    init_op: gpu::InitOp::Load,
                    finish_op: gpu::FinishOp::Store,
                }],
                depth_stencil: None,
            },
        );

        let mut rc = pass.with(&self.pipeline);
        rc.bind(0, &ParticleShaderData { globals });
        rc.bind_vertex(0, self.instance_buffer.into());
        rc.draw(0, 6, 0, count as u32);
    }

    pub fn destroy(&mut self, context: &gpu::Context) {
        context.destroy_buffer(self.instance_buffer);
        context.destroy_render_pipeline(&mut self.pipeline);
    }
}
