use blade_graphics as gpu;
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct BlackHoleUniforms {
    pub screen_size: [f32; 2],
    pub bh_center_uv: [f32; 2],
    pub time: f32,
    pub rs: f32,
    pub spin: f32,
    pub accretion_flash: f32,
    pub penrose_active: f32,
    pub jet_intensity: f32,
    pub disk_angle: f32,
    pub _pad0: f32,
}

#[derive(blade_macros::ShaderData)]
struct BlackHoleData {
    uniforms: BlackHoleUniforms,
}

pub struct BlackHoleRenderer {
    pipeline: gpu::RenderPipeline,
}

impl BlackHoleRenderer {
    pub fn new(context: &gpu::Context, surface_format: gpu::TextureFormat) -> Self {
        let source = include_str!("shaders/black_hole.wgsl");
        let shader = context.create_shader(gpu::ShaderDesc {
            source,
            naga_module: None,
        });

        let data_layout = <BlackHoleData as gpu::ShaderData>::layout();

        let pipeline = context.create_render_pipeline(gpu::RenderPipelineDesc {
            name: "black_hole_pipeline",
            data_layouts: &[&data_layout],
            vertex: shader.at("vs_main"),
            vertex_fetches: &[],
            primitive: gpu::PrimitiveState {
                topology: gpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            fragment: Some(shader.at("fs_main")),
            color_targets: &[gpu::ColorTargetState {
                format: surface_format,
                blend: Some(gpu::BlendState::ALPHA_BLENDING),
                write_mask: gpu::ColorWrites::default(),
            }],
            multisample_state: gpu::MultisampleState::default(),
        });

        Self { pipeline }
    }

    pub fn draw(
        &self,
        encoder: &mut gpu::CommandEncoder,
        target_view: gpu::TextureView,
        uniforms: BlackHoleUniforms,
    ) {
        let mut pass = encoder.render(
            "black_hole_pass",
            gpu::RenderTargetSet {
                colors: &[gpu::RenderTarget {
                    view: target_view,
                    init_op: gpu::InitOp::Clear(gpu::TextureColor::OpaqueBlack),
                    finish_op: gpu::FinishOp::Store,
                }],
                depth_stencil: None,
            },
        );

        let mut rc = pass.with(&self.pipeline);
        rc.bind(0, &BlackHoleData { uniforms });
        rc.draw(0, 3, 0, 1);
    }

    pub fn destroy(&mut self, context: &gpu::Context) {
        context.destroy_render_pipeline(&mut self.pipeline);
    }
}
