use bevy::color::prelude::*;
use bevy::ecs::system::SystemParam;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use bevy::ui::{UiTransform, Val2};
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

/// Phase for main lobby menu motion (intro / idle / exit / stub re-entry).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LobbyMenuAnimPhase {
    Entering,
    Idle,
    Exiting,
    StubReentering,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PendingNavigate {
    None,
    SinglePlayer,
    JoinLobby(u64),
    Stub,
}

#[derive(Resource)]
pub struct LobbyMainMenuAnim {
    pub phase: LobbyMenuAnimPhase,
    pub phase_time: f32,
    pub pending: PendingNavigate,
    pub clicked: Option<Entity>,
    /// After cancel-from-waiting: replay main-menu intro (slide/fade) instead of snapping idle.
    pub play_main_menu_intro_after_cancel: bool,
}

impl LobbyMainMenuAnim {
    #[inline]
    pub fn is_idle(&self) -> bool {
        self.phase == LobbyMenuAnimPhase::Idle
    }
}

/// Root backdrop (full screen behind panel).
#[derive(Component)]
pub struct LobbyMenuAnimBackdrop;

/// Main framed panel (background + border fade).
#[derive(Component)]
pub struct LobbyMenuAnimPanel;

#[derive(Component)]
pub struct LobbyMenuAnimLeftColumn;

#[derive(Component)]
pub struct LobbyMenuAnimRightColumn;

#[derive(Component)]
pub struct LobbyMenuAnimHeaderRow;

#[derive(Component)]
pub struct LobbyMenuAnimFooterBar;

#[derive(Clone, Copy, Debug)]
pub enum LobbyMenuTextRole {
    TitleMain,
    Version,
    NicknameField,
    SoloPrimary,
    SoloSecondary,
    FooterLinks,
    FooterNews,
    StubRanked,
    StubSecondary,
}

/// Text nodes that fade with the menu (chrome only; not waiting overlay).
#[derive(Component, Clone, Copy)]
pub struct LobbyMenuAnimatedText(pub LobbyMenuTextRole);

#[inline]
fn base_text_color(role: LobbyMenuTextRole) -> Color {
    match role {
        LobbyMenuTextRole::TitleMain => accent_solo_cyan(),
        LobbyMenuTextRole::Version => text_secondary(),
        LobbyMenuTextRole::NicknameField => text_primary(),
        LobbyMenuTextRole::SoloPrimary => Color::srgba(0.02, 0.05, 0.08, 1.0),
        LobbyMenuTextRole::SoloSecondary => Color::srgba(0.05, 0.12, 0.16, 0.85),
        LobbyMenuTextRole::FooterLinks => text_secondary(),
        LobbyMenuTextRole::FooterNews => text_primary(),
        LobbyMenuTextRole::StubRanked => Color::srgba(0.06, 0.05, 0.04, 1.0),
        LobbyMenuTextRole::StubSecondary => text_primary(),
    }
}

/// Bundles mutable UI queries for lobby menu motion.
///
/// Bevy B0001 treats multiple mutable queries for the same component type inside one
/// `#[derive(SystemParam)]` as conflicting unless they are wrapped in a `ParamSet`, even when the
/// marker components are logically disjoint. Keep color writers and transform writers in separate
/// sets so the animation system can mutate both safely. All `Node` mutators for the lobby chrome
/// live in `layout_node_set` so they never alias `Node` with `color_set`.
#[derive(SystemParam)]
pub struct LobbyMenuAnimVisualQueries<'w, 's> {
    /// Panel chrome (includes `Node` for `display`) plus waiting/cards layout roots — one `ParamSet`
    /// so `Node` is never aliased with the panel tuple (Bevy B0001).
    pub layout_node_set: ParamSet<
        'w,
        's,
        (
            Query<'w, 's, &'static mut Node, With<LobbyMenuAnimPanel>>,
            Query<'w, 's, &'static mut Node, With<WaitingQueueContainer>>,
            Query<'w, 's, &'static mut Node, With<LobbyCardsContainer>>,
        ),
    >,
    pub color_set: ParamSet<
        'w,
        's,
        (
            Query<'w, 's, &'static mut BackgroundColor, With<LobbyMenuAnimBackdrop>>,
            Query<'w, 's, &'static mut BackgroundColor, With<LobbyMenuAnimFooterBar>>,
            Query<
                'w,
                's,
                (&'static mut BackgroundColor, &'static mut BorderColor),
                With<NicknameFieldRoot>,
            >,
            Query<
                'w,
                's,
                (&'static mut BackgroundColor, &'static mut BorderColor),
                With<SinglePlayerButton>,
            >,
            Query<
                'w,
                's,
                (
                    Entity,
                    &'static mut BackgroundColor,
                    &'static mut BorderColor,
                    &'static ComingSoonKind,
                ),
                With<ComingSoonMenuButton>,
            >,
            Query<
                'w,
                's,
                (&'static mut BackgroundColor, &'static mut BorderColor),
                With<LobbyMenuAnimPanel>,
            >,
        ),
    >,
    pub transform_set: ParamSet<
        'w,
        's,
        (
            Query<'w, 's, &'static mut UiTransform, With<LobbyMenuAnimHeaderRow>>,
            Query<'w, 's, &'static mut UiTransform, With<LobbyMenuAnimLeftColumn>>,
            Query<'w, 's, &'static mut UiTransform, With<LobbyMenuAnimRightColumn>>,
            Query<'w, 's, &'static mut UiTransform, With<SinglePlayerButton>>,
            Query<
                'w,
                's,
                (Entity, &'static mut UiTransform),
                With<ComingSoonMenuButton>,
            >,
            Query<'w, 's, (Entity, &'static mut UiTransform), With<LobbyCardRoot>>,
        ),
    >,
    pub anim_text: Query<'w, 's, (&'static mut TextColor, &'static LobbyMenuAnimatedText)>,
}

#[inline]
fn set_clicked_menu_scale(q: &mut LobbyMenuAnimVisualQueries, ce: Entity, scale: Vec2) {
    if let Ok(mut tr) = q.transform_set.p3().get_mut(ce) {
        tr.scale = scale;
        return;
    }
    if let Ok((_, mut tr)) = q.transform_set.p4().get_mut(ce) {
        tr.scale = scale;
        return;
    }
    if let Ok((_, mut tr)) = q.transform_set.p5().get_mut(ce) {
        tr.scale = scale;
    }
}

fn reset_main_menu_click_transforms(q: &mut LobbyMenuAnimVisualQueries) {
    if let Ok(mut tr) = q.transform_set.p3().single_mut() {
        *tr = UiTransform::default();
    }
    for (_, mut tr) in q.transform_set.p4().iter_mut() {
        *tr = UiTransform::default();
    }
    for (_, mut tr) in q.transform_set.p5().iter_mut() {
        *tr = UiTransform::default();
    }
}

#[inline]
fn apply_lobby_waiting_overlay_layout(q: &mut LobbyMenuAnimVisualQueries) {
    if let Ok(mut panel_node) = q.layout_node_set.p0().single_mut() {
        panel_node.display = Display::None;
    }
    if let Ok(mut n) = q.layout_node_set.p1().single_mut() {
        n.display = Display::Flex;
    }
    if let Ok(mut n) = q.layout_node_set.p2().single_mut() {
        n.display = Display::None;
    }
}

const MENU_INTRO_DURATION: f32 = 0.22;
const MENU_EXIT_DURATION: f32 = 0.24;
const MENU_STUB_REENTER_DURATION: f32 = 0.18;

const INTRO_SLIDE_LEFT_PX: f32 = -110.0;
const INTRO_SLIDE_RIGHT_PX: f32 = 110.0;
const INTRO_HEADER_Y_PX: f32 = -48.0;

#[inline]
fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

#[inline]
fn ease_in_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn solo_button_border_color() -> Color {
    Color::srgba(0.4, 0.95, 1.0, 0.65)
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
            LobbyMenuAnimBackdrop,
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
            BackgroundColor(menu_backdrop().with_alpha(0.0)),
        ))
        .with_children(|root| {
            root.spawn((
                LobbyBrowserMainChrome,
                LobbyMenuAnimPanel,
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
                BackgroundColor(menu_panel_bg().with_alpha(0.0)),
                BorderColor::all(menu_panel_border_glow().with_alpha(0.0)),
            ))
            .with_children(|panel| {
                // ── Header: title + version (centered) ───────────────────────
                panel
                    .spawn((
                        LobbyMenuAnimHeaderRow,
                        UiTransform::from_translation(Val2::px(0.0, INTRO_HEADER_Y_PX)),
                        Node {
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(12.0),
                            ..default()
                        },
                    ))
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
                                    LobbyMenuAnimatedText(LobbyMenuTextRole::TitleMain),
                                    TextFont {
                                        font: font.clone(),
                                        font_size: 42.0,
                                        ..default()
                                    },
                                    TextColor(base_text_color(LobbyMenuTextRole::TitleMain).with_alpha(0.0)),
                                ));
                                title_col.spawn((
                                    Text::new(format!("v{GAME_VERSION_LABEL}")),
                                    LobbyMenuAnimatedText(LobbyMenuTextRole::Version),
                                    TextFont {
                                        font: font.clone(),
                                        font_size: 16.0,
                                        ..default()
                                    },
                                    TextColor(base_text_color(LobbyMenuTextRole::Version).with_alpha(0.0)),
                                ));
                            });

                        header.spawn(Node {
                            flex_grow: 1.0,
                            min_width: Val::Px(0.0),
                            ..default()
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
                            LobbyMenuAnimLeftColumn,
                            UiTransform::from_translation(Val2::px(INTRO_SLIDE_LEFT_PX, 0.0)),
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
                                Button,
                                NicknameFieldRoot,
                                Node {
                                    width: Val::Percent(100.0),
                                    height: Val::Px(32.0),
                                    min_height: Val::Px(30.0),
                                    max_height: Val::Px(36.0),
                                    padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                    border_radius: BorderRadius::all(Val::Px(8.0)),
                                    border: UiRect::all(Val::Px(1.0)),
                                    justify_content: JustifyContent::FlexStart,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                BackgroundColor(nickname_field_bg().with_alpha(0.0)),
                                BorderColor::all(nickname_field_border().with_alpha(0.0)),
                            ))
                            .with_children(|field| {
                                field.spawn((
                                    Text::new(nick_display.clone()),
                                    NicknameFieldText,
                                    LobbyMenuAnimatedText(LobbyMenuTextRole::NicknameField),
                                    TextFont {
                                        font: font.clone(),
                                        font_size: 14.0,
                                        ..default()
                                    },
                                    TextColor(base_text_color(LobbyMenuTextRole::NicknameField).with_alpha(0.0)),
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
                            LobbyMenuAnimRightColumn,
                            UiTransform::from_translation(Val2::px(INTRO_SLIDE_RIGHT_PX, 0.0)),
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
                                UiTransform::default(),
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
                                BackgroundColor(accent_solo_cyan().with_alpha(0.0)),
                                BorderColor::all(solo_button_border_color().with_alpha(0.0)),
                            ))
                            .with_children(|btn| {
                                btn.spawn((
                                    Text::new("SOLO"),
                                    LobbyMenuAnimatedText(LobbyMenuTextRole::SoloPrimary),
                                    TextFont {
                                        font: font.clone(),
                                        font_size: 22.0,
                                        ..default()
                                    },
                                    TextColor(base_text_color(LobbyMenuTextRole::SoloPrimary).with_alpha(0.0)),
                                ));
                                btn.spawn((
                                    Text::new("Single player"),
                                    LobbyMenuAnimatedText(LobbyMenuTextRole::SoloSecondary),
                                    TextFont {
                                        font: font.clone(),
                                        font_size: 13.0,
                                        ..default()
                                    },
                                    TextColor(base_text_color(LobbyMenuTextRole::SoloSecondary).with_alpha(0.0)),
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
                        LobbyMenuAnimFooterBar,
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
                        BackgroundColor(menu_footer_bg().with_alpha(0.0)),
                    ))
                    .with_children(|footer| {
                        footer.spawn((
                            Text::new("GitHub   Discord   Web"),
                            LobbyMenuAnimatedText(LobbyMenuTextRole::FooterLinks),
                            TextFont {
                                font: font.clone(),
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(base_text_color(LobbyMenuTextRole::FooterLinks).with_alpha(0.0)),
                        ));
                        footer.spawn((
                            Text::new("NEWS"),
                            LobbyMenuAnimatedText(LobbyMenuTextRole::FooterNews),
                            TextFont {
                                font: font.clone(),
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(base_text_color(LobbyMenuTextRole::FooterNews).with_alpha(0.0)),
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

    commands.insert_resource(LobbyMainMenuAnim {
        phase: LobbyMenuAnimPhase::Entering,
        phase_time: 0.0,
        pending: PendingNavigate::None,
        clicked: None,
        play_main_menu_intro_after_cancel: false,
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
        UiTransform::default(),
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
        BackgroundColor(base.with_alpha(0.0)),
        BorderColor::all(panel_border().with_alpha(0.0)),
    ))
    .with_children(|btn| {
        let role = if matches!(kind, ComingSoonKind::Ranked) {
            LobbyMenuTextRole::StubRanked
        } else {
            LobbyMenuTextRole::StubSecondary
        };
        btn.spawn((
            Text::new(label),
            LobbyMenuAnimatedText(role),
            TextFont {
                font: font.clone(),
                font_size: 16.0,
                ..default()
            },
            TextColor(base_text_color(role).with_alpha(0.0)),
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
        (Entity, &Interaction, &mut BackgroundColor, &ComingSoonKind),
        (Changed<Interaction>, With<ComingSoonMenuButton>),
    >,
    mut anim: ResMut<LobbyMainMenuAnim>,
) {
    for (ent, interaction, mut bg, kind) in &mut q {
        if !anim.is_idle() {
            continue;
        }
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
                anim.phase = LobbyMenuAnimPhase::Exiting;
                anim.phase_time = 0.0;
                anim.pending = PendingNavigate::Stub;
                anim.clicked = Some(ent);
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
    mut preloaded: Option<ResMut<crate::MapCache>>,
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
                    
                    // Prefetch the map for this lobby using the unified MapCache
                    if let Some(ref mut cache) = preloaded {
                        cache.request(&lobby.map_name, &mut commands);
                    } else {
                        let mut cache = crate::MapCache::default();
                        cache.request(&lobby.map_name, &mut commands);
                        commands.insert_resource(cache);
                    }
                    
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
                        *bg = BackgroundColor(bg_color);
                        *bd = BorderColor::all(bd_color);
                        
                        if let Some(mut t) = card_title_q.iter_mut().next() {
                            t.0 = format!("SECTOR {}", lobby.id);
                        }
                        if let Some(mut t) = card_players_q.iter_mut().next() {
                            t.0 = format!("{}/{} players", lobby.num_players, lobby.max_players);
                        }
                        if let Some((mut t, mut tc)) = card_status_q.iter_mut().next() {
                            t.0 = status_text;
                            tc.0 = text_clr;
                        }
                    } else {
                        // Spawn if it doesn't exist
                        commands.entity(cards_ent).with_children(|p| {
                            p.spawn((
                                LobbyCardRoot,
                                Button,
                                JoinLobbyButton(lobby.id),
                                UiTransform::default(),
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
        (Entity, &Interaction, &mut BackgroundColor, &JoinLobbyButton),
        (Changed<Interaction>, With<Button>, With<LobbyCardRoot>),
    >,
    mut anim: ResMut<LobbyMainMenuAnim>,
) {
    for (ent, interaction, mut bg, btn) in interaction_q.iter_mut() {
        if !anim.is_idle() {
            continue;
        }
        match *interaction {
            Interaction::Pressed => {
                *bg = accent_solo_cyan_pressed().into();
                anim.phase = LobbyMenuAnimPhase::Exiting;
                anim.phase_time = 0.0;
                anim.pending = PendingNavigate::JoinLobby(btn.0);
                anim.clicked = Some(ent);
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
        (Entity, &Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<SinglePlayerButton>),
    >,
    mut anim: ResMut<LobbyMainMenuAnim>,
) {
    for (ent, interaction, mut bg) in interaction_q.iter_mut() {
        if !anim.is_idle() {
            continue;
        }
        match *interaction {
            Interaction::Pressed => {
                info!("🎮 Iniciando Single Player...");
                *bg = accent_solo_cyan_pressed().into();
                anim.phase = LobbyMenuAnimPhase::Exiting;
                anim.phase_time = 0.0;
                anim.pending = PendingNavigate::SinglePlayer;
                anim.clicked = Some(ent);
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
    mut anim: ResMut<LobbyMainMenuAnim>,
) {
    for interaction in interaction_q.iter_mut() {
        if *interaction == Interaction::Pressed {
            joined_lobby.0 = None;
            for mut sender in senders.iter_mut() {
                let _ = sender.send::<dark_rift_shared::protocol::ReliableChannel>(ClientLeaveMessage);
                info!("🚫 Cancelled join. Sending ClientLeaveMessage.");
            }
            anim.play_main_menu_intro_after_cancel = true;
        }
    }
}

fn insert_mock_single_player_start(commands: &mut Commands, settings: &ClientSettings) {
    let mut config = dark_rift_shared::game_config::GameConfig::default();

    if let Ok(content) = std::fs::read_to_string("assets/configs/default_single_player.ron") {
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
        map_data: None,
    };

    commands.insert_resource(crate::MockSinglePlayerStart(mock_msg));
}

/// Snap all animated chrome widgets back to theme defaults (used after cancel-from-waiting and stub bounce).
pub fn apply_lobby_main_menu_idle_visuals(q: &mut LobbyMenuAnimVisualQueries) {
    if let Ok(mut bg) = q.color_set.p0().single_mut() {
        *bg = menu_backdrop().into();
    }
    if let Ok(mut panel_node) = q.layout_node_set.p0().single_mut() {
        panel_node.display = Display::Flex;
    }
    if let Ok((mut pbg, mut pbd)) = q.color_set.p5().single_mut() {
        *pbg = menu_panel_bg().into();
        *pbd = BorderColor::all(menu_panel_border_glow());
    }
    if let Ok(mut h) = q.transform_set.p0().single_mut() {
        *h = UiTransform::default();
    }
    if let Ok(mut l) = q.transform_set.p1().single_mut() {
        *l = UiTransform::from_translation(Val2::px(0.0, 0.0));
    }
    if let Ok(mut r) = q.transform_set.p2().single_mut() {
        *r = UiTransform::from_translation(Val2::px(0.0, 0.0));
    }
    if let Ok(mut f) = q.color_set.p1().single_mut() {
        *f = menu_footer_bg().into();
    }
    if let Ok((mut nbg, mut nbd)) = q.color_set.p2().single_mut() {
        *nbg = nickname_field_bg().into();
        *nbd = BorderColor::all(nickname_field_border());
    }
    if let Ok((mut sbg, mut sbd)) = q.color_set.p3().single_mut() {
        *sbg = accent_solo_cyan().into();
        *sbd = BorderColor::all(solo_button_border_color());
    }
    if let Ok(mut str) = q.transform_set.p3().single_mut() {
        *str = UiTransform::default();
    }
    for (_, mut sbg, mut sbd, kind) in q.color_set.p4().iter_mut() {
        match kind {
            ComingSoonKind::Ranked => *sbg = accent_ranked_gold().into(),
            ComingSoonKind::Secondary => *sbg = menu_secondary_button().into(),
        }
        *sbd = BorderColor::all(panel_border());
    }
    for (_, mut tr) in q.transform_set.p4().iter_mut() {
        *tr = UiTransform::default();
    }
    for (_, mut tr) in q.transform_set.p5().iter_mut() {
        *tr = UiTransform::default();
    }
    for (mut tc, role) in q.anim_text.iter_mut() {
        tc.0 = base_text_color(role.0);
    }
}

const EXIT_SLIDE_LEFT_PX: f32 = -440.0;
const EXIT_SLIDE_RIGHT_PX: f32 = 440.0;
const EXIT_HEADER_Y_PX: f32 = -76.0;

#[inline]
fn lerp_f(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Drive intro / exit / stub re-entry motion for the lobby main chrome.
pub fn system_lobby_main_menu_anim(
    time: Res<Time>,
    mut anim: ResMut<LobbyMainMenuAnim>,
    mut commands: Commands,
    mut next_state: ResMut<NextState<crate::AppState>>,
    settings: Res<ClientSettings>,
    cli: Res<Cli>,
    mut joined: ResMut<JoinedLobby>,
    mut join_senders: Query<&mut MessageSender<ClientJoinMessage>>,
    mut q: LobbyMenuAnimVisualQueries,
) {
    let dt = time.delta_secs();
    if anim.play_main_menu_intro_after_cancel {
        anim.play_main_menu_intro_after_cancel = false;
        if let Ok(mut panel_node) = q.layout_node_set.p0().single_mut() {
            panel_node.display = Display::Flex;
        }
        if let Ok(mut n) = q.layout_node_set.p1().single_mut() {
            n.display = Display::None;
        }
        if let Ok(mut n) = q.layout_node_set.p2().single_mut() {
            n.display = Display::Flex;
        }
        apply_intro_frame(0.0, &mut q);
        reset_main_menu_click_transforms(&mut q);
        anim.phase = LobbyMenuAnimPhase::Entering;
        anim.phase_time = 0.0;
        anim.pending = PendingNavigate::None;
        anim.clicked = None;
    }

    match anim.phase {
        LobbyMenuAnimPhase::Entering => {
            anim.phase_time += dt;
            let u = (anim.phase_time / MENU_INTRO_DURATION).min(1.0);
            let e = ease_out_cubic(u);
            apply_intro_frame(e, &mut q);
            if u >= 1.0 {
                anim.phase = LobbyMenuAnimPhase::Idle;
                anim.phase_time = 0.0;
            }
        }
        LobbyMenuAnimPhase::Idle => {}
        LobbyMenuAnimPhase::Exiting => {
            anim.phase_time += dt;
            let u = (anim.phase_time / MENU_EXIT_DURATION).min(1.0);
            let exit_e = ease_in_out_cubic(u);
            let fade = 1.0 - smoothstep(0.06, 1.0, u);
            let scale_u = (u / 0.28).min(1.0);
            let click_scale = 1.0 + 0.12 * ease_out_cubic(scale_u);

            if let Ok(mut l) = q.transform_set.p1().single_mut() {
                *l = UiTransform::from_translation(Val2::px(
                    lerp_f(0.0, EXIT_SLIDE_LEFT_PX, exit_e),
                    0.0,
                ));
            }
            if let Ok(mut r) = q.transform_set.p2().single_mut() {
                *r = UiTransform::from_translation(Val2::px(
                    lerp_f(0.0, EXIT_SLIDE_RIGHT_PX, exit_e),
                    0.0,
                ));
            }
            if let Ok(mut h) = q.transform_set.p0().single_mut() {
                *h = UiTransform::from_translation(Val2::px(0.0, lerp_f(0.0, EXIT_HEADER_Y_PX, exit_e)));
            }

            if let Some(ce) = anim.clicked {
                set_clicked_menu_scale(&mut q, ce, Vec2::splat(click_scale));
            }

            if let Ok(mut bg) = q.color_set.p0().single_mut() {
                *bg = menu_backdrop()
                    .with_alpha(menu_backdrop().alpha() * fade)
                    .into();
            }
            if let Ok((mut pbg, mut pbd)) = q.color_set.p5().single_mut() {
                *pbg = menu_panel_bg()
                    .with_alpha(menu_panel_bg().alpha() * fade)
                    .into();
                *pbd = BorderColor::all(
                    menu_panel_border_glow()
                        .with_alpha(menu_panel_border_glow().alpha() * fade),
                );
            }
            if let Ok(mut f) = q.color_set.p1().single_mut() {
                *f = menu_footer_bg()
                    .with_alpha(menu_footer_bg().alpha() * fade)
                    .into();
            }
            if let Ok((mut nbg, mut nbd)) = q.color_set.p2().single_mut() {
                *nbg = nickname_field_bg()
                    .with_alpha(nickname_field_bg().alpha() * fade)
                    .into();
                *nbd = BorderColor::all(
                    nickname_field_border()
                        .with_alpha(nickname_field_border().alpha() * fade),
                );
            }
            if let Ok((mut sbg, mut sbd)) = q.color_set.p3().single_mut() {
                *sbg = accent_solo_cyan()
                    .with_alpha(accent_solo_cyan().alpha() * fade)
                    .into();
                *sbd = BorderColor::all(
                    solo_button_border_color()
                        .with_alpha(solo_button_border_color().alpha() * fade),
                );
            }
            for (_, mut sbg, mut sbd, kind) in q.color_set.p4().iter_mut() {
                let base = match kind {
                    ComingSoonKind::Ranked => accent_ranked_gold(),
                    ComingSoonKind::Secondary => menu_secondary_button(),
                };
                *sbg = base.with_alpha(base.alpha() * fade).into();
                *sbd = BorderColor::all(panel_border().with_alpha(panel_border().alpha() * fade));
            }
            for (mut tc, role) in q.anim_text.iter_mut() {
                let b = base_text_color(role.0);
                tc.0 = b.with_alpha(b.alpha() * fade);
            }

            if u < 1.0 {
                return;
            }

            match anim.pending {
                PendingNavigate::SinglePlayer => {
                    insert_mock_single_player_start(&mut commands, &settings);
                    next_state.set(crate::AppState::InGame);
                }
                PendingNavigate::JoinLobby(id) => {
                    let name = settings.config.display_nickname_for_play();
                    for mut sender in join_senders.iter_mut() {
                        let msg = ClientJoinMessage {
                            name: name.clone(),
                            is_observer: cli.observer,
                            target_lobby_id: Some(id),
                        };
                        let _ = sender.send::<ReliableChannel>(msg);
                        info!("✅ Requested to join Lobby ID {} (after menu anim)", id);
                    }
                    joined.0 = Some(id);
                    apply_lobby_waiting_overlay_layout(&mut q);
                }
                PendingNavigate::Stub => {
                    anim.phase = LobbyMenuAnimPhase::StubReentering;
                    anim.phase_time = 0.0;
                    anim.pending = PendingNavigate::None;
                    return;
                }
                PendingNavigate::None => {}
            }
            anim.phase = LobbyMenuAnimPhase::Idle;
            anim.phase_time = 0.0;
            anim.pending = PendingNavigate::None;
            anim.clicked = None;
        }
        LobbyMenuAnimPhase::StubReentering => {
            anim.phase_time += dt;
            let u = (anim.phase_time / MENU_STUB_REENTER_DURATION).min(1.0);
            let e = ease_in_out_cubic(u);
            let slide_l = lerp_f(EXIT_SLIDE_LEFT_PX, 0.0, e);
            let slide_r = lerp_f(EXIT_SLIDE_RIGHT_PX, 0.0, e);
            let slide_h = lerp_f(EXIT_HEADER_Y_PX, 0.0, e);
            if let Ok(mut l) = q.transform_set.p1().single_mut() {
                *l = UiTransform::from_translation(Val2::px(slide_l, 0.0));
            }
            if let Ok(mut r) = q.transform_set.p2().single_mut() {
                *r = UiTransform::from_translation(Val2::px(slide_r, 0.0));
            }
            if let Ok(mut h) = q.transform_set.p0().single_mut() {
                *h = UiTransform::from_translation(Val2::px(0.0, slide_h));
            }
            if let Some(ce) = anim.clicked {
                set_clicked_menu_scale(
                    &mut q,
                    ce,
                    Vec2::splat(lerp_f(click_scale_at_exit_end(), 1.0, e)),
                );
            }
            if let Ok(mut bg) = q.color_set.p0().single_mut() {
                *bg = menu_backdrop()
                    .with_alpha(menu_backdrop().alpha() * e)
                    .into();
            }
            if let Ok((mut pbg, mut pbd)) = q.color_set.p5().single_mut() {
                *pbg = menu_panel_bg()
                    .with_alpha(menu_panel_bg().alpha() * e)
                    .into();
                *pbd = BorderColor::all(
                    menu_panel_border_glow()
                        .with_alpha(menu_panel_border_glow().alpha() * e),
                );
            }
            if let Ok(mut f) = q.color_set.p1().single_mut() {
                *f = menu_footer_bg()
                    .with_alpha(menu_footer_bg().alpha() * e)
                    .into();
            }
            if let Ok((mut nbg, mut nbd)) = q.color_set.p2().single_mut() {
                *nbg = nickname_field_bg()
                    .with_alpha(nickname_field_bg().alpha() * e)
                    .into();
                *nbd = BorderColor::all(
                    nickname_field_border()
                        .with_alpha(nickname_field_border().alpha() * e),
                );
            }
            if let Ok((mut sbg, mut sbd)) = q.color_set.p3().single_mut() {
                *sbg = accent_solo_cyan()
                    .with_alpha(accent_solo_cyan().alpha() * e)
                    .into();
                *sbd = BorderColor::all(
                    solo_button_border_color()
                        .with_alpha(solo_button_border_color().alpha() * e),
                );
            }
            if let Ok(mut str) = q.transform_set.p3().single_mut() {
                *str = UiTransform::default();
            }
            for (_, mut sbg, mut sbd, kind) in q.color_set.p4().iter_mut() {
                let base = match kind {
                    ComingSoonKind::Ranked => accent_ranked_gold(),
                    ComingSoonKind::Secondary => menu_secondary_button(),
                };
                *sbg = base.with_alpha(base.alpha() * e).into();
                *sbd = BorderColor::all(panel_border().with_alpha(panel_border().alpha() * e));
            }
            for (ent, mut str) in q.transform_set.p4().iter_mut() {
                if anim.clicked != Some(ent) {
                    *str = UiTransform::default();
                }
            }
            for (mut tc, role) in q.anim_text.iter_mut() {
                let b = base_text_color(role.0);
                tc.0 = b.with_alpha(b.alpha() * e);
            }
            if u < 1.0 {
                return;
            }
            apply_lobby_main_menu_idle_visuals(&mut q);
            anim.phase = LobbyMenuAnimPhase::Idle;
            anim.phase_time = 0.0;
            anim.clicked = None;
        }
    }
}

#[inline]
fn click_scale_at_exit_end() -> f32 {
    1.0 + 0.12
}

fn apply_intro_frame(e: f32, q: &mut LobbyMenuAnimVisualQueries) {
    if let Ok(mut bg) = q.color_set.p0().single_mut() {
        *bg = menu_backdrop()
            .with_alpha(menu_backdrop().alpha() * e)
            .into();
    }
    if let Ok((mut pbg, mut pbd)) = q.color_set.p5().single_mut() {
        *pbg = menu_panel_bg()
            .with_alpha(menu_panel_bg().alpha() * e)
            .into();
        *pbd = BorderColor::all(
            menu_panel_border_glow()
                .with_alpha(menu_panel_border_glow().alpha() * e),
        );
    }
    if let Ok(mut h) = q.transform_set.p0().single_mut() {
        *h = UiTransform::from_translation(Val2::px(
            0.0,
            lerp_f(INTRO_HEADER_Y_PX, 0.0, e),
        ));
    }
    if let Ok(mut l) = q.transform_set.p1().single_mut() {
        *l = UiTransform::from_translation(Val2::px(
            lerp_f(INTRO_SLIDE_LEFT_PX, 0.0, e),
            0.0,
        ));
    }
    if let Ok(mut r) = q.transform_set.p2().single_mut() {
        *r = UiTransform::from_translation(Val2::px(
            lerp_f(INTRO_SLIDE_RIGHT_PX, 0.0, e),
            0.0,
        ));
    }
    if let Ok(mut f) = q.color_set.p1().single_mut() {
        *f = menu_footer_bg()
            .with_alpha(menu_footer_bg().alpha() * e)
            .into();
    }
    if let Ok((mut nbg, mut nbd)) = q.color_set.p2().single_mut() {
        *nbg = nickname_field_bg()
            .with_alpha(nickname_field_bg().alpha() * e)
            .into();
        *nbd = BorderColor::all(
            nickname_field_border()
                .with_alpha(nickname_field_border().alpha() * e),
        );
    }
    if let Ok((mut sbg, mut sbd)) = q.color_set.p3().single_mut() {
        *sbg = accent_solo_cyan()
            .with_alpha(accent_solo_cyan().alpha() * e)
            .into();
        *sbd = BorderColor::all(
            solo_button_border_color()
                .with_alpha(solo_button_border_color().alpha() * e),
        );
    }
    for (_, mut sbg, mut sbd, kind) in q.color_set.p4().iter_mut() {
        let base = match kind {
            ComingSoonKind::Ranked => accent_ranked_gold(),
            ComingSoonKind::Secondary => menu_secondary_button(),
        };
        *sbg = base.with_alpha(base.alpha() * e).into();
        *sbd = BorderColor::all(panel_border().with_alpha(panel_border().alpha() * e));
    }
    for (mut tc, role) in q.anim_text.iter_mut() {
        let b = base_text_color(role.0);
        tc.0 = b.with_alpha(b.alpha() * e);
    }
}

pub fn system_cleanup_lobby_browser(mut commands: Commands, q_root: Query<Entity, With<LobbyBrowserRoot>>) {
    if let Some(ent) = q_root.iter().next() {
        commands.entity(ent).despawn();
    }
    commands.remove_resource::<LobbyMainMenuAnim>();
}
