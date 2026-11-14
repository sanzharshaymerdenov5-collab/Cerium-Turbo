//! Искусственный интеллект вражеских танков: навигация A*, обзор (LoS),
//! тактика (`Patrol` / `Engage` / `TakeCover`), ромбование (~30°) и упреждение AP.

pub mod ballistics;
pub mod nav_grid;
pub mod perception;

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;

use crate::ballistics::projectile::GRAVITY;
use crate::core::GameState;
use crate::tank::turret::angle_diff;
use crate::tank::{AimCommand, MuzzlePoint, PlayerTank, TankCombat, TankRoot, TankState};
use crate::voxel::{Rng, VoxelWorld};

pub use ballistics::{apply_ai_dispersion, compute_ballistic_lead};
pub use nav_grid::{VoxelNavGrid, sync_nav_grid_with_destruction};
pub use perception::has_line_of_sight;

/// Состояние конечного автомата ИИ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiState {
    Patrol,
    Engage,
    TakeCover,
}

/// Мозг ИИ-танка.
#[derive(Component, Debug, Clone)]
pub struct AiBrain {
    pub state: AiState,
    pub waypoints: Vec<Vec3>,
    pub waypoint_idx: usize,
    pub path: Vec<Vec3>,
    pub repath_timer: f32,
    pub cover_target: Option<Vec3>,
    pub cover_timer: f32,
    pub last_seen_player: Option<Vec3>,
    pub time_since_seen: f32,
    /// Знак ромбования корпуса (+1 или −1 -> ±28° к направлению на цель).
    pub diamond_sign: f32,
    pub rng_seed: u64,
    pub stuck_timer: f32,
    pub reverse_timer: f32,
    pub last_pos: Vec3,
    pub check_pos_timer: f32,
}

impl AiBrain {
    pub fn new(waypoints: Vec<Vec3>, seed: u64, spawn_pos: Vec3) -> Self {
        let diamond_sign = if seed & 1 == 0 { 1.0 } else { -1.0 };
        Self {
            state: AiState::Patrol,
            waypoints,
            waypoint_idx: 0,
            path: Vec::new(),
            repath_timer: (seed % 7) as f32 * 0.12,
            cover_target: None,
            cover_timer: 0.0,
            last_seen_player: None,
            time_since_seen: 99.0,
            diamond_sign,
            rng_seed: seed.max(1),
            stuck_timer: 0.0,
            reverse_timer: 0.0,
            last_pos: spawn_pos,
            check_pos_timer: 0.0,
        }
    }
}

pub struct AiPlugin;

impl Plugin for AiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (sync_nav_grid_with_destruction, ai_control_system)
                .chain()
                .run_if(in_state(GameState::InBattle))
                .run_if(resource_exists::<VoxelWorld>)
                .run_if(resource_exists::<VoxelNavGrid>),
        )
        .add_systems(OnExit(GameState::InBattle), cleanup_ai_resources);
    }
}

fn cleanup_ai_resources(mut commands: Commands) {
    commands.remove_resource::<VoxelNavGrid>();
}

/// Превращает желаемое мировое направление `(dx, dz)` в локальный руль и газ танка.
fn steer_towards_dir(rot: Quat, desired_dir_xz: Vec2) -> (f32, f32) {
    if desired_dir_xz.length_squared() < 1e-4 {
        return (0.0, 0.0);
    }
    let dir = desired_dir_xz.normalize();
    let (cur_yaw, _, _) = rot.to_euler(EulerRot::YXZ);
    // Вперед = −Z, при повороте Y(yaw) -> (-sin yaw, -cos yaw).
    let desired_yaw = (-dir.x).atan2(-dir.y);
    let err = angle_diff(desired_yaw, cur_yaw);
    let steer = (err / 0.45).clamp(-1.0, 1.0);
    let throttle = if err.abs() < 0.75 { 0.88 } else { 0.15 };
    (throttle, steer)
}

#[allow(clippy::too_many_arguments)]
fn ai_control_system(
    time: Res<Time>,
    world: Res<VoxelWorld>,
    nav: Res<VoxelNavGrid>,
    player_q: Query<(&Transform, &LinearVelocity, &TankCombat), With<PlayerTank>>,
    muzzles: Query<(&GlobalTransform, &MuzzlePoint)>,
    mut bots: Query<
        (
            Entity,
            &Transform,
            &TankRoot,
            &TankCombat,
            &mut TankState,
            &mut AimCommand,
            &mut AiBrain,
        ),
        Without<PlayerTank>,
    >,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    let player_info = player_q.single().ok().map(|(tf, lv, c)| {
        (tf.translation, lv.0, c.alive)
    });

    for (bot_ent, tf, root, combat, mut st, mut aim_cmd, mut brain) in &mut bots {
        if !combat.alive {
            st.throttle = 0.0;
            st.steer = 0.0;
            st.fire_queued = false;
            continue;
        }

        let pos = tf.translation;
        let rot = tf.rotation;
        let bot_eye = pos + Vec3::Y * 1.9;

        // Контроль застревания (если танк с газом почти не сдвигается 1.6 с -> сдаёт назад с разворотом).
        brain.check_pos_timer += dt;
        if brain.check_pos_timer >= 0.6 {
            let moved = pos.distance(brain.last_pos);
            if st.throttle.abs() > 0.4 && !combat.immobilized && moved < 0.6 {
                brain.stuck_timer += brain.check_pos_timer;
            } else {
                brain.stuck_timer = 0.0;
            }
            brain.last_pos = pos;
            brain.check_pos_timer = 0.0;
            if brain.stuck_timer >= 1.8 {
                brain.reverse_timer = 1.1;
                brain.stuck_timer = 0.0;
            }
        }

        let Some((player_pos, player_vel, player_alive)) = player_info else {
            st.throttle = 0.0;
            st.steer = 0.0;
            continue;
        };
        if !player_alive {
            st.throttle = 0.0;
            st.steer = 0.0;
            continue;
        }

        let player_aim_center = player_pos + Vec3::Y * 1.1;
        let dist_player = pos.distance(player_pos);
        let los = dist_player < 420.0 && has_line_of_sight(&world.grid, bot_eye, player_aim_center);

        if los {
            brain.last_seen_player = Some(player_pos);
            brain.time_since_seen = 0.0;
        } else {
            brain.time_since_seen += dt;
        }

        // Обновление состояния FSM.
        brain.cover_timer = (brain.cover_timer - dt).max(0.0);
        if los {
            if st.reload_timer > 1.3
                && dist_player < 130.0
                && !combat.immobilized
                && brain.state != AiState::TakeCover
            {
                if let Some(cover) = nav.find_cover_near(&world.grid, pos, player_pos, 24.0) {
                    brain.state = AiState::TakeCover;
                    brain.cover_target = Some(cover);
                    brain.cover_timer = 2.6;
                    brain.path = nav.find_path(pos, cover);
                } else {
                    brain.state = AiState::Engage;
                }
            } else if brain.state == AiState::TakeCover && (st.reload_timer < 0.7 || brain.cover_timer <= 0.0) {
                brain.state = AiState::Engage;
            } else if brain.state != AiState::TakeCover {
                brain.state = AiState::Engage;
            }
        } else if brain.last_seen_player.is_some() && brain.time_since_seen < 12.0 {
            if brain.state == AiState::TakeCover && brain.cover_timer > 0.0 {
                // Доигрываем укрытие, затем снова ищем цель.
            } else {
                brain.state = AiState::Engage;
            }
        } else {
            brain.state = AiState::Patrol;
        }

        brain.repath_timer -= dt;
        let (muzzle_pos, muzzle_dir) = muzzles
            .iter()
            .find(|(_, m)| m.tank == bot_ent)
            .map(|(gt, _)| (gt.translation(), gt.forward().as_vec3()))
            .unwrap_or((bot_eye, rot * Vec3::NEG_Z));

        // 1. Прицеливание и огонь.
        if los || brain.time_since_seen < 1.8 {
            let target_pt = if los {
                player_aim_center
            } else {
                brain.last_seen_player.unwrap_or(player_aim_center) + Vec3::Y * 1.1
            };
            let vel = if los { player_vel } else { Vec3::ZERO };
            let raw_lead = compute_ballistic_lead(
                muzzle_pos,
                target_pt,
                vel,
                root.spec.shell.muzzle_velocity,
                GRAVITY,
            );
            brain.rng_seed = brain.rng_seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut rng = Rng::new(brain.rng_seed);
            // Небольшой разброс (пересчитывается плавно — чуть дрожит при наведении).
            let aimed = apply_ai_dispersion(raw_lead, muzzle_pos, &mut rng, 0.008);
            aim_cmd.world_point = aimed;

            let to_aim = (raw_lead - muzzle_pos).normalize_or_zero();
            let aligned = muzzle_dir.dot(to_aim) > 0.996;
            if los && aligned && st.reload_timer <= 0.0 && !combat.gun_jammed {
                st.fire_queued = true;
            }
        } else {
            let look_pt = brain
                .path
                .first()
                .copied()
                .unwrap_or(pos + (rot * Vec3::NEG_Z) * 20.0);
            aim_cmd.world_point = look_pt + Vec3::Y * 1.4;
        }

        // 2. Вождение и ромбование корпуса.
        if brain.reverse_timer > 0.0 {
            brain.reverse_timer -= dt;
            st.throttle = -0.8;
            st.steer = brain.diamond_sign;
            continue;
        }

        match brain.state {
            AiState::Engage if los && dist_player >= 42.0 && dist_player <= 125.0 => {
                // Огневая позиция: встаём «ромбом» (~28°) или лбом (если безбашенная ПТ-САУ).
                let to_player_xz = Vec2::new(player_pos.x - pos.x, player_pos.z - pos.z);
                if to_player_xz.length_squared() > 1e-3 {
                    let base_dir = to_player_xz.normalize();
                    let angle_rad = if root.spec.yaw_limits_deg.is_some() {
                        0.0
                    } else {
                        brain.diamond_sign * 28f32.to_radians()
                    };
                    let (s, c) = angle_rad.sin_cos();
                    let diamond_dir = Vec2::new(
                        base_dir.x * c - base_dir.y * s,
                        base_dir.x * s + base_dir.y * c,
                    );
                    let (cur_yaw, _, _) = rot.to_euler(EulerRot::YXZ);
                    let target_yaw = (-diamond_dir.x).atan2(-diamond_dir.y);
                    let err = angle_diff(target_yaw, cur_yaw);
                    st.steer = (err / 0.35).clamp(-1.0, 1.0);
                    st.throttle = if dist_player > 105.0 { 0.3 } else { 0.0 };
                }
            }
            AiState::Engage => {
                let goal = brain.last_seen_player.unwrap_or(player_pos);
                if brain.repath_timer <= 0.0 || brain.path.is_empty() {
                    brain.repath_timer = 1.1;
                    brain.path = nav.find_path(pos, goal);
                }
                follow_nav_path(pos, rot, &mut brain.path, &mut st);
            }
            AiState::TakeCover => {
                if let Some(cover) = brain.cover_target {
                    if brain.path.is_empty() && pos.distance(cover) > 3.0 {
                        brain.path = nav.find_path(pos, cover);
                    }
                    follow_nav_path(pos, rot, &mut brain.path, &mut st);
                }
            }
            AiState::Patrol => {
                if !brain.waypoints.is_empty() {
                    let wp = brain.waypoints[brain.waypoint_idx % brain.waypoints.len()];
                    if Vec2::new(pos.x - wp.x, pos.z - wp.z).length() < 6.0 {
                        brain.waypoint_idx = (brain.waypoint_idx + 1) % brain.waypoints.len();
                        brain.path.clear();
                    }
                    let next_wp = brain.waypoints[brain.waypoint_idx % brain.waypoints.len()];
                    if brain.repath_timer <= 0.0 || brain.path.is_empty() {
                        brain.repath_timer = 1.4;
                        brain.path = nav.find_path(pos, next_wp);
                    }
                    follow_nav_path(pos, rot, &mut brain.path, &mut st);
                } else {
                    st.throttle = 0.0;
                    st.steer = 0.0;
                }
            }
        }
    }
}

fn follow_nav_path(pos: Vec3, rot: Quat, path: &mut Vec<Vec3>, st: &mut TankState) {
    while let Some(&first) = path.first() {
        if Vec2::new(first.x - pos.x, first.z - pos.z).length() < 3.5 {
            path.remove(0);
        } else {
            break;
        }
    }
    if let Some(&next) = path.first() {
        let dir_xz = Vec2::new(next.x - pos.x, next.z - pos.z);
        let (thr, str_cmd) = steer_towards_dir(rot, dir_xz);
        st.throttle = thr;
        st.steer = str_cmd;
    } else {
        st.throttle = 0.0;
        st.steer = 0.0;
    }
}
