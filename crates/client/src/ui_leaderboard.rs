use bevy::prelude::*;
use dark_rift_shared::game::GameState;
use crate::{ClientSettings, Cli};
use crate::ui_theme::*;
use crate::InGameEntity;

#[derive(Component)]
pub struct LeaderboardRoot;

#[derive(Component)]
pub struct LeaderboardEntryText(pub usize);

pub fn setup_leaderboard(
    mut commands: Commands,
    settings: Res<ClientSettings>,
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
                padding: UiRect::all(Val::Px(SPACE_3)),
                row_gap: Val::Px(SPACE_2),
                border_radius: BorderRadius::all(Val::Px(PANEL_RADIUS)),
                ..default()
            },
            BackgroundColor(panel_bg()),
        ))
        .insert(BorderColor::all(panel_border()))
        .insert(GlobalZIndex(850))
        .with_children(|parent| {
        parent.spawn((
            Text::new("LEADERBOARD"),
            TextFont {
                font: font.clone(),
                font_size: 16.0,
                ..default()
            },
            TextColor(accent_primary()),
            Node {
                margin: UiRect::bottom(Val::Px(SPACE_2)),
                ..default()
            },
        ));

        for i in 0..5 {
            parent.spawn((
                LeaderboardEntryText(i),
                Text::new(""),
                TextFont {
                    font: font.clone(),
                    font_size: 14.0,
                    ..default()
                },
                TextColor(text_primary()),
            ));
        }
    });
}

pub fn system_update_leaderboard(
    game_opt: Option<Res<GameState>>,
    mut q_entries: Query<(&LeaderboardEntryText, &mut Text, &mut TextColor)>,
    my_id: Option<Res<crate::MyPlayerId>>,
) {
    let Some(game) = game_opt else { return };
    let my_player_id = my_id.map(|r| r.0).unwrap_or(0);
    
    // Sort players by tile count
    let mut sorted_players: Vec<_> = game.players.iter().filter(|p| p.alive).collect();
    sorted_players.sort_by(|a, b| b.tile_count.cmp(&a.tile_count));
    
    // Lazy count total land tiles (since terrain is immutable)
    let total_land = game.map.terrain.iter().filter(|t| t.is_land()).count().max(1) as f32;
    
    for (entry, mut text, mut text_color) in q_entries.iter_mut() {
        if let Some(p) = sorted_players.get(entry.0) {
            let percentage = (p.tile_count as f32 / total_land) * 100.0;
            text.0 = format!("{}. {} - {:.1}%", entry.0 + 1, p.name, percentage);
            
            // Highlight my player
            if p.id == my_player_id {
                text_color.0 = accent_primary();
            } else {
                text_color.0 = text_primary();
            }
        } else {
            text.0 = String::new();
        }
    }
}