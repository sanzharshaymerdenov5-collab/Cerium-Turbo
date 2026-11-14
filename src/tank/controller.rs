//! Управление танком: пружинная подвеска, гусеничная тяга, стрельба, отдача, возврат на колёса.

use avian3d::prelude::*;
use bevy::prelude::*;

use super::spawn::{
    GunPivot, MuzzlePoint, PlayerTank, Suspension, TankRoot, TankState,
};
use crate::ballistics::{HitLog, spawn_projectile};
use crate::voxel::{Material, Rng, VoxelAssets, VoxelWorld, spawn_debris_burst};

/// Чистая формула вертикальной силы пружинно-демпферной стойки:
/// `compression > 0` — сжатие (м), `rel_vel_up` — скорость точки крепления вдоль нормали вверх (м/с).
/// Стойка только толкает вверх (сила ≥ 0) и ограничена `max_force`.
pub fn compute_spring_force(
    compression: f32,
    rel_vel_up: f32,
    spring_k: f32,
    damper_c: f32,
    max_force: f32,
) -> f32 {
    if compression <= 0.0 || !compression.is_finite() || !rel_vel_up.is_finite() {
        return 0.0;
    }
    (compression * spring_k - rel_vel_up * damper_c).clamp(0.0, max_force)
}

/// Читает ввод игрока в `Update` (чтобы `just_pressed` не терялся между кадрами физики).
pub fn read_player_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut q: Query<&mut TankState, With<PlayerTank>>,
) {
    let Ok(mut st) = q.single_mut() else { return };
    let mut throttle = 0.0;
    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        throttle += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown) {
        throttle -= 1.0;
    }
    let mut steer = 0.0;
    if keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft) {
        steer += 1.0;
    }
    if keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight) {
        steer -= 1.0;
    }
    st.throttle = throttle;
    st.steer = steer;
    if keys.just_pressed(KeyCode::Space) || mouse.just_pressed(MouseButton::Left) {
        st.fire_queued = true;
    }
    if keys.just_pressed(KeyCode::KeyR) {
        st.reset_queued = true;
    }
}

/// Физический шаг подвески и гусеничного привода (`FixedUpdate`).
pub fn tank_suspension_and_drive(
    time: Res<Time>,
    world: Option<Res<VoxelWorld>>,
    mut q: Query<(&mut Transform, &TankRoot, &Suspension, &mut TankState, Forces)>,
) {
    let Some(world) = world else { return };
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    for (mut tf, root, susp, mut st, mut forces) in &mut q {
        if st.reset_queued {
            st.reset_queued = false;
            let ground_y = world
                .grid
                .surface_height_at(tf.translation.x, tf.translation.z)
                .unwrap_or(root.spawn_pos.y);
            tf.translation = Vec3::new(tf.translation.x, ground_y + 0.5, tf.translation.z);
            let (yaw, _, _) = tf.rotation.to_euler(EulerRot::YXZ);
            tf.rotation = Quat::from_rotation_y(yaw);
            *forces.linear_velocity_mut() = Vec3::ZERO;
            *forces.angular_velocity_mut() = Vec3::ZERO;
            continue;
        }

        let rot = tf.rotation;
        let pos = tf.translation;
        let up = rot * Vec3::Y;
        let forward = rot * Vec3::NEG_Z;
        let right = rot * Vec3::X;
        let lin_vel = forces.linear_velocity();
        let ang_vel = forces.angular_velocity();

        st.speed_kmh = lin_vel.dot(forward) * 3.6;

        // Если танк сильно накренился, мягкий стабилизирующий момент возвращает его.
        if up.y < 0.85 {
            let corr = up.cross(Vec3::Y) * (root.spec.mass_kg * 9.81 * 1.4);
            forces.apply_torque(corr);
        }

        let max_fwd = root.spec.max_speed_kmh / 3.6;
        let max_rev = root.spec.reverse_speed_kmh / 3.6;
        let target_fwd = if st.throttle >= 0.0 {
            st.throttle * max_fwd
        } else {
            st.throttle * max_rev
        };
        let track_span = (root.spec.profile.hull_width - root.spec.profile.track_width).max(1.5);
        let target_yaw = st.steer * root.spec.max_yaw_rate_dps.to_radians();
        // При движении назад инвертируем руление для естественного заднего хода.
        let steer_dir = if st.throttle < -0.1 { -1.0 } else { 1.0 };
        let diff_v = target_yaw * steer_dir * (track_span * 0.5);

        let max_drive_per_wheel = (root.spec.max_tractive_force_kn * 1000.0) / 8.0;
        let mut grounded = 0u8;

        for w in &susp.wheels {
            let wp = pos + rot * w.local_pos;
            // Луч вниз в мировом кадре (надёжно на крутых склонах).
            let Some(hit) = world.grid.raycast(wp, Vec3::NEG_Y, susp.max_drop) else {
                continue;
            };
            let dist = hit.distance;
            if dist > susp.rest_length {
                continue;
            }
            grounded += 1;
            let compression = susp.rest_length - dist;
            let r = wp - pos;
            let vel_at_wheel = lin_vel + ang_vel.cross(r);
            let f_norm = compute_spring_force(
                compression,
                vel_at_wheel.y,
                susp.spring_k,
                susp.damper_c,
                susp.max_force,
            );
            forces.apply_force_at_point(Vec3::Y * f_norm, wp);

            // Продольная скорость конкретной гусеницы (левая: v − diff, правая: v + diff).
            let desired_track_v = target_fwd + w.side * diff_v;
            let cur_fwd_v = vel_at_wheel.dot(forward);
            let cur_lat_v = vel_at_wheel.dot(right);

            let v_err = desired_track_v - cur_fwd_v;
            let f_long = (v_err * root.spec.mass_kg * 1.1)
                .clamp(-max_drive_per_wheel, max_drive_per_wheel);
            // Боковое трение гусениц препятствует заносу.
            let f_lat = (-cur_lat_v * root.spec.mass_kg * 2.2)
                .clamp(-f_norm * 1.15, f_norm * 1.15);

            let horiz = forward * f_long + right * f_lat;
            forces.apply_force_at_point(horiz, wp);
        }

        st.grounded_wheels = grounded;
    }
}

/// Перезарядка, выстрел и импульс отдачи.
#[allow(clippy::too_many_arguments)]
pub fn tank_fire_system(
    mut commands: Commands,
    time: Res<Time>,
    assets: Option<Res<VoxelAssets>>,
    mut log: ResMut<HitLog>,
    mut tanks: Query<(&TankRoot, &mut TankState, Forces)>,
    guns: Query<(&GlobalTransform, &GunPivot)>,
    muzzles: Query<(&GlobalTransform, &MuzzlePoint)>,
    mut seed: Local<u64>,
) {
    let Some(assets) = assets else { return };
    let dt = time.delta_secs();

    for (root, mut st, mut forces) in &mut tanks {
        st.reload_timer = (st.reload_timer - dt).max(0.0);
        if !st.fire_queued {
            continue;
        }
        st.fire_queued = false;
        if st.reload_timer > 0.0 {
            continue;
        }
        st.reload_timer = root.spec.reload_s;
        log.shots += 1;

        let Some((m_gt, _)) = muzzles.iter().find(|(_, m)| m.tank == root.entity) else {
            continue;
        };
        let origin = m_gt.translation();
        let dir = m_gt.forward().as_vec3();

        spawn_projectile(&mut commands, &assets, origin, dir, root.spec.shell);

        // Дульная вспышка / дым.
        *seed = seed.wrapping_add(0x517C_C1B7_2722_0A95);
        let mut rng = Rng::new(*seed | 1);
        spawn_debris_burst(
            &mut commands,
            &assets,
            0,
            origin,
            dir,
            Material::Sand,
            6,
            0.25,
            &mut rng,
        );

        // Импульс отдачи в точке цапф орудия.
        let recoil_pt = guns
            .iter()
            .find(|(_, g)| g.tank == root.entity)
            .map(|(gt, _)| gt.translation())
            .unwrap_or(origin);
        forces.apply_linear_impulse_at_point(-dir * root.spec.recoil_impulse_ns, recoil_pt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_force_is_finite_non_negative_and_monotonic() {
        assert_eq!(compute_spring_force(0.0, 0.0, 200_000.0, 30_000.0, 500_000.0), 0.0);
        assert_eq!(compute_spring_force(-0.2, -5.0, 200_000.0, 30_000.0, 500_000.0), 0.0);
        assert_eq!(
            compute_spring_force(f32::NAN, 0.0, 200_000.0, 30_000.0, 500_000.0),
            0.0
        );

        let f1 = compute_spring_force(0.05, 0.0, 200_000.0, 30_000.0, 500_000.0);
        let f2 = compute_spring_force(0.15, 0.0, 200_000.0, 30_000.0, 500_000.0);
        assert!(f2 > f1 && f1 > 0.0);

        // Быстрое разжатие не превращает пружину в «присоску».
        let rebound = compute_spring_force(0.05, 20.0, 200_000.0, 30_000.0, 500_000.0);
        assert_eq!(rebound, 0.0);
    }
}
