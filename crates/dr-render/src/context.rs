use blade_graphics as gpu;

pub struct RenderContext {
    pub context: gpu::Context,
    pub command_encoder: gpu::CommandEncoder,
}

impl RenderContext {
    pub fn new() -> Result<Self, gpu::NotSupportedError> {
        let context = unsafe {
            gpu::Context::init(gpu::ContextDesc {
                presentation: true,
                validation: cfg!(debug_assertions),
                overlay: false,
                ..Default::default()
            })?
        };
        let command_encoder = context.create_command_encoder(gpu::CommandEncoderDesc {
            name: "main",
            buffer_count: 2,
            manual_barriers: false,
        });

        Ok(Self {
            context,
            command_encoder,
        })
    }

    pub fn create_surface<I: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle>(
        &self,
        window: &I,
        width: u32,
        height: u32,
    ) -> Result<gpu::Surface, gpu::NotSupportedError> {
        let config = gpu::SurfaceConfig {
            size: gpu::Extent {
                width: width.max(1),
                height: height.max(1),
                depth: 1,
            },
            usage: gpu::TextureUsage::TARGET,
            display_sync: gpu::DisplaySync::Recent,
            color_space: gpu::ColorSpace::Linear,
            ..Default::default()
        };
        self.context.create_surface_configured(window, config)
    }

    pub fn reset_command_encoder(&mut self) {
        self.context.destroy_command_encoder(&mut self.command_encoder);
        self.command_encoder = self.context.create_command_encoder(gpu::CommandEncoderDesc {
            name: "main",
            buffer_count: 2,
            manual_barriers: false,
        });
    }
}

impl Drop for RenderContext {
    fn drop(&mut self) {
        self.context.destroy_command_encoder(&mut self.command_encoder);
    }
}
