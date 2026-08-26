use dr_core::game::GameState;
use dr_render::black_hole_renderer::BlackHoleUniforms;
use dr_render::renderer::Renderer;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use crate::ui::GameUi;
use blade_graphics as gpu;

pub struct DarkRiftApp {
    window: Option<Arc<Window>>,
    context: Option<Arc<gpu::Context>>,
    surface: Option<gpu::Surface>,
    surface_format: gpu::TextureFormat,
    renderer: Option<Renderer>,
    gui_painter: Option<blade_egui::GuiPainter>,
    egui_winit: Option<egui_winit::State>,
    game_state: GameState,
    game_ui: GameUi,
    last_time: Instant,
    sim_time: f32,
    command_encoder: Option<gpu::CommandEncoder>,
    prev_sync_point: Option<gpu::SyncPoint>,
}

impl Default for DarkRiftApp {
    fn default() -> Self {
        Self::new()
    }
}

impl DarkRiftApp {
    pub fn new() -> Self {
        Self {
            window: None,
            context: None,
            surface: None,
            surface_format: gpu::TextureFormat::Bgra8UnormSrgb,
            renderer: None,
            gui_painter: None,
            egui_winit: None,
            game_state: GameState::new(),
            game_ui: GameUi::default(),
            last_time: Instant::now(),
            sim_time: 0.0,
            command_encoder: None,
            prev_sync_point: None,
        }
    }
}

impl ApplicationHandler for DarkRiftApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window_attributes = Window::default_attributes()
            .with_title("Dark Rift — Zero-Overhead Singularity Idle")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));

        let window = Arc::new(event_loop.create_window(window_attributes).unwrap());
        let window_size = window.inner_size();

        let context = unsafe {
            Arc::new(
                gpu::Context::init(gpu::ContextDesc {
                    presentation: true,
                    validation: cfg!(debug_assertions),
                    overlay: false,
                    ..Default::default()
                })
                .expect("Failed to initialize Blade context"),
            )
        };

        let surface = context
            .create_surface_configured(
                &*window,
                gpu::SurfaceConfig {
                    size: gpu::Extent {
                        width: window_size.width.max(1),
                        height: window_size.height.max(1),
                        depth: 1,
                    },
                    usage: gpu::TextureUsage::TARGET,
                    display_sync: gpu::DisplaySync::Recent,
                    color_space: gpu::ColorSpace::Linear,
                    ..Default::default()
                },
            )
            .expect("Failed to create Blade surface");

        let surface_format = surface.info().format;
        let renderer = Renderer::new(&context, surface_format);
        let gui_painter = blade_egui::GuiPainter::new(surface.info(), &context);

        let egui_ctx = egui::Context::default();
        let egui_winit = egui_winit::State::new(
            egui_ctx,
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(2048),
        );

        let command_encoder = context.create_command_encoder(gpu::CommandEncoderDesc {
            name: "main_command_encoder",
            buffer_count: 2,
            manual_barriers: false,
        });

        self.window = Some(window);
        self.context = Some(context);
        self.surface = Some(surface);
        self.surface_format = surface_format;
        self.renderer = Some(renderer);
        self.gui_painter = Some(gui_painter);
        self.egui_winit = Some(egui_winit);
        self.command_encoder = Some(command_encoder);
        self.last_time = Instant::now();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.as_ref() else { return; };
        let Some(egui_winit) = self.egui_winit.as_mut() else { return; };

        let response = egui_winit.on_window_event(window, &event);
        if response.consumed {
            window.request_redraw();
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                if let Some(context) = self.context.as_ref() {
                    if let Some(sp) = self.prev_sync_point.take() {
                        let _ = context.wait_for(&sp, !0);
                    }
                    if let Some(mut renderer) = self.renderer.take() {
                        renderer.destroy(context);
                    }
                    if let Some(mut gui_painter) = self.gui_painter.take() {
                        gui_painter.destroy(context);
                    }
                    if let Some(mut surface) = self.surface.take() {
                        context.destroy_surface(&mut surface);
                    }
                }
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(surface) = self.surface.as_mut() {
                    if let Some(context) = self.context.as_ref() {
                        context.reconfigure_surface(
                            surface,
                            gpu::SurfaceConfig {
                                size: gpu::Extent {
                                    width: size.width.max(1),
                                    height: size.height.max(1),
                                    depth: 1,
                                },
                                usage: gpu::TextureUsage::TARGET,
                                display_sync: gpu::DisplaySync::Recent,
                                color_space: gpu::ColorSpace::Linear,
                                ..Default::default()
                            },
                        );
                    }
                }
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - self.last_time).as_secs_f32().min(0.1);
                self.last_time = now;
                self.sim_time += dt;

                // 1. Tick game simulation
                self.game_state.tick(dt);

                let Some(surface) = self.surface.as_mut() else { return; };
                let Some(context) = self.context.as_ref() else { return; };
                let Some(renderer) = self.renderer.as_mut() else { return; };
                let Some(gui_painter) = self.gui_painter.as_mut() else { return; };
                let Some(command_encoder) = self.command_encoder.as_mut() else { return; };

                if let Some(sp) = self.prev_sync_point.take() {
                    let _ = context.wait_for(&sp, !0);
                }

                let frame = surface.acquire_frame();
                command_encoder.start();
                command_encoder.init_texture(frame.texture());

                let window_size = window.inner_size();
                let width = window_size.width as f32;
                let height = window_size.height as f32;

                // 2. Run Egui UI pass
                let raw_input = egui_winit.take_egui_input(window);
                let egui_output = egui_winit.egui_ctx().run_ui(raw_input, |egui_ctx| {
                    self.game_ui.draw(egui_ctx, &mut self.game_state);
                });

                egui_winit.handle_platform_output(window, egui_output.platform_output);
                let pixels_per_point = egui_winit::pixels_per_point(egui_winit.egui_ctx(), window);
                let primitives = egui_winit
                    .egui_ctx()
                    .tessellate(egui_output.shapes, pixels_per_point);

                gui_painter.update_textures(command_encoder, &egui_output.textures_delta, context);

                // Compute exact UV center of play area left of 380px sidebar
                let viewport_rect = egui_winit.egui_ctx().viewport_rect();
                let logical_center_x = viewport_rect.min.x + (viewport_rect.width() - 380.0) * 0.5;
                let logical_center_y = viewport_rect.center().y;
                let bh_center_uv = [
                    (logical_center_x / viewport_rect.width().max(1.0)).clamp(0.05, 0.95),
                    (logical_center_y / viewport_rect.height().max(1.0)).clamp(0.05, 0.95),
                ];

                // 3. Render Black Hole & Particles
                let jet_intensity = if self.game_state.polar_jet_active { 1.0 } else { 0.0 };
                let penrose_active = if self.game_state.black_hole.penrose_timer > 0.0 { 1.0 } else { 0.0 };

                let bh_uniforms = BlackHoleUniforms {
                    screen_size: [width, height],
                    bh_center_uv,
                    time: self.sim_time,
                    rs: self.game_state.black_hole.visual_radius,
                    spin: self.game_state.black_hole.spin,
                    accretion_flash: self.game_state.black_hole.accretion_flash,
                    penrose_active,
                    jet_intensity,
                    disk_angle: self.game_state.black_hole.disk_angle,
                    _pad0: 0.0,
                };

                let active_particle_count = self.game_state.particles.active_count as u32;
                renderer.render(
                    command_encoder,
                    context,
                    frame.texture_view(),
                    bh_uniforms,
                    &self.game_state.particles.particles,
                    active_particle_count,
                );

                // 4. Render UI Layer on top of frame
                let screen_desc = blade_egui::ScreenDescriptor {
                    physical_size: (window_size.width, window_size.height),
                    scale_factor: pixels_per_point,
                };

                let mut ui_pass = command_encoder.render(
                    "ui_pass",
                    gpu::RenderTargetSet {
                        colors: &[gpu::RenderTarget {
                            view: frame.texture_view(),
                            init_op: gpu::InitOp::Load,
                            finish_op: gpu::FinishOp::Store,
                        }],
                        depth_stencil: None,
                    },
                );

                gui_painter.paint(&mut ui_pass, &primitives, &screen_desc, context);
                drop(ui_pass);

                gui_painter.sync(context);
                command_encoder.present(frame);
                let sync_point = context.submit(command_encoder);
                gui_painter.after_submit(&sync_point);
                self.prev_sync_point = Some(sync_point);

                window.request_redraw();
            }
            _ => {}
        }
    }
}

pub type App = DarkRiftApp;
