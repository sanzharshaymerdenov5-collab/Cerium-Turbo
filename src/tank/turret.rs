//! Наведение башни (по горизонтали) и орудия (по вертикали с УВН) к точке прицеливания.

use bevy::prelude::*;

use super::camera::AimTarget;
use super::spawn::{GunPivot, PlayerTank, TankRoot, TurretPivot};

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
    aim: Res<AimTarget>,
    roots: Query<(&GlobalTransform, &TankRoot), With<PlayerTank>>,
    mut turrets: Query<
        (&mut Transform, &GlobalTransform, &TurretPivot),
        Without<GunPivot>,
    >,
    mut guns: Query<(&mut Transform, &GunPivot), Without<TurretPivot>>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let Ok((root_gt, root)) = roots.single() else {
        return;
    };
    let spec = &root.spec;

    for (mut t_tf, t_gt, pivot) in &mut turrets {
        if pivot.tank != root.entity {
            continue;
        }
        // Точка прицеливания в локальном кадре корпуса (погон башни — в base_offset).
        let inv_root = root_gt.affine().inverse();
        let local_aim = inv_root.transform_point3(aim.point) - t_tf.translation;
        // Вперёд = −Z, поворот вокруг +Y: dir = (−sin(yaw), 0, −cos(yaw)).
        let desired_yaw = (-local_aim.x).atan2(-local_aim.z);
        let (cur_yaw, _, _) = t_tf.rotation.to_euler(EulerRot::YXZ);
        let max_yaw = spec.turret_traverse_dps.to_radians() * dt;
        let new_yaw = step_angle(cur_yaw, desired_yaw, max_yaw);
        t_tf.rotation = Quat::from_rotation_y(new_yaw);

        // Вертикальное наведение в кадре башни.
        let inv_turret = t_gt.affine().inverse();
        for (mut g_tf, g_pivot) in &mut guns {
            if g_pivot.tank != root.entity {
                continue;
            }
            let aim_in_turret = inv_turret.transform_point3(aim.point) - g_tf.translation;
            let horiz = aim_in_turret.x.hypot(aim_in_turret.z).max(1e-3);
            // Положительный pitch поднимает ствол (вокруг +X: −Z переходит в +Y).
            let desired_pitch = aim_in_turret.y.atan2(horiz).clamp(
                -spec.gun_depression_deg.to_radians(),
                spec.gun_elevation_deg.to_radians(),
            );
            let (_, cur_pitch, _) = g_tf.rotation.to_euler(EulerRot::YXZ);
            let max_pitch = (spec.turret_traverse_dps * 0.9).to_radians() * dt;
            let new_pitch = step_angle(cur_pitch, desired_pitch, max_pitch).clamp(
                -spec.gun_depression_deg.to_radians(),
                spec.gun_elevation_deg.to_radians(),
            );
            g_tf.rotation = Quat::from_rotation_x(new_pitch);
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
