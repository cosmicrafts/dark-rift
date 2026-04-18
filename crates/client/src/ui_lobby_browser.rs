use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use dark_rift_shared::player::PlayerType;
use dark_rift_shared::protocol::*;
use crate::settings::{save_client_settings_config, ProfileConfig};
use crate::ui_layers::LOBBY_BROWSER;
use crate::{ClientSettings, Cli};
use crate::ui_theme::*;
use lightyear::prelude::{MessageReceiver, MessageSender};

const GAME_VERSION_LABEL: &str = env!("CARGO_PKG_VERSION");

#[derive(Resource, Default)]
pub struct JoinedLobby(pub Option<u64>);

/// True while the nickname text field is capturing keyboard input.
#[derive(Resource, Default)]
pub struct LobbyNicknameFocused(pub bool);

/// Debounced disk save after nickname edits (native only inside `save_client_settings_config`).
#[derive(Resource)]
pub struct NicknameSettingsAutosave {
    pub timer: Timer,
    pub dirty: bool,
}

impl Default for NicknameSettingsAutosave {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(0.65, TimerMode::Once),
            dirty: false,
        }
    }
}

#[derive(Component)]
pub struct LobbyBrowserRoot;

#[derive(Component)]
pub struct LobbyBrowserMainChrome;

#[derive(Component)]
pub struct LobbyCardsContainer;

#[derive(Component)]
pub struct LobbyCardRoot;

#[derive(Component)]
pub struct LobbyCardTitle;

#[derive(Component)]
pub struct LobbyCardPlayers;

#[derive(Component)]
pub struct LobbyCardStatus;

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

#[derive(Component)]
pub struct SinglePlayerButton;

#[derive(Component)]
pub struct NicknameFieldRoot;

#[derive(Component)]
pub struct NicknameFieldText;

#[derive(Component)]
pub struct ComingSoonMenuButton;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum ComingSoonKind {
    Ranked,
    Secondary,
}

#[inline]
fn is_printable_char(chr: char) -> bool {
    let is_in_private_use_area = ('\u{e000}'..='\u{f8ff}').contains(&chr)
        || ('\u{f0000}'..='\u{ffffd}').contains(&chr)
        || ('\u{100000}'..='\u{10fffd}').contains(&chr);
    !is_in_private_use_area && !chr.is_ascii_control()
}

pub fn setup_lobby_browser(mut commands: Commands, settings: Res<ClientSettings>) {
    let font = settings.font.clone();
    let nick_display = settings.config.profile.nickname.clone();

    commands.insert_resource(JoinedLobby(None));
    commands.insert_resource(LobbyNicknameFocused(false));
    commands.insert_resource(NicknameSettingsAutosave::default());

    commands
        .spawn((
            LobbyBrowserRoot,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                position_type: PositionType::Absolute,
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                padding: UiRect::all(Val::Percent(1.5)),
                ..default()
            },
            GlobalZIndex(LOBBY_BROWSER),
            BackgroundColor(menu_backdrop()),
        ))
        .with_children(|root| {
            root.spawn((
                LobbyBrowserMainChrome,
                Node {
                    width: Val::Percent(94.0),
                    height: Val::Percent(90.0),
                    max_width: Val::Px(1280.0),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(20.0)),
                    row_gap: Val::Px(16.0),
                    border_radius: BorderRadius::all(Val::Px(18.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(menu_panel_bg()),
                BorderColor::all(menu_panel_border_glow()),
            ))
            .with_children(|panel| {
                // ── Header: title (center) + profile (right) ─────────────────
                panel
                    .spawn(Node {
                        width: Val::Percent(100.0),
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(12.0),
                        ..default()
                    })
                    .with_children(|header| {
                        header.spawn(Node {
                            flex_grow: 1.0,
                            min_width: Val::Px(0.0),
                            ..default()
                        });

                        header
                            .spawn((
                                Node {
                                    flex_direction: FlexDirection::Column,
                                    align_items: AlignItems::Center,
                                    row_gap: Val::Px(4.0),
                                    ..default()
                                },
                            ))
                            .with_children(|title_col| {
                                title_col.spawn((
                                    Text::new("DARK RIFT"),
                                    TextFont {
                                        font: font.clone(),
                                        font_size: 42.0,
                                        ..default()
                                    },
                                    TextColor(accent_solo_cyan()),
                                ));
                                title_col.spawn((
                                    Text::new(format!("v{GAME_VERSION_LABEL}")),
                                    TextFont {
                                        font: font.clone(),
                                        font_size: 16.0,
                                        ..default()
                                    },
                                    TextColor(text_secondary()),
                                ));
                            });

                        header
                            .spawn(Node {
                                flex_grow: 1.0,
                                min_width: Val::Px(0.0),
                                flex_direction: FlexDirection::Row,
                                justify_content: JustifyContent::FlexEnd,
                                align_items: AlignItems::Center,
                                ..default()
                            })
                            .with_children(|profile| {
                                profile
                                    .spawn((
                                        Node {
                                            flex_direction: FlexDirection::Row,
                                            align_items: AlignItems::Center,
                                            column_gap: Val::Px(12.0),
                                            padding: UiRect::axes(Val::Px(16.0), Val::Px(10.0)),
                                            border_radius: BorderRadius::all(Val::Px(999.0)),
                                            border: UiRect::all(Val::Px(1.0)),
                                            ..default()
                                        },
                                        BackgroundColor(Color::srgba(0.08, 0.1, 0.14, 0.9)),
                                        BorderColor::all(panel_border()),
                                    ))
                                    .with_children(|capsule| {
                                        capsule.spawn((
                                            Node {
                                                width: Val::Px(44.0),
                                                height: Val::Px(44.0),
                                                border_radius: BorderRadius::all(Val::Px(999.0)),
                                                align_items: AlignItems::Center,
                                                justify_content: JustifyContent::Center,
                                                ..default()
                                            },
                                            BackgroundColor(Color::srgba(0.2, 0.22, 0.28, 1.0)),
                                        ))
                                        .with_children(|av| {
                                            av.spawn((
                                                Text::new("?"),
                                                TextFont {
                                                    font: font.clone(),
                                                    font_size: 22.0,
                                                    ..default()
                                                },
                                                TextColor(text_secondary()),
                                            ));
                                        });

                                        capsule
                                            .spawn((
                                                Node {
                                                    flex_direction: FlexDirection::Column,
                                                    row_gap: Val::Px(6.0),
                                                    min_width: Val::Px(160.0),
                                                    ..default()
                                                },
                                            ))
                                            .with_children(|nick_col| {
                                                nick_col.spawn((
                                                    Text::new("NICKNAME"),
                                                    TextFont {
                                                        font: font.clone(),
                                                        font_size: 11.0,
                                                        ..default()
                                                    },
                                                    TextColor(text_secondary()),
                                                ));
                                                nick_col
                                                    .spawn((
                                                        Button,
                                                        NicknameFieldRoot,
                                                        Node {
                                                            width: Val::Percent(100.0),
                                                            min_height: Val::Px(36.0),
                                                            padding: UiRect::axes(
                                                                Val::Px(10.0),
                                                                Val::Px(6.0),
                                                            ),
                                                            border_radius: BorderRadius::all(
                                                                Val::Px(8.0),
                                                            ),
                                                            border: UiRect::all(Val::Px(1.0)),
                                                            justify_content: JustifyContent::Center,
                                                            ..default()
                                                        },
                                                        BackgroundColor(nickname_field_bg()),
                                                        BorderColor::all(nickname_field_border()),
                                                    ))
                                                    .with_children(|field| {
                                                        field.spawn((
                                                            Text::new(nick_display.clone()),
                                                            NicknameFieldText,
                                                            TextFont {
                                                                font: font.clone(),
                                                                font_size: 18.0,
                                                                ..default()
                                                            },
                                                            TextColor(text_primary()),
                                                        ));
                                                    });
                                            });
                                    });
                            });
                    });

                // ── Body: lobby list (left) + mode buttons (right) ───────────
                panel
                    .spawn(Node {
                        width: Val::Percent(100.0),
                        flex_grow: 1.0,
                        min_height: Val::Px(200.0),
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(20.0),
                        align_items: AlignItems::Stretch,
                        ..default()
                    })
                    .with_children(|body| {
                        body.spawn((
                            Node {
                                width: Val::Percent(40.0),
                                min_width: Val::Px(260.0),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(12.0),
                                ..default()
                            },
                        ))
                        .with_children(|left| {
                            left.spawn((
                                Node {
                                    width: Val::Percent(100.0),
                                    padding: UiRect::axes(Val::Px(12.0), Val::Px(10.0)),
                                    border_radius: BorderRadius::all(Val::Px(10.0)),
                                    border: UiRect::all(Val::Px(1.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.05, 0.06, 0.08, 0.95)),
                                BorderColor::all(panel_border()),
                            ))
                            .with_children(|search| {
                                search.spawn((
                                    Text::new("SECTOR SEARCH  (coming soon)"),
                                    TextFont {
                                        font: font.clone(),
                                        font_size: 14.0,
                                        ..default()
                                    },
                                    TextColor(text_secondary()),
                                ));
                                search.spawn((
                                    Text::new("TAG: xeon-sector"),
                                    TextFont {
                                        font: font.clone(),
                                        font_size: 12.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgba(0.45, 0.5, 0.55, 1.0)),
                                ));
                            });

                            left.spawn((
                                Node {
                                    width: Val::Percent(100.0),
                                    flex_grow: 1.0,
                                    min_height: Val::Px(180.0),
                                    flex_direction: FlexDirection::Column,
                                    row_gap: Val::Px(14.0),
                                    padding: UiRect::all(Val::Px(8.0)),
                                    align_items: AlignItems::Stretch,
                                    overflow: Overflow::scroll_y(),
                                    ..default()
                                },
                                LobbyCardsContainer,
                            ));
                        });

                        body.spawn((
                            Node {
                                width: Val::Percent(34.0),
                                min_width: Val::Px(220.0),
                                flex_direction: FlexDirection::Column,
                                justify_content: JustifyContent::FlexStart,
                                row_gap: Val::Px(14.0),
                                padding: UiRect::horizontal(Val::Px(8.0)),
                                ..default()
                            },
                        ))
                        .with_children(|right| {
                            right.spawn((
                                Button,
                                SinglePlayerButton,
                                Node {
                                    width: Val::Percent(100.0),
                                    min_height: Val::Px(56.0),
                                    padding: UiRect::axes(Val::Px(24.0), Val::Px(18.0)),
                                    border_radius: BorderRadius::all(Val::Px(14.0)),
                                    border: UiRect::all(Val::Px(2.0)),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                BackgroundColor(accent_solo_cyan()),
                                BorderColor::all(Color::srgba(0.4, 0.95, 1.0, 0.65)),
                            ))
                            .with_children(|btn| {
                                btn.spawn((
                                    Text::new("SOLO"),
                                    TextFont {
                                        font: font.clone(),
                                        font_size: 22.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgba(0.02, 0.05, 0.08, 1.0)),
                                ));
                                btn.spawn((
                                    Text::new("Single player"),
                                    TextFont {
                                        font: font.clone(),
                                        font_size: 13.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgba(0.05, 0.12, 0.16, 0.85)),
                                ));
                            });

                            spawn_stub_button(right, &font, "RANKED MATCH", ComingSoonKind::Ranked);
                            spawn_stub_button(right, &font, "CREATE LOBBY", ComingSoonKind::Secondary);
                            spawn_stub_button(right, &font, "JOIN LOBBY", ComingSoonKind::Secondary);
                        });
                    });

                // ── Footer ───────────────────────────────────────────────────
                panel
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            min_height: Val::Px(44.0),
                            flex_direction: FlexDirection::Row,
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            padding: UiRect::horizontal(Val::Px(12.0)),
                            border_radius: BorderRadius::all(Val::Px(10.0)),
                            ..default()
                        },
                        BackgroundColor(menu_footer_bg()),
                    ))
                    .with_children(|footer| {
                        footer.spawn((
                            Text::new("GitHub   Discord   Web"),
                            TextFont {
                                font: font.clone(),
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(text_secondary()),
                        ));
                        footer.spawn((
                            Text::new("NEWS"),
                            TextFont {
                                font: font.clone(),
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(text_primary()),
                        ));
                    });
            });

            // Waiting overlay (drawn above chrome)
            root.spawn((
                WaitingQueueContainer,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    top: Val::Px(0.0),
                    bottom: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(20.0),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.01, 0.02, 0.05, 0.92)),
            ))
            .with_children(|wait| {
                wait.spawn((
                    Text::new("WAITING FOR PLAYERS..."),
                    TextFont {
                        font: font.clone(),
                        font_size: 24.0,
                        ..default()
                    },
                    TextColor(text_secondary()),
                    WaitingStatusText,
                ));
                wait.spawn((
                    Text::new(""),
                    TextFont {
                        font: font.clone(),
                        font_size: 18.0,
                        ..default()
                    },
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
                ))
                .with_children(|btn| {
                    btn.spawn((
                        Text::new("CANCEL"),
                        TextFont {
                            font: font.clone(),
                            font_size: 18.0,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));
                });
            });
        });
}

fn spawn_stub_button(parent: &mut ChildSpawnerCommands<'_>, font: &Handle<Font>, label: &str, kind: ComingSoonKind) {
    let base = match kind {
        ComingSoonKind::Ranked => accent_ranked_gold(),
        ComingSoonKind::Secondary => menu_secondary_button(),
    };
    parent.spawn((
        Button,
        ComingSoonMenuButton,
        kind,
        Node {
            width: Val::Percent(100.0),
            min_height: Val::Px(44.0),
            padding: UiRect::axes(Val::Px(20.0), Val::Px(12.0)),
            border_radius: BorderRadius::all(Val::Px(10.0)),
            border: UiRect::all(Val::Px(1.0)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(base),
        BorderColor::all(panel_border()),
    ))
    .with_children(|btn| {
        btn.spawn((
            Text::new(label),
            TextFont {
                font: font.clone(),
                font_size: 16.0,
                ..default()
            },
            TextColor(if matches!(kind, ComingSoonKind::Ranked) {
                Color::srgba(0.06, 0.05, 0.04, 1.0)
            } else {
                text_primary()
            }),
        ));
    });
}

fn queue_save(auto: &mut NicknameSettingsAutosave) {
    auto.dirty = true;
    auto.timer.reset();
}

fn flush_save(
    settings: &ClientSettings,
    auto: &mut NicknameSettingsAutosave,
) {
    if !auto.dirty {
        return;
    }
    if let Err(e) = save_client_settings_config(&settings.config) {
        warn!("Failed to save settings.ron: {e}");
    } else {
        auto.dirty = false;
        auto.timer.reset();
    }
}

pub fn system_nickname_field_focus_from_click(
    q: Query<&Interaction, (Changed<Interaction>, With<NicknameFieldRoot>)>,
    mut focus: ResMut<LobbyNicknameFocused>,
    mut nick_border: Query<&mut BorderColor, With<NicknameFieldRoot>>,
) {
    for interaction in &q {
        if *interaction == Interaction::Pressed {
            focus.0 = true;
            for mut b in &mut nick_border {
                *b = BorderColor::all(nickname_field_border_focused());
            }
        }
    }
}

pub fn system_clear_lobby_nickname_focus(
    mut focus: ResMut<LobbyNicknameFocused>,
    mut auto: ResMut<NicknameSettingsAutosave>,
    settings: Res<ClientSettings>,
    single: Query<&Interaction, (Changed<Interaction>, With<SinglePlayerButton>)>,
    join: Query<&Interaction, (Changed<Interaction>, With<JoinLobbyButton>)>,
    cancel: Query<&Interaction, (Changed<Interaction>, With<CancelJoinButton>)>,
    stubs: Query<&Interaction, (Changed<Interaction>, With<ComingSoonMenuButton>)>,
    mut nick_border: Query<&mut BorderColor, With<NicknameFieldRoot>>,
) {
    let mut pressed_other = false;
    for i in single.iter().chain(join.iter()).chain(cancel.iter()).chain(stubs.iter()) {
        if *i == Interaction::Pressed {
            pressed_other = true;
            break;
        }
    }
    if pressed_other && focus.0 {
        focus.0 = false;
        for mut b in &mut nick_border {
            *b = BorderColor::all(nickname_field_border());
        }
        flush_save(&*settings, &mut auto);
    }
}

pub fn system_nickname_keyboard_when_focused(
    mut focus: ResMut<LobbyNicknameFocused>,
    mut auto: ResMut<NicknameSettingsAutosave>,
    mut settings: ResMut<ClientSettings>,
    mut reader: MessageReader<KeyboardInput>,
    mut text_q: Query<&mut Text, With<NicknameFieldText>>,
    mut nick_border: Query<&mut BorderColor, With<NicknameFieldRoot>>,
) {
    if !focus.0 {
        return;
    }

    let Ok(mut text) = text_q.single_mut() else {
        return;
    };

    for ev in reader.read() {
        if !ev.state.is_pressed() {
            continue;
        }

        if matches!(&ev.logical_key, Key::Escape) {
            focus.0 = false;
            for mut b in &mut nick_border {
                *b = BorderColor::all(nickname_field_border());
            }
            flush_save(&*settings, &mut auto);
            continue;
        }

        if matches!(&ev.logical_key, Key::Enter) {
            focus.0 = false;
            for mut b in &mut nick_border {
                *b = BorderColor::all(nickname_field_border());
            }
            flush_save(&*settings, &mut auto);
            continue;
        }

        if matches!(&ev.logical_key, Key::Backspace) {
            text.0.pop();
            settings.config.profile.nickname = text.0.clone();
            settings.config.clamp_profile_nickname_in_place();
            text.0 = settings.config.profile.nickname.clone();
            queue_save(&mut auto);
            continue;
        }

        if let Some(inserted) = ev.text.as_ref() {
            if inserted.chars().all(is_printable_char) {
                for ch in inserted.chars() {
                    if text.0.chars().count() >= ProfileConfig::NICKNAME_MAX_CHARS {
                        break;
                    }
                    text.0.push(ch);
                }
                settings.config.profile.nickname = text.0.clone();
                settings.config.clamp_profile_nickname_in_place();
                text.0 = settings.config.profile.nickname.clone();
                queue_save(&mut auto);
            }
        }
    }
}

pub fn system_nickname_autosave_tick(
    time: Res<Time>,
    settings: Res<ClientSettings>,
    mut auto: ResMut<NicknameSettingsAutosave>,
) {
    if !auto.dirty {
        return;
    }
    auto.timer.tick(time.delta());
    if auto.timer.just_finished() {
        if let Err(e) = save_client_settings_config(&settings.config) {
            warn!("Failed to save settings.ron (autosave): {e}");
        } else {
            auto.dirty = false;
        }
    }
}

pub fn system_handle_coming_soon_click(
    mut q: Query<
        (&Interaction, &mut BackgroundColor, &ComingSoonKind),
        (Changed<Interaction>, With<ComingSoonMenuButton>),
    >,
) {
    for (interaction, mut bg, kind) in &mut q {
        match *interaction {
            Interaction::Pressed => {
                let label = match kind {
                    ComingSoonKind::Ranked => "RANKED MATCH",
                    ComingSoonKind::Secondary => "CREATE/JOIN (stub)",
                };
                info!("Menu action not implemented yet: {label}");
                *bg = match kind {
                    ComingSoonKind::Ranked => accent_ranked_gold_pressed().into(),
                    ComingSoonKind::Secondary => Color::srgba(0.1, 0.11, 0.14, 1.0).into(),
                };
            }
            Interaction::Hovered => {
                *bg = match kind {
                    ComingSoonKind::Ranked => accent_ranked_gold_hover().into(),
                    ComingSoonKind::Secondary => menu_secondary_button_hover().into(),
                };
            }
            Interaction::None => {
                *bg = match kind {
                    ComingSoonKind::Ranked => accent_ranked_gold().into(),
                    ComingSoonKind::Secondary => menu_secondary_button().into(),
                };
            }
        }
    }
}

/// Which lobby to show in the browser: same priority as server `handle_client_joins`.
fn primary_lobby_for_browser(lobbies: &[LobbyInfo]) -> Option<LobbyInfo> {
    if lobbies.is_empty() {
        return None;
    }
    let mut counting: Vec<&LobbyInfo> = lobbies.iter().filter(|l| l.is_counting_down).collect();
    if !counting.is_empty() {
        counting.sort_by_key(|l| l.id);
        return Some(counting[0].clone());
    }
    let mut waiting: Vec<&LobbyInfo> = lobbies.iter().collect();
    waiting.sort_by_key(|l| l.id);
    Some(waiting[0].clone())
}

pub fn system_update_lobby_browser(
    mut commands: Commands,
    mut receivers: Query<&mut MessageReceiver<ServerLobbiesBroadcastMessage>>,
    mut chrome_q: Query<&mut Node, (With<LobbyBrowserMainChrome>, Without<LobbyCardsContainer>, Without<WaitingQueueContainer>)>,
    mut container_q: Query<(Entity, &mut Node), (With<LobbyCardsContainer>, Without<WaitingQueueContainer>, Without<LobbyBrowserMainChrome>)>,
    mut queue_q: Query<(&mut Node, &Children), (With<WaitingQueueContainer>, Without<LobbyCardsContainer>, Without<LobbyBrowserMainChrome>)>,
    mut status_text_q: Query<&mut Text, (With<WaitingStatusText>, Without<WaitingPlayersText>, Without<LobbyCardTitle>, Without<LobbyCardPlayers>, Without<LobbyCardStatus>)>,
    mut players_text_q: Query<&mut Text, (With<WaitingPlayersText>, Without<WaitingStatusText>, Without<LobbyCardTitle>, Without<LobbyCardPlayers>, Without<LobbyCardStatus>)>,
    mut card_root_q: Query<(Entity, &mut BackgroundColor, &mut BorderColor, &mut JoinLobbyButton), With<LobbyCardRoot>>,
    mut card_title_q: Query<&mut Text, (With<LobbyCardTitle>, Without<WaitingStatusText>, Without<WaitingPlayersText>, Without<LobbyCardPlayers>, Without<LobbyCardStatus>)>,
    mut card_players_q: Query<&mut Text, (With<LobbyCardPlayers>, Without<WaitingStatusText>, Without<WaitingPlayersText>, Without<LobbyCardTitle>, Without<LobbyCardStatus>)>,
    mut card_status_q: Query<(&mut Text, &mut TextColor), (With<LobbyCardStatus>, Without<WaitingStatusText>, Without<WaitingPlayersText>, Without<LobbyCardTitle>, Without<LobbyCardPlayers>)>,
    children_q: Query<&Children>,
    settings: Res<ClientSettings>,
    joined_lobby: Option<Res<JoinedLobby>>,
) {
    let Some((cards_ent, mut cards_node)) = container_q.iter_mut().next() else {
        return;
    };
    let Some((mut queue_node, _)) = queue_q.iter_mut().next() else {
        return;
    };
    let Ok(mut chrome_node) = chrome_q.single_mut() else {
        return;
    };
    let target_id = joined_lobby.and_then(|r| r.0);

    for mut receiver in receivers.iter_mut() {
        for msg in receiver.receive() {
            if let Some(target) = target_id {
                chrome_node.display = Display::None;
                queue_node.display = Display::Flex;
                cards_node.display = Display::None;

                if let Some(lobby) = msg.lobbies.iter().find(|l| l.id == target) {
                    if let Some(mut status) = status_text_q.iter_mut().next() {
                        if lobby.is_counting_down {
                            **status = format!("STARTING IN {:.0}s", lobby.timer_secs);
                        } else {
                            **status = format!(
                                "WAITING FOR PLAYERS... {}/{}",
                                lobby.num_players, lobby.max_players
                            );
                        }
                    }
                    if let Some(mut players_text) = players_text_q.iter_mut().next() {
                        if !lobby.player_names.is_empty() {
                            **players_text = format!(
                                "CONNECTED PLAYERS\n\n{}",
                                lobby.player_names.join("\n")
                            );
                        } else {
                            **players_text = "ERROR: You are not in the lobby".to_string();
                        }
                    }
                }
            } else {
                chrome_node.display = Display::Flex;
                queue_node.display = Display::None;
                cards_node.display = Display::Flex;

                if let Some(lobby) = primary_lobby_for_browser(&msg.lobbies) {
                    let font = settings.font.clone();
                    let status_text = if lobby.is_counting_down {
                        format!("STARTING IN {:.0}s", lobby.timer_secs)
                    } else {
                        "WAITING FOR PLAYERS".to_string()
                    };

                    let is_counting = lobby.is_counting_down;
                    let bg_color = if is_counting {
                        accent_solo_cyan()
                    } else {
                        panel_bg()
                    };
                    let bd_color = if is_counting {
                        menu_panel_border_glow()
                    } else {
                        panel_border()
                    };
                    let text_clr = if is_counting {
                        Color::srgba(0.02, 0.06, 0.08, 1.0)
                    } else {
                        text_secondary()
                    };

                    // Try to update existing card
                    if let Some((_, mut bg, mut bd, mut jlb)) = card_root_q.iter_mut().next() {
                        jlb.0 = lobby.id;
                        *bd = BorderColor::all(bd_color);
                        
                        if let Some(mut t) = card_title_q.iter_mut().next() {
                            t.0 = format!("SECTOR {}", lobby.id);
                        }
                        if let Some(mut t) = card_players_q.iter_mut().next() {
                            t.0 = format!("{}/{} players", lobby.num_players, lobby.max_players);
                        }
                        if let Some((mut t, _)) = card_status_q.iter_mut().next() {
                            t.0 = status_text;
                        }
                    } else {
                        // Spawn if it doesn't exist
                        commands.entity(cards_ent).with_children(|p| {
                            p.spawn((
                                LobbyCardRoot,
                                Button,
                                JoinLobbyButton(lobby.id),
                                Node {
                                    width: Val::Percent(100.0),
                                    min_height: Val::Px(180.0),
                                    flex_direction: FlexDirection::Column,
                                    justify_content: JustifyContent::SpaceBetween,
                                    border_radius: BorderRadius::all(Val::Px(12.0)),
                                    border: UiRect::all(Val::Px(2.0)),
                                    overflow: Overflow::clip(),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.58, 0.71, 0.43, 1.0)), // Map background placeholder
                                BorderColor::all(bd_color),
                            ))
                            .with_children(|card| {
                                // Top Area
                                card.spawn(Node {
                                    width: Val::Percent(100.0),
                                    flex_direction: FlexDirection::Row,
                                    justify_content: JustifyContent::SpaceBetween,
                                    padding: UiRect::all(Val::Px(12.0)),
                                    ..default()
                                }).with_children(|top| {
                                    // Left Pills
                                    top.spawn(Node {
                                        flex_direction: FlexDirection::Column,
                                        row_gap: Val::Px(6.0),
                                        ..default()
                                    }).with_children(|left| {
                                        let blue_pill_bg = Color::srgba(0.04, 0.47, 0.75, 1.0);
                                        
                                        left.spawn((
                                            Node {
                                                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                                border_radius: BorderRadius::all(Val::Px(4.0)),
                                                ..default()
                                            },
                                            BackgroundColor(blue_pill_bg),
                                        )).with_children(|p| {
                                            p.spawn((
                                                Text::new("COMPACT MAP"),
                                                TextFont {
                                                    font: font.clone(),
                                                    font_size: 13.0,
                                                    ..default()
                                                },
                                                TextColor(Color::WHITE),
                                            ));
                                        });

                                        left.spawn((
                                            Node {
                                                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                                border_radius: BorderRadius::all(Val::Px(4.0)),
                                                ..default()
                                            },
                                            BackgroundColor(blue_pill_bg),
                                        )).with_children(|p| {
                                            p.spawn((
                                                Text::new("RANDOM SPAWN"),
                                                TextFont {
                                                    font: font.clone(),
                                                    font_size: 13.0,
                                                    ..default()
                                                },
                                                TextColor(Color::WHITE),
                                            ));
                                        });
                                    });

                                    // Right Pill (Status / Timer)
                                    top.spawn((
                                        Node {
                                            padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                            border_radius: BorderRadius::all(Val::Px(4.0)),
                                            height: Val::Px(24.0),
                                            align_items: AlignItems::Center,
                                            ..default()
                                        },
                                        BackgroundColor(Color::srgba(0.04, 0.47, 0.75, 1.0)),
                                    )).with_children(|right| {
                                        right.spawn((
                                            LobbyCardStatus,
                                            Text::new(status_text),
                                            TextFont {
                                                font: font.clone(),
                                                font_size: 13.0,
                                                ..default()
                                            },
                                            TextColor(Color::WHITE),
                                        ));
                                    });
                                });

                                // Bottom Area (Player count + Footer)
                                card.spawn(Node {
                                    width: Val::Percent(100.0),
                                    flex_direction: FlexDirection::Column,
                                    justify_content: JustifyContent::FlexEnd,
                                    ..default()
                                }).with_children(|bot| {
                                    // Player count pill (right-aligned, overlaps map)
                                    bot.spawn(Node {
                                        width: Val::Percent(100.0),
                                        justify_content: JustifyContent::FlexEnd,
                                        padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                                        ..default()
                                    }).with_children(|p_row| {
                                        p_row.spawn((
                                            Node {
                                                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                                border_radius: BorderRadius::all(Val::Px(4.0)),
                                                ..default()
                                            },
                                            BackgroundColor(Color::srgba(0.12, 0.12, 0.12, 0.9)),
                                        )).with_children(|p_pill| {
                                            p_pill.spawn((
                                                LobbyCardPlayers,
                                                Text::new(format!("{}/{} players", lobby.num_players, lobby.max_players)),
                                                TextFont {
                                                    font: font.clone(),
                                                    font_size: 13.0,
                                                    ..default()
                                                },
                                                TextColor(Color::WHITE),
                                            ));
                                        });
                                    });

                                    // Footer area
                                    bot.spawn((
                                        Node {
                                            width: Val::Percent(100.0),
                                            flex_direction: FlexDirection::Column,
                                            padding: UiRect::axes(Val::Px(16.0), Val::Px(12.0)),
                                            ..default()
                                        },
                                        BackgroundColor(Color::srgba(0.35, 0.38, 0.30, 0.95)),
                                    )).with_children(|footer| {
                                        footer.spawn((
                                            LobbyCardTitle,
                                            Text::new(format!("SECTOR {}", lobby.id)),
                                            TextFont {
                                                font: font.clone(),
                                                font_size: 22.0,
                                                ..default()
                                            },
                                            TextColor(Color::WHITE),
                                        ));
                                        footer.spawn((
                                            Text::new("FREE FOR ALL"),
                                            TextFont {
                                                font: font.clone(),
                                                font_size: 14.0,
                                                ..default()
                                            },
                                            TextColor(Color::srgba(0.8, 0.8, 0.8, 1.0)),
                                        ));
                                    });
                                });
                            });
                        });
                    }
                } else {
                    // No lobby found, despawn if exists
                    if let Ok(children) = children_q.get(cards_ent) {
                        for child in children.iter() {
                            commands.entity(child).despawn();
                        }
                    }
                }
            }
        }
    }
}

pub fn system_handle_join_clicks(
    mut interaction_q: Query<
        (&Interaction, &mut BackgroundColor, &JoinLobbyButton),
        (Changed<Interaction>, With<Button>),
    >,
    mut senders: Query<&mut MessageSender<ClientJoinMessage>>,
    cli: Res<Cli>,
    settings: Res<ClientSettings>,
    mut joined_lobby: ResMut<JoinedLobby>,
) {
    for (interaction, mut bg, btn) in interaction_q.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                let name = settings.config.display_nickname_for_play();
                for mut sender in senders.iter_mut() {
                    let msg = ClientJoinMessage {
                        name: name.clone(),
                        is_observer: cli.observer,
                        target_lobby_id: Some(btn.0),
                    };
                    let _ = sender.send::<ReliableChannel>(msg);
                    info!("✅ Requested to join Lobby ID {}", btn.0);
                }
                *bg = accent_solo_cyan_pressed().into();
                joined_lobby.0 = Some(btn.0);
            }
            Interaction::Hovered => {
                *bg = accent_solo_cyan_hover().into();
            }
            Interaction::None => {
                *bg = panel_bg().into();
            }
        }
    }
}

pub fn system_handle_single_player_click(
    mut interaction_q: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<SinglePlayerButton>),
    >,
    mut commands: Commands,
    mut next_state: ResMut<NextState<crate::AppState>>,
    settings: Res<ClientSettings>,
) {
    for (interaction, mut bg) in interaction_q.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                info!("🎮 Iniciando Single Player...");
                *bg = accent_solo_cyan_pressed().into();

                let mut config = dark_rift_shared::game_config::GameConfig::default();

                if let Ok(content) = std::fs::read_to_string("assets/configs/default_single_player.ron")
                {
                    if let Ok(parsed) = ron::from_str(&content) {
                        config = parsed;
                        info!("✅ Loaded Single Player config from .ron template!");
                    } else {
                        warn!("⚠️ Failed to parse .ron template, using fallback defaults.");
                    }
                } else {
                    config.bot_count = 7;
                }

                let spawn_x = config.map_width / 2;
                let spawn_y = config.map_height / 2;
                let player_name = settings.config.display_nickname_for_play();

                let mock_msg = ServerStartMessage {
                    config,
                    my_player_id: Some(1),
                    seed: 42069,
                    players: vec![PlayerInfo {
                        id: 1,
                        name: player_name,
                        player_type: PlayerType::Human,
                        color: [1.0, 0.2, 0.2],
                        spawn_x,
                        spawn_y,
                    }],
                    missed_turns: vec![],
                };

                commands.insert_resource(crate::MockSinglePlayerStart(mock_msg));
                next_state.set(crate::AppState::InGame);
            }
            Interaction::Hovered => *bg = accent_solo_cyan_hover().into(),
            Interaction::None => *bg = accent_solo_cyan().into(),
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

pub fn system_cleanup_lobby_browser(mut commands: Commands, q_root: Query<Entity, With<LobbyBrowserRoot>>) {
    if let Some(ent) = q_root.iter().next() {
        commands.entity(ent).despawn();
    }
}
