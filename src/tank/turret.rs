//! Наведение башни (по горизонтали) и орудия (по вертикали с УВН) к точке прицеливания.

use bevy::math::Affine3A;
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

/// Вычисляет актуальные мировые координаты цапф орудия и дульного среза по текущим
/// локальным `Transform` башни/орудия/ствола без однокадрового отставания `GlobalTransform`.
pub fn compute_gun_and_muzzle_world(
    root_gt: &GlobalTransform,
    turret_tf: &Transform,
    gun_tf: &Transform,
    muzzle_tf: &Transform,
) -> (Vec3, GlobalTransform) {
    let turret_affine = root_gt.affine()
        * Affine3A::from_translation(turret_tf.translation)
        * Affine3A::from_quat(turret_tf.rotation);
    let gun_affine = turret_affine
        * Affine3A::from_translation(gun_tf.translation)
        * Affine3A::from_quat(gun_tf.rotation);
    let muzzle_affine = gun_affine
        * Affine3A::from_translation(muzzle_tf.translation)
        * Affine3A::from_quat(muzzle_tf.rotation);
    (gun_affine.translation.into(), GlobalTransform::from(muzzle_affine))
}

pub fn aim_turret_and_gun(
    time: Res<Time>,
    roots: Query<(&GlobalTransform, &TankRoot, &AimCommand, Option<&TankCombat>)>,
    mut turrets: Query<(&mut Transform, &TurretPivot), Without<GunPivot>>,
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

        for (mut t_tf, pivot) in &mut turrets {
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

            // Вертикальное наведение в актуальном кадре башни/рубки (без задержки GlobalTransform).
            let turret_affine = root_gt.affine()
                * Affine3A::from_translation(t_tf.translation)
                * Affine3A::from_quat(t_tf.rotation);
            let inv_turret = turret_affine.inverse();
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
    use crate::tank::spec::load_tank_spec;
    use std::time::Duration;
    use bevy::time::TimeUpdateStrategy;

    #[test]
    fn step_angle_takes_shortest_arc_across_pi() {
        let a = 170f32.to_radians();
        let b = -170f32.to_radians();
        let d = angle_diff(b, a).to_degrees();
        assert!((d - 20.0).abs() < 1e-3, "d={d}");
        let stepped = step_angle(a, b, 5f32.to_radians());
        assert!(angle_diff(b, stepped).abs() < angle_diff(b, a).abs());
    }

    #[test]
    fn horizon_aim_converges_gun_level_without_downward_bias() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                1.0 / 60.0,
            )))
            .add_systems(Update, aim_turret_and_gun);

        let spec = load_tank_spec("t34_85").expect("t34_85 spec");
        let root_pos = Vec3::new(0.0, 3.0, 140.0);
        // Цель на горизонте на высоте оптической оси камеры (5.65 м на дистанции 800 м).
        let horizon_point = Vec3::new(0.0, 5.65, -660.0);

        let root_ent = app.world_mut().spawn_empty().id();
        let turret_offset = Vec3::from_array(spec.profile.turret_pivot);
        let gun_offset = Vec3::from_array(spec.profile.gun_pivot);
        let muzzle_offset = Vec3::new(0.0, 0.0, -(spec.profile.barrel_length + 0.15));

        app.world_mut().entity_mut(root_ent).insert((
            Transform::from_translation(root_pos),
            GlobalTransform::from_translation(root_pos),
            TankRoot {
                entity: root_ent,
                spec: spec.clone(),
                spawn_pos: root_pos,
                spawn_yaw: 0.0,
            },
            AimCommand {
                world_point: horizon_point,
            },
        ));
        let turret_ent = app
            .world_mut()
            .spawn((
                TurretPivot { tank: root_ent },
                Transform::from_translation(turret_offset),
            ))
            .id();
        let gun_ent = app
            .world_mut()
            .spawn((
                GunPivot { tank: root_ent },
                Transform::from_translation(gun_offset),
            ))
            .id();
        let muzzle_ent = app
            .world_mut()
            .spawn((Transform::from_translation(muzzle_offset),))
            .id();
        app.world_mut().entity_mut(gun_ent).add_child(muzzle_ent);
        app.world_mut().entity_mut(turret_ent).add_child(gun_ent);
        app.world_mut().entity_mut(root_ent).add_child(turret_ent);

        app.finish();
        app.cleanup();
        for _ in 0..30 {
            app.update();
        }

        let root_gt = *app.world().get::<GlobalTransform>(root_ent).unwrap();
        let t_tf = *app.world().get::<Transform>(turret_ent).unwrap();
        let g_tf = *app.world().get::<Transform>(gun_ent).unwrap();
        let m_tf = *app.world().get::<Transform>(muzzle_ent).unwrap();
        let (_, muzzle_gt) = compute_gun_and_muzzle_world(&root_gt, &t_tf, &g_tf, &m_tf);

        let to_horizon = (horizon_point - muzzle_gt.translation()).normalize();
        let muzzle_fwd = muzzle_gt.forward().as_vec3();
        let dot = muzzle_fwd.dot(to_horizon).clamp(-1.0, 1.0);
        let angle_err_deg = dot.acos().to_degrees();
        assert!(
            angle_err_deg < 0.05 && muzzle_fwd.y >= 0.0,
            "при прицеливании в горизонт ствол должен смотреть точно в горизонт: angle_err={angle_err_deg:.3}°, muzzle_fwd={muzzle_fwd:?}"
        );
    }
}
