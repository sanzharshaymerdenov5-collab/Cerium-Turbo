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

/// Зоны бронирования танка.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ArmorZone {
    HullFrontUpper,
    HullFrontLower,
    HullSideLeft,
    HullSideRight,
    HullRear,
    HullRoof,
    TurretFront,
    TurretSideLeft,
    TurretSideRight,
    TurretRear,
    TurretRoof,
}

impl ArmorZone {
    pub fn ru_name(self) -> &'static str {
        match self {
            Self::HullFrontUpper => "ВЛД",
            Self::HullFrontLower => "НЛД",
            Self::HullSideLeft => "левый борт корпуса",
            Self::HullSideRight => "правый борт корпуса",
            Self::HullRear => "корма корпуса",
            Self::HullRoof => "крыша корпуса",
            Self::TurretFront => "лоб башни / маски",
            Self::TurretSideLeft => "левый борт башни",
            Self::TurretSideRight => "правый борт башни",
            Self::TurretRear => "корма башни",
            Self::TurretRoof => "крыша башни",
        }
    }
}

/// Толщины брони и наклон ВЛД, мм / град.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArmorScheme {
    pub hull_front_upper_mm: f32,
    pub hull_front_lower_mm: f32,
    pub hull_side_mm: f32,
    pub hull_rear_mm: f32,
    pub hull_roof_mm: f32,
    pub turret_front_mm: f32,
    pub turret_side_mm: f32,
    pub turret_rear_mm: f32,
    pub turret_roof_mm: f32,
    pub vld_slope_deg: f32,
}

impl Default for ArmorScheme {
    fn default() -> Self {
        Self {
            hull_front_upper_mm: 45.0,
            hull_front_lower_mm: 45.0,
            hull_side_mm: 45.0,
            hull_rear_mm: 40.0,
            hull_roof_mm: 20.0,
            turret_front_mm: 90.0,
            turret_side_mm: 75.0,
            turret_rear_mm: 52.0,
            turret_roof_mm: 20.0,
            vld_slope_deg: 60.0,
        }
    }
}

/// Форма корпуса процедурного воксель-билдера.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HullShape {
    #[default]
    SlopedT34,
    LightBox,
    HeavyBox,
    Casemate,
    PikeNose,
}

/// Форма башни (или рубки) процедурного воксель-билдера.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TurretShape {
    #[default]
    HexCast,
    CompactLight,
    BoxHeavy,
    FixedCasemate,
    RoundedDome,
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
    #[serde(default)]
    pub hull_shape: HullShape,
    #[serde(default)]
    pub turret_shape: TurretShape,
}

fn default_max_hp() -> f32 {
    850.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TankSpec {
    pub id: String,
    pub name: String,
    pub nation: String,
    #[serde(default = "default_max_hp")]
    pub max_hp: f32,
    pub mass_kg: f32,
    pub max_speed_kmh: f32,
    pub reverse_speed_kmh: f32,
    pub max_yaw_rate_dps: f32,
    pub engine_power_kw: f32,
    pub max_tractive_force_kn: f32,
    pub turret_traverse_dps: f32,
    #[serde(default)]
    pub yaw_limits_deg: Option<(f32, f32)>,
    pub gun_depression_deg: f32,
    pub gun_elevation_deg: f32,
    pub reload_s: f32,
    pub recoil_impulse_ns: f32,
    pub shell: ShellSpec,
    #[serde(default)]
    pub armor: ArmorScheme,
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
