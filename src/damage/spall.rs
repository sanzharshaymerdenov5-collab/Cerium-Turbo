//! Заброневое кинетическое поражение сплошным снарядом `SolidAP` (Этап 6):
//! расчёт остаточного пробития и генерация детерминированного 3D-конуса осколков (Spall Cone).

use bevy::math::Vec3;

use crate::ballistics::penetration::{effective_armor_mm, penetration_at};
use crate::ballistics::shell::ShellSpec;
use crate::voxel::Rng;

/// Остаточное пробитие сплошного сердечника после преодоления бронеплиты, мм.
pub fn compute_residual_penetration(
    shell: &ShellSpec,
    dist_m: f32,
    armor_mm: f32,
    angle_deg: f32,
) -> f32 {
    let pen = penetration_at(shell, dist_m);
    let eff = effective_armor_mm(shell, armor_mm, angle_deg);
    (pen - eff).max(0.0)
}

/// Определяет параметры кинетического конуса осколков по толщине пробитой брони и калибру `SolidAP`:
/// `(число_осколков, половинный_угол_конуса_в_градусах)`.
pub fn spall_cone_params(shell: &ShellSpec, residual_pen_mm: f32, armor_mm: f32) -> (usize, f32) {
    let cal_bonus = ((shell.caliber_mm - 75.0) / 10.0).max(0.0) as usize * 2;
    let armor_bonus = (armor_mm / 35.0).floor() as usize * 2;
    let count = (14 + cal_bonus + armor_bonus).clamp(14, 24);
    let half_angle_deg = (24.0 + armor_mm * 0.08 + (residual_pen_mm * 0.03).min(6.0)).clamp(24.0, 35.0);
    (count, half_angle_deg)
}

/// Строит детерминированный набор единичных лучей в 3D-конусе вокруг `dir` с углом `half_angle_deg`.
pub fn generate_spall_rays(
    dir: Vec3,
    seed: u64,
    count: usize,
    half_angle_deg: f32,
) -> Vec<Vec3> {
    let forward = dir.normalize_or_zero();
    if forward == Vec3::ZERO || count == 0 {
        return Vec::new();
    }

    let ref_up = if forward.y.abs() < 0.92 { Vec3::Y } else { Vec3::X };
    let right = forward.cross(ref_up).normalize();
    let up = right.cross(forward).normalize();

    let max_theta = half_angle_deg.clamp(5.0, 60.0).to_radians();
    let mut rng = Rng::new(seed | 1);
    let mut rays = Vec::with_capacity(count);
    let golden_angle = 2.399_963_2_f32;

    for i in 0..count {
        let u = ((i as f32 + 0.35 + 0.3 * rng.f32()) / count as f32).clamp(0.02, 1.0);
        let theta = u.sqrt() * max_theta;
        let phi = (i as f32) * golden_angle + rng.f32() * 0.25;
        let (st, ct) = theta.sin_cos();
        let (sp, cp) = phi.sin_cos();
        let ray = (forward * ct + (right * cp + up * sp) * st).normalize();
        rays.push(ray);
    }
    rays
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tank::load_tank_spec;

    #[test]
    fn spall_rays_stay_within_cone_and_are_deterministic() {
        let half_deg = 28.0f32;
        let a = generate_spall_rays(Vec3::NEG_Z, 12345, 20, half_deg);
        let b = generate_spall_rays(Vec3::NEG_Z, 12345, 20, half_deg);
        assert_eq!(a.len(), 20);
        assert_eq!(a, b);

        let min_cos = (half_deg + 0.05).to_radians().cos();
        for r in &a {
            assert!((r.length() - 1.0).abs() < 1e-4);
            let dot = r.dot(Vec3::NEG_Z);
            assert!(dot >= min_cos, "луч вышел за пределы конуса: dot={dot} < {min_cos}");
        }
    }

    #[test]
    fn residual_penetration_and_spall_params_scale_with_caliber_and_armor() {
        let t34 = load_tank_spec("t34_85").unwrap();
        let su100 = load_tank_spec("su_100").unwrap();
        let res_85 = compute_residual_penetration(&t34.shell, 100.0, 45.0, 0.0);
        let res_100 = compute_residual_penetration(&su100.shell, 100.0, 45.0, 0.0);
        assert!(res_100 > res_85 && res_85 > 50.0);

        let (c_thin, a_thin) = spall_cone_params(&t34.shell, res_85, 25.0);
        let (c_thick, a_thick) = spall_cone_params(&su100.shell, res_100, 100.0);
        assert!(c_thick >= c_thin);
        assert!(a_thick >= a_thin);
    }
}
