use bevy::prelude::*;
use dark_rift_shared::protocol::*;
use crate::{ClientSettings, Cli, rand_client_id};
use crate::ui_theme::*;
use lightyear::prelude::{MessageReceiver, MessageSender};


#[derive(Resource, Default)]
pub struct JoinedLobby(pub Option<u64>);

#[derive(Component)]
pub struct LobbyBrowserRoot;

#[derive(Component)]
pub struct LobbyCardsContainer;

#[derive(Component)]
pub struct JoinLobbyButton(pub u64);

#[derive(Component)]
pub struct WaitingQueueContainer;

#[derive(Component)]
pub struct CancelJoinButton;

#[derive(Component)]
pub struct WaitingStatusText;

#[derive(Component)]
pub struct WaitingPlayersText;

pub fn setup_lobby_browser(
    mut commands: Commands,
    settings: Res<ClientSettings>,
) {
    let font = settings.font.clone();
    commands.insert_resource(JoinedLobby(None));
    
    commands.spawn((
        LobbyBrowserRoot,
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            position_type: PositionType::Absolute,
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        ZIndex(300),
        BackgroundColor(Color::srgba(0.01, 0.02, 0.05, 0.95)),
    )).with_children(|p| {
        p.spawn((
            Text::new("THE DARK RIFT"),
            TextFont { font: font.clone(), font_size: 48.0, ..default() },
            TextColor(text_primary()),
            Node {
                margin: UiRect::bottom(Val::Px(40.0)),
                ..default()
            }
        ));

        // Container for Cards
        p.spawn((
            Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                column_gap: Val::Px(24.0),
                row_gap: Val::Px(24.0),
                max_width: Val::Px(1000.0),
                ..default()
            },
            LobbyCardsContainer,
        ));
        
        // Container for Waiting Queue (starts hidden)
        p.spawn((
            Node {
                display: Display::None,
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(20.0),
                ..default()
            },
            WaitingQueueContainer,
        )).with_children(|wait| {
            wait.spawn((
                Text::new("WAITING FOR PLAYERS..."),
                TextFont { font: font.clone(), font_size: 24.0, ..default() },
                TextColor(text_secondary()),
                WaitingStatusText,
            ));
            wait.spawn((
                Text::new(""),
                TextFont { font: font.clone(), font_size: 18.0, ..default() },
                TextColor(Color::srgb(0.7, 0.7, 0.8)),
                WaitingPlayersText,
            ));
            wait.spawn((
                Button,
                CancelJoinButton,
                Node {
                    padding: UiRect::axes(Val::Px(30.0), Val::Px(15.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(accent_danger()),
            )).with_children(|btn| {
                btn.spawn((
                    Text::new("CANCEL"),
                    TextFont { font: font.clone(), font_size: 18.0, ..default() },
                    TextColor(Color::WHITE),
                ));
            });
        });
    });
}

pub fn system_update_lobby_browser(
    mut commands: Commands,
    mut receivers: Query<&mut MessageReceiver<ServerLobbiesBroadcastMessage>>,
    mut container_q: Query<(Entity, &mut Node), (With<LobbyCardsContainer>, Without<WaitingQueueContainer>)>,
    mut queue_q: Query<(&mut Node, &Children), (With<WaitingQueueContainer>, Without<LobbyCardsContainer>)>,
    mut status_text_q: Query<&mut Text, (With<WaitingStatusText>, Without<WaitingPlayersText>)>,
    mut players_text_q: Query<&mut Text, (With<WaitingPlayersText>, Without<WaitingStatusText>)>,
    children_q: Query<&Children>,
    settings: Res<ClientSettings>,
    joined_lobby: Option<Res<JoinedLobby>>,
) {
    let Some((cards_ent, mut cards_node)) = container_q.iter_mut().next() else { return; };
    let Some((mut queue_node, _)) = queue_q.iter_mut().next() else { return; };
    let target_id = joined_lobby.map(|r| r.0).flatten();

    for mut receiver in receivers.iter_mut() {
        for msg in receiver.receive() {
            if let Some(target) = target_id {
                // We are waiting in a lobby
                cards_node.display = Display::None;
                queue_node.display = Display::Flex;
                
                if let Some(lobby) = msg.lobbies.iter().find(|l| l.id == target) {
                    if let Some(mut status) = status_text_q.iter_mut().next() {
                        if lobby.is_counting_down {
                            **status = format!("STARTING IN {:.0}s", lobby.timer_secs);
                        } else {
                            **status = format!("WAITING FOR PLAYERS... {}/{}", lobby.num_players, lobby.max_players);
                        }
                    }
                    if let Some(mut players_text) = players_text_q.iter_mut().next() {
                        if !lobby.player_names.is_empty() {
                            **players_text = format!("CONNECTED PLAYERS\n\n{}", lobby.player_names.join("\n"));
                        } else {
                            **players_text = "ERROR: You are not in the lobby".to_string();
                        }
                    }
                }
            } else {
                // We are in browser mode
                cards_node.display = Display::Flex;
                queue_node.display = Display::None;
                
                if let Ok(children) = children_q.get(cards_ent) {
                    for child in children.iter() {
                        commands.entity(child).despawn();
                    }
                }
                
                for lobby in msg.lobbies {
                    let font = settings.font.clone();
                    let status_text = if lobby.is_counting_down {
                        format!("STARTING IN {:.0}s", lobby.timer_secs)
                    } else {
                        format!("WAITING FOR PLAYERS")
                    };
                    
                    let is_counting = lobby.is_counting_down;
                    let bg_color = if is_counting { accent_primary() } else { panel_bg() };

                    commands.entity(cards_ent).with_children(|p| {
                        p.spawn((
                            Button,
                            JoinLobbyButton(lobby.id),
                            Node {
                                width: Val::Px(300.0),
                                height: Val::Px(180.0),
                                flex_direction: FlexDirection::Column,
                                justify_content: JustifyContent::SpaceBetween,
                                padding: UiRect::all(Val::Px(20.0)),
                                border_radius: BorderRadius::all(Val::Px(16.0)),
                                border: UiRect::all(Val::Px(2.0)),
                                ..default()
                            },
                            BackgroundColor(bg_color),
                            BorderColor::all(panel_border()),
                        )).with_children(|card| {
                            card.spawn((
                                Text::new(format!("LOBBY {}", lobby.id)),
                                TextFont { font: font.clone(), font_size: 28.0, ..default() },
                                TextColor(Color::WHITE),
                            ));
                            card.spawn((
                                Text::new(format!("{}/{} Players Connected", lobby.num_players, lobby.max_players)),
                                TextFont { font: font.clone(), font_size: 16.0, ..default() },
                                TextColor(text_primary()),
                            ));
                            card.spawn((
                                Node {
                                    padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                                    border_radius: BorderRadius::all(Val::Px(8.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
                            )).with_children(|badge| {
                                badge.spawn((
                                    Text::new(status_text),
                                    TextFont { font: font.clone(), font_size: 14.0, ..default() },
                                    TextColor(if is_counting { Color::WHITE } else { text_secondary() }),
                                ));
                            });
                        });
                    });
                }
            }
        }
    }
}

pub fn system_handle_join_clicks(
    mut interaction_q: Query<(&Interaction, &mut BackgroundColor, &JoinLobbyButton), (Changed<Interaction>, With<Button>)>,
    mut senders: Query<&mut MessageSender<ClientJoinMessage>>,
    _cli: Res<Cli>,
    mut joined_lobby: ResMut<JoinedLobby>,
) {
    for (interaction, mut bg, btn) in interaction_q.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                for mut sender in senders.iter_mut() {
                    let msg = ClientJoinMessage { 
                        name: format!("Rig-{}", rand_client_id() % 1000), 
                        is_observer: _cli.observer, 
                        target_lobby_id: Some(btn.0),
                    };
                    let _ = sender.send::<ReliableChannel>(msg);
                    info!("✅ Requested to join Lobby ID {}", btn.0);
                }
                *bg = BackgroundColor(accent_home_pressed());
                joined_lobby.0 = Some(btn.0);
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.2, 0.4, 0.8, 1.0));
            }
            Interaction::None => {}
        }
    }
}

pub fn system_handle_cancel_join(
    mut interaction_q: Query<&Interaction, (Changed<Interaction>, With<CancelJoinButton>)>,
    mut senders: Query<&mut MessageSender<ClientLeaveMessage>>,
    mut joined_lobby: ResMut<JoinedLobby>,
) {
    for interaction in interaction_q.iter_mut() {
        if *interaction == Interaction::Pressed {
            joined_lobby.0 = None;
            for mut sender in senders.iter_mut() {
                let _ = sender.send::<dark_rift_shared::protocol::ReliableChannel>(ClientLeaveMessage);
                info!("🚫 Cancelled join. Sending ClientLeaveMessage.");
            }
        }
    }
}

pub fn system_cleanup_lobby_browser(
    mut commands: Commands,
    q_root: Query<Entity, With<LobbyBrowserRoot>>,
) {
    if let Some(ent) = q_root.iter().next() {
        commands.entity(ent).despawn();
    }
}
