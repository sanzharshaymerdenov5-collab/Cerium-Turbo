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
    pub hills: Vec<HillSpec>,
    pub ditches: Vec<DitchSpec>,
    pub structures: Vec<StructureSpec>,
    pub targets: Vec<TargetSpec>,
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

pub const PROVING_GROUNDS_RON: &str = include_str!("../../assets/maps/proving_grounds.ron");

pub fn load_map_spec(id: &str) -> Option<MapSpec> {
    match id {
        "proving_grounds" => MapSpec::from_ron(PROVING_GROUNDS_RON).ok(),
        _ => None,
    }
}

pub fn available_maps() -> Vec<MapSpec> {
    ["proving_grounds"]
        .iter()
        .filter_map(|id| load_map_spec(id))
        .collect()
}
