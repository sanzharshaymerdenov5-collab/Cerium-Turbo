//! Материалы вокселей: цвет и разрушаемость.

use serde::{Deserialize, Serialize};

/// Разрушаемость вокселя.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoxelDestructibility {
    /// Рельеф (земля, скалы, песок, снег) — неразрушаем:
    /// стабильная физика подвески и навигации ИИ.
    IndestructibleTerrain,
    /// Постройки и укрытия — разрушаются снарядами.
    DestructibleStructure,
    /// Прочие (броня мишеней, детали танков) — не принадлежат миру.
    Other,
}

/// Материал вокселя (хранится как `u8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum Material {
    #[default]
    Air = 0,
    // --- Рельеф (неразрушаемый) ---
    Soil = 1,
    Grass = 2,
    Rock = 3,
    Sand = 4,
    Snow = 5,
    Ice = 6,
    // --- Постройки и укрытия (разрушаемые) ---
    Brick = 10,
    Concrete = 11,
    Wood = 12,
    Sandbag = 13,
    // --- Мишени ---
    TargetSteel = 14,
    TargetPost = 15,
    // --- Детали танков ---
    TankOlive = 20,
    TankDark = 21,
    TankTrack = 22,
    TankSteel = 23,
    TankStar = 24,
    TankWheel = 25,
}

/// Список всех непустых материалов (для создания ассетов и тестов).
pub const ALL_MATERIALS: [Material; 18] = [
    Material::Soil,
    Material::Grass,
    Material::Rock,
    Material::Sand,
    Material::Snow,
    Material::Ice,
    Material::Brick,
    Material::Concrete,
    Material::Wood,
    Material::Sandbag,
    Material::TargetSteel,
    Material::TargetPost,
    Material::TankOlive,
    Material::TankDark,
    Material::TankTrack,
    Material::TankSteel,
    Material::TankStar,
    Material::TankWheel,
];

impl Material {
    pub fn from_u8(v: u8) -> Material {
        ALL_MATERIALS
            .iter()
            .copied()
            .find(|m| *m as u8 == v)
            .unwrap_or(Material::Air)
    }

    pub fn is_air(self) -> bool {
        self == Material::Air
    }

    pub fn destructibility(self) -> VoxelDestructibility {
        match self {
            Material::Soil
            | Material::Grass
            | Material::Rock
            | Material::Sand
            | Material::Snow
            | Material::Ice => VoxelDestructibility::IndestructibleTerrain,
            Material::Brick | Material::Concrete | Material::Wood | Material::Sandbag => {
                VoxelDestructibility::DestructibleStructure
            }
            _ => VoxelDestructibility::Other,
        }
    }

    pub fn is_destructible(self) -> bool {
        self.destructibility() == VoxelDestructibility::DestructibleStructure
    }

    /// Цвет в sRGB (0..1).
    pub fn srgb(self) -> [f32; 3] {
        match self {
            Material::Air => [0.0, 0.0, 0.0],
            Material::Soil => [0.43, 0.31, 0.20],
            Material::Grass => [0.30, 0.52, 0.22],
            Material::Rock => [0.48, 0.47, 0.45],
            Material::Sand => [0.80, 0.70, 0.47],
            Material::Snow => [0.93, 0.95, 0.98],
            Material::Ice => [0.65, 0.82, 0.92],
            Material::Brick => [0.66, 0.30, 0.22],
            Material::Concrete => [0.62, 0.62, 0.60],
            Material::Wood => [0.52, 0.36, 0.20],
            Material::Sandbag => [0.74, 0.66, 0.46],
            Material::TargetSteel => [0.55, 0.58, 0.62],
            Material::TargetPost => [0.25, 0.25, 0.27],
            Material::TankOlive => [0.31, 0.38, 0.20],
            Material::TankDark => [0.20, 0.25, 0.14],
            Material::TankTrack => [0.12, 0.12, 0.12],
            Material::TankSteel => [0.30, 0.31, 0.32],
            Material::TankStar => [0.75, 0.10, 0.08],
            Material::TankWheel => [0.17, 0.19, 0.12],
        }
    }

    /// Цвет в линейном RGBA (для вершинных цветов меша).
    pub fn linear_rgba(self) -> [f32; 4] {
        let [r, g, b] = self.srgb();
        [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b), 1.0]
    }
}

pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_u8() {
        for m in ALL_MATERIALS {
            assert_eq!(Material::from_u8(m as u8), m);
        }
        assert_eq!(Material::from_u8(0), Material::Air);
        assert_eq!(Material::from_u8(200), Material::Air);
    }

    #[test]
    fn terrain_is_indestructible_structures_are() {
        for m in [
            Material::Soil,
            Material::Grass,
            Material::Rock,
            Material::Sand,
            Material::Snow,
            Material::Ice,
        ] {
            assert!(!m.is_destructible(), "{m:?} должен быть неразрушаемым");
        }
        for m in [
            Material::Brick,
            Material::Concrete,
            Material::Wood,
            Material::Sandbag,
        ] {
            assert!(m.is_destructible(), "{m:?} должен быть разрушаемым");
        }
        assert!(!Material::Air.is_destructible());
    }
}
