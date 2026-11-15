//! Описание танка (загружается из `.ron`).

use serde::{Deserialize, Serialize};

use crate::ballistics::ShellSpec;

/// Нация техники.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Nation {
    #[default]
    Ussr,
    Germany,
    Usa,
}

impl Nation {
    pub const fn all() -> [Nation; 3] {
        [Self::Ussr, Self::Germany, Self::Usa]
    }

    pub fn ru_name(self) -> &'static str {
        match self {
            Self::Ussr => "СССР",
            Self::Germany => "Германия",
            Self::Usa => "США",
        }
    }

    pub fn ru_short(self) -> &'static str {
        match self {
            Self::Ussr => "СССР",
            Self::Germany => "ГЕР",
            Self::Usa => "США",
        }
    }

    pub fn accent(self) -> bevy::prelude::Color {
        match self {
            Self::Ussr => bevy::prelude::Color::srgb(0.76, 0.28, 0.24),
            Self::Germany => bevy::prelude::Color::srgb(0.68, 0.60, 0.36),
            Self::Usa => bevy::prelude::Color::srgb(0.42, 0.66, 0.34),
        }
    }
}

/// Класс боевой машины.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TankClass {
    Light,
    #[default]
    Medium,
    Heavy,
    TankDestroyer,
}

impl TankClass {
    pub fn ru_short(self) -> &'static str {
        match self {
            Self::Light => "ЛТ",
            Self::Medium => "СТ",
            Self::Heavy => "ТТ",
            Self::TankDestroyer => "ПТ",
        }
    }

    pub fn ru_name(self) -> &'static str {
        match self {
            Self::Light => "Лёгкий танк",
            Self::Medium => "Средний танк",
            Self::Heavy => "Тяжёлый танк",
            Self::TankDestroyer => "ПТ-САУ",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Light => "◆",
            Self::Medium => "◈",
            Self::Heavy => "■",
            Self::TankDestroyer => "▼",
        }
    }
}

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

/// Точный исторический архетип визуальной модели танка.
/// При значении `Auto` билдер использует пары `HullShape` + `TurretShape` (обратная совместимость).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TankVisualModel {
    #[default]
    Auto,
    Ms1,
    Bt7,
    T50,
    T34_76,
    Kv1,
    T34_85,
    Is1,
    T44,
    Is2,
    Su100,
    Is3,
    T54_46,
    Pz2C,
    Pz3,
    Pz3E,
    Pz4A,
    Pz4,
    Panther,
    Tiger1,
    Tiger2,
    M48A1,
    M2A4,
    M3Stuart,
    M5A1Stuart,
    M8A1,
    M24Chaffee,
    M4A1_75,
    M4A3E8,
    Jumbo76,
    M26Pershing,
    T29,
    T32,
    T34Usa,
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
    #[serde(default)]
    pub visual_model: TankVisualModel,
}

impl ProceduralTankProfile {
    /// Определяет, оснащён ли ствол дульным тормозом.
    pub fn has_muzzle_brake(&self) -> bool {
        match self.visual_model {
            TankVisualModel::Tiger1
            | TankVisualModel::Tiger2
            | TankVisualModel::Panther
            | TankVisualModel::Pz4
            | TankVisualModel::M48A1
            | TankVisualModel::Is2
            | TankVisualModel::Is3
            | TankVisualModel::M26Pershing
            | TankVisualModel::T29
            | TankVisualModel::T32
            | TankVisualModel::T34Usa
            | TankVisualModel::M4A3E8 => true,
            TankVisualModel::Auto => matches!(
                self.turret_shape,
                TurretShape::BoxHeavy | TurretShape::RoundedDome
            ),
            _ => false,
        }
    }
}

fn default_max_hp() -> f32 {
    850.0
}

fn default_tier() -> u8 {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TankSpec {
    pub id: String,
    pub name: String,
    pub nation: String,
    #[serde(default)]
    pub nation_id: Nation,
    #[serde(default = "default_tier")]
    pub tier: u8,
    #[serde(default)]
    pub tank_class: TankClass,
    #[serde(default)]
    pub xp_cost: u32,
    #[serde(default)]
    pub credit_cost: u32,
    #[serde(default)]
    pub prerequisite: Option<String>,
    #[serde(default)]
    pub alt_prerequisite: Option<String>,
    #[serde(default)]
    pub gun_short: String,
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
pub const MS_1_RON: &str = include_str!("../../assets/tanks/ms_1.ron");
pub const BT_7_RON: &str = include_str!("../../assets/tanks/bt_7.ron");
pub const T_50_RON: &str = include_str!("../../assets/tanks/t_50.ron");
pub const T34_76_RON: &str = include_str!("../../assets/tanks/t34_76.ron");
pub const KV_1_RON: &str = include_str!("../../assets/tanks/kv_1.ron");
pub const T34_85_RON: &str = include_str!("../../assets/tanks/t34_85.ron");
pub const IS_1_RON: &str = include_str!("../../assets/tanks/is_1.ron");
pub const T_44_RON: &str = include_str!("../../assets/tanks/t_44.ron");
pub const IS_2_RON: &str = include_str!("../../assets/tanks/is_2.ron");
pub const T_54_46_RON: &str = include_str!("../../assets/tanks/t_54_46.ron");
pub const SU_100_RON: &str = include_str!("../../assets/tanks/su_100.ron");
pub const IS_3_RON: &str = include_str!("../../assets/tanks/is_3.ron");

pub const PZ_2_C_RON: &str = include_str!("../../assets/tanks/pz_2_c.ron");
pub const PZ_3_RON: &str = include_str!("../../assets/tanks/pz_3.ron");
pub const PZ_3_E_RON: &str = include_str!("../../assets/tanks/pz_3_e.ron");
pub const PZ_4_A_RON: &str = include_str!("../../assets/tanks/pz_4_a.ron");
pub const PZ_4_RON: &str = include_str!("../../assets/tanks/pz_4.ron");
pub const PANTHER_RON: &str = include_str!("../../assets/tanks/panther.ron");
pub const TIGER_1_RON: &str = include_str!("../../assets/tanks/tiger_1.ron");
pub const TIGER_2_RON: &str = include_str!("../../assets/tanks/tiger_2.ron");
pub const M48A1_RON: &str = include_str!("../../assets/tanks/m48a1.ron");

pub const M2A4_RON: &str = include_str!("../../assets/tanks/m2a4.ron");
pub const M3_STUART_RON: &str = include_str!("../../assets/tanks/m3_stuart.ron");
pub const M5A1_STUART_RON: &str = include_str!("../../assets/tanks/m5a1_stuart.ron");
pub const M8A1_RON: &str = include_str!("../../assets/tanks/m8a1.ron");
pub const M24_CHAFFEE_RON: &str = include_str!("../../assets/tanks/m24_chaffee.ron");
pub const M4A1_75_RON: &str = include_str!("../../assets/tanks/m4a1_75.ron");
pub const M4A3E8_RON: &str = include_str!("../../assets/tanks/m4a3e8.ron");
pub const JUMBO_76_RON: &str = include_str!("../../assets/tanks/jumbo_76.ron");
pub const M26_PERSHING_RON: &str = include_str!("../../assets/tanks/m26_pershing.ron");
pub const T29_RON: &str = include_str!("../../assets/tanks/t29.ron");
pub const T32_RON: &str = include_str!("../../assets/tanks/t32.ron");
pub const T34_USA_RON: &str = include_str!("../../assets/tanks/t34_usa.ron");

/// Находит описание по идентификатору.
pub fn load_tank_spec(id: &str) -> Option<TankSpec> {
    match id {
        "ms_1" => TankSpec::from_ron(MS_1_RON).ok(),
        "bt_7" => TankSpec::from_ron(BT_7_RON).ok(),
        "t_50" => TankSpec::from_ron(T_50_RON).ok(),
        "t34_76" => TankSpec::from_ron(T34_76_RON).ok(),
        "kv_1" => TankSpec::from_ron(KV_1_RON).ok(),
        "t34_85" => TankSpec::from_ron(T34_85_RON).ok(),
        "is_1" => TankSpec::from_ron(IS_1_RON).ok(),
        "t_44" => TankSpec::from_ron(T_44_RON).ok(),
        "is_2" => TankSpec::from_ron(IS_2_RON).ok(),
        "t_54_46" => TankSpec::from_ron(T_54_46_RON).ok(),
        "is_3" => TankSpec::from_ron(IS_3_RON).ok(),
        "su_100" => TankSpec::from_ron(SU_100_RON).ok(),
        "pz_2_c" | "pz_1" => TankSpec::from_ron(PZ_2_C_RON).ok(),
        "pz_3_e" => TankSpec::from_ron(PZ_3_E_RON).ok(),
        "pz_3" => TankSpec::from_ron(PZ_3_RON).ok(),
        "pz_4_a" => TankSpec::from_ron(PZ_4_A_RON).ok(),
        "pz_4" => TankSpec::from_ron(PZ_4_RON).ok(),
        "panther" => TankSpec::from_ron(PANTHER_RON).ok(),
        "tiger_1" => TankSpec::from_ron(TIGER_1_RON).ok(),
        "tiger_2" => TankSpec::from_ron(TIGER_2_RON).ok(),
        "m48a1" => TankSpec::from_ron(M48A1_RON).ok(),
        "m2a4" => TankSpec::from_ron(M2A4_RON).ok(),
        "m3_stuart" => TankSpec::from_ron(M3_STUART_RON).ok(),
        "m5a1_stuart" => TankSpec::from_ron(M5A1_STUART_RON).ok(),
        "m8a1" => TankSpec::from_ron(M8A1_RON).ok(),
        "m24_chaffee" => TankSpec::from_ron(M24_CHAFFEE_RON).ok(),
        "m4a1_75" => TankSpec::from_ron(M4A1_75_RON).ok(),
        "m4a3e8" => TankSpec::from_ron(M4A3E8_RON).ok(),
        "jumbo_76" => TankSpec::from_ron(JUMBO_76_RON).ok(),
        "m26_pershing" => TankSpec::from_ron(M26_PERSHING_RON).ok(),
        "t29" => TankSpec::from_ron(T29_RON).ok(),
        "t32" => TankSpec::from_ron(T32_RON).ok(),
        "t34_usa" => TankSpec::from_ron(T34_USA_RON).ok(),
        _ => None,
    }
}

/// Список доступных танков (для ангара, в порядке нация + уровень I–VII).
pub fn available_tanks() -> Vec<TankSpec> {
    [
        // СССР (11 машин, уровни I–VII)
        "ms_1",
        "bt_7",
        "t_50",
        "t34_76",
        "kv_1",
        "t34_85",
        "is_1",
        "t_44",
        "is_2",
        "t_54_46",
        "is_3",
        // Германия (8 машин, уровни I–VI)
        "pz_2_c",
        "pz_3_e",
        "pz_4_a",
        "pz_4",
        "panther",
        "tiger_1",
        "tiger_2",
        "m48a1",
        // США (11 машин, уровни I–VII)
        "m2a4",
        "m3_stuart",
        "m5a1_stuart",
        "m8a1",
        "m4a1_75",
        "m4a3e8",
        "jumbo_76",
        "m26_pershing",
        "t29",
        "t32",
        "t34_usa",
    ]
    .iter()
    .filter_map(|id| load_tank_spec(id))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ballistics::ShellType;

    #[test]
    fn all_thirty_historical_tanks_parse_and_follow_ap_only() {
        let tanks = available_tanks();
        assert_eq!(tanks.len(), 30);
        for t in &tanks {
            assert_eq!(t.shell.shell_type, ShellType::SolidAP, "{}", t.id);
            assert!(t.max_hp >= 350.0, "{}: hp={}", t.id, t.max_hp);
            assert!((t.profile.gun_voxel_size - 0.04).abs() < 1e-6, "{}", t.id);
            assert!((1..=7).contains(&t.tier), "{}: tier={}", t.id, t.tier);
            assert!(!t.gun_short.is_empty(), "{}", t.id);
            if t.tier == 1 {
                assert_eq!(t.xp_cost, 0, "{}", t.id);
                assert_eq!(t.credit_cost, 0, "{}", t.id);
                assert_eq!(t.prerequisite, None, "{}", t.id);
            } else {
                assert!(t.prerequisite.is_some(), "{}", t.id);
                assert!(t.xp_cost > 0 && t.credit_cost > 0, "{}", t.id);
            }
        }

        // Проверяем ветки СССР (I–VII): 11 машин.
        let ussr_count = tanks.iter().filter(|t| t.nation_id == Nation::Ussr).count();
        let ger_count = tanks.iter().filter(|t| t.nation_id == Nation::Germany).count();
        let usa_count = tanks.iter().filter(|t| t.nation_id == Nation::Usa).count();
        assert_eq!((ussr_count, ger_count, usa_count), (11, 8, 11));

        let t50 = load_tank_spec("t_50").unwrap();
        assert_eq!(t50.tier, 3);
        assert_eq!(t50.prerequisite.as_deref(), Some("bt_7"));

        let t34_76 = load_tank_spec("t34_76").unwrap();
        let kv1 = load_tank_spec("kv_1").unwrap();
        assert_eq!((t34_76.tier, kv1.tier), (4, 4));
        assert_eq!(t34_76.prerequisite.as_deref(), Some("t_50"));
        assert_eq!(kv1.prerequisite.as_deref(), Some("t_50"));

        let t54 = load_tank_spec("t_54_46").unwrap();
        let is3 = load_tank_spec("is_3").unwrap();
        assert_eq!((t54.tier, is3.tier), (7, 7));
        assert_eq!(t54.prerequisite.as_deref(), Some("t_44"));
        assert_eq!(is3.prerequisite.as_deref(), Some("is_2"));
        assert_eq!(is3.profile.hull_shape, HullShape::PikeNose);
        assert!(is3.armor.turret_front_mm >= 220.0);

        // Проверяем ветки Германии (I–VI): 8 машин.
        let pz2 = load_tank_spec("pz_2_c").unwrap();
        assert_eq!(pz2.tier, 1);
        assert_eq!(pz2.name, "Pz II ausf C");
        let pz4 = load_tank_spec("pz_4").unwrap();
        assert_eq!(pz4.tier, 3);
        assert_eq!(pz4.prerequisite.as_deref(), Some("pz_3_e"));
        assert_eq!(pz4.alt_prerequisite.as_deref(), Some("pz_4_a"));

        let tiger1 = load_tank_spec("tiger_1").unwrap();
        let tiger2 = load_tank_spec("tiger_2").unwrap();
        let m48 = load_tank_spec("m48a1").unwrap();
        assert_eq!(tiger1.name, "Pz VI H");
        assert_eq!(tiger1.tier, 4);
        assert_eq!(tiger1.prerequisite.as_deref(), Some("pz_4"));
        assert_eq!(tiger2.tier, 5);
        assert_eq!(tiger2.prerequisite.as_deref(), Some("tiger_1"));
        assert_eq!(tiger2.alt_prerequisite.as_deref(), Some("panther"));
        assert_eq!(m48.tier, 6);
        assert_eq!(m48.prerequisite.as_deref(), Some("tiger_2"));

        // Проверяем ветки США (I–VII): 11 машин.
        let m8a1 = load_tank_spec("m8a1").unwrap();
        assert_eq!(m8a1.tier, 3);
        assert_eq!(m8a1.tank_class, TankClass::TankDestroyer);
        assert_eq!(m8a1.yaw_limits_deg, None);
        let m4a1 = load_tank_spec("m4a1_75").unwrap();
        assert_eq!(m4a1.tier, 4);
        assert_eq!(m4a1.prerequisite.as_deref(), Some("m5a1_stuart"));
        assert_eq!(m4a1.alt_prerequisite.as_deref(), Some("m8a1"));
        let t32 = load_tank_spec("t32").unwrap();
        let t34 = load_tank_spec("t34_usa").unwrap();
        assert_eq!((t32.tier, t34.tier), (7, 7));
        assert_eq!(t32.prerequisite.as_deref(), Some("m26_pershing"));
        assert_eq!(t34.prerequisite.as_deref(), Some("t29"));

        let bt7 = load_tank_spec("bt_7").unwrap();
        assert!(bt7.max_speed_kmh >= 52.0);
    }

    #[test]
    fn nation_and_class_helpers_return_russian_labels() {
        assert_eq!(Nation::all(), [Nation::Ussr, Nation::Germany, Nation::Usa]);
        assert_eq!(Nation::Ussr.ru_name(), "СССР");
        assert_eq!(Nation::Germany.ru_short(), "ГЕР");
        assert_eq!(Nation::Usa.ru_name(), "США");
        assert_ne!(Nation::Ussr.accent(), Nation::Germany.accent());
        assert_ne!(Nation::Germany.accent(), Nation::Usa.accent());

        assert_eq!(TankClass::Light.ru_short(), "ЛТ");
        assert_eq!(TankClass::Medium.ru_short(), "СТ");
        assert_eq!(TankClass::Heavy.ru_short(), "ТТ");
        assert_eq!(TankClass::TankDestroyer.ru_short(), "ПТ");
        assert_eq!(TankClass::TankDestroyer.icon(), "▼");
    }

    #[test]
    fn legacy_ron_without_new_fields_deserializes_with_defaults() {
        let legacy = r#"(
            id: "legacy",
            name: "Legacy",
            nation: "СССР",
            mass_kg: 20000.0,
            max_speed_kmh: 40.0,
            reverse_speed_kmh: 12.0,
            max_yaw_rate_dps: 30.0,
            engine_power_kw: 250.0,
            max_tractive_force_kn: 150.0,
            turret_traverse_dps: 24.0,
            gun_depression_deg: 6.0,
            gun_elevation_deg: 20.0,
            reload_s: 5.0,
            recoil_impulse_ns: 30000.0,
            shell: (
                shell_type: SolidAP,
                caliber_mm: 76.0,
                muzzle_velocity: 700.0,
                pen_at_100m_mm: 100.0,
                pen_drop_per_100m_mm: 4.0,
                normalization_deg: 5.0,
                ricochet_deg: 70.0,
                arcade_damage: 120.0,
            ),
            visual: Procedural,
            profile: (
                voxel_size: 0.08,
                gun_voxel_size: 0.04,
                hull_length: 5.0,
                hull_width: 2.6,
                track_width: 0.4,
                track_height: 0.8,
                deck_height: 1.1,
                turret_pivot: (0.0, 1.1, -0.2),
                turret_radius_x: 0.8,
                turret_radius_z: 1.0,
                turret_height: 0.65,
                gun_pivot: (0.0, 0.35, -0.85),
                barrel_length: 3.0,
                barrel_radius: 0.08,
            ),
        )"#;
        let spec = TankSpec::from_ron(legacy).expect("legacy ron should parse");
        assert_eq!(spec.nation_id, Nation::Ussr);
        assert_eq!(spec.tier, 1);
        assert_eq!(spec.tank_class, TankClass::Medium);
        assert_eq!(spec.xp_cost, 0);
        assert_eq!(spec.credit_cost, 0);
        assert_eq!(spec.prerequisite, None);
        assert_eq!(spec.gun_short, "");
        assert_eq!(spec.max_hp, 850.0);
    }
}
