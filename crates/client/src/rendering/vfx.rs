use bevy::prelude::*;
use crate::*;

pub fn setup_attack_ring_mesh(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    settings: Res<ClientSettings>,
) {
    let s = clamp_ui_scale(settings.config.ui.click_feedback_scale);
    let mesh = Annulus::new(5.0 * s, 8.8 * s)
        .mesh()
        .resolution(56)
        .build();
    commands.insert_resource(AttackRingMesh(meshes.add(mesh)));
}

pub fn setup_fleet_visual_meshes(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    settings: Res<ClientSettings>,
) {
    let sb = clamp_ui_scale(settings.config.ui.fleet_boat_scale);
    let sw = clamp_ui_scale(settings.config.ui.fleet_wake_scale);
    let st = clamp_ui_scale(settings.config.ui.fleet_target_ring_scale);
    let boat = Annulus::new(0.35 * sb, 2.9 * sb)
        .mesh()
        .resolution(28)
        .build();
    let wake_dot = Annulus::new(0.1 * sw, 1.15 * sw)
        .mesh()
        .resolution(14)
        .build();
    let target_ring = Annulus::new(4.0 * st, 7.4 * st)
        .mesh()
        .resolution(52)
        .build();
    commands.insert_resource(FleetVisualMeshes {
        boat: meshes.add(boat),
        wake_dot: meshes.add(wake_dot),
        target_ring: meshes.add(target_ring),
    });
}

pub fn setup_structure_marker_meshes(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    let place_ring = Annulus::new(0.12, 1.35)
        .mesh()
        .resolution(20)
        .build();
    let sam_r = config::SAM_RANGE_TILES as f32 * TILE_SIZE;
    let sam_range = Annulus::new((sam_r - 0.28).max(0.12), sam_r + 0.28)
        .mesh()
        .resolution(36)
        .build();
    let def_r = config::DEFENSE_POST_RANGE as f32 * TILE_SIZE;
    let defense_range = Annulus::new((def_r - 0.22).max(0.12), def_r + 0.22)
        .mesh()
        .resolution(30)
        .build();
    commands.insert_resource(StructureMarkerMeshes {
        place_ring: meshes.add(place_ring),
        sam_range: meshes.add(sam_range),
        defense_range: meshes.add(defense_range),
    });
}

pub fn impact_particle_effect() -> Particle2dEffect {
    Particle2dEffect {
        spawn_rate: 80.0,
        spawn_amount: 3,
        emission_shape: EmissionShape::Circle(5.0),
        lifetime: Rval::new(0.28, 0.06),
        linear_speed: Some(Rval(90.0, 0.35)),
        direction: Some(Rval(Vec2::Y, 1.0)),
        scale: Some(Rval(3.0, 0.35)),
        linear_damp: Some(Rval(3.5, 0.15)),
        ..Default::default()
    }
}

pub fn setup_combat_vfx(mut commands: Commands, mut effects: ResMut<Assets<Particle2dEffect>>) {
    let handle = effects.add(impact_particle_effect());
    commands.insert_resource(CombatVfxCache(handle));
}