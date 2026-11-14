//! Типы снарядов.
//!
//! # Правило AP-Only
//! До завершения Этапа 6 существует **только** [`ShellType::SolidAP`]
//! (сплошной бронебойный). APHE/HE/HEAT/APCR — только после Этапа 6.

use serde::{Deserialize, Serialize};

/// Тип снаряда. Намеренно один вариант (см. правило AP-Only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ShellType {
    /// Сплошной бронебойный снаряд.
    #[default]
    SolidAP,
}

impl ShellType {
    /// Все существующие типы снарядов (для guard-теста правила AP-Only).
    pub const ALL: [ShellType; 1] = [ShellType::SolidAP];
}

fn default_arcade_damage() -> f32 {
    180.0
}

/// Параметры снаряда (из `.ron` танка).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ShellSpec {
    pub shell_type: ShellType,
    pub caliber_mm: f32,
    /// Начальная скорость, м/с.
    pub muzzle_velocity: f32,
    /// Бронепробитие на 100 м (по нормали), мм.
    pub pen_at_100m_mm: f32,
    /// Потеря бронепробития на каждые 100 м, мм.
    pub pen_drop_per_100m_mm: f32,
    /// Нормализация снаряда (уменьшает эффективный угол), град.
    pub normalization_deg: f32,
    /// Угол встречи, выше которого — рикошет, град.
    pub ricochet_deg: f32,
    /// Урон по общему ХП в аркадном режиме WoT (Этап 3+).
    #[serde(default = "default_arcade_damage")]
    pub arcade_damage: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guard правила AP-Only: до Этапа 6 допустим ровно один тип снаряда.
    #[test]
    fn only_solid_ap_exists() {
        assert_eq!(ShellType::ALL.len(), 1);
        assert_eq!(ShellType::ALL[0], ShellType::SolidAP);
        match ShellType::default() {
            ShellType::SolidAP => {}
        }
    }
}
