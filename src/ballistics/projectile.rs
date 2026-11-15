//! Снаряды: полёт, столкновение с вокселями (DDA) и плитами брони, журнал попаданий.

use std::collections::VecDeque;

use bevy::prelude::*;

use super::geometry::ray_plate;
use super::penetration::{ImpactOutcome, evaluate_impact};
use super::shell::ShellSpec;
use crate::core::{GameState, Team};
use crate::tank::{ArmorOwner, ArmorZone};
use crate::voxel::{Debris, Material, Rng, VoxelAssets, VoxelWorld, spawn_debris_burst};

pub const GRAVITY: f32 = 9.81;
/// Время жизни снаряда, с.
pub const SHELL_LIFETIME: f32 = 10.0;
/// Максимум рикошетов.
pub const MAX_BOUNCES: u8 = 3;
/// Радиус воронки в постройках, м.
pub const CRATER_RADIUS: f32 = 0.6;

/// Летящий снаряд.
#[derive(Component, Debug, Clone)]
pub struct Projectile {
    pub velocity: Vec3,
    pub spec: ShellSpec,
    pub shooter: Option<Entity>,
    pub shooter_team: Option<Team>,
    pub traveled: f32,
    pub age: f32,
    pub bounces: u8,
}

/// Плита брони (локальная +Z = нормаль; попадание двустороннее).
#[derive(Component, Debug, Clone, Copy)]
pub struct ArmorPlate {
    pub thickness_mm: f32,
    pub half_extents: Vec2,
}

/// Описание мишени для журнала.
#[derive(Component, Debug, Clone)]
pub struct TargetInfo {
    pub name: String,
}

/// Событие (Bevy Message) попадания снаряда в бронеплиту — обрабатывается системой урона.
#[derive(Message, Debug, Clone)]
pub struct ArmorImpactMessage {
    pub plate_entity: Entity,
    pub owner_tank: Option<Entity>,
    pub zone: Option<ArmorZone>,
    pub target_name: String,
    pub shooter: Option<Entity>,
    pub shooter_team: Option<Team>,
    pub hit_point: Vec3,
    pub hit_normal: Vec3,
    pub ray_dir: Vec3,
    pub dist_m: f32,
    pub angle_deg: f32,
    pub armor_thickness_mm: f32,
    pub shell: ShellSpec,
    pub outcome: ImpactOutcome,
}

/// Событие попадания снаряда в воксели карты (постройка или рельеф).
#[derive(Message, Debug, Clone)]
pub struct TerrainImpactMessage {
    pub point: Vec3,
    pub normal: Vec3,
    pub material: Material,
    pub destructible: bool,
    pub removed_count: usize,
    pub caliber_mm: f32,
    pub shooter_team: Option<Team>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitKind {
    Ricochet,
    NoPenetration,
    Penetration,
    Structure,
    Terrain,
}

#[derive(Debug, Clone)]
pub struct HitRecord {
    pub text: String,
    pub kind: HitKind,
    pub age: f32,
}

/// Журнал попаданий для HUD.
#[derive(Resource, Debug, Default)]
pub struct HitLog {
    pub entries: VecDeque<HitRecord>,
    pub shots: u32,
    pub penetrations: u32,
}

impl HitLog {
    pub const MAX_ENTRIES: usize = 6;
    pub const ENTRY_LIFETIME: f32 = 8.0;

    pub fn push(&mut self, kind: HitKind, text: String) {
        if kind == HitKind::Penetration {
            self.penetrations += 1;
        }
        self.entries.push_back(HitRecord {
            text,
            kind,
            age: 0.0,
        });
        while self.entries.len() > Self::MAX_ENTRIES {
            self.entries.pop_front();
        }
    }
}

/// Создаёт снаряд.
pub fn spawn_projectile(
    commands: &mut Commands,
    assets: &VoxelAssets,
    origin: Vec3,
    dir: Vec3,
    spec: ShellSpec,
    shooter: Option<Entity>,
    shooter_team: Option<Team>,
) {
    let dir = dir.normalize_or_zero();
    commands.spawn((
        Projectile {
            velocity: dir * spec.muzzle_velocity,
            spec,
            shooter,
            shooter_team,
            traveled: 0.0,
            age: 0.0,
            bounces: 0,
        },
        Mesh3d(assets.cube.clone()),
        MeshMaterial3d(assets.tracer.clone()),
        Transform::from_translation(origin)
            .looking_to(dir, Vec3::Y)
            .with_scale(Vec3::new(0.12, 0.12, 1.2)),
        DespawnOnExit(GameState::InBattle),
    ));
}

pub struct BallisticsPlugin;

impl Plugin for BallisticsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HitLog>()
            .add_message::<ArmorImpactMessage>()
            .add_message::<TerrainImpactMessage>()
            .add_systems(
                FixedUpdate,
                step_projectiles
                    .run_if(in_state(GameState::InBattle))
                    .run_if(resource_exists::<VoxelWorld>),
            )
            .add_systems(Update, age_hit_log);
    }
}

fn age_hit_log(time: Res<Time>, mut log: ResMut<HitLog>) {
    let dt = time.delta_secs();
    for e in log.entries.iter_mut() {
        e.age += dt;
    }
    while log
        .entries
        .front()
        .is_some_and(|e| e.age > HitLog::ENTRY_LIFETIME)
    {
        log.entries.pop_front();
    }
}

enum Impact {
    Plate {
        t: f32,
        plate_entity: Entity,
        owner: Option<ArmorOwner>,
        point: Vec3,
        normal: Vec3,
        angle: f32,
        thickness: f32,
        name: String,
    },
    Voxel {
        t: f32,
        hit: crate::voxel::VoxelHit,
    },
}

impl Impact {
    fn t(&self) -> f32 {
        match self {
            Impact::Plate { t, .. } | Impact::Voxel { t, .. } => *t,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn step_projectiles(
    mut commands: Commands,
    time: Res<Time>,
    mut world: ResMut<VoxelWorld>,
    plates: Query<(Entity, &GlobalTransform, &ArmorPlate, &TargetInfo, Option<&ArmorOwner>)>,
    mut shells: Query<(Entity, &mut Projectile, &mut Transform)>,
    debris: Query<(), With<Debris>>,
    assets: Option<Res<VoxelAssets>>,
    mut log: ResMut<HitLog>,
    mut impacts: MessageWriter<ArmorImpactMessage>,
    mut terrain_impacts: MessageWriter<TerrainImpactMessage>,
    mut seed: Local<u64>,
) {
    let Some(assets) = assets else { return };
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    *seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut rng = Rng::new(*seed | 1);
    let mut debris_count = debris.iter().count();

    for (entity, mut shell, mut tf) in &mut shells {
        shell.age += dt;
        if shell.age > SHELL_LIFETIME || tf.translation.y < -50.0 {
            commands.entity(entity).despawn();
            continue;
        }
        shell.velocity.y -= GRAVITY * dt;
        let seg = shell.velocity * dt;
        let len = seg.length();
        if len < 1e-6 {
            continue;
        }
        let dir = seg / len;
        let origin = tf.translation;

        let mut best: Option<Impact> = world
            .grid
            .raycast(origin, dir, len)
            .map(|hit| Impact::Voxel { t: hit.distance, hit });

        for (plate_entity, gt, plate, info, owner) in &plates {
            // Первый пролёт после выстрела не задевает собственную броню стрелка.
            if shell.bounces == 0
                && owner.is_some_and(|o| Some(o.tank) == shell.shooter)
            {
                continue;
            }
            let (_, rot, pos) = gt.to_scale_rotation_translation();
            if let Some(h) = ray_plate(origin, dir, len, pos, rot, plate.half_extents) {
                if best.as_ref().is_none_or(|b| h.t < b.t()) {
                    best = Some(Impact::Plate {
                        t: h.t,
                        plate_entity,
                        owner: owner.copied(),
                        point: h.point,
                        normal: h.normal,
                        angle: h.angle_deg,
                        thickness: plate.thickness_mm,
                        name: info.name.clone(),
                    });
                }
            }
        }

        let Some(impact) = best else {
            shell.traveled += len;
            tf.translation = origin + seg;
            tf.rotation = Transform::IDENTITY.looking_to(dir, Vec3::Y).rotation;
            continue;
        };

        let dist = shell.traveled + impact.t();
        match impact {
            Impact::Plate {
                plate_entity,
                owner,
                point,
                normal,
                angle,
                thickness,
                name,
                ..
            } => {
                let outcome = evaluate_impact(&shell.spec, dist, thickness, angle);
                let (kind, text) = match outcome {
                    ImpactOutcome::Ricochet => (
                        HitKind::Ricochet,
                        format!("{name}: РИКОШЕТ (угол {angle:.0}°, {dist:.0} м)"),
                    ),
                    ImpactOutcome::NoPenetration { effective_mm } => (
                        HitKind::NoPenetration,
                        format!(
                            "{name}: НЕ ПРОБИЛ (эфф. {effective_mm:.0} мм, угол {angle:.0}°, {dist:.0} м)"
                        ),
                    ),
                    ImpactOutcome::Penetration { effective_mm } => (
                        HitKind::Penetration,
                        format!(
                            "{name}: ПРОБИТИЕ (эфф. {effective_mm:.0} мм, угол {angle:.0}°, {dist:.0} м)"
                        ),
                    ),
                };
                if shell.shooter_team != Some(Team::Enemy) || owner.is_some() {
                    log.push(kind, text);
                }

                impacts.write(ArmorImpactMessage {
                    plate_entity,
                    owner_tank: owner.map(|o| o.tank),
                    zone: owner.map(|o| o.zone),
                    target_name: name,
                    shooter: shell.shooter,
                    shooter_team: shell.shooter_team,
                    hit_point: point,
                    hit_normal: normal,
                    ray_dir: dir,
                    dist_m: dist,
                    angle_deg: angle,
                    armor_thickness_mm: thickness,
                    shell: shell.spec,
                    outcome,
                });

                debris_count += 3;
                spawn_debris_burst(
                    &mut commands,
                    &assets,
                    debris_count,
                    point,
                    normal,
                    Material::TargetSteel,
                    3,
                    0.22,
                    &mut rng,
                );
                if outcome == ImpactOutcome::Ricochet && shell.bounces < MAX_BOUNCES {
                    let v = shell.velocity;
                    shell.velocity = (v - 2.0 * v.dot(normal) * normal) * 0.6;
                    shell.bounces += 1;
                    shell.traveled = dist;
                    tf.translation = point + normal * 0.05;
                    let nd = shell.velocity.normalize_or_zero();
                    tf.rotation = Transform::IDENTITY.looking_to(nd, Vec3::Y).rotation;
                } else {
                    commands.entity(entity).despawn();
                }
            }
            Impact::Voxel { hit, .. } => {
                if hit.material.is_destructible() {
                    let res = world.carve_sphere(hit.point + dir * 0.25, CRATER_RADIUS);
                    let removed_count = res.removed.len();
                    if shell.shooter_team != Some(Team::Enemy) {
                        log.push(
                            HitKind::Structure,
                            format!(
                                "Постройка ({:?}): разрушено {} вокселей",
                                hit.material,
                                removed_count
                            ),
                        );
                    }
                    terrain_impacts.write(TerrainImpactMessage {
                        point: hit.point,
                        normal: hit.normal,
                        material: hit.material,
                        destructible: true,
                        removed_count,
                        caliber_mm: shell.spec.caliber_mm,
                        shooter_team: shell.shooter_team,
                    });
                    let step = (removed_count / 30).max(1);
                    for (v, m) in res.removed.iter().step_by(step) {
                        debris_count += 1;
                        let p = world.grid.voxel_center(*v);
                        let vs = world.grid.voxel_size;
                        spawn_debris_burst(
                            &mut commands,
                            &assets,
                            debris_count,
                            p,
                            hit.normal,
                            *m,
                            1,
                            vs,
                            &mut rng,
                        );
                    }
                } else {
                    if shell.shooter_team != Some(Team::Enemy) {
                        log.push(HitKind::Terrain, "Рельеф: снаряд застрял".to_string());
                    }
                    terrain_impacts.write(TerrainImpactMessage {
                        point: hit.point,
                        normal: hit.normal,
                        material: hit.material,
                        destructible: false,
                        removed_count: 0,
                        caliber_mm: shell.spec.caliber_mm,
                        shooter_team: shell.shooter_team,
                    });
                    debris_count += 6;
                    spawn_debris_burst(
                        &mut commands,
                        &assets,
                        debris_count,
                        hit.point,
                        hit.normal,
                        hit.material,
                        6,
                        world.grid.voxel_size,
                        &mut rng,
                    );
                }
                commands.entity(entity).despawn();
            }
        }
    }
}
