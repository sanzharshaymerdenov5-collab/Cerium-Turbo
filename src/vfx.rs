//! Визуальные эффекты и боевые анимации:
//! - Башня при подрыве БК (реалистичный баллистический полёт, вращение, удар о грунт и отскок);
//! - Откат/накат ствола и отдача камеры;
//! - Дульная вспышка, боковые газы дульного тормоза, искры и пороховой дым;
//! - Искры рикошета, непробития, прожиг пробития и взрыв/пыль по вокселям;
//! - Анимация пожара (поднимающиеся языки пламени, искры, чёрный дым и мерцающий свет);
//! - Выхлоп двигателя и пыль/снег из-под гусениц на ходу.

#![allow(clippy::type_complexity)]

use std::f32::consts::FRAC_PI_2;

use bevy::prelude::*;

use crate::ballistics::{ArmorImpactMessage, TerrainImpactMessage, penetration::ImpactOutcome};
use crate::core::GameState;
use crate::damage::{TankActionKind, TankActionMessage, TankDestroyedMessage, TankModules};
use crate::tank::{
    BarrelMesh, CameraShake, GunPivot, PlayerTank, TankCombat, TankFiredMessage, TankRoot,
    TankState, TurretPivot, TurretShape,
};
use crate::voxel::{Material, Rng, VoxelWorld};

/// Лимит одновременно активных 3D-частиц VFX (плавный FPS в WebGL2/WASM).
pub const MAX_VFX_PARTICLES: usize = 360;

/// Событие падения сорванной башни на грунт (для звукового эффекта удара).
#[derive(Message, Debug, Clone)]
pub struct TurretLandedMessage {
    pub pos: Vec3,
    pub impact_speed: f32,
}

/// Общие меши и материалы для боевых эффектов.
#[derive(Resource)]
pub struct VfxAssets {
    pub cube: Handle<Mesh>,
    pub flash_bright: Handle<StandardMaterial>,
    pub flame_yellow: Handle<StandardMaterial>,
    pub flame_orange: Handle<StandardMaterial>,
    pub flame_red: Handle<StandardMaterial>,
    pub spark_ricochet: Handle<StandardMaterial>,
    pub smoke_light: Handle<StandardMaterial>,
    pub smoke_dark: Handle<StandardMaterial>,
    pub dust_sand: Handle<StandardMaterial>,
    pub dust_snow: Handle<StandardMaterial>,
    pub shockwave: Handle<StandardMaterial>,
}

impl FromWorld for VfxAssets {
    fn from_world(world: &mut World) -> Self {
        let cube = world
            .resource_mut::<Assets<Mesh>>()
            .add(Cuboid::new(1.0, 1.0, 1.0));
        let mut mats = world.resource_mut::<Assets<StandardMaterial>>();
        let flash_bright = mats.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.96, 0.72),
            emissive: LinearRgba::new(11.0, 7.2, 2.2, 1.0),
            unlit: true,
            ..default()
        });
        let flame_yellow = mats.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.85, 0.25),
            emissive: LinearRgba::new(7.5, 4.4, 0.65, 1.0),
            unlit: true,
            ..default()
        });
        let flame_orange = mats.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.45, 0.10),
            emissive: LinearRgba::new(5.8, 1.7, 0.18, 1.0),
            unlit: true,
            ..default()
        });
        let flame_red = mats.add(StandardMaterial {
            base_color: Color::srgb(0.88, 0.20, 0.06),
            emissive: LinearRgba::new(3.6, 0.45, 0.06, 1.0),
            unlit: true,
            ..default()
        });
        let spark_ricochet = mats.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.88, 0.42),
            emissive: LinearRgba::new(10.0, 5.8, 1.4, 1.0),
            unlit: true,
            ..default()
        });
        let smoke_light = mats.add(StandardMaterial {
            base_color: Color::srgba(0.72, 0.71, 0.68, 0.68),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 1.0,
            ..default()
        });
        let smoke_dark = mats.add(StandardMaterial {
            base_color: Color::srgba(0.14, 0.14, 0.15, 0.82),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 1.0,
            ..default()
        });
        let dust_sand = mats.add(StandardMaterial {
            base_color: Color::srgba(0.76, 0.65, 0.44, 0.72),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 1.0,
            ..default()
        });
        let dust_snow = mats.add(StandardMaterial {
            base_color: Color::srgba(0.92, 0.95, 0.98, 0.72),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 1.0,
            ..default()
        });
        let shockwave = mats.add(StandardMaterial {
            base_color: Color::srgba(1.0, 0.68, 0.25, 0.55),
            emissive: LinearRgba::new(5.0, 2.4, 0.5, 1.0),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        });
        Self {
            cube,
            flash_bright,
            flame_yellow,
            flame_orange,
            flame_red,
            spark_ricochet,
            smoke_light,
            smoke_dark,
            dust_sand,
            dust_snow,
            shockwave,
        }
    }
}

/// 3D-частица вспышки, искры, огня или дыма.
#[derive(Component, Debug)]
pub struct VfxParticle {
    pub velocity: Vec3,
    pub drag: f32,
    pub gravity: f32,
    pub spin: Vec3,
    pub age: f32,
    pub lifetime: f32,
    pub start_scale: Vec3,
    pub end_scale: Vec3,
    pub align_to_vel: bool,
}

/// Кратковременный точечный свет вспышки выстрела/взрыва.
#[derive(Component, Debug)]
pub struct VfxFlashLight {
    pub age: f32,
    pub lifetime: f32,
    pub peak_intensity: f32,
}

/// Динамический мерцающий свет пожара на корпусе горящего танка.
#[derive(Component, Debug)]
pub struct TankFireLight {
    pub tank: Entity,
    pub phase: f32,
}

/// Анимация отката и гидравлического наката ствола.
#[derive(Component, Debug)]
pub struct BarrelRecoil {
    pub peak_offset: f32,
    pub elapsed: f32,
    pub kick_dur: f32,
    pub recover_dur: f32,
}

/// Физика сорванной при детонации БК башни (в мировых координатах).
#[derive(Component, Debug)]
pub struct ThrownTurret {
    pub tank: Entity,
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
    pub turret_height: f32,
    pub turret_radius: f32,
    pub bounces: u8,
    pub settled: bool,
    pub trail_acc: f32,
    pub post_smoke_timer: f32,
}

/// Остаточный дым и тлеющие угли подбитого танка.
#[derive(Component, Debug)]
pub struct TankWreckSmoke {
    pub remaining: f32,
    pub acc: f32,
    pub intense: bool,
}

/// Таймеры периодических эффектов пожара и езды на танке.
#[derive(Component, Debug, Default)]
pub struct TankVfxTimers {
    pub fire_acc: f32,
    pub drive_acc: f32,
}

pub struct VfxPlugin;

impl Plugin for VfxPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<TurretLandedMessage>()
            .add_systems(
                Update,
                (
                    handle_tank_fired_vfx,
                    handle_armor_impact_vfx,
                    handle_terrain_impact_vfx,
                    handle_tank_destroyed_vfx,
                    handle_tank_action_vfx,
                    animate_barrel_recoil,
                    animate_detonated_turrets,
                    animate_tank_fires,
                    animate_tank_drive_and_wreck_vfx,
                    update_vfx_particles_and_lights,
                )
                    .chain()
                    .run_if(in_state(GameState::InBattle)),
            );

        if app.world().contains_resource::<Assets<StandardMaterial>>()
            && app.world().contains_resource::<Assets<Mesh>>()
        {
            app.init_resource::<VfxAssets>();
        }
    }

    fn finish(&self, app: &mut App) {
        if !app.world().contains_resource::<VfxAssets>()
            && app.world().contains_resource::<Assets<StandardMaterial>>()
            && app.world().contains_resource::<Assets<Mesh>>()
        {
            app.init_resource::<VfxAssets>();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_particle(
    commands: &mut Commands,
    assets: &VfxAssets,
    count_guard: &mut usize,
    pos: Vec3,
    velocity: Vec3,
    drag: f32,
    gravity: f32,
    spin: Vec3,
    lifetime: f32,
    start_scale: Vec3,
    end_scale: Vec3,
    align_to_vel: bool,
    mat: Handle<StandardMaterial>,
) {
    if *count_guard >= MAX_VFX_PARTICLES {
        return;
    }
    *count_guard += 1;
    let rot = if align_to_vel && velocity.length_squared() > 1e-4 {
        Transform::IDENTITY
            .looking_to(velocity.normalize(), Vec3::Y)
            .rotation
    } else {
        Quat::from_euler(
            EulerRot::YXZ,
            spin.y * 0.3,
            spin.x * 0.25,
            spin.z * 0.35,
        )
    };
    commands.spawn((
        VfxParticle {
            velocity,
            drag,
            gravity,
            spin,
            age: 0.0,
            lifetime: lifetime.max(0.04),
            start_scale,
            end_scale,
            align_to_vel,
        },
        Mesh3d(assets.cube.clone()),
        MeshMaterial3d(mat),
        Transform {
            translation: pos,
            rotation: rot,
            scale: start_scale,
        },
        DespawnOnExit(GameState::InBattle),
    ));
}

fn spawn_flash_light(
    commands: &mut Commands,
    pos: Vec3,
    color: Color,
    peak_intensity: f32,
    range: f32,
    lifetime: f32,
) {
    commands.spawn((
        VfxFlashLight {
            age: 0.0,
            lifetime: lifetime.max(0.03),
            peak_intensity,
        },
        PointLight {
            color,
            intensity: peak_intensity,
            range,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_translation(pos),
        DespawnOnExit(GameState::InBattle),
    ));
}

fn ortho_basis(dir: Vec3) -> (Vec3, Vec3) {
    let n = dir.normalize_or(Vec3::Y);
    let up_ref = if n.y.abs() > 0.9 { Vec3::X } else { Vec3::Y };
    let u = n.cross(up_ref).normalize_or(Vec3::X);
    let v = n.cross(u).normalize_or(Vec3::Z);
    (u, v)
}

#[allow(clippy::too_many_arguments)]
fn handle_tank_fired_vfx(
    mut commands: Commands,
    mut fired_r: MessageReader<TankFiredMessage>,
    assets: Option<Res<VfxAssets>>,
    barrels: Query<(Entity, &BarrelMesh)>,
    player_q: Query<&GlobalTransform, With<PlayerTank>>,
    mut shake: Option<ResMut<CameraShake>>,
    existing_particles: Query<(), With<VfxParticle>>,
    mut seed: Local<u64>,
) {
    let mut p_count = existing_particles.iter().count();
    let player_pos = player_q.single().ok().map(|gt| gt.translation());

    for msg in fired_r.read() {
        // 1. Запуск отката ствола.
        let peak = (0.18 + msg.caliber_mm * 0.0022).clamp(0.24, 0.46);
        let recover_dur = (0.42 + (msg.caliber_mm - 75.0).max(0.0) * 0.0065).clamp(0.42, 0.80);
        for (barrel_ent, bm) in &barrels {
            if bm.tank == msg.tank {
                commands.entity(barrel_ent).insert(BarrelRecoil {
                    peak_offset: peak,
                    elapsed: 0.0,
                    kick_dur: 0.055,
                    recover_dur,
                });
            }
        }

        // 2. Тряска камеры от выстрела игрока или очень близкого выстрела.
        if let Some(ref mut cam_shake) = shake {
            if msg.is_player {
                let punch = (0.20 + msg.caliber_mm * 0.0020).clamp(0.28, 0.50);
                cam_shake.add_trauma(punch, -msg.muzzle_dir);
            } else if let Some(ppos) = player_pos {
                let d = ppos.distance(msg.muzzle_pos);
                if d < 32.0 {
                    let near_punch = (1.0 - d / 32.0) * 0.18;
                    cam_shake.add_trauma(near_punch, (ppos - msg.muzzle_pos).normalize_or_zero());
                }
            }
        }

        let Some(ref assets) = assets else { continue };
        *seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut rng = Rng::new(*seed | 1);
        let dir = msg.muzzle_dir.normalize_or(Vec3::NEG_Z);
        let (right, up) = ortho_basis(dir);
        let cal_scale = (msg.caliber_mm / 85.0).clamp(0.85, 1.45);

        // 3. Яркая дульная вспышка + конус выхлопа пороховых газов.
        spawn_particle(
            &mut commands,
            assets,
            &mut p_count,
            msg.muzzle_pos + dir * 0.25,
            dir * 3.0,
            6.0,
            0.0,
            Vec3::splat(3.0),
            0.085,
            Vec3::splat(0.32 * cal_scale),
            Vec3::splat(0.95 * cal_scale),
            false,
            assets.flash_bright.clone(),
        );
        for _ in 0..3 {
            let spread = right * rng.signed() * 2.4 + up * rng.signed() * 2.4;
            let spd = (7.0 + rng.f32() * 9.0) * cal_scale;
            spawn_particle(
                &mut commands,
                assets,
                &mut p_count,
                msg.muzzle_pos + dir * (0.2 + rng.f32() * 0.5),
                dir * spd + spread,
                4.8,
                0.8,
                Vec3::new(rng.signed() * 5.0, rng.signed() * 5.0, 0.0),
                0.12 + rng.f32() * 0.07,
                Vec3::new(0.20, 0.20, 0.62) * cal_scale,
                Vec3::splat(0.06),
                true,
                assets.flame_orange.clone(),
            );
        }

        // 4. Боковые струи дульного тормоза (у Tiger I / ИС-3).
        if msg.has_muzzle_brake {
            for side in [-1.0f32, 1.0] {
                let side_dir = (right * side + dir * 0.18).normalize_or_zero();
                let port = msg.muzzle_pos - dir * 0.14 + right * (side * 0.18);
                spawn_particle(
                    &mut commands,
                    assets,
                    &mut p_count,
                    port,
                    side_dir * (8.5 * cal_scale),
                    5.5,
                    0.5,
                    Vec3::ZERO,
                    0.09,
                    Vec3::new(0.14, 0.14, 0.45) * cal_scale,
                    Vec3::splat(0.05),
                    true,
                    assets.flame_yellow.clone(),
                );
                spawn_particle(
                    &mut commands,
                    assets,
                    &mut p_count,
                    port + side_dir * 0.2,
                    side_dir * (4.5 * cal_scale) + Vec3::Y * 0.6,
                    2.6,
                    1.2,
                    Vec3::splat(1.2),
                    0.65 + rng.f32() * 0.35,
                    Vec3::splat(0.24 * cal_scale),
                    Vec3::splat(0.88 * cal_scale),
                    false,
                    assets.smoke_light.clone(),
                );
            }
        }

        // 5. Раскалённые искры выстрела.
        for _ in 0..8 {
            let jitter = right * rng.signed() * 0.32 + up * rng.signed() * 0.32;
            let spark_dir = (dir + jitter).normalize_or_zero();
            let spd = 14.0 + rng.f32() * 14.0;
            spawn_particle(
                &mut commands,
                assets,
                &mut p_count,
                msg.muzzle_pos + dir * 0.15,
                spark_dir * spd,
                2.8,
                -9.81,
                Vec3::ZERO,
                0.22 + rng.f32() * 0.18,
                Vec3::new(0.05, 0.05, 0.34),
                Vec3::new(0.02, 0.02, 0.08),
                true,
                assets.spark_ricochet.clone(),
            );
        }

        // 6. Пороховое облако дыма, поднимающееся от дульного среза.
        for i in 0..7 {
            let jitter = right * rng.signed() * 1.8 + up * rng.signed() * 1.4;
            let vel = dir * (2.5 + rng.f32() * 4.5) + jitter + Vec3::Y * 0.7;
            spawn_particle(
                &mut commands,
                assets,
                &mut p_count,
                msg.muzzle_pos + dir * (0.3 + i as f32 * 0.28),
                vel,
                2.6,
                1.15,
                Vec3::new(rng.signed() * 1.5, rng.signed() * 1.5, rng.signed() * 1.5),
                0.80 + rng.f32() * 0.65,
                Vec3::splat(0.24 * cal_scale),
                Vec3::splat((0.85 + rng.f32() * 0.45) * cal_scale),
                false,
                assets.smoke_light.clone(),
            );
        }

        spawn_flash_light(
            &mut commands,
            msg.muzzle_pos + dir * 0.45,
            Color::srgb(1.0, 0.68, 0.24),
            18_000.0 * cal_scale,
            16.0,
            0.09,
        );
    }
}

fn handle_armor_impact_vfx(
    mut commands: Commands,
    mut impacts: MessageReader<ArmorImpactMessage>,
    assets: Option<Res<VfxAssets>>,
    player_q: Query<Entity, With<PlayerTank>>,
    mut shake: Option<ResMut<CameraShake>>,
    existing_particles: Query<(), With<VfxParticle>>,
    mut seed: Local<u64>,
) {
    let player_ent = player_q.single().ok();
    let mut p_count = existing_particles.iter().count();

    for msg in impacts.read() {
        // Тряска камеры игрока при попадании по нему.
        if let (Some(pe), Some(owner)) = (player_ent, msg.owner_tank)
            && pe == owner
            && let Some(ref mut s) = shake
        {
            let amt = match msg.outcome {
                ImpactOutcome::Penetration { .. } => 0.48,
                ImpactOutcome::NoPenetration { .. } => 0.24,
                ImpactOutcome::Ricochet => 0.16,
            };
            s.add_trauma(amt, msg.ray_dir);
        }

        let Some(ref assets) = assets else { continue };
        *seed = seed.wrapping_add(0xD1B5_4A32_D192_ED03);
        let mut rng = Rng::new(*seed | 1);
        let normal = msg.hit_normal.normalize_or(Vec3::Y);
        let (t1, t2) = ortho_basis(normal);
        let pos = msg.hit_point + normal * 0.04;

        match msg.outcome {
            ImpactOutcome::Ricochet => {
                // Отражённый вектор рикошета + веер ярких удлинённых трассеров-искр.
                let refl = (msg.ray_dir - 2.0 * msg.ray_dir.dot(normal) * normal)
                    .normalize_or(normal);
                let (ru, rv) = ortho_basis(refl);
                spawn_particle(
                    &mut commands,
                    assets,
                    &mut p_count,
                    pos,
                    refl * 1.5,
                    6.0,
                    0.0,
                    Vec3::splat(3.0),
                    0.09,
                    Vec3::splat(0.28),
                    Vec3::splat(0.55),
                    false,
                    assets.flash_bright.clone(),
                );
                for _ in 0..14 {
                    let cone = ru * rng.signed() * 0.34 + rv * rng.signed() * 0.34;
                    let sdir = (refl + cone + normal * 0.14).normalize_or_zero();
                    let spd = 15.0 + rng.f32() * 17.0;
                    spawn_particle(
                        &mut commands,
                        assets,
                        &mut p_count,
                        pos,
                        sdir * spd,
                        1.4,
                        -9.81,
                        Vec3::ZERO,
                        0.26 + rng.f32() * 0.22,
                        Vec3::new(0.06, 0.06, 0.56),
                        Vec3::new(0.02, 0.02, 0.10),
                        true,
                        assets.spark_ricochet.clone(),
                    );
                }
                spawn_flash_light(
                    &mut commands,
                    pos + normal * 0.15,
                    Color::srgb(1.0, 0.78, 0.30),
                    14_000.0,
                    10.0,
                    0.08,
                );
            }
            ImpactOutcome::NoPenetration { .. } => {
                // Непробитие: радиальный веер искр вдоль плоскости бронелиста + немного пыли.
                spawn_particle(
                    &mut commands,
                    assets,
                    &mut p_count,
                    pos,
                    normal * 0.6,
                    6.0,
                    0.0,
                    Vec3::splat(2.0),
                    0.09,
                    Vec3::splat(0.22),
                    Vec3::splat(0.45),
                    false,
                    assets.flash_bright.clone(),
                );
                for _ in 0..12 {
                    let ang = rng.f32() * std::f32::consts::TAU;
                    let radial = (t1 * ang.cos() + t2 * ang.sin()) * (0.75 + rng.f32() * 0.45)
                        + normal * (0.20 + rng.f32() * 0.35);
                    let spd = 7.5 + rng.f32() * 10.5;
                    spawn_particle(
                        &mut commands,
                        assets,
                        &mut p_count,
                        pos,
                        radial.normalize_or_zero() * spd,
                        2.5,
                        -9.81,
                        Vec3::ZERO,
                        0.22 + rng.f32() * 0.16,
                        Vec3::new(0.05, 0.05, 0.36),
                        Vec3::new(0.02, 0.02, 0.07),
                        true,
                        assets.flame_yellow.clone(),
                    );
                }
                for _ in 0..3 {
                    let drift = (t1 * rng.signed() + t2 * rng.signed() + normal * 1.2)
                        .normalize_or_zero();
                    spawn_particle(
                        &mut commands,
                        assets,
                        &mut p_count,
                        pos,
                        drift * 1.8 + Vec3::Y * 0.6,
                        2.5,
                        1.0,
                        Vec3::splat(1.4),
                        0.55 + rng.f32() * 0.3,
                        Vec3::splat(0.16),
                        Vec3::splat(0.55),
                        false,
                        assets.smoke_light.clone(),
                    );
                }
                spawn_flash_light(
                    &mut commands,
                    pos + normal * 0.15,
                    Color::srgb(1.0, 0.66, 0.22),
                    11_000.0,
                    8.5,
                    0.08,
                );
            }
            ImpactOutcome::Penetration { .. } => {
                // Пробитие: яркая вспышка прожига, раскалённые искры-осколки и чёрный дым.
                spawn_particle(
                    &mut commands,
                    assets,
                    &mut p_count,
                    pos,
                    normal * 0.8,
                    5.0,
                    0.0,
                    Vec3::splat(3.5),
                    0.12,
                    Vec3::splat(0.28),
                    Vec3::splat(0.70),
                    false,
                    assets.flash_bright.clone(),
                );
                for i in 0..11 {
                    let out_bias = if i % 3 == 0 {
                        msg.ray_dir * 0.9
                    } else {
                        normal * 0.85
                    };
                    let sdir = (out_bias + t1 * rng.signed() * 0.75 + t2 * rng.signed() * 0.75)
                        .normalize_or_zero();
                    let spd = 9.0 + rng.f32() * 12.0;
                    let mat = if i % 2 == 0 {
                        assets.flame_yellow.clone()
                    } else {
                        assets.flame_orange.clone()
                    };
                    spawn_particle(
                        &mut commands,
                        assets,
                        &mut p_count,
                        pos,
                        sdir * spd,
                        2.0,
                        -9.81,
                        Vec3::ZERO,
                        0.28 + rng.f32() * 0.24,
                        Vec3::new(0.06, 0.06, 0.42),
                        Vec3::new(0.02, 0.02, 0.08),
                        true,
                        mat,
                    );
                }
                for _ in 0..5 {
                    let v = normal * (1.2 + rng.f32() * 1.4)
                        + t1 * rng.signed() * 0.9
                        + t2 * rng.signed() * 0.9
                        + Vec3::Y * 0.9;
                    spawn_particle(
                        &mut commands,
                        assets,
                        &mut p_count,
                        pos,
                        v,
                        2.1,
                        1.4,
                        Vec3::splat(1.8),
                        0.85 + rng.f32() * 0.55,
                        Vec3::splat(0.22),
                        Vec3::splat(0.80),
                        false,
                        assets.smoke_dark.clone(),
                    );
                }
                spawn_flash_light(
                    &mut commands,
                    pos + normal * 0.2,
                    Color::srgb(1.0, 0.55, 0.16),
                    17_000.0,
                    12.0,
                    0.12,
                );
            }
        }
    }
}

fn handle_terrain_impact_vfx(
    mut commands: Commands,
    mut terrain_r: MessageReader<TerrainImpactMessage>,
    assets: Option<Res<VfxAssets>>,
    existing_particles: Query<(), With<VfxParticle>>,
    mut seed: Local<u64>,
) {
    let Some(ref assets) = assets else { return };
    let mut p_count = existing_particles.iter().count();

    for msg in terrain_r.read() {
        *seed = seed.wrapping_add(0xA076_1D64_78BD_642F);
        let mut rng = Rng::new(*seed | 1);
        let normal = msg.normal.normalize_or(Vec3::Y);
        let (t1, t2) = ortho_basis(normal);
        let pos = msg.point + normal * 0.08;

        let dust_mat = match msg.material {
            Material::Sand | Material::Sandbag => assets.dust_sand.clone(),
            Material::Snow | Material::Ice => assets.dust_snow.clone(),
            Material::Brick => assets.smoke_dark.clone(),
            _ => assets.smoke_light.clone(),
        };

        if msg.destructible {
            spawn_particle(
                &mut commands,
                assets,
                &mut p_count,
                pos,
                normal * 1.0,
                5.0,
                0.0,
                Vec3::splat(2.5),
                0.11,
                Vec3::splat(0.32),
                Vec3::splat(0.85),
                false,
                assets.flame_orange.clone(),
            );
            for _ in 0..7 {
                let d = (normal + Vec3::Y * 0.8 + t1 * rng.signed() + t2 * rng.signed())
                    .normalize_or_zero();
                spawn_particle(
                    &mut commands,
                    assets,
                    &mut p_count,
                    pos,
                    d * (2.5 + rng.f32() * 3.5),
                    2.0,
                    1.2,
                    Vec3::splat(1.8),
                    0.9 + rng.f32() * 0.7,
                    Vec3::splat(0.28),
                    Vec3::splat(1.05),
                    false,
                    dust_mat.clone(),
                );
            }
            spawn_flash_light(
                &mut commands,
                pos + normal * 0.25,
                Color::srgb(1.0, 0.58, 0.18),
                13_000.0,
                11.0,
                0.10,
            );
        } else {
            spawn_particle(
                &mut commands,
                assets,
                &mut p_count,
                pos,
                normal * 0.6,
                5.0,
                0.0,
                Vec3::ZERO,
                0.08,
                Vec3::splat(0.20),
                Vec3::splat(0.45),
                false,
                assets.flame_yellow.clone(),
            );
            for _ in 0..6 {
                let d = (normal * 0.9 + Vec3::Y * 1.2 + t1 * rng.signed() * 0.8 + t2 * rng.signed() * 0.8)
                    .normalize_or_zero();
                spawn_particle(
                    &mut commands,
                    assets,
                    &mut p_count,
                    pos,
                    d * (2.8 + rng.f32() * 3.8),
                    2.4,
                    -2.0,
                    Vec3::splat(1.4),
                    0.7 + rng.f32() * 0.5,
                    Vec3::splat(0.24),
                    Vec3::splat(0.85),
                    false,
                    dust_mat.clone(),
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_tank_destroyed_vfx(
    mut commands: Commands,
    mut destroyed_r: MessageReader<TankDestroyedMessage>,
    assets: Option<Res<VfxAssets>>,
    roots: Query<
        (&Transform, &GlobalTransform, Option<&TankRoot>, Option<&PlayerTank>),
        (Without<TurretPivot>, Without<GunPivot>),
    >,
    turrets: Query<
        (Entity, &GlobalTransform, &Transform, &TurretPivot),
        (Without<ThrownTurret>, Without<TankRoot>, Without<GunPivot>),
    >,
    mut guns: Query<(&mut Transform, &GunPivot), (Without<TurretPivot>, Without<TankRoot>)>,
    fire_lights: Query<(Entity, &TankFireLight)>,
    mut shake: Option<ResMut<CameraShake>>,
    player_q: Query<&GlobalTransform, With<PlayerTank>>,
    existing_particles: Query<(), With<VfxParticle>>,
    mut seed: Local<u64>,
) {
    let mut p_count = existing_particles.iter().count();
    let player_pos = player_q.single().ok().map(|gt| gt.translation());

    for msg in destroyed_r.read() {
        *seed = seed.wrapping_add(0xF135_79BD_2468_ACE0);
        let mut rng = Rng::new(*seed | 1);

        // Гасим привязанный динамический свет старого пожара (взрыв даст свой или дым).
        for (l_ent, fl) in &fire_lights {
            if fl.tank == msg.tank {
                commands.entity(l_ent).despawn();
            }
        }

        if let Ok(mut ec) = commands.get_entity(msg.tank) {
            ec.insert(TankWreckSmoke {
                remaining: if msg.ammo_detonated { 28.0 } else { 20.0 },
                acc: 0.0,
                intense: msg.ammo_detonated,
            });
        }

        let root_info = roots.get(msg.tank).ok();
        let hull_world_tf = root_info.map(|(tf, gt, _, _)| {
            if gt.translation() == Vec3::ZERO && tf.translation != Vec3::ZERO {
                *tf
            } else {
                gt.compute_transform()
            }
        });
        let spec_opt = root_info.and_then(|(_, _, r, _)| r.map(|x| &x.spec));

        if msg.ammo_detonated {
            // 1. Полноценная баллистическая детонация БК: открепляем башню в мир и задаём импульс.
            let is_casemate = spec_opt
                .is_some_and(|s| s.profile.turret_shape == TurretShape::FixedCasemate);
            let t_height = spec_opt.map_or(0.72, |s| s.profile.turret_height);
            let t_radius = spec_opt.map_or(0.95, |s| s.profile.turret_radius_x);

            for (turret_ent, t_gt, t_local_tf, tp) in &turrets {
                if tp.tank != msg.tank {
                    continue;
                }
                let mut world_tf = if t_gt.translation() == Vec3::ZERO && t_local_tf.translation != Vec3::ZERO
                {
                    hull_world_tf.unwrap_or_default() * *t_local_tf
                } else {
                    t_gt.compute_transform()
                };
                world_tf.scale = Vec3::ONE;

                let eject_angle = rng.f32() * std::f32::consts::TAU;
                let horiz = Vec3::new(eject_angle.cos(), 0.0, eject_angle.sin());
                let (vy, h_spd, spin_amp) = if is_casemate {
                    (5.6 + rng.f32() * 1.6, 1.4 + rng.f32() * 1.2, 2.2)
                } else {
                    (9.8 + rng.f32() * 3.6, 2.1 + rng.f32() * 2.2, 4.4)
                };

                commands
                    .entity(turret_ent)
                    .remove::<ChildOf>()
                    .insert((
                        world_tf,
                        DespawnOnExit(GameState::InBattle),
                        ThrownTurret {
                            tank: msg.tank,
                            velocity: Vec3::Y * vy + horiz * h_spd,
                            angular_velocity: Vec3::new(
                                rng.signed() * spin_amp,
                                rng.signed() * (spin_amp * 0.7),
                                rng.signed() * spin_amp,
                            ),
                            turret_height: t_height,
                            turret_radius: t_radius,
                            bounces: 0,
                            settled: false,
                            trail_acc: 0.0,
                            post_smoke_timer: 12.0,
                        },
                    ));
            }

            // 2. Мощный взрыв БК (огненный столб, ударная волна, разлёт углей и чёрный шлейф).
            if let Some(ref mut cam_shake) = shake {
                let d = player_pos.map_or(0.0, |p| p.distance(msg.pos));
                if d < 75.0 {
                    let trauma = (1.0 - d / 75.0).powi(1) * 0.95;
                    cam_shake.add_trauma(trauma, Vec3::Y);
                }
            }

            if let Some(ref assets) = assets {
                let core_pos = msg.pos + Vec3::Y * 1.25;
                // Взрывное ядро и расширяющаяся ударная волна.
                spawn_particle(
                    &mut commands,
                    assets,
                    &mut p_count,
                    core_pos,
                    Vec3::Y * 1.5,
                    3.0,
                    0.5,
                    Vec3::splat(2.0),
                    0.26,
                    Vec3::splat(0.9),
                    Vec3::splat(2.8),
                    false,
                    assets.flash_bright.clone(),
                );
                spawn_particle(
                    &mut commands,
                    assets,
                    &mut p_count,
                    msg.pos + Vec3::Y * 0.35,
                    Vec3::ZERO,
                    1.0,
                    0.0,
                    Vec3::ZERO,
                    0.38,
                    Vec3::new(0.8, 0.08, 0.8),
                    Vec3::new(9.5, 0.04, 9.5),
                    false,
                    assets.shockwave.clone(),
                );
                // Вертикальный столб пламени из погона башни.
                for i in 0..14 {
                    let spread = Vec3::new(rng.signed() * 2.4, 4.5 + rng.f32() * 7.5, rng.signed() * 2.4);
                    let mat = match i % 3 {
                        0 => assets.flame_yellow.clone(),
                        1 => assets.flame_orange.clone(),
                        _ => assets.flame_red.clone(),
                    };
                    spawn_particle(
                        &mut commands,
                        assets,
                        &mut p_count,
                        core_pos + Vec3::new(rng.signed() * 0.5, 0.2, rng.signed() * 0.5),
                        spread,
                        1.8,
                        2.5,
                        Vec3::new(rng.signed() * 3.0, rng.signed() * 3.0, rng.signed() * 3.0),
                        0.45 + rng.f32() * 0.45,
                        Vec3::splat(0.45 + rng.f32() * 0.35),
                        Vec3::splat(0.10),
                        false,
                        mat,
                    );
                }
                // Фонтан горящих искр во все стороны.
                for _ in 0..16 {
                    let v = Vec3::new(rng.signed() * 9.0, 5.0 + rng.f32() * 9.0, rng.signed() * 9.0);
                    spawn_particle(
                        &mut commands,
                        assets,
                        &mut p_count,
                        core_pos,
                        v,
                        1.1,
                        -9.81,
                        Vec3::ZERO,
                        0.65 + rng.f32() * 0.55,
                        Vec3::new(0.08, 0.08, 0.55),
                        Vec3::new(0.03, 0.03, 0.12),
                        true,
                        assets.spark_ricochet.clone(),
                    );
                }
                // Густой чёрный взрывной дым.
                for _ in 0..12 {
                    let v = Vec3::new(rng.signed() * 2.8, 3.0 + rng.f32() * 4.5, rng.signed() * 2.8);
                    spawn_particle(
                        &mut commands,
                        assets,
                        &mut p_count,
                        core_pos,
                        v,
                        1.5,
                        1.6,
                        Vec3::splat(1.2),
                        1.6 + rng.f32() * 1.1,
                        Vec3::splat(0.55),
                        Vec3::splat(2.1),
                        false,
                        assets.smoke_dark.clone(),
                    );
                }
                spawn_flash_light(
                    &mut commands,
                    core_pos + Vec3::Y * 0.6,
                    Color::srgb(1.0, 0.52, 0.12),
                    55_000.0,
                    28.0,
                    0.42,
                );
            }
        } else {
            // Обычное уничтожение без отрыва башни: опускаем ствол и даём клубы огня и дыма.
            for (mut g_tf, g) in &mut guns {
                if g.tank == msg.tank {
                    g_tf.rotation = Quat::from_rotation_x(-0.14);
                }
            }
            if let Some(ref mut cam_shake) = shake {
                let d = player_pos.map_or(0.0, |p| p.distance(msg.pos));
                if d < 45.0 {
                    cam_shake.add_trauma((1.0 - d / 45.0) * 0.45, Vec3::Y);
                }
            }
            if let Some(ref assets) = assets {
                let core_pos = msg.pos + Vec3::Y * 1.1;
                for i in 0..8 {
                    let v = Vec3::new(rng.signed() * 2.5, 2.5 + rng.f32() * 4.0, rng.signed() * 2.5);
                    spawn_particle(
                        &mut commands,
                        assets,
                        &mut p_count,
                        core_pos,
                        v,
                        2.0,
                        1.8,
                        Vec3::splat(2.0),
                        0.45 + rng.f32() * 0.35,
                        Vec3::splat(0.35),
                        Vec3::splat(0.10),
                        false,
                        if i % 2 == 0 {
                            assets.flame_orange.clone()
                        } else {
                            assets.flame_red.clone()
                        },
                    );
                }
                for _ in 0..8 {
                    let v = Vec3::new(rng.signed() * 1.8, 2.2 + rng.f32() * 2.8, rng.signed() * 1.8);
                    spawn_particle(
                        &mut commands,
                        assets,
                        &mut p_count,
                        core_pos,
                        v,
                        1.6,
                        1.5,
                        Vec3::splat(1.0),
                        1.4 + rng.f32() * 0.8,
                        Vec3::splat(0.45),
                        Vec3::splat(1.45),
                        false,
                        assets.smoke_dark.clone(),
                    );
                }
                spawn_flash_light(
                    &mut commands,
                    core_pos + Vec3::Y * 0.3,
                    Color::srgb(1.0, 0.45, 0.12),
                    28_000.0,
                    18.0,
                    0.26,
                );
            }
        }
    }
}

fn handle_tank_action_vfx(
    mut commands: Commands,
    mut actions: MessageReader<TankActionMessage>,
    assets: Option<Res<VfxAssets>>,
    fire_lights: Query<(Entity, &TankFireLight)>,
    existing_particles: Query<(), With<VfxParticle>>,
    mut seed: Local<u64>,
) {
    let mut p_count = existing_particles.iter().count();

    for msg in actions.read() {
        if msg.kind == TankActionKind::FireExtinguished {
            // Мгновенно гасим динамический свет пожара и пускаем облако пены/пара.
            for (l_ent, fl) in &fire_lights {
                if fl.tank == msg.tank {
                    commands.entity(l_ent).despawn();
                }
            }
            if let Some(ref assets) = assets {
                *seed = seed.wrapping_add(0x7F4A_7C15_9E37_79B9);
                let mut rng = Rng::new(*seed | 1);
                for _ in 0..10 {
                    let v = Vec3::new(
                        rng.signed() * 2.4,
                        1.6 + rng.f32() * 2.5,
                        rng.signed() * 2.4,
                    );
                    spawn_particle(
                        &mut commands,
                        assets,
                        &mut p_count,
                        msg.pos + Vec3::Y * 1.15,
                        v,
                        2.8,
                        0.6,
                        Vec3::splat(1.5),
                        0.65 + rng.f32() * 0.35,
                        Vec3::splat(0.26),
                        Vec3::splat(0.95),
                        false,
                        assets.dust_snow.clone(),
                    );
                }
            }
        }
    }
}

/// Плавная анимация отката и гидравлического наката ствола (`BarrelMesh`).
pub fn animate_barrel_recoil(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &BarrelMesh, &mut BarrelRecoil, &mut Transform)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (ent, bm, mut rec, mut tf) in &mut q {
        rec.elapsed += dt;
        let offset = if rec.elapsed < rec.kick_dur {
            let u = (rec.elapsed / rec.kick_dur).clamp(0.0, 1.0);
            rec.peak_offset * (u * FRAC_PI_2).sin()
        } else if rec.elapsed < rec.kick_dur + rec.recover_dur {
            let u = 1.0 - ((rec.elapsed - rec.kick_dur) / rec.recover_dur).clamp(0.0, 1.0);
            rec.peak_offset * u * u
        } else {
            0.0
        };
        tf.translation = bm.rest_translation + Vec3::new(0.0, 0.0, offset);
        if rec.elapsed >= rec.kick_dur + rec.recover_dur {
            commands.entity(ent).remove::<BarrelRecoil>();
        }
    }
}

/// Физика падения, вращения, отскока и остановки сорванных при детонации БК башен.
#[allow(clippy::too_many_arguments)]
pub fn animate_detonated_turrets(
    mut commands: Commands,
    time: Res<Time>,
    world: Option<Res<VoxelWorld>>,
    assets: Option<Res<VfxAssets>>,
    hulls: Query<(Entity, &Transform, &GlobalTransform, Option<&TankRoot>), Without<ThrownTurret>>,
    mut turrets: Query<(&mut Transform, &mut ThrownTurret)>,
    mut landed_w: MessageWriter<TurretLandedMessage>,
    existing_particles: Query<(), With<VfxParticle>>,
    mut seed: Local<u64>,
) {
    let dt = time.delta_secs().min(0.05);
    if dt <= 0.0 {
        return;
    }
    let mut p_count = existing_particles.iter().count();

    for (mut tf, mut tt) in &mut turrets {
        if tt.settled {
            if tt.post_smoke_timer > 0.0 {
                tt.post_smoke_timer -= dt;
                tt.trail_acc += dt;
                if tt.trail_acc >= 0.24 {
                    tt.trail_acc = 0.0;
                    if let Some(ref assets) = assets {
                        *seed = seed.wrapping_add(977);
                        let mut rng = Rng::new(*seed | 1);
                        spawn_particle(
                            &mut commands,
                            assets,
                            &mut p_count,
                            tf.translation + Vec3::Y * 0.2,
                            Vec3::new(rng.signed() * 0.4, 1.5 + rng.f32() * 0.8, rng.signed() * 0.4),
                            1.4,
                            0.8,
                            Vec3::splat(0.6),
                            1.1 + rng.f32() * 0.5,
                            Vec3::splat(0.24),
                            Vec3::splat(0.75),
                            false,
                            assets.smoke_dark.clone(),
                        );
                    }
                }
            }
            continue;
        }

        tt.velocity.y -= 9.81 * dt;
        tf.translation += tt.velocity * dt;
        let spin_angle = tt.angular_velocity * dt;
        let d_rot = Quat::from_euler(EulerRot::YXZ, spin_angle.y, spin_angle.x, spin_angle.z);
        tf.rotation = (d_rot * tf.rotation).normalize();

        // Шлейф чёрного дыма и искр в полёте.
        tt.trail_acc += dt;
        if tt.trail_acc >= 0.045 {
            tt.trail_acc = 0.0;
            if let Some(ref assets) = assets {
                *seed = seed.wrapping_add(1307);
                let mut rng = Rng::new(*seed | 1);
                spawn_particle(
                    &mut commands,
                    assets,
                    &mut p_count,
                    tf.translation,
                    Vec3::new(rng.signed() * 0.6, 1.2 + rng.f32() * 1.0, rng.signed() * 0.6),
                    1.6,
                    0.9,
                    Vec3::splat(1.2),
                    0.85 + rng.f32() * 0.45,
                    Vec3::splat(0.32),
                    Vec3::splat(0.95),
                    false,
                    assets.smoke_dark.clone(),
                );
            }
        }

        // Вычисляем уровень опоры (грунт карты или крыша собственного корпуса, если башня упала обратно).
        let mut base_floor_y = world
            .as_ref()
            .and_then(|w| w.grid.surface_height_at(tf.translation.x, tf.translation.z))
            .unwrap_or(0.0);

        for (h_ent, h_tf, h_gt, root_opt) in &hulls {
            if h_ent != tt.tank {
                continue;
            }
            let h_pos = if h_gt.translation() == Vec3::ZERO && h_tf.translation != Vec3::ZERO {
                h_tf.translation
            } else {
                h_gt.translation()
            };
            let xz_dist = Vec2::new(tf.translation.x - h_pos.x, tf.translation.z - h_pos.z).length();
            if xz_dist < 1.65 {
                let deck_y = h_pos.y + root_opt.map_or(1.1, |r| r.spec.profile.deck_height + 0.12);
                base_floor_y = base_floor_y.max(deck_y);
            }
        }

        // Высота центра погона башни с учётом её текущего опрокидывания/переворота.
        let up_y = (tf.rotation * Vec3::Y).y;
        let down_clearance = ((-up_y).max(0.0) * tt.turret_height * 0.85)
            .max((1.0 - up_y.abs()) * tt.turret_radius * 0.62)
            .max(0.14);
        let floor_y = base_floor_y + down_clearance;

        if tf.translation.y <= floor_y && tt.velocity.y < 0.0 {
            let impact_speed = tt.velocity.y.abs();
            tf.translation.y = floor_y;

            if impact_speed > 2.2 {
                landed_w.write(TurretLandedMessage {
                    pos: tf.translation,
                    impact_speed,
                });
                if let Some(ref assets) = assets {
                    *seed = seed.wrapping_add(4447);
                    let mut rng = Rng::new(*seed | 1);
                    for _ in 0..8 {
                        let ang = rng.f32() * std::f32::consts::TAU;
                        let dir = Vec3::new(ang.cos(), 0.4 + rng.f32() * 0.5, ang.sin());
                        spawn_particle(
                            &mut commands,
                            assets,
                            &mut p_count,
                            Vec3::new(tf.translation.x, base_floor_y + 0.15, tf.translation.z),
                            dir * (2.5 + rng.f32() * 3.0),
                            2.4,
                            -1.5,
                            Vec3::splat(1.2),
                            0.75 + rng.f32() * 0.4,
                            Vec3::splat(0.28),
                            Vec3::splat(0.95),
                            false,
                            assets.smoke_light.clone(),
                        );
                    }
                }
            }

            if tt.bounces < 2 && impact_speed > 2.0 {
                tt.velocity.y = impact_speed * 0.26;
                tt.velocity.x *= 0.46;
                tt.velocity.z *= 0.46;
                tt.angular_velocity *= 0.34;
                tt.bounces += 1;
            } else {
                tt.velocity = Vec3::ZERO;
                tt.angular_velocity = Vec3::ZERO;
                tt.settled = true;
            }
        }
    }
}

/// Анимация горящего танка: живое пламя над МТО, искры, чёрный шлейф и мерцающий свет.
#[allow(clippy::too_many_arguments)]
fn animate_tank_fires(
    mut commands: Commands,
    time: Res<Time>,
    assets: Option<Res<VfxAssets>>,
    mut tanks: Query<(
        Entity,
        &Transform,
        &GlobalTransform,
        &TankRoot,
        &TankCombat,
        &TankModules,
        Option<&mut TankVfxTimers>,
    )>,
    mut fire_lights: Query<(Entity, &mut TankFireLight, &mut PointLight, &mut Transform), Without<TankRoot>>,
    existing_particles: Query<(), With<VfxParticle>>,
    mut seed: Local<u64>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let mut p_count = existing_particles.iter().count();

    for (tank_ent, tf, gt, root, combat, modules, timers_opt) in &mut tanks {
        let burning = combat.alive && modules.fire_timer > 0.0;
        let tank_pos = if gt.translation() == Vec3::ZERO && tf.translation != Vec3::ZERO {
            tf.translation
        } else {
            gt.translation()
        };
        let tank_rot = if gt.translation() == Vec3::ZERO && tf.translation != Vec3::ZERO {
            tf.rotation
        } else {
            gt.to_scale_rotation_translation().1
        };
        let engine_pos = tank_pos
            + tank_rot
                * Vec3::new(
                    0.0,
                    root.spec.profile.deck_height + 0.08,
                    root.spec.profile.hull_length * 0.26,
                );

        if burning {
            let Some(mut timers) = timers_opt else {
                commands.entity(tank_ent).insert(TankVfxTimers {
                    fire_acc: 0.1,
                    drive_acc: 0.0,
                });
                continue;
            };

            // Динамический мерцающий свет пожара.
            let mut has_light = false;
            for (_, mut fl, mut pl, mut ltf) in &mut fire_lights {
                if fl.tank == tank_ent {
                    has_light = true;
                    fl.phase += dt * 24.0;
                    ltf.translation = engine_pos + Vec3::Y * 0.45;
                    let flicker = 0.72 + 0.28 * (fl.phase.sin() * 0.6 + (fl.phase * 1.73).cos() * 0.4);
                    pl.intensity = 15_000.0 * flicker;
                }
            }
            if !has_light {
                commands.spawn((
                    TankFireLight {
                        tank: tank_ent,
                        phase: 0.0,
                    },
                    PointLight {
                        color: Color::srgb(1.0, 0.42, 0.08),
                        intensity: 14_000.0,
                        range: 18.0,
                        shadow_maps_enabled: false,
                        ..default()
                    },
                    Transform::from_translation(engine_pos + Vec3::Y * 0.45),
                    DespawnOnExit(GameState::InBattle),
                ));
            }

            timers.fire_acc += dt;
            if timers.fire_acc >= 0.055 {
                timers.fire_acc = 0.0;
                if let Some(ref assets) = assets {
                    *seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
                    let mut rng = Rng::new(*seed | 1);
                    for i in 0..3 {
                        let offset = tank_rot
                            * Vec3::new(rng.signed() * 0.55, 0.04, rng.signed() * 0.55);
                        let mat = match (i + (rng.next_u64() & 1) as usize) % 3 {
                            0 => assets.flame_yellow.clone(),
                            1 => assets.flame_orange.clone(),
                            _ => assets.flame_red.clone(),
                        };
                        spawn_particle(
                            &mut commands,
                            assets,
                            &mut p_count,
                            engine_pos + offset,
                            Vec3::new(rng.signed() * 0.9, 2.8 + rng.f32() * 2.4, rng.signed() * 0.9),
                            1.8,
                            2.2,
                            Vec3::new(rng.signed() * 3.0, rng.signed() * 3.0, rng.signed() * 3.0),
                            0.28 + rng.f32() * 0.22,
                            Vec3::splat(0.24 + rng.f32() * 0.16),
                            Vec3::splat(0.05),
                            false,
                            mat,
                        );
                    }
                    // Дым и редкие искры огня.
                    spawn_particle(
                        &mut commands,
                        assets,
                        &mut p_count,
                        engine_pos + Vec3::Y * 0.25,
                        Vec3::new(rng.signed() * 0.7, 2.8 + rng.f32() * 1.8, rng.signed() * 0.7),
                        1.2,
                        1.5,
                        Vec3::splat(1.1),
                        1.25 + rng.f32() * 0.65,
                        Vec3::splat(0.28),
                        Vec3::splat(1.05),
                        false,
                        assets.smoke_dark.clone(),
                    );
                    if rng.f32() < 0.45 {
                        spawn_particle(
                            &mut commands,
                            assets,
                            &mut p_count,
                            engine_pos,
                            Vec3::new(rng.signed() * 2.2, 4.2 + rng.f32() * 2.5, rng.signed() * 2.2),
                            1.2,
                            -4.0,
                            Vec3::ZERO,
                            0.45 + rng.f32() * 0.25,
                            Vec3::splat(0.06),
                            Vec3::splat(0.02),
                            false,
                            assets.spark_ricochet.clone(),
                        );
                    }
                }
            }
        } else {
            for (l_ent, fl, _, _) in &fire_lights {
                if fl.tank == tank_ent {
                    commands.entity(l_ent).despawn();
                }
            }
        }
    }
}

/// Выхлоп двигателя, пыль/снег из-под гусениц на ходу и дым от подбитых остовов.
#[allow(clippy::too_many_arguments)]
fn animate_tank_drive_and_wreck_vfx(
    mut commands: Commands,
    time: Res<Time>,
    world: Option<Res<VoxelWorld>>,
    assets: Option<Res<VfxAssets>>,
    mut live_tanks: Query<
        (
            Entity,
            &Transform,
            &GlobalTransform,
            &TankRoot,
            &TankState,
            &TankCombat,
            Option<&mut TankVfxTimers>,
        ),
        Without<TankWreckSmoke>,
    >,
    mut wrecks: Query<(Entity, &GlobalTransform, &TankRoot, &mut TankWreckSmoke)>,
    existing_particles: Query<(), With<VfxParticle>>,
    mut seed: Local<u64>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let Some(ref assets) = assets else { return };
    let mut p_count = existing_particles.iter().count();

    for (tank_ent, tf, gt, root, st, combat, timers_opt) in &mut live_tanks {
        if !combat.alive {
            continue;
        }
        let Some(mut timers) = timers_opt else {
            commands.entity(tank_ent).insert(TankVfxTimers::default());
            continue;
        };

        let driving = st.speed_kmh.abs() > 5.0 || st.throttle.abs() > 0.15;
        let interval = if driving { 0.11 } else { 0.34 };
        timers.drive_acc += dt;
        if timers.drive_acc < interval {
            continue;
        }
        timers.drive_acc = 0.0;

        *seed = seed.wrapping_add(0x517C_C1B7_2722_0A95);
        let mut rng = Rng::new(*seed | 1);
        let pos = if gt.translation() == Vec3::ZERO && tf.translation != Vec3::ZERO {
            tf.translation
        } else {
            gt.translation()
        };
        let rot = if gt.translation() == Vec3::ZERO && tf.translation != Vec3::ZERO {
            tf.rotation
        } else {
            gt.to_scale_rotation_translation().1
        };

        // 1. Выхлоп двигателя с кормы.
        let p = &root.spec.profile;
        let exhaust_local = Vec3::new(
            (rng.next_u64() & 1) as f32 * 0.8 - 0.4,
            p.deck_height * 0.72,
            p.hull_length * 0.48,
        );
        let ex_world = pos + rot * exhaust_local;
        let back = rot * Vec3::Z;
        spawn_particle(
            &mut commands,
            assets,
            &mut p_count,
            ex_world,
            back * (1.2 + st.throttle.abs() * 1.4) + Vec3::Y * 1.1,
            2.2,
            0.9,
            Vec3::splat(0.8),
            0.48 + rng.f32() * 0.28,
            Vec3::splat(0.11),
            Vec3::splat(0.36),
            false,
            assets.smoke_dark.clone(),
        );

        // 2. Пыль или снежная пыль из-под гусениц на скорости.
        if st.speed_kmh.abs() > 8.0 && st.grounded_wheels >= 4 {
            let side_x = p.hull_width * 0.5 - p.track_width * 0.5;
            let track_z = if st.speed_kmh >= 0.0 {
                p.hull_length * 0.42
            } else {
                -p.hull_length * 0.42
            };
            let surf_mat = world
                .as_ref()
                .and_then(|w| w.grid.surface_height_and_material_at(pos.x, pos.z))
                .map(|(_, m)| m)
                .unwrap_or(Material::Soil);
            let dust_mat = match surf_mat {
                Material::Snow | Material::Ice => assets.dust_snow.clone(),
                Material::Sand => assets.dust_sand.clone(),
                _ => assets.smoke_light.clone(),
            };
            let sgn = if (rng.next_u64() & 1) == 0 { -1.0 } else { 1.0 };
            let wp = pos + rot * Vec3::new(sgn * side_x, 0.25, track_z);
            spawn_particle(
                &mut commands,
                assets,
                &mut p_count,
                wp,
                back * (st.speed_kmh.signum() * 1.5) + Vec3::Y * (1.2 + rng.f32() * 0.8),
                2.5,
                0.3,
                Vec3::splat(1.2),
                0.55 + rng.f32() * 0.3,
                Vec3::splat(0.18),
                Vec3::splat(0.68),
                false,
                dust_mat,
            );
        }
    }

    // Дым из подбитых остовов.
    for (ent, gt, root, mut ws) in &mut wrecks {
        ws.remaining -= dt;
        if ws.remaining <= 0.0 {
            commands.entity(ent).remove::<TankWreckSmoke>();
            continue;
        }
        ws.acc += dt;
        let interval = if ws.intense { 0.14 } else { 0.20 };
        if ws.acc < interval {
            continue;
        }
        ws.acc = 0.0;
        *seed = seed.wrapping_add(7919);
        let mut rng = Rng::new(*seed | 1);
        let origin = gt.translation()
            + Vec3::Y * (root.spec.profile.deck_height + if ws.intense { 0.15 } else { 0.25 });
        spawn_particle(
            &mut commands,
            assets,
            &mut p_count,
            origin + Vec3::new(rng.signed() * 0.35, 0.0, rng.signed() * 0.35),
            Vec3::new(rng.signed() * 0.6, 2.2 + rng.f32() * 1.4, rng.signed() * 0.6),
            1.1,
            1.2,
            Vec3::splat(0.7),
            1.5 + rng.f32() * 0.7,
            Vec3::splat(0.32),
            Vec3::splat(if ws.intense { 1.35 } else { 1.05 }),
            false,
            assets.smoke_dark.clone(),
        );
        if ws.intense && rng.f32() < 0.35 {
            spawn_particle(
                &mut commands,
                assets,
                &mut p_count,
                origin,
                Vec3::new(rng.signed() * 1.2, 3.0 + rng.f32() * 1.8, rng.signed() * 1.2),
                1.6,
                1.4,
                Vec3::splat(2.0),
                0.28,
                Vec3::splat(0.18),
                Vec3::splat(0.05),
                false,
                assets.flame_orange.clone(),
            );
        }
    }
}

fn update_vfx_particles_and_lights(
    mut commands: Commands,
    time: Res<Time>,
    mut particles: Query<(Entity, &mut VfxParticle, &mut Transform)>,
    mut lights: Query<(Entity, &mut VfxFlashLight, &mut PointLight)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    for (ent, mut p, mut tf) in &mut particles {
        p.age += dt;
        if p.age >= p.lifetime {
            commands.entity(ent).despawn();
            continue;
        }
        p.velocity.y += p.gravity * dt;
        let drag = p.drag;
        if drag > 0.0 {
            p.velocity *= (-drag * dt).exp();
        }
        tf.translation += p.velocity * dt;

        let t = (p.age / p.lifetime).clamp(0.0, 1.0);
        tf.scale = p.start_scale.lerp(p.end_scale, t);
        if p.align_to_vel && p.velocity.length_squared() > 0.02 {
            tf.rotation = Transform::IDENTITY
                .looking_to(p.velocity.normalize(), Vec3::Y)
                .rotation;
        } else if p.spin != Vec3::ZERO {
            tf.rotate_local_x(p.spin.x * dt);
            tf.rotate_local_y(p.spin.y * dt);
        }
    }

    for (ent, mut fl, mut pl) in &mut lights {
        fl.age += dt;
        if fl.age >= fl.lifetime {
            commands.entity(ent).despawn();
            continue;
        }
        let remain = 1.0 - (fl.age / fl.lifetime).clamp(0.0, 1.0);
        pl.intensity = fl.peak_intensity * remain * remain;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::core::Team;
    use crate::tank::{load_tank_spec, spawn_tank};
    use crate::voxel::{VoxelAssets, VoxelGrid};
    use bevy::state::app::StatesPlugin;
    use bevy::time::TimeUpdateStrategy;

    #[test]
    fn ammo_detonation_launches_and_lands_turret_on_ground() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            TransformPlugin,
            StatesPlugin,
        ))
        .init_state::<GameState>()
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_resource::<VoxelAssets>()
        .init_resource::<CameraShake>()
        .add_message::<TankFiredMessage>()
        .add_message::<ArmorImpactMessage>()
        .add_message::<TerrainImpactMessage>()
        .add_message::<TankDestroyedMessage>()
        .add_message::<TankActionMessage>()
        .add_plugins(VfxPlugin)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 30.0,
        )));

        // Плоский пол до y = 2.0 м.
        let mut grid = VoxelGrid::new(UVec3::new(32, 8, 32), 1.0, Vec3::new(-16.0, 0.0, -16.0));
        grid.fill_box(IVec3::new(0, 0, 0), IVec3::new(31, 1, 31), Material::Grass);
        app.insert_resource(VoxelWorld::new(grid));
        app.world_mut()
            .resource_mut::<NextState<GameState>>()
            .set(GameState::InBattle);
        app.finish();
        app.cleanup();

        let tank_ent;
        {
            let world = app.world_mut();
            let spec = load_tank_spec("t34_85").unwrap();
            let assets_clone = {
                let a = world.resource::<VoxelAssets>();
                VoxelAssets {
                    material: a.material.clone(),
                    cube: a.cube.clone(),
                    tracer: a.tracer.clone(),
                    debris: a.debris.clone(),
                }
            };
            let mut meshes = world.remove_resource::<Assets<Mesh>>().unwrap();
            let mut cmds = world.commands();
            tank_ent = spawn_tank(
                &mut cmds,
                &mut meshes,
                &assets_clone,
                spec,
                Vec3::new(0.0, 2.0, 0.0),
                0.0,
                false,
            );
            world.insert_resource(meshes);
            world.flush();
        }
        app.update();

        let initial_turret_y = {
            let mut q = app.world_mut().query::<(&GlobalTransform, &TurretPivot)>();
            q.iter(app.world())
                .find(|(_, tp)| tp.tank == tank_ent)
                .map(|(gt, _)| gt.translation().y)
                .unwrap()
        };

        // Подрываем боеукладку -> башня должна взлететь и затем упасть на землю (а не зависнуть).
        app.world_mut().write_message(TankDestroyedMessage {
            tank: tank_ent,
            pos: Vec3::new(0.0, 2.0, 0.0),
            team: Team::Enemy,
            killer: None,
            ammo_detonated: true,
        });

        let mut max_y = initial_turret_y;
        let mut saw_thrown = false;
        for _ in 0..140 {
            app.update();
            let mut q = app
                .world_mut()
                .query::<(&Transform, &TurretPivot, Option<&ThrownTurret>)>();
            if let Some((tf, _, tt)) = q.iter(app.world()).find(|(_, tp, _)| tp.tank == tank_ent) {
                max_y = max_y.max(tf.translation.y);
                if tt.is_some() {
                    saw_thrown = true;
                }
            }
        }

        assert!(saw_thrown, "TurretPivot должен получить ThrownTurret и отделиться от корпуса");
        assert!(
            max_y > initial_turret_y + 3.0,
            "башня должна взлететь выше +3 м: init={initial_turret_y:.2}, max={max_y:.2}"
        );

        let mut q = app
            .world_mut()
            .query::<(&Transform, &TurretPivot, &ThrownTurret, Option<&ChildOf>)>();
        let (final_tf, _, tt, child_of) = q
            .iter(app.world())
            .find(|(_, tp, _, _)| tp.tank == tank_ent)
            .unwrap();
        assert!(child_of.is_none(), "сорванная башня должна быть откреплена от корпуса");
        assert!(tt.settled, "башня должна упасть на землю и остановиться, vel={:?}", tt.velocity);
        assert!(
            final_tf.translation.y >= 2.0 && final_tf.translation.y <= 4.2,
            "итоговая высота башни должна быть у земли/остова, получено {}",
            final_tf.translation.y
        );
    }

    #[test]
    fn barrel_recoil_kicks_back_and_recovers_to_rest_position() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin, StatesPlugin))
            .init_state::<GameState>()
            .init_resource::<CameraShake>()
            .add_message::<TankFiredMessage>()
            .add_message::<ArmorImpactMessage>()
            .add_message::<TerrainImpactMessage>()
            .add_message::<TankDestroyedMessage>()
            .add_message::<TankActionMessage>()
            .add_plugins(VfxPlugin)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                0.02,
            )));
        app.world_mut()
            .resource_mut::<NextState<GameState>>()
            .set(GameState::InBattle);
        app.finish();
        app.cleanup();

        let tank = app.world_mut().spawn_empty().id();
        let rest = Vec3::new(-0.2, -0.2, -4.0);
        let barrel = app
            .world_mut()
            .spawn((
                BarrelMesh {
                    tank,
                    rest_translation: rest,
                },
                Transform::from_translation(rest),
            ))
            .id();
        app.update();

        app.world_mut().write_message(TankFiredMessage {
            tank,
            team: Some(Team::Player),
            muzzle_pos: Vec3::ZERO,
            muzzle_dir: Vec3::NEG_Z,
            gun_pivot: Vec3::ZERO,
            caliber_mm: 122.0,
            is_player: true,
            has_muzzle_brake: true,
        });

        let mut max_z_shift = 0.0f32;
        for _ in 0..50 {
            app.update();
            let tf = app.world().get::<Transform>(barrel).unwrap();
            max_z_shift = max_z_shift.max(tf.translation.z - rest.z);
        }
        assert!(max_z_shift > 0.25, "ствол должен откатиться назад (+Z): {max_z_shift}");
        let end_tf = app.world().get::<Transform>(barrel).unwrap();
        assert!(
            (end_tf.translation - rest).length() < 1e-3,
            "после наката ствол должен вернуться в исходное положение: {:?}",
            end_tf.translation
        );
    }

    #[test]
    fn vfx_spawns_particles_for_shot_ricochet_and_fire() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            TransformPlugin,
            StatesPlugin,
        ))
        .init_state::<GameState>()
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_resource::<CameraShake>()
        .add_message::<TankFiredMessage>()
        .add_message::<ArmorImpactMessage>()
        .add_message::<TerrainImpactMessage>()
        .add_message::<TankDestroyedMessage>()
        .add_message::<TankActionMessage>()
        .add_plugins(VfxPlugin)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            0.03,
        )));
        app.world_mut()
            .resource_mut::<NextState<GameState>>()
            .set(GameState::InBattle);
        app.finish();
        app.cleanup();
        app.update();

        let spec = load_tank_spec("tiger_1").unwrap();
        let dummy = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
                TankRoot {
                    entity: Entity::PLACEHOLDER,
                    spec: spec.clone(),
                    spawn_pos: Vec3::ZERO,
                    spawn_yaw: 0.0,
                },
                TankCombat::new(Team::Player, spec.name.clone()),
                {
                    let mut m = TankModules::for_spec(&spec);
                    m.fire_timer = 4.0;
                    m
                },
            ))
            .id();

        app.world_mut().write_message(TankFiredMessage {
            tank: dummy,
            team: Some(Team::Player),
            muzzle_pos: Vec3::new(0.0, 2.0, -4.0),
            muzzle_dir: Vec3::NEG_Z,
            gun_pivot: Vec3::new(0.0, 2.0, -1.0),
            caliber_mm: 88.0,
            is_player: true,
            has_muzzle_brake: true,
        });
        app.world_mut().write_message(ArmorImpactMessage {
            plate_entity: dummy,
            owner_tank: Some(dummy),
            zone: None,
            target_name: "Броня".into(),
            shooter: None,
            shooter_team: Some(Team::Enemy),
            hit_point: Vec3::new(0.0, 1.5, 0.0),
            hit_normal: Vec3::Z,
            ray_dir: Vec3::new(0.5, -0.1, -0.85).normalize(),
            dist_m: 120.0,
            angle_deg: 76.0,
            armor_thickness_mm: 100.0,
            shell: spec.shell,
            outcome: ImpactOutcome::Ricochet,
        });
        for _ in 0..4 {
            app.update();
        }

        let particle_cnt = app
            .world_mut()
            .query::<&VfxParticle>()
            .iter(app.world())
            .count();
        let fire_light_cnt = app
            .world_mut()
            .query::<&TankFireLight>()
            .iter(app.world())
            .count();
        assert!(particle_cnt >= 20, "должны появиться частицы выстрела, рикошета и пожара: {particle_cnt}");
        assert_eq!(fire_light_cnt, 1, "у горящего танка должен быть мерцающий свет пожара");

        // Тушим пожар -> свет пожара исчезает.
        app.world_mut()
            .get_mut::<TankModules>(dummy)
            .unwrap()
            .fire_timer = 0.0;
        app.update();
        let fire_light_after = app
            .world_mut()
            .query::<&TankFireLight>()
            .iter(app.world())
            .count();
        assert_eq!(fire_light_after, 0, "после тушения пожара свет должен погаснуть");
    }
}
