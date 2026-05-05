use bevy::prelude::*;
use dark_rift_shared::game::GameState;
use dark_rift_shared::protocol::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use instant::Instant;
use crate::*;

#[cfg(not(target_arch = "wasm32"))]
pub const NATIVE_NETCODE_SOCKET: &str = "74.208.246.177:25565";

#[cfg(target_arch = "wasm32")]
pub const WASM_MULTIPLAYER_WSS_URL: &str = "wss://darkrift.ai/ws/";

#[cfg(target_arch = "wasm32")]
pub const WASM_NETCODE_SOCKET: &str = "74.208.246.177:25565";

pub fn multiplayer_netcode_socket() -> SocketAddr {
    #[cfg(target_arch = "wasm32")]
    {
        WASM_NETCODE_SOCKET
            .parse()
            .expect("WASM_NETCODE_SOCKET must be a numeric IPv4:port (DNS not available on wasm32-unknown-unknown)")
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        NATIVE_NETCODE_SOCKET
            .parse()
            .expect("NATIVE_NETCODE_SOCKET must be a valid SocketAddr string")
    }
}

pub fn setup_network(mut commands: Commands, cli: Res<Cli>) {
    let server_addr: SocketAddr = multiplayer_netcode_socket();
    let client_id = rand_client_id();

    let mut netcode_cfg = NetcodeConfig::default();
    netcode_cfg.client_timeout_secs = 120;

    let netcode = NetcodeClient::new(
        Authentication::Manual {
            server_addr,
            client_id,
            private_key: Key::default(),
            protocol_id: 0,
        },
        netcode_cfg,
    ).expect("Failed to build NetcodeClient");

    #[cfg(target_arch = "wasm32")]
    let ws_io = WebSocketClientIo::from_url(
        aeronet_websocket::client::ClientConfig::default(),
        WASM_MULTIPLAYER_WSS_URL,
    );
    #[cfg(not(target_arch = "wasm32"))]
    let ws_io = WebSocketClientIo::from_addr(
        aeronet_websocket::client::ClientConfig::default(),
        WebSocketScheme::Plain,
    );

    let client_entity = commands.spawn((
        Name::from("DarkRiftClient"),
        netcode,
        PeerAddr(server_addr),
        ws_io,
    )).id();

    commands.insert_resource(NetTiming {
        connect_started: Some(Instant::now()),
        ..default()
    });

    // Trigger Netcode connection (which internally initiates LinkStart)
    commands.trigger(lightyear::prelude::client::Connect { entity: client_entity });

    #[cfg(target_arch = "wasm32")]
    debug!(
        "Connecting (WASM): transport {} | netcode {:?} (client_id={}, observer={})",
        WASM_MULTIPLAYER_WSS_URL,
        server_addr,
        client_id,
        cli.observer
    );
    #[cfg(not(target_arch = "wasm32"))]
    debug!(
        "Connecting ws://{}... (client_id={}, observer={})",
        NATIVE_NETCODE_SOCKET,
        client_id,
        cli.observer
    );
}

pub fn handle_connection(
    mut client_q: Query<&mut MessageSender<ClientJoinMessage>, Added<lightyear::prelude::client::Connected>>,
    cli: Res<Cli>,
) {
    for mut sender in client_q.iter_mut() {
        if cli.bot {
            debug!("Bot connected to transport; joining lobby...");
            let msg = ClientJoinMessage { 
                name: format!("HydraBot-{}", rand_client_id() % 1000), 
                is_observer: cli.observer, 
                target_lobby_id: None, // Joins any available lobby or seeds a new one
            };
            sender.send::<ReliableChannel>(msg);
        } else {
            debug!("Connected to transport; lobby browser");
        }
    }
}

pub fn rand_client_id() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let millis = js_sys::Date::now() as u64;
        millis
            .wrapping_mul(6364136223846793005)
            .wrapping_add(seq)
            .wrapping_add(js_sys::Math::random().to_bits())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        nanos as u64
    }
}

pub fn try_launch_fleet_intent(
    game: &GameState,
    wc: &WaterComponents,
    path_scratch: &mut WaterPathfinderScratch,
    pid: u16,
    target_owner: u16,
    target_tile: u32,
    troops: Option<f64>,
    log_ctx: &str,
) -> Option<GameplayIntent> {
    let Some(launcher) = game.player(pid) else {
        bevy::log::warn!("{log_ctx}: launcher player {pid} not found");
        return None;
    };
    let border = &launcher.border_tiles;
    let target_border = if target_owner != 0 {
        match game.player(target_owner) {
            Some(p) => Some(&p.border_tiles),
            None => {
                bevy::log::warn!("{log_ctx}: target_owner {target_owner} not found");
                return None;
            }
        }
    } else {
        None
    };
    match resolve_fleet_route(
        &game.map,
        wc,
        path_scratch,
        pid,
        target_owner,
        target_tile,
        border,
        target_border,
        true,
    ) {
        Ok(_) => Some(GameplayIntent::LaunchFleet {
            target_tile,
            troops,
        }),
        Err(e) => {
            bevy::log::warn!("{log_ctx}: {e}");
            None
        }
    }
}

pub fn send_gameplay_intent(
    intent: GameplayIntent,
    pid: u16,
    local_buf: Option<ResMut<LocalIntentBuffer>>,
    senders: &mut Query<&mut MessageSender<ClientGameplayMessage>>,
) {
    if let Some(mut buf) = local_buf {
        buf.0.push(StampedIntent {
            player_id: pid,
            intent,
        });
    } else {
        for mut sender in senders.iter_mut() {
            sender.send::<ReliableChannel>(ClientGameplayMessage { intent: intent.clone() });
        }
    }
}