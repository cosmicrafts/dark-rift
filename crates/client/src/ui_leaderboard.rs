use bevy::prelude::*;
use dark_rift_shared::game::GameState;
use crate::{ClientSettings, Cli};
use crate::ui_theme::*;
use crate::InGameEntity;

#[derive(Component)]
pub struct LeaderboardRoot;

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
                row_gap: Val::Px(10.0),
                border_radius: BorderRadius::all(Val::Px(PANEL_RADIUS)),
                width: Val::Px(420.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.08, 0.11, 0.16, 0.95)),
        ))
        .insert(BorderColor::all(panel_border()))
        .insert(GlobalZIndex(850))
        .with_children(|parent| {
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

            // 5 Rows
            for i in 0..5 {
                parent.spawn(Node {
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
}

pub fn system_update_leaderboard(
    game_opt: Option<Res<GameState>>,
    mut q_rank: Query<(&LbCellRank, &mut Text), (Without<LbCellName>, Without<LbCellOwned>, Without<LbCellGold>, Without<LbCellMax>)>,
    mut q_name: Query<(&LbCellName, &mut Text, &mut TextColor), (Without<LbCellRank>, Without<LbCellOwned>, Without<LbCellGold>, Without<LbCellMax>)>,
    mut q_owned: Query<(&LbCellOwned, &mut Text), (Without<LbCellRank>, Without<LbCellName>, Without<LbCellGold>, Without<LbCellMax>)>,
    mut q_gold: Query<(&LbCellGold, &mut Text), (Without<LbCellRank>, Without<LbCellName>, Without<LbCellOwned>, Without<LbCellMax>)>,
    mut q_max: Query<(&LbCellMax, &mut Text), (Without<LbCellRank>, Without<LbCellName>, Without<LbCellOwned>, Without<LbCellGold>)>,
    my_id: Option<Res<crate::MyPlayerId>>,
    mut cached_land: Local<Option<f32>>,
) {
    let Some(game) = game_opt else { return };
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

        if let Some((_, mut text)) = q_rank.iter_mut().find(|(c, _)| c.0 == idx) { text.0 = rank_str; }
        if let Some((_, mut text, mut color)) = q_name.iter_mut().find(|(c, _, _)| c.0 == idx) { 
            text.0 = name_str; 
            color.0 = if is_me { accent_primary() } else { Color::WHITE };
        }
        if let Some((_, mut text)) = q_owned.iter_mut().find(|(c, _)| c.0 == idx) { text.0 = owned_str; }
        if let Some((_, mut text)) = q_gold.iter_mut().find(|(c, _)| c.0 == idx) { text.0 = gold_str; }
        if let Some((_, mut text)) = q_max.iter_mut().find(|(c, _)| c.0 == idx) { text.0 = max_str; }
    };

    for i in 0..5 {
        update_cell(i, sorted_players.get(i));
    }
}