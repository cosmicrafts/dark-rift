use bevy::prelude::*;
use dark_rift_shared::protocol::*;
use crate::{ClientSettings, Cli, rand_client_id};
use crate::ui_theme::*;
use lightyear::prelude::{MessageReceiver, MessageSender};

#[derive(Component)]
pub struct LobbyBrowserRoot;

#[derive(Component)]
pub struct LobbyCardsContainer;

#[derive(Component)]
pub struct JoinLobbyButton(pub u64);

pub fn setup_lobby_browser(
    mut commands: Commands,
    settings: Res<ClientSettings>,
) {
    let font = settings.font.clone();
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
            Text::new("OPENFRONT LOBBIES"),
            TextFont { font: font.clone(), font_size: 48.0, ..default() },
            TextColor(text_primary()),
            Node {
                margin: UiRect::bottom(Val::Px(40.0)),
                ..default()
            }
        ));

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
    });
}

pub fn system_update_lobby_browser(
    mut commands: Commands,
    mut receivers: Query<&mut MessageReceiver<ServerLobbiesBroadcastMessage>>,
    container_q: Query<Entity, With<LobbyCardsContainer>>,
    children_q: Query<&Children>,
    settings: Res<ClientSettings>,
) {
    let Some(container_ent) = container_q.iter().next() else { return; };

    for mut receiver in receivers.iter_mut() {
        for msg in receiver.receive() {
            if let Ok(children) = children_q.get(container_ent) {
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

                commands.entity(container_ent).with_children(|p| {
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
                        // Title
                        card.spawn((
                            Text::new(format!("LOBBY {}", lobby.id)),
                            TextFont { font: font.clone(), font_size: 28.0, ..default() },
                            TextColor(Color::WHITE),
                        ));
                        // Players
                        card.spawn((
                            Text::new(format!("{}/{} Players Connected", lobby.num_players, lobby.max_players)),
                            TextFont { font: font.clone(), font_size: 16.0, ..default() },
                            TextColor(text_primary()),
                        ));
                        // Timer badge
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

pub fn system_handle_join_clicks(
    mut interaction_q: Query<(&Interaction, &mut BackgroundColor, &JoinLobbyButton), (Changed<Interaction>, With<Button>)>,
    mut senders: Query<&mut MessageSender<ClientJoinMessage>>,
    _cli: Res<Cli>,
    mut commands: Commands,
    root_q: Query<Entity, With<LobbyBrowserRoot>>,
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
                // Hide Lobby Browser immediately upon click to prevent double inputs
                if let Some(root) = root_q.iter().next() {
                    commands.entity(root).despawn();
                }
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.2, 0.4, 0.8, 1.0));
            }
            Interaction::None => {
                // Restore logic handled by rebuilds mostly, 
                // but if we don't rebuild we should restore to panel_bg or accent_primary.
                // Rebuilds happen 2 times a second anyway.
            }
        }
    }
}
