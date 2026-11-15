//! Описание карты (из `.ron`).

use serde::{Deserialize, Serialize};

use crate::voxel::Material;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StructureMaterial {
    Brick,
    Concrete,
    Wood,
    Sandbag,
}

impl StructureMaterial {
    pub fn to_material(self) -> Material {
        match self {
            Self::Brick => Material::Brick,
            Self::Concrete => Material::Concrete,
            Self::Wood => Material::Wood,
            Self::Sandbag => Material::Sandbag,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlatZone {
    pub min_xz: (f32, f32),
    pub max_xz: (f32, f32),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HillSpec {
    pub center_xz: (f32, f32),
    pub radius_m: f32,
    pub height_m: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DitchSpec {
    pub min_xz: (f32, f32),
    pub max_xz: (f32, f32),
    pub depth_m: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StructureSpec {
    Wall {
        min_xz: (f32, f32),
        max_xz: (f32, f32),
        height_m: f32,
        kind: StructureMaterial,
    },
    House {
        center_xz: (f32, f32),
        size_xz: (f32, f32),
        height_m: f32,
        kind: StructureMaterial,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetSpec {
    pub name: String,
    pub pos_xz: (f32, f32),
    pub thickness_mm: f32,
    /// Наклон плиты от вертикали назад (0° = вертикальный лист, 60° = наклон как у ВЛД Т-34).
    pub slope_deg: f32,
    pub size_xy: (f32, f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MapBiome {
    #[default]
    Temperate,
    City,
    Desert,
    Winter,
    Industrial,
    Highland,
}

impl MapBiome {
    pub fn ru_name(self) -> &'static str {
        match self {
            Self::Temperate => "Поля и холмы (умеренный)",
            Self::City => "Городские руины",
            Self::Desert => "Пустыня / каньон",
            Self::Winter => "Зимний фронт (гололёд)",
            Self::Industrial => "Промзона / ЖД",
            Self::Highland => "Горы / перевал (высокогорье)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnemySpawnSpec {
    pub pos_xz: (f32, f32),
    #[serde(default)]
    pub yaw_deg: f32,
    #[serde(default)]
    pub tank_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapSpec {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub biome: MapBiome,
    pub voxel_size: f32,
    pub dims: (u32, u32, u32),
    pub base_height_m: f32,
    pub spawn_xz: (f32, f32),
    pub spawn_yaw_deg: f32,
    pub flat_zones: Vec<FlatZone>,
    #[serde(default)]
    pub ice_zones: Vec<FlatZone>,
    pub hills: Vec<HillSpec>,
    pub ditches: Vec<DitchSpec>,
    pub structures: Vec<StructureSpec>,
    pub targets: Vec<TargetSpec>,
    #[serde(default)]
    pub ally_spawns: Vec<EnemySpawnSpec>,
    #[serde(default)]
    pub enemy_spawns: Vec<EnemySpawnSpec>,
    #[serde(default)]
    pub waypoints: Vec<(f32, f32)>,
}

impl MapSpec {
    pub fn from_ron(s: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(s)
    }
}

pub const PROVING_GROUNDS_ID: &str = "proving_grounds";

pub const PROVING_GROUNDS_RON: &str = include_str!("../../assets/maps/proving_grounds.ron");
pub const CITY_RUINS_RON: &str = include_str!("../../assets/maps/city_ruins.ron");
pub const DESERT_CANYON_RON: &str = include_str!("../../assets/maps/desert_canyon.ron");
pub const FROZEN_FORT_RON: &str = include_str!("../../assets/maps/frozen_fort.ron");
pub const RAIL_YARD_RON: &str = include_str!("../../assets/maps/rail_yard.ron");
pub const PROKHOROVKA_RIDGE_RON: &str = include_str!("../../assets/maps/prokhorovka_ridge.ron");
pub const ALPINE_PASS_RON: &str = include_str!("../../assets/maps/alpine_pass.ron");
pub const BERLIN_PLAZA_RON: &str = include_str!("../../assets/maps/berlin_plaza.ron");
pub const OASIS_CITADEL_RON: &str = include_str!("../../assets/maps/oasis_citadel.ron");
pub const NORDIC_FJORD_RON: &str = include_str!("../../assets/maps/nordic_fjord.ron");

pub const RANDOM_MAP_ID: &str = "random";

pub fn load_map_spec(id: &str) -> Option<MapSpec> {
    match id {
        "proving_grounds" => MapSpec::from_ron(PROVING_GROUNDS_RON).ok(),
        "city_ruins" => MapSpec::from_ron(CITY_RUINS_RON).ok(),
        "desert_canyon" => MapSpec::from_ron(DESERT_CANYON_RON).ok(),
        "frozen_fort" => MapSpec::from_ron(FROZEN_FORT_RON).ok(),
        "rail_yard" => MapSpec::from_ron(RAIL_YARD_RON).ok(),
        "prokhorovka_ridge" => MapSpec::from_ron(PROKHOROVKA_RIDGE_RON).ok(),
        "alpine_pass" => MapSpec::from_ron(ALPINE_PASS_RON).ok(),
        "berlin_plaza" => MapSpec::from_ron(BERLIN_PLAZA_RON).ok(),
        "oasis_citadel" => MapSpec::from_ron(OASIS_CITADEL_RON).ok(),
        "nordic_fjord" => MapSpec::from_ron(NORDIC_FJORD_RON).ok(),
        _ => None,
    }
}

pub fn available_maps() -> Vec<MapSpec> {
    [
        "proving_grounds",
        "city_ruins",
        "desert_canyon",
        "frozen_fort",
        "rail_yard",
        "prokhorovka_ridge",
        "alpine_pass",
        "berlin_plaza",
        "oasis_citadel",
        "nordic_fjord",
    ]
    .iter()
    .filter_map(|id| load_map_spec(id))
    .collect()
}

/// Боевые карты, участвующие в случайной выборке (`"Полигон"` исключён).
pub fn random_map_pool() -> Vec<MapSpec> {
    available_maps()
        .into_iter()
        .filter(|m| m.id != PROVING_GROUNDS_ID)
        .collect()
}

/// Разрешает выбранный идентификатор карты (включая `RANDOM_MAP_ID = "random"`) в конкретную карту боя.
pub fn resolve_map_spec(selected_id: &str, seed: u64) -> MapSpec {
    if selected_id == RANDOM_MAP_ID {
        let pool = random_map_pool();
        if !pool.is_empty() {
            let mut rng = crate::voxel::Rng::new(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let idx = ((rng.next_u64() >> 32) as usize) % pool.len();
            return pool[idx].clone();
        }
    }
    load_map_spec(selected_id)
        .or_else(|| available_maps().into_iter().next())
        .expect("at least one built-in map must exist")
}
