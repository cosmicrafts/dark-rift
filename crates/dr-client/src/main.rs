use dr_client::app::App;

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    env_logger::init();
    log::info!("Starting Dark Rift — Zero-Overhead Singularity Engine");

    let event_loop = winit::event_loop::EventLoop::new().unwrap();
    let mut app = App::new();
    let _ = event_loop.run_app(&mut app);
}

#[cfg(target_arch = "wasm32")]
fn main() {}
