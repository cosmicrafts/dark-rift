use bevy::prelude::*;
use bevy::app::ScheduleRunnerPlugin;
use lightyear::prelude::*;
use lightyear::prelude::server::*;
use lightyear::netcode::NetcodeServer;
use lightyear::websocket::server::{WebSocketServerIo, ServerConfig as WebSocketConfig, Identity};
use std::time::Duration;
use std::net::{Ipv4Addr, SocketAddr};

use dark_rift_shared::game::GameState;
use dark_rift_shared::protocol::*;

fn main() {
    let mut app = App::new();
    
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 60.0))));
    app.add_plugins(bevy::log::LogPlugin::default());
    app.add_plugins(SharedProtocolPlugin);
    
    // Lightyear 0.26 Server setup
    app.add_plugins(ServerPlugins::default());

    app.add_systems(Startup, setup);
    app.add_systems(Update, broadcast_state);
    
    app.run();
}

fn setup(mut commands: Commands) {
    let game = GameState::new(42);
    info!("Server starting with StarMap containing {} systems.", game.map.system_count());
    commands.insert_resource(game);

    let server_addr = SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 5000);
    
    let sans = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];
    let ws_config = WebSocketConfig::builder()
        .with_bind_address(server_addr)
        .with_identity(
            Identity::self_signed(sans).unwrap(),
        );

    let server_entity = commands.spawn((
        Name::from("Server"),
        NetcodeServer::new(NetcodeConfig::default()),
        LocalAddr(server_addr),
        WebSocketServerIo { config: ws_config },
    )).id();
    
    commands.trigger(Start { entity: server_entity });
}

// ─── Networking Systems ──────────────────────────────────────────────────────

fn broadcast_state(
    game: Res<GameState>,
    mut clients: Query<&mut MessageSender<SyncMapState>>,
) {
    if !game.is_changed() { return; }
    
    let msg = SyncMapState { state: Some(game.clone()) };
    for mut sender in clients.iter_mut() {
        let _ = sender.send(msg.clone());
    }
}
