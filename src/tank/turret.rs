//! Наведение башни (по горизонтали) и орудия (по вертикали с УВН) к точке прицеливания.

use bevy::prelude::*;

use super::armor::TankCombat;
use super::spawn::{AimCommand, GunPivot, TankRoot, TurretPivot};

/// Разность углов в `[-π, π]`.
pub fn angle_diff(target: f32, current: f32) -> f32 {
    (target - current + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI
}

/// Шаг поворота к целевому углу с ограничением угловой скорости.
pub fn step_angle(current: f32, target: f32, max_step: f32) -> f32 {
    let d = angle_diff(target, current);
    current + d.clamp(-max_step, max_step)
}

pub fn aim_turret_and_gun(
    time: Res<Time>,
    roots: Query<(&GlobalTransform, &TankRoot, &AimCommand, Option<&TankCombat>)>,
    mut turrets: Query<(&mut Transform, &GlobalTransform, &TurretPivot), Without<GunPivot>>,
    mut guns: Query<(&mut Transform, &GunPivot), Without<TurretPivot>>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    for (root_gt, root, aim_cmd, combat) in &roots {
        if combat.is_some_and(|c| !c.alive) {
            continue;
        }
        let traverse_mult = combat.map_or(1.0, |c| c.traverse_mult);
        let elevation_mult = combat.map_or(1.0, |c| c.elevation_mult);
        let spec = &root.spec;
        let inv_root = root_gt.affine().inverse();

        for (mut t_tf, t_gt, pivot) in &mut turrets {
            if pivot.tank != root.entity {
                continue;
            }
            let local_aim = inv_root.transform_point3(aim_cmd.world_point) - t_tf.translation;
            let mut desired_yaw = (-local_aim.x).atan2(-local_aim.z);
            let (cur_yaw, _, _) = t_tf.rotation.to_euler(EulerRot::YXZ);

            let new_yaw = if let Some((min_deg, max_deg)) = spec.yaw_limits_deg {
                let min_r = min_deg.to_radians();
                let max_r = max_deg.to_radians();
                desired_yaw = desired_yaw.clamp(min_r, max_r);
                let max_yaw = (spec.turret_traverse_dps * traverse_mult).to_radians() * dt;
                step_angle(cur_yaw, desired_yaw, max_yaw).clamp(min_r, max_r)
            } else {
                let max_yaw = (spec.turret_traverse_dps * traverse_mult).to_radians() * dt;
                step_angle(cur_yaw, desired_yaw, max_yaw)
            };
            t_tf.rotation = Quat::from_rotation_y(new_yaw);

            // Вертикальное наведение в кадре башни/рубки.
            let inv_turret = t_gt.affine().inverse();
            for (mut g_tf, g_pivot) in &mut guns {
                if g_pivot.tank != root.entity {
                    continue;
                }
                let aim_in_turret =
                    inv_turret.transform_point3(aim_cmd.world_point) - g_tf.translation;
                let horiz = aim_in_turret.x.hypot(aim_in_turret.z).max(1e-3);
                let desired_pitch = aim_in_turret.y.atan2(horiz).clamp(
                    -spec.gun_depression_deg.to_radians(),
                    spec.gun_elevation_deg.to_radians(),
                );
                let (_, cur_pitch, _) = g_tf.rotation.to_euler(EulerRot::YXZ);
                let pitch_speed = (spec.turret_traverse_dps * 0.9 * elevation_mult)
                    .to_radians()
                    * dt;
                let new_pitch = step_angle(cur_pitch, desired_pitch, pitch_speed).clamp(
                    -spec.gun_depression_deg.to_radians(),
                    spec.gun_elevation_deg.to_radians(),
                );
                g_tf.rotation = Quat::from_rotation_x(new_pitch);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_angle_takes_shortest_arc_across_pi() {
        let a = 170f32.to_radians();
        let b = -170f32.to_radians();
        let d = angle_diff(b, a).to_degrees();
        assert!((d - 20.0).abs() < 1e-3, "d={d}");
        let stepped = step_angle(a, b, 5f32.to_radians());
        assert!(angle_diff(b, stepped).abs() < angle_diff(b, a).abs());
    }
}
