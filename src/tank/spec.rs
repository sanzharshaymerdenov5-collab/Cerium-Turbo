//! Описание танка (загружается из `.ron`).

use serde::{Deserialize, Serialize};

use crate::ballistics::ShellSpec;

/// Откуда берётся воксельная модель.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VoxelVisualSource {
    /// Процедурная генерация по [`ProceduralTankProfile`].
    Procedural,
    /// `.vox`-файл (MagicaVoxel) через `dot_vox` — для полировки визуала.
    VoxFile(String),
}

/// Габариты процедурной модели (метры; кадр танка: вперёд −Z, вверх +Y, вправо +X,
/// начало координат — центр по низу гусениц).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProceduralTankProfile {
    pub voxel_size: f32,
    pub gun_voxel_size: f32,
    pub hull_length: f32,
    pub hull_width: f32,
    pub track_width: f32,
    pub track_height: f32,
    pub deck_height: f32,
    /// Положение погона башни в кадре танка.
    pub turret_pivot: [f32; 3],
    pub turret_radius_x: f32,
    pub turret_radius_z: f32,
    pub turret_height: f32,
    /// Положение цапф орудия в кадре башни.
    pub gun_pivot: [f32; 3],
    pub barrel_length: f32,
    pub barrel_radius: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TankSpec {
    pub id: String,
    pub name: String,
    pub nation: String,
    pub mass_kg: f32,
    pub max_speed_kmh: f32,
    pub reverse_speed_kmh: f32,
    pub max_yaw_rate_dps: f32,
    pub engine_power_kw: f32,
    pub max_tractive_force_kn: f32,
    pub turret_traverse_dps: f32,
    pub gun_depression_deg: f32,
    pub gun_elevation_deg: f32,
    pub reload_s: f32,
    pub recoil_impulse_ns: f32,
    pub shell: ShellSpec,
    pub visual: VoxelVisualSource,
    pub profile: ProceduralTankProfile,
}

impl TankSpec {
    pub fn from_ron(s: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(s)
    }
}

/// Встроенные в бинарник описания танков (wasm-совместимо, без async-загрузки).
pub const T34_85_RON: &str = include_str!("../../assets/tanks/t34_85.ron");

/// Находит описание по идентификатору.
pub fn load_tank_spec(id: &str) -> Option<TankSpec> {
    match id {
        "t34_85" => TankSpec::from_ron(T34_85_RON).ok(),
        _ => None,
    }
}

/// Список доступных танков (для ангара).
pub fn available_tanks() -> Vec<TankSpec> {
    ["t34_85"].iter().filter_map(|id| load_tank_spec(id)).collect()
}
