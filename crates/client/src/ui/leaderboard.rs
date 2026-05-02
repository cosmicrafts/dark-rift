use bevy::prelude::*;
use dark_rift_shared::engine::DarkRiftEngine;
use crate::{ClientSettings, Cli};
use crate::ui::theme::*;
use crate::InGameEntity;

#[derive(Component)]
pub struct LeaderboardRoot;

#[derive(Component)]
pub struct LeaderboardToggleButton;

#[derive(Component)]
pub struct LeaderboardContent;

#[derive(Component)] pub struct LbCellRank(pub usize);
#[derive(Component)] pub struct LbCellName(pub usize);
#[derive(Component)] pub struct LbCellOwned(pub usize);
#[derive(Component)] pub struct LbCellGold(pub usize);
#[derive(Component)] pub struct LbCellMax(pub usize);

pub fn format_short(v: f64) -> String {
    if v >= 1_000_000.0 {
        format!("{:.2}M", v / 1_000_000.0)
    } else if v >= 1_000.0 {
        format!("{:.0}K", v / 1_000.0)
    } else {
        format!("{:.0}", v)
    }
}

pub fn setup_leaderboard(
    mut commands: Commands,
    settings: Res<ClientSettings>,
    asset_server: Res<AssetServer>,
    cli: Res<Cli>,
) {
    if cli.bot { return; }

    let font = settings.font.clone();
    
    commands
        .spawn((
            InGameEntity,
            LeaderboardRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(SPACE_3),
                left: Val::Px(SPACE_3),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                row_gap: Val::Px(10.0),
                ..default()
            },
        ))
        .insert(GlobalZIndex(850))
        .with_children(|root| {
            // TOGGLE BUTTON (Siempre visible)
            root.spawn((
                Button,
                LeaderboardToggleButton,
                Node {
                    width: Val::Px(48.0),
                    height: Val::Px(48.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(BUTTON_RADIUS)),
                    ..default()
                },
                BackgroundColor(panel_bg()),
                BorderColor::all(panel_border()),
            )).with_children(|b| {
                b.spawn((
                    ImageNode::new(asset_server.load("icons/leaderboard.png")),
                    Node { width: Val::Px(24.0), height: Val::Px(24.0), ..default() }
                ));
            });

            // PANEL CONTENIDO
            root.spawn((
                LeaderboardContent,
                Node {
                    display: Display::None, // Oculto por defecto
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(SPACE_3)),
                    row_gap: Val::Px(10.0),
                    border_radius: BorderRadius::all(Val::Px(PANEL_RADIUS)),
                    width: Val::Px(420.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.08, 0.11, 0.16, 0.95)),
                BorderColor::all(panel_border()),
            )).with_children(|parent| {
                // Header Row
            parent.spawn(Node {
                flex_direction: FlexDirection::Row,
                width: Val::Percent(100.0),
                border: UiRect::bottom(Val::Px(1.0)),
                padding: UiRect::bottom(Val::Px(6.0)),
                ..default()
            }).insert(BorderColor::all(Color::srgba(0.3, 0.4, 0.5, 0.5)))
            .with_children(|row| {
                let fs = 14.0;
                let c = Color::srgba(0.8, 0.8, 0.85, 1.0);
                row.spawn((Text::new("#"), TextFont { font: font.clone(), font_size: fs, ..default() }, TextColor(c), Node { width: Val::Px(30.0), ..default() }));
                row.spawn((Text::new("Player"), TextFont { font: font.clone(), font_size: fs, ..default() }, TextColor(c), Node { width: Val::Px(140.0), ..default() }));
                row.spawn((Text::new("Owned..."), TextFont { font: font.clone(), font_size: fs, ..default() }, TextColor(c), Node { width: Val::Px(70.0), ..default() }));
                row.spawn((Text::new("Gold"), TextFont { font: font.clone(), font_size: fs, ..default() }, TextColor(c), Node { width: Val::Px(60.0), ..default() }));
                row.spawn((Text::new("Max troops"), TextFont { font: font.clone(), font_size: fs, ..default() }, TextColor(c), Node { width: Val::Px(80.0), ..default() }));
            });

            parent.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    width: Val::Percent(100.0),
                    max_height: Val::Px(240.0),
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
                bevy::ui::ScrollPosition::default(),
            )).with_children(|scroll| {
                // 15 Rows
                for i in 0..15 {
                    scroll.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        width: Val::Percent(100.0),
                        border: UiRect::bottom(Val::Px(1.0)),
                        padding: UiRect::bottom(Val::Px(4.0)),
                        ..default()
                    }).insert(BorderColor::all(Color::srgba(0.3, 0.4, 0.5, 0.2)))
                    .with_children(|row| {
                        let fs = 15.0;
                        let c = Color::WHITE;
                        row.spawn((LbCellRank(i), Text::new(""), TextFont { font: font.clone(), font_size: fs, ..default() }, TextColor(c), Node { width: Val::Px(30.0), ..default() }));
                        row.spawn((LbCellName(i), Text::new(""), TextFont { font: font.clone(), font_size: fs, ..default() }, TextColor(c), Node { width: Val::Px(140.0), ..default() }));
                        row.spawn((LbCellOwned(i), Text::new(""), TextFont { font: font.clone(), font_size: fs, ..default() }, TextColor(c), Node { width: Val::Px(70.0), ..default() }));
                        row.spawn((LbCellGold(i), Text::new(""), TextFont { font: font.clone(), font_size: fs, ..default() }, TextColor(c), Node { width: Val::Px(60.0), ..default() }));
                        row.spawn((LbCellMax(i), Text::new(""), TextFont { font: font.clone(), font_size: fs, ..default() }, TextColor(c), Node { width: Val::Px(80.0), ..default() }));
                    });
                }
            });
            });
        });
}

pub fn system_update_leaderboard(
    engine_opt: Option<Res<DarkRiftEngine>>,
    q_panel: Query<&Node, With<LeaderboardContent>>,
    mut q_rank: Query<(&LbCellRank, &mut Text), (Without<LbCellName>, Without<LbCellOwned>, Without<LbCellGold>, Without<LbCellMax>)>,
    mut q_name: Query<(&LbCellName, &mut Text, &mut TextColor), (Without<LbCellRank>, Without<LbCellOwned>, Without<LbCellGold>, Without<LbCellMax>)>,
    mut q_owned: Query<(&LbCellOwned, &mut Text), (Without<LbCellRank>, Without<LbCellName>, Without<LbCellGold>, Without<LbCellMax>)>,
    mut q_gold: Query<(&LbCellGold, &mut Text), (Without<LbCellRank>, Without<LbCellName>, Without<LbCellOwned>, Without<LbCellMax>)>,
    mut q_max: Query<(&LbCellMax, &mut Text), (Without<LbCellRank>, Without<LbCellName>, Without<LbCellOwned>, Without<LbCellGold>)>,
    my_id: Option<Res<crate::MyPlayerId>>,
    mut cached_land: Local<Option<f32>>,
) {
    let Some(engine) = engine_opt.as_deref() else { return };
    let game = &engine.state;
    let panel_hidden = q_panel
        .iter()
        .next()
        .map_or(true, |n| n.display == Display::None);
    if panel_hidden {
        return;
    }
    let my_player_id = my_id.map(|r| r.0).unwrap_or(0);
    
    // Sort players by tile count
    let mut sorted_players: Vec<_> = game.players.iter().filter(|p| p.alive).collect();
    sorted_players.sort_by(|a, b| b.tile_count.cmp(&a.tile_count));
    
    let total_land = match *cached_land {
        Some(val) => val,
        None => {
            let val = game.map.terrain.iter().filter(|t| t.is_land()).count().max(1) as f32;
            *cached_land = Some(val);
            val
        }
    };

    let mut update_cell = |idx: usize, p_opt: Option<&&dark_rift_shared::player::Player>| {
        let (rank_str, name_str, owned_str, gold_str, max_str, is_me) = match p_opt {
            Some(p) => {
                let percentage = (p.tile_count as f32 / total_land) * 100.0;
                let mut trunc_name = p.name.clone();
                if trunc_name.chars().count() > 14 {
                    trunc_name = trunc_name.chars().take(12).collect::<String>() + "...";
                }
                (
                    format!("{}", idx + 1),
                    trunc_name,
                    format!("{:.1}%", percentage),
                    format_short(p.troops),
                    format_short(p.max_troops),
                    p.id == my_player_id
                )
            },
            None => (String::new(), String::new(), String::new(), String::new(), String::new(), false)
        };

        if let Some((_, mut text)) = q_rank.iter_mut().find(|(c, _)| c.0 == idx) {
            if text.0 != rank_str {
                text.0 = rank_str;
            }
        }
        if let Some((_, mut text, mut color)) = q_name.iter_mut().find(|(c, _, _)| c.0 == idx) {
            if text.0 != name_str {
                text.0 = name_str;
            }
            let nc = if is_me {
                accent_primary()
            } else {
                Color::WHITE
            };
            if color.0 != nc {
                color.0 = nc;
            }
        }
        if let Some((_, mut text)) = q_owned.iter_mut().find(|(c, _)| c.0 == idx) {
            if text.0 != owned_str {
                text.0 = owned_str;
            }
        }
        if let Some((_, mut text)) = q_gold.iter_mut().find(|(c, _)| c.0 == idx) {
            if text.0 != gold_str {
                text.0 = gold_str;
            }
        }
        if let Some((_, mut text)) = q_max.iter_mut().find(|(c, _)| c.0 == idx) {
            if text.0 != max_str {
                text.0 = max_str;
            }
        }
    };

    for i in 0..15 {
        update_cell(i, sorted_players.get(i));
    }
}

pub fn system_handle_leaderboard_toggle(
    mut q_interaction: Query<&Interaction, (Changed<Interaction>, With<LeaderboardToggleButton>)>,
    mut q_content: Query<&mut Node, With<LeaderboardContent>>,
) {
    for interaction in q_interaction.iter_mut() {
        if *interaction == Interaction::Pressed {
            for mut node in q_content.iter_mut() {
                node.display = match node.display {
                    Display::None => Display::Flex,
                    _ => Display::None,
                };
            }
        }
    }
}