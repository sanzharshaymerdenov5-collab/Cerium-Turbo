//! Модель пробития сплошным снарядом (AP): рикошет, нормализация, потеря с дистанцией.

use super::shell::ShellSpec;

/// Итог попадания по броне.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImpactOutcome {
    Ricochet,
    /// Не пробил; `effective_mm` — эффективная толщина.
    NoPenetration { effective_mm: f32 },
    Penetration { effective_mm: f32 },
}

/// Бронепробитие на дистанции `dist_m` (по нормали), мм.
pub fn penetration_at(spec: &ShellSpec, dist_m: f32) -> f32 {
    (spec.pen_at_100m_mm - spec.pen_drop_per_100m_mm * (dist_m - 100.0) / 100.0).max(0.0)
}

/// Оценка попадания.
/// `impact_angle_deg` — угол между траекторией и нормалью плиты (0° = по нормали).
pub fn evaluate_impact(
    spec: &ShellSpec,
    dist_m: f32,
    armor_mm: f32,
    impact_angle_deg: f32,
) -> ImpactOutcome {
    if impact_angle_deg > spec.ricochet_deg {
        return ImpactOutcome::Ricochet;
    }
    let eff_angle = (impact_angle_deg - spec.normalization_deg).max(0.0);
    let effective_mm = armor_mm / eff_angle.to_radians().cos().max(1e-3);
    if penetration_at(spec, dist_m) >= effective_mm {
        ImpactOutcome::Penetration { effective_mm }
    } else {
        ImpactOutcome::NoPenetration { effective_mm }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ballistics::shell::ShellType;

    fn spec() -> ShellSpec {
        ShellSpec {
            shell_type: ShellType::SolidAP,
            caliber_mm: 85.0,
            muzzle_velocity: 800.0,
            pen_at_100m_mm: 130.0,
            pen_drop_per_100m_mm: 4.0,
            normalization_deg: 5.0,
            ricochet_deg: 70.0,
        }
    }

    #[test]
    fn penetration_decreases_with_distance() {
        let s = spec();
        assert_eq!(penetration_at(&s, 100.0), 130.0);
        assert_eq!(penetration_at(&s, 500.0), 114.0);
        assert!(penetration_at(&s, 1e6) >= 0.0);
    }

    #[test]
    fn ricochet_above_limit() {
        assert_eq!(evaluate_impact(&spec(), 100.0, 10.0, 71.0), ImpactOutcome::Ricochet);
        assert!(!matches!(
            evaluate_impact(&spec(), 100.0, 10.0, 69.0),
            ImpactOutcome::Ricochet
        ));
    }

    #[test]
    fn normal_hit_uses_nominal_thickness() {
        match evaluate_impact(&spec(), 100.0, 100.0, 0.0) {
            ImpactOutcome::Penetration { effective_mm } => {
                assert!((effective_mm - 100.0).abs() < 1e-3)
            }
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn angle_increases_effective_thickness_with_normalization() {
        // 60° с нормализацией 5° → эффективный угол 55°: 60 / cos(55°) ≈ 104.6 мм
        match evaluate_impact(&spec(), 100.0, 60.0, 60.0) {
            ImpactOutcome::Penetration { effective_mm } => {
                assert!((effective_mm - 60.0 / 55f32.to_radians().cos()).abs() < 1e-2);
            }
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn thick_sloped_armor_stops_shell() {
        assert!(matches!(
            evaluate_impact(&spec(), 250.0, 100.0, 60.0),
            ImpactOutcome::NoPenetration { .. }
        ));
    }
}
