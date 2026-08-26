use crate::black_hole_renderer::{BlackHoleRenderer, BlackHoleUniforms};
use crate::particle_renderer::{ParticleGlobals, ParticleRenderer};
use blade_graphics as gpu;
use dr_core::particles::{ParticleGpu, MAX_PARTICLES};

pub struct Renderer {
    black_hole: BlackHoleRenderer,
    particles: ParticleRenderer,
}

impl Renderer {
    pub fn new(context: &gpu::Context, surface_format: gpu::TextureFormat) -> Self {
        let black_hole = BlackHoleRenderer::new(context, surface_format);
        let particles = ParticleRenderer::new(context, surface_format, MAX_PARTICLES);

        Self {
            black_hole,
            particles,
        }
    }

    pub fn render(
        &mut self,
        encoder: &mut gpu::CommandEncoder,
        context: &gpu::Context,
        target_view: gpu::TextureView,
        bh_uniforms: BlackHoleUniforms,
        particle_data: &[ParticleGpu],
        active_particle_count: u32,
    ) {
        // Pass 1: Fullscreen Black Hole + Gravitational Lensing + Accretion Disk
        self.black_hole.draw(encoder, target_view, bh_uniforms);

        // Pass 2: Instanced Particles orbiting the black hole
        let particle_globals = ParticleGlobals {
            screen_size: bh_uniforms.screen_size,
            bh_center_uv: bh_uniforms.bh_center_uv,
            time: bh_uniforms.time,
            _pad0: 0.0,
            _pad1: 0.0,
            _pad2: 0.0,
        };
        self.particles.draw(
            encoder,
            context,
            target_view,
            particle_globals,
            particle_data,
            active_particle_count,
        );
    }

    pub fn destroy(&mut self, context: &gpu::Context) {
        self.particles.destroy(context);
        self.black_hole.destroy(context);
    }
}
