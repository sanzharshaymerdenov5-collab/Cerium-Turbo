//! Расчёт баллистического упреждения ИИ для сплошного снаряда `SolidAP`.

use bevy::math::Vec3;

use crate::voxel::Rng;

/// Вычисляет точку прицеливания с учётом:
/// 1) движения цели (`target_vel`),
/// 2) падения сплошного AP-снаряда под действием гравитации (`gravity`).
pub fn compute_ballistic_lead(
    muzzle_pos: Vec3,
    target_pos: Vec3,
    target_vel: Vec3,
    shell_speed: f32,
    gravity: f32,
) -> Vec3 {
    let speed = shell_speed.max(100.0);
    let mut t = muzzle_pos.distance(target_pos) / speed;
    let mut predicted = target_pos;
    for _ in 0..3 {
        predicted = target_pos + target_vel * t;
        t = muzzle_pos.distance(predicted) / speed;
    }
    predicted + Vec3::Y * (0.5 * gravity * t * t)
}

/// Добавляет небольшой разброс прицеливания (радианы) для реалистичной стрельбы ИИ.
pub fn apply_ai_dispersion(aim_point: Vec3, muzzle_pos: Vec3, rng: &mut Rng, spread_rad: f32) -> Vec3 {
    let to = aim_point - muzzle_pos;
    let dist = to.length().max(1.0);
    let dir = to / dist;
    let right = dir.cross(Vec3::Y).normalize_or_zero();
    let up = right.cross(dir).normalize_or_zero();
    let ox = rng.signed() * spread_rad * dist;
    let oy = rng.signed() * spread_rad * 0.6 * dist;
    aim_point + right * ox + up * oy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ballistic_lead_compensates_gravity_and_target_velocity() {
        let muzzle = Vec3::new(0.0, 2.0, 0.0);
        let target = Vec3::new(0.0, 2.0, -400.0);
        let vel = Vec3::new(15.0, 0.0, 0.0);
        let aim = compute_ballistic_lead(muzzle, target, vel, 800.0, 9.81);

        // Время полёта ~0.5 с -> цель сместится примерно на +7.5 м по X,
        // а снаряд просядет на 0.5 * 9.81 * 0.25 ≈ 1.226 м по Y.
        assert!((aim.x - 7.5).abs() < 0.3, "aim.x={}", aim.x);
        assert!(aim.y > target.y + 1.0 && aim.y < target.y + 1.5, "aim.y={}", aim.y);
    }
}
