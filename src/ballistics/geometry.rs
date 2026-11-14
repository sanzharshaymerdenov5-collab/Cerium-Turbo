//! Чистая геометрия: пересечение отрезка с прямоугольной плитой брони.

use bevy::math::{Quat, Vec2, Vec3};

/// Результат пересечения луча с плитой.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlateHit {
    /// Расстояние вдоль луча.
    pub t: f32,
    pub point: Vec3,
    /// Нормаль плиты, обращённая к источнику луча.
    pub normal: Vec3,
    /// Угол между лучом и нормалью, град (0° = по нормали).
    pub angle_deg: f32,
}

/// Луч (`dir` нормализован) против плиты: центр `center`, поворот `rot`
/// (локальная +Z = нормаль), полуразмеры `half` по локальным X/Y.
pub fn ray_plate(
    origin: Vec3,
    dir: Vec3,
    max_dist: f32,
    center: Vec3,
    rot: Quat,
    half: Vec2,
) -> Option<PlateHit> {
    let n = rot * Vec3::Z;
    let denom = dir.dot(n);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (center - origin).dot(n) / denom;
    if !(0.0..=max_dist).contains(&t) {
        return None;
    }
    let point = origin + dir * t;
    let local = rot.inverse() * (point - center);
    if local.x.abs() > half.x || local.y.abs() > half.y {
        return None;
    }
    let normal = if denom < 0.0 { n } else { -n };
    let angle_deg = denom.abs().clamp(0.0, 1.0).acos().to_degrees();
    Some(PlateHit {
        t,
        point,
        normal,
        angle_deg,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_plate_head_on() {
        let h = ray_plate(
            Vec3::new(0.0, 0.0, 10.0),
            Vec3::NEG_Z,
            100.0,
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec2::new(1.0, 1.0),
        )
        .unwrap();
        assert!((h.t - 10.0).abs() < 1e-4);
        assert!(h.angle_deg < 1e-2);
        assert_eq!(h.normal, Vec3::Z);
    }

    #[test]
    fn misses_outside_extents_and_beyond_range() {
        let r = |o: Vec3, max: f32| {
            ray_plate(o, Vec3::NEG_Z, max, Vec3::ZERO, Quat::IDENTITY, Vec2::new(1.0, 1.0))
        };
        assert!(r(Vec3::new(2.0, 0.0, 10.0), 100.0).is_none());
        assert!(r(Vec3::new(0.0, 0.0, 10.0), 5.0).is_none());
    }

    #[test]
    fn tilted_plate_gives_expected_angle() {
        let rot = Quat::from_rotation_x(-60f32.to_radians());
        let h = ray_plate(
            Vec3::new(0.0, 0.0, 50.0),
            Vec3::NEG_Z,
            100.0,
            Vec3::ZERO,
            rot,
            Vec2::new(2.0, 2.0),
        )
        .unwrap();
        assert!((h.angle_deg - 60.0).abs() < 0.1, "{}", h.angle_deg);
        assert!(h.normal.y > 0.0, "наклон назад: нормаль смотрит вверх");
    }

    #[test]
    fn back_side_is_hit_too() {
        let h = ray_plate(
            Vec3::new(0.0, 0.0, -10.0),
            Vec3::Z,
            100.0,
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec2::splat(1.0),
        )
        .unwrap();
        assert_eq!(h.normal, Vec3::NEG_Z);
    }
}
