//! Ангар в стиле World of Tanks (Обновление 1.4.1):
//! - Промышленный 3D-цех со смотровой ямой, рельсами, кран-балкой, трубами, прожекторами и дымкой;
//! - Экономика (серебро/опыт, награды за бой), дерево исследований 3 наций (СССР, Германия, США)
//!   × V тиров с альтернативными ветками (18 исторических машин AP-Only);
//! - Фильтры карусели по нациям и технике в ангаре, предпросмотр любой машины в 3D и X-Ray `[V]`,
//!   переключение режимов `[M]`, выбор случайной карты `+10%` и 5 локаций 5v5.

#![allow(deprecated)]

use std::f32::consts::PI;

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::math::{IVec3, UVec3};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;

use crate::audio::{AudioSettings, SoundPreset, UiSoundMessage};
use crate::ballistics::penetration::{effective_armor_mm, penetration_at};
use crate::core::{DamageRulesMode, GameState, SelectedMap, SelectedTank, TankId};
use crate::damage::{CrewRole, ModuleKind, TankCrew, TankModules};
use crate::maps::{MapBiome, MapSpec, RANDOM_MAP_ID, available_maps, load_map_spec};
use crate::progression::{
    LastBattleReward, PlayerProgression, buy_tank, ensure_starters, has_prerequisite_researched,
    is_owned, is_researched, research_tank,
};
use crate::tank::armor::{hull_plate_specs, turret_plate_specs};
use crate::tank::{
    HangarPreviewTank, Nation, TankSpec, available_tanks, load_tank_spec, spawn_preview_tank,
};
use crate::voxel::{Material, VoxelAssets, VoxelGrid, voxel_model_bundle};

/// Фильтр карусели техники в нижней части ангара.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HangarFilter {
    #[default]
    AllOwned,
    Nation(Nation),
}

/// Настройки и текущее состояние камеры, предпросмотра, фильтров и дерева исследований в ангаре.
#[derive(Resource, Debug)]
pub struct HangarViewState {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub auto_spin: bool,
    pub xray_armor: bool,
    pub preview_tank_id: String,
    pub spawned_preview_id: String,
    pub filter_tab: HangarFilter,
    pub show_tech_tree: bool,
    pub notice: Option<(String, Color)>,
    pub prev_xray_btn: bool,
    pub prev_mode_btn: bool,
    pub prev_spin_btn: bool,
    pub prev_audio_btn: bool,
    pub prev_pressed_btn: Option<HangarButton>,
    pub suppress_click_frames: u8,
}

impl Default for HangarViewState {
    fn default() -> Self {
        Self {
            // Ракурс 3/4 спереди-слева: пушка направлена вперёд и влево, справа вид на смотровую яму.
            yaw: PI + 0.52,
            pitch: 0.19,
            distance: 8.6,
            auto_spin: false,
            xray_armor: false,
            preview_tank_id: String::new(),
            spawned_preview_id: String::new(),
            filter_tab: HangarFilter::AllOwned,
            show_tech_tree: false,
            notice: None,
            prev_xray_btn: false,
            prev_mode_btn: false,
            prev_spin_btn: false,
            prev_audio_btn: false,
            prev_pressed_btn: None,
            suppress_click_frames: 0,
        }
    }
}

#[derive(Component)]
pub struct HangarTurntable;

#[derive(Component)]
pub struct HangarXrayRoot;

#[derive(Component)]
pub struct HangarCamera;

#[derive(Component)]
pub struct HangarUiRoot;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum HangarButton {
    Noop,
    SelectFilter(HangarFilter),
    SelectTank(usize),
    InspectTank(usize),
    SelectOwnedAndClose(usize),
    ResearchTank(usize),
    BuyTank(usize),
    ToggleTechTree,
    SelectRandomMap,
    SelectMap(usize),
    ToggleMode,
    ToggleXray,
    ToggleSpin,
    ToggleAudio,
    StartBattle,
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum HangarLabel {
    HeroTitle,
    HeroSubtitle,
    HeaderStatus,
    MapBriefTitle,
    MapBriefBody,
    Info,
    ShellSlot,
    ToggleXray,
    ToggleMode,
    ToggleSpin,
    ToggleAudio,
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum HangarBar {
    Firepower,
    Survivability,
    Armor,
    Mobility,
}

/// Нормализованные рейтинги ТТХ (`0..1`) для шкал в ангаре WoT.
#[derive(Debug, Clone, Copy)]
pub struct TankRatings {
    pub firepower: f32,
    pub survivability: f32,
    pub armor: f32,
    pub mobility: f32,
}

/// Классификация и метаданные техники в стиле WoT (берутся из `TankSpec`).
#[derive(Debug, Clone)]
pub struct TankWotMeta {
    pub tier: &'static str,
    pub class_icon: &'static str,
    pub class_short: &'static str,
    pub nation_short: &'static str,
    pub gun_short: String,
    pub accent: Color,
}

pub fn tier_roman(tier: u8) -> &'static str {
    match tier {
        1 => "I",
        2 => "II",
        3 => "III",
        4 => "IV",
        5 => "V",
        _ => "I",
    }
}

pub fn tank_wot_meta(spec: &TankSpec) -> TankWotMeta {
    let gun_short = if spec.gun_short.is_empty() {
        format!("{:.0}мм SolidAP", spec.shell.caliber_mm)
    } else {
        spec.gun_short.clone()
    };
    TankWotMeta {
        tier: tier_roman(spec.tier),
        class_icon: spec.tank_class.icon(),
        class_short: spec.tank_class.ru_short(),
        nation_short: spec.nation_id.ru_short(),
        gun_short,
        accent: spec.nation_id.accent(),
    }
}

pub fn compute_tank_ratings(spec: &TankSpec) -> TankRatings {
    let pen = penetration_at(&spec.shell, 100.0);
    let dpm_proxy = (spec.shell.arcade_damage / spec.reload_s.max(1.0)) * 2.1;
    let firepower = ((pen * 0.56 + dpm_proxy * 0.62) / 210.0).clamp(0.14, 0.98);
    let survivability = (spec.max_hp / 1620.0).clamp(0.16, 0.98);

    let eff_vld = effective_armor_mm(
        &spec.shell,
        spec.armor.hull_front_upper_mm,
        spec.armor.vld_slope_deg,
    );
    let armor = ((eff_vld * 0.55 + spec.armor.turret_front_mm * 0.65) / 260.0).clamp(0.12, 0.98);

    let hp_per_t = spec.engine_power_kw / (spec.mass_kg / 1000.0).max(1.0);
    let mobility =
        ((spec.max_speed_kmh / 60.0) * 0.65 + (hp_per_t / 20.0) * 0.35).clamp(0.18, 0.98);

    TankRatings {
        firepower,
        survivability,
        armor,
        mobility,
    }
}

fn biome_short_ru(biome: MapBiome) -> &'static str {
    match biome {
        MapBiome::Temperate => "Полигон",
        MapBiome::City => "Город",
        MapBiome::Desert => "Пустыня",
        MapBiome::Winter => "Зима / лёд",
        MapBiome::Industrial => "Промзона",
    }
}

fn biome_accent(biome: MapBiome) -> Color {
    match biome {
        MapBiome::Temperate => Color::srgb(0.38, 0.78, 0.42),
        MapBiome::City => Color::srgb(0.84, 0.52, 0.28),
        MapBiome::Desert => Color::srgb(0.92, 0.72, 0.30),
        MapBiome::Winter => Color::srgb(0.45, 0.80, 0.98),
        MapBiome::Industrial => Color::srgb(0.68, 0.74, 0.84),
    }
}

fn resolve_tank_name(id: Option<&str>, tanks: &[TankSpec]) -> String {
    match id {
        Some(tid) => tanks
            .iter()
            .find(|t| t.id == tid)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| tid.to_string()),
        None => "Танк ИИ".to_string(),
    }
}

fn teams_lineup_ru(map_spec: &MapSpec, tanks: &[TankSpec]) -> String {
    let allies: Vec<String> = map_spec
        .ally_spawns
        .iter()
        .map(|s| resolve_tank_name(s.tank_id.as_deref(), tanks))
        .collect();
    let enemies: Vec<String> = map_spec
        .enemy_spawns
        .iter()
        .map(|s| resolve_tank_name(s.tank_id.as_deref(), tanks))
        .collect();
    let ally_str = if allies.is_empty() {
        "Союз (Вы): одиночный выезд".to_string()
    } else {
        format!("Союзники ({}): {}", allies.len(), allies.join(", "))
    };
    let enemy_str = if enemies.is_empty() {
        "Противник: мишени полигона".to_string()
    } else {
        format!("Противник ({}): {}", enemies.len(), enemies.join(", "))
    };
    format!("{ally_str}\n{enemy_str}")
}

fn map_brief_text(
    selected_map_id: &str,
    maps: &[MapSpec],
    tanks: &[TankSpec],
) -> (String, String, Color) {
    if selected_map_id == RANDOM_MAP_ID {
        return (
            "Случайная карта  •  5v5 Автовыбор".to_string(),
            "Перед началом боя 5v5 выбирается случайная локация из 5 расширенных карт.\nБонус случайной карты: +10% серебра и опыта".to_string(),
            Color::srgb(0.95, 0.76, 0.36),
        );
    }
    let map_spec = maps
        .iter()
        .find(|m| m.id == selected_map_id)
        .cloned()
        .or_else(|| load_map_spec("proving_grounds"))
        .or_else(|| maps.first().cloned())
        .expect("default map");
    (
        format!("{}  •  {} (5v5)", map_spec.name, biome_short_ru(map_spec.biome)),
        format!("{}\n{}", map_spec.description, teams_lineup_ru(&map_spec, tanks)),
        biome_accent(map_spec.biome),
    )
}

fn format_hangar_ttx_compact(spec: &TankSpec) -> String {
    let p_100 = penetration_at(&spec.shell, 100.0);
    let p_500 = penetration_at(&spec.shell, 500.0);
    let hp_per_t = spec.engine_power_kw / (spec.mass_kg / 1000.0).max(1.0);
    let yaw_str = match spec.yaw_limits_deg {
        Some((a, b)) => format!("УГН {a:+.0}°..{b:+.0}° ({:.0}°/с)", spec.turret_traverse_dps),
        None => format!("Башня {:.0}°/с", spec.turret_traverse_dps),
    };
    let a = &spec.armor;
    format!(
        "Пробитие AP: {:.0} / {:.0} мм (100/500м)  •  Рикошет >{:.0}°\n\
         Урон SolidAP: {:.0} HP  •  Перезарядка: {:.1} с  •  {:.0} м/с\n\
         Корпус: ВЛД {:.0}@{:.0}° / НЛД {:.0} / Борт {:.0} / Корма {:.0} мм\n\
         Башня: Лоб {:.0} / Борт {:.0} / Корма {:.0} мм\n\
         Ход: +{:.0} / -{:.0} км/ч  •  Масса: {:.1} т ({:.1} кВт/т)\n\
         Наведение: {}  •  УВН -{:.0}°..+{:.0}°",
        p_100,
        p_500,
        spec.shell.ricochet_deg,
        spec.shell.arcade_damage,
        spec.reload_s,
        spec.shell.muzzle_velocity,
        a.hull_front_upper_mm,
        a.vld_slope_deg,
        a.hull_front_lower_mm,
        a.hull_side_mm,
        a.hull_rear_mm,
        a.turret_front_mm,
        a.turret_side_mm,
        a.turret_rear_mm,
        spec.max_speed_kmh,
        spec.reverse_speed_kmh,
        spec.mass_kg / 1000.0,
        hp_per_t,
        yaw_str,
        spec.gun_depression_deg,
        spec.gun_elevation_deg,
    )
}

fn armor_preview_color(mm: f32) -> Color {
    if mm >= 100.0 {
        Color::srgba(0.95, 0.24, 0.22, 0.28)
    } else if mm >= 55.0 {
        Color::srgba(0.98, 0.75, 0.18, 0.24)
    } else {
        Color::srgba(0.30, 0.88, 0.42, 0.20)
    }
}

fn spawn_xray_overlay(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    spec: &TankSpec,
    visible: bool,
) -> Entity {
    let unit_cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let mut children = Vec::new();

    for plate in hull_plate_specs(spec) {
        let mat = materials.add(StandardMaterial {
            base_color: armor_preview_color(plate.thickness_mm),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            cull_mode: None,
            ..default()
        });
        children.push(
            commands
                .spawn((
                    Mesh3d(unit_cube.clone()),
                    MeshMaterial3d(mat),
                    Transform::from_translation(plate.local_pos)
                        .with_rotation(plate.local_rot)
                        .with_scale(Vec3::new(
                            plate.half_extents.x * 2.0,
                            plate.half_extents.y * 2.0,
                            0.04,
                        )),
                ))
                .id(),
        );
    }

    let turret_yaw = if spec.yaw_limits_deg.is_some() { 0.0 } else { 0.20 };
    let turret_tf = Transform::from_translation(Vec3::from_array(spec.profile.turret_pivot))
        .with_rotation(Quat::from_rotation_y(turret_yaw));

    for plate in turret_plate_specs(spec) {
        let p_tf =
            turret_tf * Transform::from_translation(plate.local_pos).with_rotation(plate.local_rot);
        let mat = materials.add(StandardMaterial {
            base_color: armor_preview_color(plate.thickness_mm),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            cull_mode: None,
            ..default()
        });
        children.push(
            commands
                .spawn((
                    Mesh3d(unit_cube.clone()),
                    MeshMaterial3d(mat),
                    p_tf.with_scale(Vec3::new(
                        plate.half_extents.x * 2.0,
                        plate.half_extents.y * 2.0,
                        0.04,
                    )),
                ))
                .id(),
        );
    }

    let gun_pos = turret_tf.transform_point(
        Vec3::from_array(spec.profile.gun_pivot)
            + Vec3::new(0.0, 0.0, -spec.profile.barrel_length * 0.5),
    );
    let gun_mat = materials.add(StandardMaterial {
        base_color: Color::srgba(0.70, 0.74, 0.78, 0.45),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    });
    children.push(
        commands
            .spawn((
                Mesh3d(unit_cube.clone()),
                MeshMaterial3d(gun_mat),
                Transform::from_translation(gun_pos)
                    .with_rotation(turret_tf.rotation)
                    .with_scale(Vec3::new(
                        spec.profile.barrel_radius * 2.0,
                        spec.profile.barrel_radius * 2.0,
                        spec.profile.barrel_length,
                    )),
            ))
            .id(),
    );

    let mods = TankModules::for_spec(spec);
    for m in &mods.modules {
        let base_tf = if m.in_turret {
            turret_tf * Transform::from_translation(m.local_center)
        } else {
            Transform::from_translation(m.local_center)
        };
        let color = match m.kind {
            ModuleKind::AmmoRack => Color::srgb(0.98, 0.22, 0.20),
            ModuleKind::Engine => Color::srgb(0.98, 0.60, 0.14),
            ModuleKind::FuelTank => Color::srgb(0.20, 0.85, 0.98),
            ModuleKind::GunBreech => Color::srgb(0.95, 0.95, 0.95),
            ModuleKind::TurretRing => Color::srgb(0.55, 0.72, 1.0),
            ModuleKind::LeftTrack | ModuleKind::RightTrack => Color::srgb(0.38, 0.85, 0.38),
        };
        let mat = materials.add(StandardMaterial {
            base_color: color,
            unlit: true,
            ..default()
        });
        children.push(
            commands
                .spawn((
                    Mesh3d(unit_cube.clone()),
                    MeshMaterial3d(mat),
                    base_tf.with_scale(m.half_extents * 2.0),
                ))
                .id(),
        );
    }

    let crew = TankCrew::for_spec(spec);
    let crew_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.96, 0.88, 0.24),
        unlit: true,
        ..default()
    });
    for member in &crew.members {
        let base_tf = if member.in_turret {
            turret_tf * Transform::from_translation(member.local_center)
        } else {
            Transform::from_translation(member.local_center)
        };
        children.push(
            commands
                .spawn((
                    Mesh3d(unit_cube.clone()),
                    MeshMaterial3d(crew_mat.clone()),
                    base_tf.with_scale(member.half_extents * 1.7),
                ))
                .id(),
        );
    }

    commands
        .spawn((
            Name::new("HangarXrayRoot"),
            HangarXrayRoot,
            Transform::IDENTITY,
            if visible {
                Visibility::Visible
            } else {
                Visibility::Hidden
            },
            DespawnOnExit(GameState::Hangar),
        ))
        .add_children(&children)
        .id()
}

/// Строит воксельный подиум цеха со смотровой ямой, лестницей спуска и жёлтой разметкой.
fn build_workshop_pit_bay() -> VoxelGrid {
    // 0.25 м воксели: 44 x 6 x 48 = 11 x 1.5 x 12 м, верхний уровень (y=5) ровно на y = 0.0.
    let vs = 0.25;
    let dims = UVec3::new(44, 6, 48);
    let origin = Vec3::new(-5.5, -1.5, -6.0);
    let mut g = VoxelGrid::new(dims, vs, origin);

    // Базовая плита бетонного пола.
    g.fill_box(IVec3::new(0, 0, 0), IVec3::new(43, 5, 47), Material::Concrete);

    // 1. Центральная смотровая яма точно между гусеницами (|x| < 0.75 -> индексы 19..25).
    g.fill_box(IVec3::new(19, 1, 7), IVec3::new(24, 5, 39), Material::Air);
    // Жёлтые предупредительные канты вокруг центральной ямы.
    for z in 6..=40 {
        g.set(IVec3::new(18, 5, z), Material::EnemyMark);
        g.set(IVec3::new(25, 5, z), Material::EnemyMark);
    }
    for x in 18..=25 {
        g.set(IVec3::new(x, 5, 6), Material::EnemyMark);
        g.set(IVec3::new(x, 5, 40), Material::EnemyMark);
    }
    // Стальной настил решётки в носовой части ямы.
    g.fill_box(IVec3::new(19, 4, 8), IVec3::new(24, 4, 12), Material::TargetPost);

    // 2. Правая боковая смотровая траншея со ступеньками (как в новом ангаре WoT!).
    g.fill_box(IVec3::new(32, 1, 14), IVec3::new(37, 5, 34), Material::Air);
    for step in 0..4 {
        let z_s = 14 + step;
        let y_top = 4 - step;
        if y_top >= 1 {
            g.fill_box(
                IVec3::new(32, 1, z_s),
                IVec3::new(37, y_top, z_s),
                Material::TargetPost,
            );
        }
    }
    // Жёлтая окантовка спуска справа.
    for z in 13..=35 {
        g.set(IVec3::new(31, 5, z), Material::EnemyMark);
        g.set(IVec3::new(38, 5, z), Material::EnemyMark);
    }

    // 3. Тёмные стальные проезжие полосы под гусеницы танка.
    g.fill_box(IVec3::new(13, 5, 6), IVec3::new(17, 5, 41), Material::TargetPost);
    g.fill_box(IVec3::new(26, 5, 6), IVec3::new(30, 5, 41), Material::TargetPost);

    // 4. Небольшие верстак/ящики с инструментом справа от ямы.
    g.fill_box(IVec3::new(39, 5, 19), IVec3::new(41, 5, 25), Material::Wood);
    g.fill_box(IVec3::new(39, 5, 11), IVec3::new(40, 5, 13), Material::TankDark);
    g.fill_box(IVec3::new(14, 5, 3), IVec3::new(16, 5, 4), Material::Wood);

    g
}

/// Возводит промышленный 3D-ангар WoT вокруг танковой стоянки: пол, рельсы, колонны, фермы, кран и окна цеха.
fn spawn_hangar_environment(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    assets: &VoxelAssets,
) {
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let mut env_children = Vec::new();

    // Воксельный стенд со смотровой ямой и ступеньками.
    let bay = build_workshop_pit_bay();
    env_children.push(commands.spawn(voxel_model_bundle(&bay, meshes, assets)).id());

    let floor_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.10, 0.115, 0.13),
        perceptual_roughness: 0.90,
        metallic: 0.10,
        ..default()
    });
    let steel_dark = materials.add(StandardMaterial {
        base_color: Color::srgb(0.14, 0.16, 0.185),
        perceptual_roughness: 0.62,
        metallic: 0.65,
        ..default()
    });
    let steel_mid = materials.add(StandardMaterial {
        base_color: Color::srgb(0.22, 0.25, 0.28),
        perceptual_roughness: 0.55,
        metallic: 0.55,
        ..default()
    });
    let hazard_amber = materials.add(StandardMaterial {
        base_color: Color::srgb(0.84, 0.56, 0.14),
        perceptual_roughness: 0.48,
        metallic: 0.25,
        ..default()
    });
    let pipe_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.24, 0.27, 0.30),
        perceptual_roughness: 0.50,
        metallic: 0.70,
        ..default()
    });
    let silhouette_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.08, 0.095, 0.11),
        perceptual_roughness: 0.78,
        metallic: 0.35,
        ..default()
    });
    let window_glow = materials.add(StandardMaterial {
        base_color: Color::srgb(0.45, 0.72, 0.92),
        emissive: LinearRgba::new(0.85, 1.38, 1.95, 1.0),
        unlit: true,
        ..default()
    });
    let lamp_warm = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.86, 0.58),
        emissive: LinearRgba::new(2.8, 1.9, 0.9, 1.0),
        unlit: true,
        ..default()
    });

    let mut add_box = |pos: Vec3, scale: Vec3, mat: Handle<StandardMaterial>| {
        env_children.push(
            commands
                .spawn((
                    Mesh3d(cube.clone()),
                    MeshMaterial3d(mat),
                    Transform::from_translation(pos).with_scale(scale),
                ))
                .id(),
        );
    };

    // Широкий бетонный пол ангара по периметру стенда (чтобы не перекрывать смотровые ямы).
    add_box(Vec3::new(0.0, -0.05, 15.0), Vec3::new(48.0, 0.10, 18.0), floor_mat.clone());
    add_box(Vec3::new(0.0, -0.05, -15.0), Vec3::new(48.0, 0.10, 18.0), floor_mat.clone());
    add_box(Vec3::new(-14.75, -0.05, 0.0), Vec3::new(18.5, 0.10, 12.0), floor_mat.clone());
    add_box(Vec3::new(14.75, -0.05, 0.0), Vec3::new(18.5, 0.10, 12.0), floor_mat.clone());

    // Стальные рельсы по полу цеха слева и справа от стоянки.
    for rx in [-7.2f32, -5.8, 6.2, 7.6] {
        add_box(Vec3::new(rx, 0.03, 0.0), Vec3::new(0.11, 0.06, 42.0), steel_mid.clone());
    }

    // Колонны каркаса цеха вдоль левой и правой стен.
    for x in [-11.5f32, 11.5] {
        for z in [-16.0f32, -8.0, 0.0, 8.0, 16.0, 22.0] {
            add_box(Vec3::new(x, 5.5, z), Vec3::new(0.75, 11.0, 0.85), steel_dark.clone());
            // Усиление у основания колонны.
            add_box(Vec3::new(x, 0.45, z), Vec3::new(1.05, 0.9, 1.15), steel_mid.clone());
        }
        // Продольные подкрановые пути.
        add_box(Vec3::new(x * 0.93, 7.6, 3.0), Vec3::new(0.55, 0.70, 40.0), steel_mid.clone());
        add_box(Vec3::new(x * 0.93, 7.25, 3.0), Vec3::new(0.22, 0.18, 40.0), hazard_amber.clone());
    }

    // Поперечные фермы крыши и мостовой кран (узнаваемый силуэт заводского ангара WoT).
    for z in [-12.0f32, -4.0, 4.0, 12.0, 20.0] {
        add_box(Vec3::new(0.0, 10.2, z), Vec3::new(24.0, 0.65, 0.55), steel_dark.clone());
        add_box(Vec3::new(0.0, 8.9, z), Vec3::new(23.5, 0.30, 0.30), steel_mid.clone());
    }
    // Мостовой кран на заднем плане (`z = 7.5`) с жёлтой балкой и тельфером.
    add_box(Vec3::new(0.0, 8.15, 7.5), Vec3::new(21.4, 0.62, 0.75), hazard_amber.clone());
    add_box(Vec3::new(-2.4, 7.65, 7.5), Vec3::new(1.5, 0.75, 1.1), steel_dark.clone());

    // Промышленные трубы и воздуховоды по задней стене (`z = 16.5..18.5`).
    add_box(Vec3::new(0.0, 4.8, 16.8), Vec3::new(26.0, 0.95, 0.95), pipe_mat.clone());
    add_box(Vec3::new(0.0, 6.1, 17.4), Vec3::new(26.0, 0.65, 0.65), steel_mid.clone());
    add_box(Vec3::new(-4.5, 2.5, 16.5), Vec3::new(0.55, 5.0, 0.55), steel_mid.clone());
    add_box(Vec3::new(5.5, 2.5, 16.5), Vec3::new(0.55, 5.0, 0.55), steel_mid.clone());

    // Задняя и боковые стены со светящимися окнами цеха (контровой голубовато-стальной свет).
    add_box(Vec3::new(0.0, 5.5, 23.5), Vec3::new(34.0, 12.0, 0.8), steel_dark.clone());
    add_box(Vec3::new(-23.0, 5.5, 2.0), Vec3::new(0.8, 12.0, 44.0), steel_dark.clone());
    add_box(Vec3::new(23.0, 5.5, 2.0), Vec3::new(0.8, 12.0, 44.0), steel_dark.clone());
    add_box(Vec3::new(0.0, 5.5, -23.5), Vec3::new(34.0, 12.0, 0.8), steel_dark.clone());

    for wx in [-8.5f32, -2.8, 2.8, 8.5] {
        add_box(Vec3::new(wx, 5.8, 22.8), Vec3::new(4.2, 4.2, 0.15), window_glow.clone());
    }
    for wz in [-10.0f32, 0.0, 10.0] {
        add_box(Vec3::new(22.2, 6.2, wz), Vec3::new(0.15, 3.8, 5.2), window_glow.clone());
        add_box(Vec3::new(-22.2, 6.2, wz), Vec3::new(0.15, 3.8, 5.2), window_glow.clone());
    }

    // Потолочные софиты.
    for lx in [-4.5f32, 0.0, 4.5] {
        add_box(Vec3::new(lx, 8.65, -1.5), Vec3::new(1.2, 0.14, 0.45), lamp_warm.clone());
    }

    // Заводские ящики, бочка и запасной каток у переднего правого края ямы.
    add_box(Vec3::new(4.1, 0.28, -2.6), Vec3::new(0.85, 0.56, 0.55), pipe_mat);
    add_box(Vec3::new(3.4, 0.20, -3.5), Vec3::new(0.55, 0.40, 0.55), hazard_amber);
    add_box(Vec3::new(-4.3, 0.35, 3.8), Vec3::new(1.10, 0.70, 0.70), steel_mid);

    // Фоновые силуэты машин в цеху (придают масштаб завода из скриншота WoT).
    for (bx, bz, yaw) in [(-8.6f32, 12.8f32, 0.35f32), (9.2, 13.4, -0.45)] {
        let rot = Quat::from_rotation_y(yaw);
        let hull_p = Vec3::new(bx, 0.75, bz);
        env_children.push(
            commands
                .spawn((
                    Mesh3d(cube.clone()),
                    MeshMaterial3d(silhouette_mat.clone()),
                    Transform::from_translation(hull_p)
                        .with_rotation(rot)
                        .with_scale(Vec3::new(2.8, 1.3, 5.8)),
                ))
                .id(),
        );
        env_children.push(
            commands
                .spawn((
                    Mesh3d(cube.clone()),
                    MeshMaterial3d(silhouette_mat.clone()),
                    Transform::from_translation(hull_p + Vec3::Y * 0.95)
                        .with_rotation(rot)
                        .with_scale(Vec3::new(1.8, 0.85, 2.2)),
                ))
                .id(),
        );
        env_children.push(
            commands
                .spawn((
                    Mesh3d(cube.clone()),
                    MeshMaterial3d(silhouette_mat.clone()),
                    Transform::from_translation(hull_p + Vec3::Y * 1.0 + rot * Vec3::new(0.0, 0.0, -2.5))
                        .with_rotation(rot)
                        .with_scale(Vec3::new(0.18, 0.18, 3.8)),
                ))
                .id(),
        );
    }

    commands
        .spawn((
            Name::new("HangarEnvironment"),
            Transform::IDENTITY,
            Visibility::default(),
            DespawnOnExit(GameState::Hangar),
        ))
        .add_children(&env_children);

    // 1. Тёплый направленный свет цеха сверху спереди-слева (рисует объём граней).
    commands.spawn((
        DirectionalLight {
            color: Color::srgb(1.0, 0.92, 0.78),
            illuminance: 8_400.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(-7.5, 13.0, -9.0).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
        DespawnOnExit(GameState::Hangar),
    ));

    // 2. Тёплая подсветка из смотровой ямы на днище и гусеницы.
    commands.spawn((
        PointLight {
            color: Color::srgb(1.0, 0.66, 0.22),
            intensity: 16_000.0,
            range: 9.5,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(0.15, -0.35, -1.65),
        DespawnOnExit(GameState::Hangar),
    ));

    // 3. Контровой холодный свет от промышленных окон цеха (делает силуэт выразительным).
    commands.spawn((
        PointLight {
            color: Color::srgb(0.42, 0.74, 1.0),
            intensity: 28_000.0,
            range: 28.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(5.5, 6.2, 12.0),
        DespawnOnExit(GameState::Hangar),
    ));
}

fn owned_combat_spec(selected_tank: &SelectedTank, prog: &PlayerProgression) -> TankSpec {
    if is_owned(prog, &selected_tank.0.0)
        && let Some(spec) = load_tank_spec(&selected_tank.0.0)
    {
        return spec;
    }
    load_tank_spec("ms_1")
        .or_else(|| available_tanks().into_iter().next())
        .expect("at least one tank spec")
}

fn filtered_carousel_indices(
    all_tanks: &[TankSpec],
    prog: &PlayerProgression,
    filter: HangarFilter,
) -> Vec<usize> {
    all_tanks
        .iter()
        .enumerate()
        .filter(|(_, t)| match filter {
            HangarFilter::AllOwned => is_owned(prog, &t.id),
            HangarFilter::Nation(n) => t.nation_id == n,
        })
        .map(|(idx, _)| idx)
        .collect()
}

fn filter_tab_button(
    parent: &mut ChildSpawnerCommands,
    filter: HangarFilter,
    label: String,
    active: bool,
    accent: Color,
) {
    parent
        .spawn((
            Interaction::default(),
            HangarButton::SelectFilter(filter),
            Node {
                padding: UiRect::axes(Val::Px(11.0), Val::Px(4.0)),
                border: UiRect::all(Val::Px(if active { 1.5 } else { 1.0 })),
                border_radius: BorderRadius::all(Val::Px(3.0)),
                ..default()
            },
            BackgroundColor(if active {
                Color::srgba(0.18, 0.25, 0.34, 0.95)
            } else {
                Color::srgba(0.05, 0.07, 0.10, 0.85)
            }),
            BorderColor::all(if active {
                accent
            } else {
                Color::srgba(0.28, 0.34, 0.42, 0.75)
            }),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(label),
                TextFont::from_font_size(11.5),
                TextColor(if active {
                    Color::srgb(0.98, 0.96, 0.88)
                } else {
                    Color::srgb(0.74, 0.80, 0.88)
                }),
            ));
        });
}

fn prerequisite_label_ru(spec: &TankSpec, all_tanks: &[TankSpec]) -> String {
    let resolve = |id: &str| -> String {
        all_tanks
            .iter()
            .find(|x| x.id == id)
            .map_or_else(|| id.to_string(), |x| x.name.clone())
    };
    match (&spec.prerequisite, &spec.alt_prerequisite) {
        (Some(p), Some(alt)) => format!("{} / {}", resolve(p), resolve(alt)),
        (Some(p), None) => resolve(p),
        (None, Some(alt)) => resolve(alt),
        (None, None) => "предыдущий".to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_tech_tree_modal(
    root: &mut ChildSpawnerCommands,
    all_tanks: &[TankSpec],
    prog: &PlayerProgression,
    selected_tank: &SelectedTank,
    view: &HangarViewState,
) {
    root.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
    ))
    .with_children(|overlay| {
        // Задний слой-блокиратор: перехватывает клики/наведение, чтобы камера под окном не вращалась.
        overlay.spawn((
            Interaction::default(),
            HangarButton::Noop,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.01, 0.015, 0.025, 0.78)),
        ));

        // Основное окно дерева исследований.
        overlay
            .spawn((
                Node {
                    width: Val::Percent(96.0),
                    max_width: Val::Px(1260.0),
                    padding: UiRect::all(Val::Px(12.0)),
                    border: UiRect::all(Val::Px(1.5)),
                    border_radius: BorderRadius::all(Val::Px(6.0)),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.032, 0.046, 0.068, 0.97)),
                BorderColor::all(Color::srgb(0.84, 0.62, 0.24)),
            ))
            .with_children(|modal| {
                // Шапка дерева исследований.
                modal
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                            border: UiRect::bottom(Val::Px(1.0)),
                            column_gap: Val::Px(10.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.05, 0.075, 0.11, 0.95)),
                        BorderColor::all(Color::srgba(0.32, 0.40, 0.50, 0.8)),
                    ))
                    .with_children(|mh| {
                        mh.spawn((
                            Text::new("ДЕРЕВО ИССЛЕДОВАНИЙ (3 НАЦИИ × V ТИРОВ • 18 МАШИН SOLID-AP)"),
                            TextFont::from_font_size(14.0),
                            TextColor(Color::srgb(0.96, 0.86, 0.56)),
                        ));

                        mh.spawn(Node {
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(12.0),
                            ..default()
                        })
                        .with_children(|mid| {
                            mid.spawn((
                                Text::new(format!("Серебро: {}", prog.credits)),
                                TextFont::from_font_size(13.0),
                                TextColor(Color::srgb(0.94, 0.86, 0.52)),
                            ));
                            mid.spawn((
                                Text::new(format!("Опыт: {}", prog.xp)),
                                TextFont::from_font_size(13.0),
                                TextColor(Color::srgb(0.56, 0.86, 1.0)),
                            ));
                            if let Some((ref msg, col)) = view.notice {
                                mid.spawn((
                                    Text::new(msg.clone()),
                                    TextFont::from_font_size(11.5),
                                    TextColor(col),
                                ));
                            }
                        });

                        mh.spawn((
                            Interaction::default(),
                            HangarButton::ToggleTechTree,
                            Node {
                                padding: UiRect::axes(Val::Px(12.0), Val::Px(5.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                border_radius: BorderRadius::all(Val::Px(3.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.28, 0.12, 0.12)),
                            BorderColor::all(Color::srgb(0.88, 0.42, 0.36)),
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new("Закрыть [T / Esc]"),
                                TextFont::from_font_size(12.0),
                                TextColor(Color::WHITE),
                            ));
                        });
                    });

                // 3 колонки наций (СССР, Германия, США) с параллельными альтернативными ветками на одном ранге.
                modal
                    .spawn(Node {
                        width: Val::Percent(100.0),
                        column_gap: Val::Px(10.0),
                        align_items: AlignItems::Stretch,
                        ..default()
                    })
                    .with_children(|cols| {
                        for nation in Nation::all() {
                            let accent = nation.accent();
                            cols.spawn((
                                Node {
                                    flex_grow: 1.0,
                                    flex_basis: Val::Px(0.0),
                                    flex_direction: FlexDirection::Column,
                                    padding: UiRect::all(Val::Px(8.0)),
                                    border: UiRect {
                                        top: Val::Px(3.0),
                                        left: Val::Px(1.0),
                                        right: Val::Px(1.0),
                                        bottom: Val::Px(1.0),
                                    },
                                    border_radius: BorderRadius::all(Val::Px(4.0)),
                                    row_gap: Val::Px(3.0),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.04, 0.056, 0.08, 0.92)),
                                BorderColor::all(accent),
                            ))
                            .with_children(|col| {
                                col.spawn((
                                    Text::new(format!("{}  (I -> V)", nation.ru_name())),
                                    TextFont::from_font_size(13.0),
                                    TextColor(Color::srgb(0.95, 0.96, 0.98)),
                                ));

                                let branch: Vec<(usize, &TankSpec)> = all_tanks
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, t)| t.nation_id == nation)
                                    .collect();

                                let mut prev_count = 0usize;
                                for tier in 1..=5u8 {
                                    let row_tanks: Vec<(usize, &TankSpec)> = branch
                                        .iter()
                                        .copied()
                                        .filter(|(_, t)| t.tier == tier)
                                        .collect();
                                    if row_tanks.is_empty() {
                                        continue;
                                    }

                                    if prev_count > 0 {
                                        let connector = match (prev_count, row_tanks.len()) {
                                            (1, 2) => "↙   ↘",
                                            (2, 2) => "↓   ↓",
                                            (2, 1) => "↘   ↙",
                                            _ => "↓",
                                        };
                                        col.spawn((
                                            Node {
                                                align_self: AlignSelf::Center,
                                                ..default()
                                            },
                                            Text::new(connector),
                                            TextFont::from_font_size(10.5),
                                            TextColor(Color::srgb(0.55, 0.65, 0.78)),
                                        ));
                                    }
                                    prev_count = row_tanks.len();
                                    let is_split = row_tanks.len() > 1;

                                    col.spawn(Node {
                                        width: Val::Percent(100.0),
                                        flex_direction: FlexDirection::Row,
                                        column_gap: Val::Px(6.0),
                                        align_items: AlignItems::Stretch,
                                        ..default()
                                    })
                                    .with_children(|tier_row| {
                                        for &(g_idx, t) in &row_tanks {
                                            let owned = is_owned(prog, &t.id);
                                            let researched = is_researched(prog, &t.id);
                                            let is_combat = t.id == selected_tank.0.0;
                                            let is_preview = t.id == view.preview_tank_id;
                                            let prereq_met = has_prerequisite_researched(prog, t);
                                            let tm = tank_wot_meta(t);
                                            let pen = penetration_at(&t.shell, 100.0);

                                            let (badge, badge_col, card_bg, card_border) = if owned {
                                                (
                                                    if is_combat {
                                                        "В БОЮ"
                                                    } else if is_split {
                                                        "АНГАР"
                                                    } else {
                                                        "В АНГАРЕ"
                                                    },
                                                    Color::srgb(0.45, 0.94, 0.54),
                                                    Color::srgba(0.09, 0.18, 0.12, 0.94),
                                                    if is_combat {
                                                        Color::srgb(0.96, 0.68, 0.22)
                                                    } else {
                                                        Color::srgb(0.32, 0.74, 0.42)
                                                    },
                                                )
                                            } else if researched {
                                                (
                                                    "ОТКРЫТ",
                                                    Color::srgb(0.98, 0.82, 0.36),
                                                    Color::srgba(0.16, 0.14, 0.08, 0.94),
                                                    Color::srgb(0.86, 0.68, 0.28),
                                                )
                                            } else if prereq_met {
                                                (
                                                    if is_split { "ДОСТ." } else { "ДОСТУПЕН" },
                                                    Color::srgb(0.54, 0.86, 1.0),
                                                    Color::srgba(0.08, 0.13, 0.20, 0.94),
                                                    Color::srgb(0.36, 0.64, 0.92),
                                                )
                                            } else {
                                                (
                                                    "ЗАКРЫТ",
                                                    Color::srgb(0.56, 0.62, 0.70),
                                                    Color::srgba(0.05, 0.06, 0.08, 0.88),
                                                    Color::srgba(0.24, 0.28, 0.34, 0.8),
                                                )
                                            };

                                            tier_row
                                                .spawn((
                                                    Node {
                                                        flex_grow: 1.0,
                                                        flex_basis: Val::Px(0.0),
                                                        flex_direction: FlexDirection::Column,
                                                        padding: UiRect::axes(
                                                            Val::Px(if is_split { 6.0 } else { 8.0 }),
                                                            Val::Px(5.0),
                                                        ),
                                                        border: UiRect::all(Val::Px(
                                                            if is_preview { 2.0 } else { 1.0 },
                                                        )),
                                                        border_radius: BorderRadius::all(Val::Px(
                                                            4.0,
                                                        )),
                                                        row_gap: Val::Px(3.0),
                                                        ..default()
                                                    },
                                                    BackgroundColor(card_bg),
                                                    BorderColor::all(card_border),
                                                ))
                                                .with_children(|card| {
                                                    card.spawn(Node {
                                                        width: Val::Percent(100.0),
                                                        justify_content:
                                                            JustifyContent::SpaceBetween,
                                                        align_items: AlignItems::Center,
                                                        ..default()
                                                    })
                                                    .with_children(|r1| {
                                                        r1.spawn((
                                                            Text::new(format!(
                                                                "{} {} {} {}",
                                                                tm.tier,
                                                                tm.class_icon,
                                                                tm.class_short,
                                                                t.name
                                                            )),
                                                            TextFont::from_font_size(if is_split {
                                                                11.0
                                                            } else {
                                                                12.0
                                                            }),
                                                            TextColor(
                                                                if owned || researched || prereq_met
                                                                {
                                                                    Color::srgb(0.96, 0.97, 0.99)
                                                                } else {
                                                                    Color::srgb(0.62, 0.67, 0.74)
                                                                },
                                                            ),
                                                        ));
                                                        r1.spawn((
                                                            Text::new(badge),
                                                            TextFont::from_font_size(9.5),
                                                            TextColor(badge_col),
                                                        ));
                                                    });

                                                    card.spawn((
                                                        Text::new(if is_split {
                                                            format!(
                                                                "{} • {:.0}мм • {:.0}HP",
                                                                tm.gun_short, pen, t.max_hp
                                                            )
                                                        } else {
                                                            format!(
                                                                "{} • Проб. {:.0}мм • {:.0} HP",
                                                                tm.gun_short, pen, t.max_hp
                                                            )
                                                        }),
                                                        TextFont::from_font_size(if is_split {
                                                            9.5
                                                        } else {
                                                            10.5
                                                        }),
                                                        TextColor(Color::srgb(0.72, 0.80, 0.88)),
                                                    ));

                                                    // Строка действий (строго соседние кнопки без вложенных Interaction!).
                                                    card.spawn(Node {
                                                        width: Val::Percent(100.0),
                                                        justify_content:
                                                            JustifyContent::SpaceBetween,
                                                        align_items: AlignItems::Center,
                                                        column_gap: Val::Px(4.0),
                                                        ..default()
                                                    })
                                                    .with_children(|actions| {
                                                        actions
                                                            .spawn((
                                                                Interaction::default(),
                                                                HangarButton::InspectTank(g_idx),
                                                                Node {
                                                                    padding: UiRect::axes(
                                                                        Val::Px(if is_split {
                                                                            5.0
                                                                        } else {
                                                                            7.0
                                                                        }),
                                                                        Val::Px(3.0),
                                                                    ),
                                                                    border: UiRect::all(Val::Px(
                                                                        1.0,
                                                                    )),
                                                                    border_radius:
                                                                        BorderRadius::all(Val::Px(
                                                                            3.0,
                                                                        )),
                                                                    ..default()
                                                                },
                                                                BackgroundColor(Color::srgba(
                                                                    0.10, 0.15, 0.22, 0.92,
                                                                )),
                                                                BorderColor::all(Color::srgb(
                                                                    0.38, 0.52, 0.68,
                                                                )),
                                                            ))
                                                            .with_children(|b| {
                                                                b.spawn((
                                                                    Text::new(if is_split {
                                                                        "3D"
                                                                    } else {
                                                                        "Осмотр 3D"
                                                                    }),
                                                                    TextFont::from_font_size(10.0),
                                                                    TextColor(Color::srgb(
                                                                        0.86, 0.92, 0.98,
                                                                    )),
                                                                ));
                                                            });

                                                        if owned {
                                                            actions
                                                                .spawn((
                                                                    Interaction::default(),
                                                                    HangarButton::SelectOwnedAndClose(
                                                                        g_idx,
                                                                    ),
                                                                    Node {
                                                                        padding: UiRect::axes(
                                                                            Val::Px(if is_split {
                                                                                6.0
                                                                            } else {
                                                                                8.0
                                                                            }),
                                                                            Val::Px(3.0),
                                                                        ),
                                                                        border: UiRect::all(
                                                                            Val::Px(1.0),
                                                                        ),
                                                                        border_radius:
                                                                            BorderRadius::all(
                                                                                Val::Px(3.0),
                                                                            ),
                                                                        ..default()
                                                                    },
                                                                    BackgroundColor(if is_combat {
                                                                        Color::srgb(
                                                                            0.18, 0.36, 0.20,
                                                                        )
                                                                    } else {
                                                                        Color::srgb(
                                                                            0.14, 0.26, 0.16,
                                                                        )
                                                                    }),
                                                                    BorderColor::all(Color::srgb(
                                                                        0.44, 0.86, 0.48,
                                                                    )),
                                                                ))
                                                                .with_children(|b| {
                                                                    b.spawn((
                                                                        Text::new(if is_combat {
                                                                            if is_split {
                                                                                "В бою"
                                                                            } else {
                                                                                "Выбран в бой"
                                                                            }
                                                                        } else if is_split {
                                                                            "В бой"
                                                                        } else {
                                                                            "Выбрать в бой"
                                                                        }),
                                                                        TextFont::from_font_size(
                                                                            10.0,
                                                                        ),
                                                                        TextColor(Color::srgb(
                                                                            0.90, 0.98, 0.90,
                                                                        )),
                                                                    ));
                                                                });
                                                        } else if researched {
                                                            let can_afford = prog.credits
                                                                >= t.credit_cost as u64;
                                                            actions
                                                                .spawn((
                                                                    Interaction::default(),
                                                                    HangarButton::BuyTank(g_idx),
                                                                    Node {
                                                                        padding: UiRect::axes(
                                                                            Val::Px(if is_split {
                                                                                6.0
                                                                            } else {
                                                                                8.0
                                                                            }),
                                                                            Val::Px(3.0),
                                                                        ),
                                                                        border: UiRect::all(
                                                                            Val::Px(1.0),
                                                                        ),
                                                                        border_radius:
                                                                            BorderRadius::all(
                                                                                Val::Px(3.0),
                                                                            ),
                                                                        ..default()
                                                                    },
                                                                    BackgroundColor(
                                                                        if can_afford {
                                                                            Color::srgb(
                                                                                0.46, 0.28, 0.08,
                                                                            )
                                                                        } else {
                                                                            Color::srgba(
                                                                                0.24, 0.12, 0.10,
                                                                                0.92,
                                                                            )
                                                                        },
                                                                    ),
                                                                    BorderColor::all(
                                                                        if can_afford {
                                                                            Color::srgb(
                                                                                0.96, 0.72, 0.24,
                                                                            )
                                                                        } else {
                                                                            Color::srgb(
                                                                                0.62, 0.34, 0.28,
                                                                            )
                                                                        },
                                                                    ),
                                                                ))
                                                                .with_children(|b| {
                                                                    b.spawn((
                                                                        Text::new(if is_split {
                                                                            format!(
                                                                                "Купить: {}с",
                                                                                t.credit_cost
                                                                            )
                                                                        } else {
                                                                            format!(
                                                                                "Купить: {} сер.",
                                                                                t.credit_cost
                                                                            )
                                                                        }),
                                                                        TextFont::from_font_size(
                                                                            10.0,
                                                                        ),
                                                                        TextColor(Color::srgb(
                                                                            1.0, 0.92, 0.72,
                                                                        )),
                                                                    ));
                                                                });
                                                        } else if prereq_met {
                                                            let can_xp =
                                                                prog.xp >= t.xp_cost as u64;
                                                            actions
                                                                .spawn((
                                                                    Interaction::default(),
                                                                    HangarButton::ResearchTank(
                                                                        g_idx,
                                                                    ),
                                                                    Node {
                                                                        padding: UiRect::axes(
                                                                            Val::Px(if is_split {
                                                                                6.0
                                                                            } else {
                                                                                8.0
                                                                            }),
                                                                            Val::Px(3.0),
                                                                        ),
                                                                        border: UiRect::all(
                                                                            Val::Px(1.0),
                                                                        ),
                                                                        border_radius:
                                                                            BorderRadius::all(
                                                                                Val::Px(3.0),
                                                                            ),
                                                                        ..default()
                                                                    },
                                                                    BackgroundColor(if can_xp {
                                                                        Color::srgb(
                                                                            0.12, 0.28, 0.44,
                                                                        )
                                                                    } else {
                                                                        Color::srgba(
                                                                            0.16, 0.12, 0.18, 0.90,
                                                                        )
                                                                    }),
                                                                    BorderColor::all(if can_xp {
                                                                        Color::srgb(0.44, 0.80, 1.0)
                                                                    } else {
                                                                        Color::srgb(
                                                                            0.46, 0.42, 0.56,
                                                                        )
                                                                    }),
                                                                ))
                                                                .with_children(|b| {
                                                                    b.spawn((
                                                                        Text::new(if is_split {
                                                                            format!(
                                                                                "Откр.: {}о",
                                                                                t.xp_cost
                                                                            )
                                                                        } else {
                                                                            format!(
                                                                                "Исследовать: {} оп.",
                                                                                t.xp_cost
                                                                            )
                                                                        }),
                                                                        TextFont::from_font_size(
                                                                            10.0,
                                                                        ),
                                                                        TextColor(Color::srgb(
                                                                            0.86, 0.96, 1.0,
                                                                        )),
                                                                    ));
                                                                });
                                                        } else {
                                                            let prereq_name =
                                                                prerequisite_label_ru(t, all_tanks);
                                                            actions.spawn((
                                                                Text::new(if is_split {
                                                                    format!(
                                                                        "Нужен {prereq_name}"
                                                                    )
                                                                } else {
                                                                    format!(
                                                                        "Нужен {prereq_name} ({} оп.)",
                                                                        t.xp_cost
                                                                    )
                                                                }),
                                                                TextFont::from_font_size(9.5),
                                                                TextColor(Color::srgb(
                                                                    0.56, 0.60, 0.68,
                                                                )),
                                                            ));
                                                        }
                                                    });
                                                });
                                        }
                                    });
                                }
                            });
                        }
                    });
            });
    });
}

#[allow(clippy::too_many_arguments)]
fn spawn_hangar_ui(
    commands: &mut Commands,
    all_tanks: &[TankSpec],
    maps: &[MapSpec],
    selected_tank: &SelectedTank,
    selected_map: &SelectedMap,
    mode: DamageRulesMode,
    view: &HangarViewState,
    prog: &PlayerProgression,
    last_reward: &LastBattleReward,
    preview_spec: &TankSpec,
    muted: bool,
) {
    let combat_spec = owned_combat_spec(selected_tank, prog);
    let (brief_title, brief_body, brief_accent) =
        map_brief_text(&selected_map.0, maps, all_tanks);
    let meta = tank_wot_meta(preview_spec);
    let ratings = compute_tank_ratings(preview_spec);
    let wt_mode = mode == DamageRulesMode::ModulesOnlyWt;
    let visible_indices = filtered_carousel_indices(all_tanks, prog, view.filter_tab);
    let preview_global_idx = all_tanks
        .iter()
        .position(|t| t.id == preview_spec.id)
        .unwrap_or(0);
    let preview_owned = is_owned(prog, &preview_spec.id);
    let preview_researched = is_researched(prog, &preview_spec.id);
    let preview_prereq_met = has_prerequisite_researched(prog, preview_spec);
    let owned_count = all_tanks.iter().filter(|t| is_owned(prog, &t.id)).count();

    commands
        .spawn((
            HangarUiRoot,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::SpaceBetween,
                padding: UiRect::all(Val::Px(10.0)),
                row_gap: Val::Px(5.0),
                ..default()
            },
            DespawnOnExit(GameState::Hangar),
        ))
        .with_children(|root| {
            // 1. ВЕРХНЯЯ ПАНЕЛЬ + КРУПНАЯ КНОПКА «В БОЙ!» + ТИТУЛ МАШИНЫ + ПРОГРЕСС
            root.spawn(Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(5.0),
                ..default()
            })
            .with_children(|header| {
                // Тёмная верхняя строка навигации WoT с экономикой и кнопкой дерева.
                header
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            padding: UiRect::axes(Val::Px(12.0), Val::Px(5.0)),
                            border: UiRect::bottom(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(4.0)),
                            column_gap: Val::Px(8.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.04, 0.055, 0.072, 0.92)),
                        BorderColor::all(Color::srgba(0.32, 0.38, 0.46, 0.72)),
                    ))
                    .with_children(|bar| {
                        // Лево: бренд, версия, режим и баланс серебра/опыта.
                        bar.spawn(Node {
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(8.0),
                            ..default()
                        })
                        .with_children(|left| {
                            left.spawn((
                                Text::new("CERIUM-TURBO"),
                                TextFont::from_font_size(15.0),
                                TextColor(Color::srgb(0.95, 0.96, 0.98)),
                            ));
                            left.spawn((
                                Node {
                                    padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                                    border: UiRect::all(Val::Px(1.0)),
                                    border_radius: BorderRadius::all(Val::Px(3.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.68, 0.30, 0.08, 0.92)),
                                BorderColor::all(Color::srgb(0.96, 0.56, 0.22)),
                            ))
                            .with_children(|b| {
                                b.spawn((
                                    Text::new("v1.4.1"),
                                    TextFont::from_font_size(11.5),
                                    TextColor(Color::srgb(1.0, 0.92, 0.78)),
                                ));
                            });
                            left.spawn((
                                HangarLabel::HeaderStatus,
                                Text::new(format!("• {}", mode.ru_label())),
                                TextFont::from_font_size(11.5),
                                TextColor(Color::srgb(0.72, 0.80, 0.88)),
                            ));
                            left.spawn((
                                Node {
                                    padding: UiRect::axes(Val::Px(8.0), Val::Px(2.0)),
                                    border: UiRect::all(Val::Px(1.0)),
                                    border_radius: BorderRadius::all(Val::Px(3.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.08, 0.11, 0.16, 0.92)),
                                BorderColor::all(Color::srgba(0.45, 0.52, 0.62, 0.65)),
                            ))
                            .with_children(|econ| {
                                econ.spawn((
                                    Text::new(format!(
                                        "Серебро: {}  |  Опыт: {}",
                                        prog.credits, prog.xp
                                    )),
                                    TextFont::from_font_size(12.0),
                                    TextColor(Color::srgb(0.96, 0.86, 0.50)),
                                ));
                            });
                        });

                        // Центр: фирменная оранжево-красная кнопка «В БОЙ!».
                        bar.spawn((
                            Interaction::default(),
                            HangarButton::StartBattle,
                            Node {
                                padding: UiRect::axes(Val::Px(24.0), Val::Px(7.0)),
                                border: UiRect::all(Val::Px(2.0)),
                                border_radius: BorderRadius::all(Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.84, 0.29, 0.10)),
                            BorderColor::all(Color::srgb(1.0, 0.56, 0.26)),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                Text::new(format!("В БОЙ! ({})", combat_spec.name)),
                                TextFont::from_font_size(15.5),
                                TextColor(Color::WHITE),
                            ));
                        });

                        // Право: кнопка дерева исследований [T], краткая сводка и кнопка звука [N].
                        bar.spawn(Node {
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(8.0),
                            ..default()
                        })
                        .with_children(|right| {
                            right
                                .spawn((
                                    Interaction::default(),
                                    HangarButton::ToggleTechTree,
                                    Node {
                                        padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)),
                                        border: UiRect::all(Val::Px(1.5)),
                                        border_radius: BorderRadius::all(Val::Px(3.0)),
                                        ..default()
                                    },
                                    BackgroundColor(if view.show_tech_tree {
                                        Color::srgb(0.32, 0.24, 0.08)
                                    } else {
                                        Color::srgb(0.12, 0.20, 0.30)
                                    }),
                                    BorderColor::all(if view.show_tech_tree {
                                        Color::srgb(0.96, 0.74, 0.26)
                                    } else {
                                        Color::srgb(0.45, 0.70, 0.95)
                                    }),
                                ))
                                .with_children(|b| {
                                    b.spawn((
                                        Text::new("[T] Дерево"),
                                        TextFont::from_font_size(12.0),
                                        TextColor(Color::srgb(0.94, 0.96, 1.0)),
                                    ));
                                });

                            right.spawn((
                                Text::new("5v5 • 18 машин • AP-Only"),
                                TextFont::from_font_size(11.0),
                                TextColor(Color::srgb(0.68, 0.75, 0.82)),
                            ));

                            right
                                .spawn((
                                    Interaction::default(),
                                    HangarButton::ToggleAudio,
                                    Node {
                                        padding: UiRect::axes(Val::Px(9.0), Val::Px(5.0)),
                                        border: UiRect::all(Val::Px(1.0)),
                                        border_radius: BorderRadius::all(Val::Px(3.0)),
                                        ..default()
                                    },
                                    BackgroundColor(if muted {
                                        Color::srgb(0.32, 0.14, 0.14)
                                    } else {
                                        Color::srgb(0.12, 0.24, 0.20)
                                    }),
                                    BorderColor::all(if muted {
                                        Color::srgb(0.84, 0.38, 0.36)
                                    } else {
                                        Color::srgb(0.42, 0.78, 0.62)
                                    }),
                                ))
                                .with_children(|btn| {
                                    btn.spawn((
                                        HangarLabel::ToggleAudio,
                                        Text::new(if muted {
                                            "Звук [N]: ВЫКЛ"
                                        } else {
                                            "Звук [N]: ВКЛ"
                                        }),
                                        TextFont::from_font_size(11.5),
                                        TextColor(Color::srgb(0.90, 0.96, 0.94)),
                                    ));
                                });
                        });
                    });

                // Центрированный заголовок осматриваемой машины + кнопка исследования/покупки при осмотре заблокированной.
                header
                    .spawn((
                        Node {
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(12.0),
                            padding: UiRect::axes(Val::Px(18.0), Val::Px(4.0)),
                            border_radius: BorderRadius::all(Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.03, 0.045, 0.06, 0.62)),
                    ))
                    .with_children(|hero| {
                        hero.spawn(Node {
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|titles| {
                            titles.spawn((
                                HangarLabel::HeroTitle,
                                Text::new(format!(
                                    "{} {} {}   {}   •   {}",
                                    meta.tier,
                                    meta.class_icon,
                                    meta.class_short,
                                    preview_spec.name,
                                    meta.nation_short
                                )),
                                TextFont::from_font_size(18.0),
                                TextColor(Color::srgb(0.96, 0.97, 0.98)),
                            ));
                            titles.spawn((
                                HangarLabel::HeroSubtitle,
                                Text::new(format!(
                                    "{} • SolidAP {:.0} мм • {:.0} HP",
                                    meta.gun_short,
                                    penetration_at(&preview_spec.shell, 100.0),
                                    preview_spec.max_hp
                                )),
                                TextFont::from_font_size(11.5),
                                TextColor(Color::srgb(0.76, 0.82, 0.88)),
                            ));
                        });

                        if preview_owned {
                            let is_active = preview_spec.id == combat_spec.id;
                            if is_active {
                                hero.spawn((
                                    Node {
                                        padding: UiRect::axes(Val::Px(9.0), Val::Px(4.0)),
                                        border: UiRect::all(Val::Px(1.0)),
                                        border_radius: BorderRadius::all(Val::Px(3.0)),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgba(0.10, 0.24, 0.14, 0.90)),
                                    BorderColor::all(Color::srgb(0.36, 0.80, 0.44)),
                                ))
                                .with_children(|b| {
                                    b.spawn((
                                        Text::new("В АНГАРЕ • ГОТОВ К БОЮ"),
                                        TextFont::from_font_size(11.0),
                                        TextColor(Color::srgb(0.76, 0.98, 0.80)),
                                    ));
                                });
                            } else {
                                hero.spawn((
                                    Interaction::default(),
                                    HangarButton::SelectTank(preview_global_idx),
                                    Node {
                                        padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                        border: UiRect::all(Val::Px(1.0)),
                                        border_radius: BorderRadius::all(Val::Px(3.0)),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgb(0.14, 0.30, 0.16)),
                                    BorderColor::all(Color::srgb(0.45, 0.88, 0.48)),
                                ))
                                .with_children(|b| {
                                    b.spawn((
                                        Text::new("Выбрать в бой"),
                                        TextFont::from_font_size(11.5),
                                        TextColor(Color::WHITE),
                                    ));
                                });
                            }
                        } else if preview_researched {
                            let can_afford = prog.credits >= preview_spec.credit_cost as u64;
                            hero.spawn((
                                Interaction::default(),
                                HangarButton::BuyTank(preview_global_idx),
                                Node {
                                    padding: UiRect::axes(Val::Px(11.0), Val::Px(4.0)),
                                    border: UiRect::all(Val::Px(1.5)),
                                    border_radius: BorderRadius::all(Val::Px(3.0)),
                                    ..default()
                                },
                                BackgroundColor(if can_afford {
                                    Color::srgb(0.46, 0.28, 0.08)
                                } else {
                                    Color::srgba(0.24, 0.12, 0.10, 0.92)
                                }),
                                BorderColor::all(if can_afford {
                                    Color::srgb(0.96, 0.72, 0.24)
                                } else {
                                    Color::srgb(0.65, 0.36, 0.28)
                                }),
                            ))
                            .with_children(|b| {
                                b.spawn((
                                    Text::new(format!("Купить: {} сер.", preview_spec.credit_cost)),
                                    TextFont::from_font_size(11.5),
                                    TextColor(Color::srgb(1.0, 0.92, 0.72)),
                                ));
                            });
                        } else if preview_prereq_met {
                            let can_xp = prog.xp >= preview_spec.xp_cost as u64;
                            hero.spawn((
                                Interaction::default(),
                                HangarButton::ResearchTank(preview_global_idx),
                                Node {
                                    padding: UiRect::axes(Val::Px(11.0), Val::Px(4.0)),
                                    border: UiRect::all(Val::Px(1.5)),
                                    border_radius: BorderRadius::all(Val::Px(3.0)),
                                    ..default()
                                },
                                BackgroundColor(if can_xp {
                                    Color::srgb(0.12, 0.28, 0.44)
                                } else {
                                    Color::srgba(0.15, 0.12, 0.18, 0.90)
                                }),
                                BorderColor::all(if can_xp {
                                    Color::srgb(0.44, 0.80, 1.0)
                                } else {
                                    Color::srgb(0.46, 0.42, 0.56)
                                }),
                            ))
                            .with_children(|b| {
                                b.spawn((
                                    Text::new(format!("Исследовать: {} оп.", preview_spec.xp_cost)),
                                    TextFont::from_font_size(11.5),
                                    TextColor(Color::srgb(0.86, 0.96, 1.0)),
                                ));
                            });
                        } else {
                            let prereq_name = prerequisite_label_ru(preview_spec, all_tanks);
                            hero.spawn((
                                Node {
                                    padding: UiRect::axes(Val::Px(9.0), Val::Px(4.0)),
                                    border: UiRect::all(Val::Px(1.0)),
                                    border_radius: BorderRadius::all(Val::Px(3.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.10, 0.09, 0.12, 0.86)),
                                BorderColor::all(Color::srgba(0.40, 0.36, 0.48, 0.8)),
                            ))
                            .with_children(|b| {
                                b.spawn((
                                    Text::new(format!(
                                        "Осмотр (нужен {prereq_name} • {} оп.)",
                                        preview_spec.xp_cost
                                    )),
                                    TextFont::from_font_size(11.0),
                                    TextColor(Color::srgb(0.76, 0.78, 0.86)),
                                ));
                            });
                        }
                    });

                // Полоса итогов прошлого боя или системного уведомления о покупке/исследовании.
                if view.notice.is_some() || last_reward.active {
                    header
                        .spawn((
                            Node {
                                padding: UiRect::axes(Val::Px(14.0), Val::Px(3.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                border_radius: BorderRadius::all(Val::Px(3.0)),
                                column_gap: Val::Px(16.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.05, 0.075, 0.11, 0.88)),
                            BorderColor::all(Color::srgba(0.45, 0.56, 0.68, 0.72)),
                        ))
                        .with_children(|strip| {
                            if last_reward.active {
                                let outcome = if last_reward.victory {
                                    "Победа"
                                } else if last_reward.early_exit {
                                    "Досрочный выход"
                                } else {
                                    "Поражение"
                                };
                                let bonus = if last_reward.random_bonus {
                                    " +10% случ."
                                } else {
                                    ""
                                };
                                strip.spawn((
                                    Text::new(format!(
                                        "Прошлый бой ({outcome}{bonus}): +{} сер.  +{} оп.  •  Фраги: {}  Проб.: {}  Урон: {:.0}",
                                        last_reward.credits_earned,
                                        last_reward.xp_earned,
                                        last_reward.kills,
                                        last_reward.penetrations,
                                        last_reward.damage
                                    )),
                                    TextFont::from_font_size(11.0),
                                    TextColor(if last_reward.victory {
                                        Color::srgb(0.60, 0.95, 0.62)
                                    } else {
                                        Color::srgb(0.95, 0.84, 0.56)
                                    }),
                                ));
                            }
                            if let Some((ref msg, col)) = view.notice {
                                strip.spawn((
                                    Text::new(msg.clone()),
                                    TextFont::from_font_size(11.0),
                                    TextColor(col),
                                ));
                            }
                        });
                }
            });

            // 2. СРЕДНЯЯ ЗОНА: ЛЕВОЕ МЕНЮ (РЕЖИМЫ И КАРТЫ), СВОБОДНЫЙ ЦЕНТР С ТАНКОМ, ПРАВЫЕ КАРТОЧКИ
            root.spawn(Node {
                width: Val::Percent(100.0),
                flex_grow: 1.0,
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|mid| {
                // Левая панель WoT: режимы, инспекция и список локаций.
                mid.spawn((
                    Node {
                        width: Val::Px(232.0),
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::all(Val::Px(9.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(4.0)),
                        row_gap: Val::Px(5.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.035, 0.05, 0.068, 0.82)),
                    BorderColor::all(Color::srgba(0.30, 0.36, 0.44, 0.65)),
                ))
                .with_children(|left| {
                    left.spawn((
                        Text::new("РЕЖИМЫ И ИНСПЕКЦИЯ"),
                        TextFont::from_font_size(10.5),
                        TextColor(Color::srgb(0.64, 0.72, 0.80)),
                    ));

                    // Кнопка переключения режима WoT / War Thunder.
                    left.spawn((
                        Interaction::default(),
                        HangarButton::ToggleMode,
                        Node {
                            padding: UiRect::axes(Val::Px(9.0), Val::Px(6.0)),
                            border: UiRect::all(Val::Px(1.5)),
                            border_radius: BorderRadius::all(Val::Px(3.0)),
                            ..default()
                        },
                        BackgroundColor(if wt_mode {
                            Color::srgb(0.42, 0.22, 0.08)
                        } else {
                            Color::srgb(0.18, 0.32, 0.16)
                        }),
                        BorderColor::all(if wt_mode {
                            Color::srgb(0.95, 0.60, 0.20)
                        } else {
                            Color::srgb(0.52, 0.82, 0.40)
                        }),
                    ))
                    .with_children(|b| {
                        b.spawn((
                            HangarLabel::ToggleMode,
                            Text::new(format!("Режим [M]: {}", mode.ru_label())),
                            TextFont::from_font_size(12.0),
                            TextColor(Color::WHITE),
                        ));
                    });

                    // Кнопка X-Ray брони и экипажа.
                    left.spawn((
                        Interaction::default(),
                        HangarButton::ToggleXray,
                        Node {
                            padding: UiRect::axes(Val::Px(9.0), Val::Px(5.0)),
                            border: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(3.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.11, 0.17, 0.25)),
                        BorderColor::all(Color::srgb(0.42, 0.62, 0.82)),
                    ))
                    .with_children(|b| {
                        b.spawn((
                            HangarLabel::ToggleXray,
                            Text::new(if view.xray_armor {
                                "Броня/экипаж [V]: ВКЛ"
                            } else {
                                "Броня/экипаж [V]"
                            }),
                            TextFont::from_font_size(12.0),
                            TextColor(Color::srgb(0.86, 0.94, 1.0)),
                        ));
                    });

                    // Кнопка автовращения танка на стенде.
                    left.spawn((
                        Interaction::default(),
                        HangarButton::ToggleSpin,
                        Node {
                            padding: UiRect::axes(Val::Px(9.0), Val::Px(4.0)),
                            border: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(3.0)),
                            ..default()
                        },
                        BackgroundColor(if view.auto_spin {
                            Color::srgb(0.18, 0.28, 0.36)
                        } else {
                            Color::srgba(0.08, 0.11, 0.15, 0.85)
                        }),
                        BorderColor::all(Color::srgb(0.32, 0.42, 0.52)),
                    ))
                    .with_children(|b| {
                        b.spawn((
                            HangarLabel::ToggleSpin,
                            Text::new(if view.auto_spin {
                                "Поворот [O]: ВКЛ"
                            } else {
                                "Поворот [O]: ВЫКЛ"
                            }),
                            TextFont::from_font_size(11.5),
                            TextColor(Color::srgb(0.80, 0.87, 0.94)),
                        ));
                    });

                    left.spawn((
                        Node {
                            margin: UiRect::top(Val::Px(3.0)),
                            ..default()
                        },
                        Text::new("КАРТА БОЯ [Q / E]"),
                        TextFont::from_font_size(10.5),
                        TextColor(Color::srgb(0.64, 0.72, 0.80)),
                    ));

                    let is_random = selected_map.0 == RANDOM_MAP_ID;
                    left.spawn((
                        Interaction::default(),
                        HangarButton::SelectRandomMap,
                        Node {
                            padding: UiRect::axes(Val::Px(9.0), Val::Px(4.0)),
                            border: UiRect {
                                left: Val::Px(if is_random { 3.0 } else { 1.0 }),
                                top: Val::Px(1.0),
                                right: Val::Px(1.0),
                                bottom: Val::Px(1.0),
                            },
                            border_radius: BorderRadius::all(Val::Px(3.0)),
                            flex_direction: FlexDirection::Column,
                            ..default()
                        },
                        BackgroundColor(if is_random {
                            Color::srgba(0.22, 0.18, 0.08, 0.94)
                        } else {
                            Color::srgba(0.06, 0.08, 0.11, 0.78)
                        }),
                        BorderColor::all(if is_random {
                            Color::srgb(0.95, 0.76, 0.36)
                        } else {
                            Color::srgba(0.24, 0.30, 0.36, 0.75)
                        }),
                    ))
                    .with_children(|b| {
                        b.spawn((
                            Text::new("Случайная карта"),
                            TextFont::from_font_size(12.0),
                            TextColor(Color::srgb(0.98, 0.88, 0.56)),
                        ));
                        b.spawn((
                            Text::new("Автовыбор (+10% награда)"),
                            TextFont::from_font_size(10.0),
                            TextColor(Color::srgb(0.76, 0.74, 0.62)),
                        ));
                    });

                    for (i, m) in maps.iter().enumerate() {
                        let active = m.id == selected_map.0;
                        left.spawn((
                            Interaction::default(),
                            HangarButton::SelectMap(i),
                            Node {
                                padding: UiRect::axes(Val::Px(9.0), Val::Px(4.0)),
                                border: UiRect {
                                    left: Val::Px(if active { 3.0 } else { 1.0 }),
                                    top: Val::Px(1.0),
                                    right: Val::Px(1.0),
                                    bottom: Val::Px(1.0),
                                },
                                border_radius: BorderRadius::all(Val::Px(3.0)),
                                flex_direction: FlexDirection::Column,
                                ..default()
                            },
                            BackgroundColor(if active {
                                Color::srgba(0.14, 0.24, 0.34, 0.94)
                            } else {
                                Color::srgba(0.06, 0.08, 0.11, 0.78)
                            }),
                            BorderColor::all(if active {
                                biome_accent(m.biome)
                            } else {
                                Color::srgba(0.24, 0.30, 0.36, 0.75)
                            }),
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new(m.name.clone()),
                                TextFont::from_font_size(12.0),
                                TextColor(Color::srgb(0.92, 0.95, 0.98)),
                            ));
                            b.spawn((
                                Text::new(biome_short_ru(m.biome)),
                                TextFont::from_font_size(10.0),
                                TextColor(Color::srgb(0.64, 0.74, 0.84)),
                            ));
                        });
                    }
                });

                // Правая панель WoT: карточка карты/составов 5v5 + карточка ТТХ со шкалами.
                mid.spawn(Node {
                    width: Val::Px(308.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    ..default()
                })
                .with_children(|right| {
                    right
                        .spawn((
                            Node {
                                padding: UiRect::all(Val::Px(9.0)),
                                border: UiRect {
                                    left: Val::Px(3.0),
                                    top: Val::Px(1.0),
                                    right: Val::Px(1.0),
                                    bottom: Val::Px(1.0),
                                },
                                border_radius: BorderRadius::all(Val::Px(4.0)),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(4.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.04, 0.055, 0.075, 0.86)),
                            BorderColor::all(brief_accent),
                        ))
                        .with_children(|mc| {
                            mc.spawn((
                                HangarLabel::MapBriefTitle,
                                Text::new(brief_title),
                                TextFont::from_font_size(13.0),
                                TextColor(Color::srgb(0.96, 0.90, 0.66)),
                            ));
                            mc.spawn((
                                HangarLabel::MapBriefBody,
                                Text::new(brief_body),
                                TextFont::from_font_size(10.5),
                                TextColor(Color::srgb(0.78, 0.84, 0.90)),
                            ));
                        });

                    right
                        .spawn((
                            Node {
                                padding: UiRect::all(Val::Px(9.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                border_radius: BorderRadius::all(Val::Px(4.0)),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(5.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.04, 0.055, 0.075, 0.86)),
                            BorderColor::all(Color::srgba(0.32, 0.38, 0.46, 0.72)),
                        ))
                        .with_children(|ttx| {
                            ttx.spawn((
                                Text::new("ХАРАКТЕРИСТИКИ МАШИНЫ"),
                                TextFont::from_font_size(11.0),
                                TextColor(Color::srgb(0.95, 0.76, 0.36)),
                            ));

                            for (title, val, tag, col) in [
                                (
                                    "Огневая мощь",
                                    ratings.firepower,
                                    HangarBar::Firepower,
                                    Color::srgb(0.94, 0.48, 0.22),
                                ),
                                (
                                    "Живучесть",
                                    ratings.survivability,
                                    HangarBar::Survivability,
                                    Color::srgb(0.36, 0.82, 0.42),
                                ),
                                (
                                    "Бронирование",
                                    ratings.armor,
                                    HangarBar::Armor,
                                    Color::srgb(0.90, 0.76, 0.28),
                                ),
                                (
                                    "Подвижность",
                                    ratings.mobility,
                                    HangarBar::Mobility,
                                    Color::srgb(0.38, 0.76, 0.96),
                                ),
                            ] {
                                ttx.spawn(Node {
                                    flex_direction: FlexDirection::Column,
                                    row_gap: Val::Px(2.0),
                                    ..default()
                                })
                                .with_children(|row| {
                                    row.spawn((
                                        Text::new(title),
                                        TextFont::from_font_size(10.5),
                                        TextColor(Color::srgb(0.76, 0.82, 0.88)),
                                    ));
                                    row.spawn((
                                        Node {
                                            width: Val::Percent(100.0),
                                            height: Val::Px(5.5),
                                            border_radius: BorderRadius::all(Val::Px(2.0)),
                                            ..default()
                                        },
                                        BackgroundColor(Color::srgba(0.10, 0.13, 0.17, 0.95)),
                                    ))
                                    .with_children(|track| {
                                        track.spawn((
                                            tag,
                                            Node {
                                                width: Val::Percent(val * 100.0),
                                                height: Val::Percent(100.0),
                                                border_radius: BorderRadius::all(Val::Px(2.0)),
                                                ..default()
                                            },
                                            BackgroundColor(col),
                                        ));
                                    });
                                });
                            }

                            ttx.spawn((
                                HangarLabel::Info,
                                Text::new(format_hangar_ttx_compact(preview_spec)),
                                TextFont::from_font_size(10.5),
                                TextColor(Color::srgb(0.80, 0.86, 0.92)),
                            ));
                        });
                });
            });

            // 3. НИЖНИЙ БЛОК: ЭКИПАЖ/БК + ФИЛЬТРЫ НАЦИЙ + КАРУСЕЛЬ ТЕХНИКИ + ПОДСКАЗКИ
            root.spawn(Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(5.0),
                ..default()
            })
            .with_children(|bottom| {
                // Полоса экипажа и боекомплекта.
                bottom
                    .spawn((
                        Node {
                            padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                            border: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(4.0)),
                            column_gap: Val::Px(8.0),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.04, 0.055, 0.075, 0.88)),
                        BorderColor::all(Color::srgba(0.34, 0.40, 0.48, 0.7)),
                    ))
                    .with_children(|strip| {
                        for role in [
                            CrewRole::Commander,
                            CrewRole::Gunner,
                            CrewRole::Driver,
                            CrewRole::Loader,
                        ] {
                            strip
                                .spawn((
                                    Node {
                                        padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                                        border: UiRect::all(Val::Px(1.0)),
                                        border_radius: BorderRadius::all(Val::Px(3.0)),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgba(0.11, 0.14, 0.16, 0.92)),
                                    BorderColor::all(Color::srgb(0.42, 0.48, 0.34)),
                                ))
                                .with_children(|slot| {
                                    slot.spawn((
                                        Text::new(role.ru_short()),
                                        TextFont::from_font_size(10.5),
                                        TextColor(Color::srgb(0.88, 0.92, 0.80)),
                                    ));
                                });
                        }

                        strip.spawn((
                            Text::new("│"),
                            TextFont::from_font_size(13.0),
                            TextColor(Color::srgba(0.40, 0.46, 0.54, 0.8)),
                        ));

                        strip
                            .spawn((
                                Node {
                                    padding: UiRect::axes(Val::Px(8.0), Val::Px(2.0)),
                                    border: UiRect::all(Val::Px(1.5)),
                                    border_radius: BorderRadius::all(Val::Px(3.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.28, 0.18, 0.08, 0.94)),
                                BorderColor::all(Color::srgb(0.96, 0.64, 0.24)),
                            ))
                            .with_children(|slot| {
                                slot.spawn((
                                    HangarLabel::ShellSlot,
                                    Text::new(format!(
                                        "[1] {:.0}мм SolidAP",
                                        preview_spec.shell.caliber_mm
                                    )),
                                    TextFont::from_font_size(11.0),
                                    TextColor(Color::srgb(1.0, 0.90, 0.68)),
                                ));
                            });

                        for label in ["[5] Ремк. ×2", "[6] Огнет. ×2"] {
                            strip
                                .spawn((
                                    Node {
                                        padding: UiRect::axes(Val::Px(7.0), Val::Px(2.0)),
                                        border: UiRect::all(Val::Px(1.0)),
                                        border_radius: BorderRadius::all(Val::Px(3.0)),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgba(0.08, 0.13, 0.18, 0.92)),
                                    BorderColor::all(Color::srgb(0.32, 0.50, 0.66)),
                                ))
                                .with_children(|slot| {
                                    slot.spawn((
                                        Text::new(label),
                                        TextFont::from_font_size(10.5),
                                        TextColor(Color::srgb(0.82, 0.92, 0.98)),
                                    ));
                                });
                        }
                    });

                // Фильтры карусели: [В ангаре (N)] | [СССР] | [Германия] | [США] | [T Дерево].
                bottom
                    .spawn(Node {
                        column_gap: Val::Px(6.0),
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|tabs| {
                        filter_tab_button(
                            tabs,
                            HangarFilter::AllOwned,
                            format!("В ангаре ({owned_count})"),
                            view.filter_tab == HangarFilter::AllOwned,
                            Color::srgb(0.45, 0.88, 0.48),
                        );
                        for nation in Nation::all() {
                            filter_tab_button(
                                tabs,
                                HangarFilter::Nation(nation),
                                format!("{} (I–V)", nation.ru_name()),
                                view.filter_tab == HangarFilter::Nation(nation),
                                nation.accent(),
                            );
                        }
                        tabs.spawn((
                            Interaction::default(),
                            HangarButton::ToggleTechTree,
                            Node {
                                padding: UiRect::axes(Val::Px(11.0), Val::Px(4.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                border_radius: BorderRadius::all(Val::Px(3.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.14, 0.22, 0.34, 0.92)),
                            BorderColor::all(Color::srgb(0.50, 0.76, 0.98)),
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new("[T] Дерево исследований"),
                                TextFont::from_font_size(11.5),
                                TextColor(Color::srgb(0.88, 0.95, 1.0)),
                            ));
                        });
                    });

                // Нижняя карусель танков выбранного фильтра.
                let compact = visible_indices.len() > 6;
                bottom
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            padding: UiRect::all(Val::Px(5.0)),
                            border: UiRect::top(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(4.0)),
                            column_gap: Val::Px(6.0),
                            row_gap: Val::Px(5.0),
                            flex_wrap: FlexWrap::Wrap,
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.03, 0.042, 0.056, 0.92)),
                        BorderColor::all(Color::srgba(0.32, 0.38, 0.46, 0.72)),
                    ))
                    .with_children(|carousel| {
                        for (slot_i, &g_idx) in visible_indices.iter().enumerate() {
                            let t = &all_tanks[g_idx];
                            let tm = tank_wot_meta(t);
                            let owned = is_owned(prog, &t.id);
                            let researched = is_researched(prog, &t.id);
                            let is_combat = t.id == combat_spec.id;
                            let is_preview = t.id == preview_spec.id;

                            let status_badge = if is_combat {
                                "В БОЮ"
                            } else if is_preview {
                                "ОСМОТР"
                            } else if owned {
                                tm.nation_short
                            } else if researched {
                                "ОТКРЫТ"
                            } else {
                                "ЗАКРЫТ"
                            };

                            let card_bg = if is_combat && is_preview {
                                Color::srgba(0.20, 0.24, 0.14, 0.96)
                            } else if is_preview {
                                Color::srgba(0.14, 0.20, 0.28, 0.96)
                            } else if owned {
                                Color::srgba(0.07, 0.10, 0.12, 0.92)
                            } else if researched {
                                Color::srgba(0.13, 0.12, 0.08, 0.90)
                            } else {
                                Color::srgba(0.045, 0.055, 0.075, 0.85)
                            };

                            let card_border = if is_preview {
                                Color::srgb(0.96, 0.64, 0.20)
                            } else if is_combat {
                                Color::srgb(0.42, 0.86, 0.44)
                            } else if owned {
                                tm.accent
                            } else if researched {
                                Color::srgb(0.76, 0.62, 0.26)
                            } else {
                                Color::srgba(0.22, 0.26, 0.34, 0.8)
                            };

                            let sub_line = if owned {
                                format!(
                                    "{:.0}мм • {:.0} HP • {:.0} км/ч",
                                    t.shell.caliber_mm, t.max_hp, t.max_speed_kmh
                                )
                            } else if researched {
                                format!("Купить: {} сер.", t.credit_cost)
                            } else {
                                format!("{} оп. / {} сер.", t.xp_cost, t.credit_cost)
                            };

                            carousel
                                .spawn((
                                    Interaction::default(),
                                    HangarButton::SelectTank(g_idx),
                                    Node {
                                        flex_grow: if compact { 0.0 } else { 1.0 },
                                        min_width: Val::Px(if compact { 132.0 } else { 148.0 }),
                                        max_width: Val::Px(if compact { 166.0 } else { 235.0 }),
                                        padding: UiRect::axes(
                                            Val::Px(9.0),
                                            Val::Px(if compact { 4.0 } else { 6.0 }),
                                        ),
                                        border: UiRect {
                                            top: Val::Px(3.0),
                                            left: Val::Px(if is_preview { 2.0 } else { 1.0 }),
                                            right: Val::Px(if is_preview { 2.0 } else { 1.0 }),
                                            bottom: Val::Px(if is_preview { 2.0 } else { 1.0 }),
                                        },
                                        border_radius: BorderRadius::all(Val::Px(4.0)),
                                        flex_direction: FlexDirection::Column,
                                        row_gap: Val::Px(2.0),
                                        ..default()
                                    },
                                    BackgroundColor(card_bg),
                                    BorderColor::all(card_border),
                                ))
                                .with_children(|card| {
                                    card.spawn(Node {
                                        width: Val::Percent(100.0),
                                        justify_content: JustifyContent::SpaceBetween,
                                        ..default()
                                    })
                                    .with_children(|top_row| {
                                        let key_str = if slot_i < 9 {
                                            format!("[{}] ", slot_i + 1)
                                        } else {
                                            String::new()
                                        };
                                        top_row.spawn((
                                            Text::new(format!(
                                                "{key_str}{} {} {}",
                                                tm.tier, tm.class_icon, tm.class_short
                                            )),
                                            TextFont::from_font_size(10.5),
                                            TextColor(Color::srgb(0.96, 0.80, 0.42)),
                                        ));
                                        top_row.spawn((
                                            Text::new(status_badge),
                                            TextFont::from_font_size(10.0),
                                            TextColor(if is_combat {
                                                Color::srgb(0.50, 0.94, 0.56)
                                            } else if researched && !owned {
                                                Color::srgb(0.96, 0.80, 0.36)
                                            } else if !owned {
                                                Color::srgb(0.58, 0.64, 0.72)
                                            } else {
                                                Color::srgb(0.70, 0.78, 0.86)
                                            }),
                                        ));
                                    });
                                    card.spawn((
                                        Text::new(t.name.clone()),
                                        TextFont::from_font_size(if compact { 12.0 } else { 13.0 }),
                                        TextColor(if owned || researched {
                                            Color::srgb(0.96, 0.97, 0.99)
                                        } else {
                                            Color::srgb(0.64, 0.70, 0.78)
                                        }),
                                    ));
                                    card.spawn((
                                        Text::new(sub_line),
                                        TextFont::from_font_size(10.0),
                                        TextColor(if owned {
                                            Color::srgb(0.72, 0.78, 0.86)
                                        } else if researched {
                                            Color::srgb(0.95, 0.82, 0.45)
                                        } else {
                                            Color::srgb(0.54, 0.60, 0.68)
                                        }),
                                    ));
                                });
                        }
                    });

                // Сервисная строка управления в самом низу.
                bottom.spawn((
                    Text::new(
                        "ЛКМ/ПКМ: 360° осмотр  •  Колёсико: зум  •  1–9 / ←→: танк  •  Q / E: карта  •  T: дерево  •  M: режим  •  V: рентген  •  O: поворот  •  N: звук  •  Enter: в бой",
                    ),
                    TextFont::from_font_size(10.5),
                    TextColor(Color::srgb(0.62, 0.68, 0.76)),
                ));
            });

            // 4. МОДАЛЬНОЕ ОКНО ДЕРЕВА ИССЛЕДОВАНИЙ `[T]`
            if view.show_tech_tree {
                spawn_tech_tree_modal(root, all_tanks, prog, selected_tank, view);
            }
        });
}

#[allow(clippy::too_many_arguments)]
pub fn setup_hangar(
    mut commands: Commands,
    mut selected_tank: ResMut<SelectedTank>,
    selected_map: Res<SelectedMap>,
    mode: Res<DamageRulesMode>,
    mut view: ResMut<HangarViewState>,
    mut prog_opt: Option<ResMut<PlayerProgression>>,
    last_reward_opt: Option<Res<LastBattleReward>>,
    audio: Option<Res<AudioSettings>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    assets: Res<VoxelAssets>,
) {
    commands.insert_resource(ClearColor(Color::srgb(0.055, 0.072, 0.096)));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.58, 0.72, 0.90),
        brightness: 320.0,
        ..default()
    });

    if let Some(ref mut prog) = prog_opt {
        ensure_starters(prog);
        if !is_owned(prog, &selected_tank.0.0) {
            selected_tank.0 = TankId("ms_1".into());
        }
    }
    let default_prog = PlayerProgression::default();
    let prog = prog_opt.as_deref().unwrap_or(&default_prog);
    let default_last = LastBattleReward::default();
    let last_reward = last_reward_opt.as_deref().unwrap_or(&default_last);

    let combat_spec = owned_combat_spec(&selected_tank, prog);
    selected_tank.0 = TankId(combat_spec.id.clone());

    view.show_tech_tree = false;
    view.notice = None;
    view.prev_pressed_btn = None;
    view.suppress_click_frames = 0;
    if view.preview_tank_id.is_empty() || load_tank_spec(&view.preview_tank_id).is_none() {
        view.preview_tank_id = combat_spec.id.clone();
    }
    let preview_spec = load_tank_spec(&view.preview_tank_id).unwrap_or(combat_spec);
    view.spawned_preview_id = preview_spec.id.clone();

    spawn_hangar_environment(&mut commands, &mut meshes, &mut materials, &assets);

    let preview = spawn_preview_tank(&mut commands, &mut meshes, &assets, &preview_spec);
    if view.xray_armor {
        commands.entity(preview).insert(Visibility::Hidden);
    }
    let xray_root = spawn_xray_overlay(
        &mut commands,
        &mut meshes,
        &mut materials,
        &preview_spec,
        view.xray_armor,
    );

    // Стенд вращает саму машину над ямой при включённом автоповороте.
    commands
        .spawn((
            Name::new("HangarTurntable"),
            HangarTurntable,
            Transform::IDENTITY,
            Visibility::default(),
            DespawnOnExit(GameState::Hangar),
        ))
        .add_children(&[preview, xray_root]);

    let cam_pos = hangar_camera_pos(&view);
    commands.spawn((
        HangarCamera,
        Camera3d::default(),
        DistanceFog {
            color: Color::srgba(0.065, 0.088, 0.116, 0.96),
            directional_light_color: Color::NONE,
            directional_light_exponent: 8.0,
            falloff: FogFalloff::Linear {
                start: 10.5,
                end: 42.0,
            },
        },
        Transform::from_translation(cam_pos).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
        DespawnOnExit(GameState::Hangar),
    ));

    let all_tanks = available_tanks();
    let maps = available_maps();
    let muted = audio.as_ref().is_some_and(|a| a.muted);

    spawn_hangar_ui(
        &mut commands,
        &all_tanks,
        &maps,
        &selected_tank,
        &selected_map,
        *mode,
        &view,
        prog,
        last_reward,
        &preview_spec,
        muted,
    );
}

fn hangar_camera_pos(view: &HangarViewState) -> Vec3 {
    let focus = Vec3::new(0.0, 1.0, 0.0);
    let r = view.distance;
    let cp = view.pitch.cos();
    focus
        + Vec3::new(
            r * cp * view.yaw.sin(),
            r * view.pitch.sin(),
            r * cp * view.yaw.cos(),
        )
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn update_hangar(
    mut commands: Commands,
    time: Res<Time>,
    input: (
        Res<ButtonInput<KeyCode>>,
        Res<ButtonInput<MouseButton>>,
        Res<AccumulatedMouseMotion>,
        Res<AccumulatedMouseScroll>,
    ),
    mut next_state: ResMut<NextState<GameState>>,
    mut selected_tank: ResMut<SelectedTank>,
    mut selected_map: ResMut<SelectedMap>,
    mut mode: ResMut<DamageRulesMode>,
    mut view: ResMut<HangarViewState>,
    mut prog_pair: (
        Option<ResMut<PlayerProgression>>,
        Option<Res<LastBattleReward>>,
    ),
    mut audio_pair: (
        Option<ResMut<AudioSettings>>,
        Option<ResMut<Messages<UiSoundMessage>>>,
    ),
    mut assets_pair: (
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
        Res<VoxelAssets>,
    ),
    mut scene_q: ParamSet<(
        Query<(Entity, &mut Transform), With<HangarTurntable>>,
        Query<(Entity, &mut Visibility), With<HangarPreviewTank>>,
        Query<(Entity, &mut Visibility), With<HangarXrayRoot>>,
        Query<&mut Transform, With<HangarCamera>>,
    )>,
    ui_roots: Query<Entity, With<HangarUiRoot>>,
    mut labels: Query<(&HangarLabel, &mut Text)>,
    mut bars: Query<(&HangarBar, &mut Node)>,
    mut btns: Query<(
        &HangarButton,
        &Interaction,
        &mut BackgroundColor,
        &mut BorderColor,
    )>,
) {
    let (keys, mouse_btn, motion, scroll) = input;
    let all_tanks = available_tanks();
    let maps = available_maps();
    let total_map_choices = maps.len() + 1;

    let default_prog = PlayerProgression::default();
    let (prog_snapshot, mut ui_dirty) = {
        let snap = prog_pair.0.as_deref().unwrap_or(&default_prog).clone();
        (snap, false)
    };

    if let Some(ref mut prog) = prog_pair.0 {
        ensure_starters(prog);
        if !is_owned(prog, &selected_tank.0.0) {
            selected_tank.0 = TankId("ms_1".into());
            ui_dirty = true;
        }
    }

    let visible_indices = filtered_carousel_indices(&all_tanks, &prog_snapshot, view.filter_tab);
    let cur_vis_pos = visible_indices
        .iter()
        .position(|&gi| all_tanks[gi].id == view.preview_tank_id)
        .unwrap_or(0);

    let mut new_global_idx: Option<usize> = None;
    let cur_map_choice = if selected_map.0 == RANDOM_MAP_ID {
        0
    } else {
        maps.iter()
            .position(|m| m.id == selected_map.0)
            .map_or(1, |i| i + 1)
    };
    let mut new_map_choice: Option<usize> = None;

    if !view.show_tech_tree {
        let digit_keys = [
            KeyCode::Digit1,
            KeyCode::Digit2,
            KeyCode::Digit3,
            KeyCode::Digit4,
            KeyCode::Digit5,
            KeyCode::Digit6,
            KeyCode::Digit7,
            KeyCode::Digit8,
            KeyCode::Digit9,
        ];
        for (i, &k) in digit_keys.iter().enumerate() {
            if keys.just_pressed(k) && i < visible_indices.len() {
                new_global_idx = Some(visible_indices[i]);
            }
        }
        if keys.just_pressed(KeyCode::ArrowRight) && !visible_indices.is_empty() {
            let next_pos = (cur_vis_pos + 1) % visible_indices.len();
            new_global_idx = Some(visible_indices[next_pos]);
        }
        if keys.just_pressed(KeyCode::ArrowLeft) && !visible_indices.is_empty() {
            let prev_pos = (cur_vis_pos + visible_indices.len() - 1) % visible_indices.len();
            new_global_idx = Some(visible_indices[prev_pos]);
        }
        if keys.just_pressed(KeyCode::KeyE) && total_map_choices > 1 {
            new_map_choice = Some((cur_map_choice + 1) % total_map_choices);
        }
        if keys.just_pressed(KeyCode::KeyQ) && total_map_choices > 1 {
            new_map_choice = Some((cur_map_choice + total_map_choices - 1) % total_map_choices);
        }
    }

    let mut ui_hovered = false;
    let mut toggle_xray = keys.just_pressed(KeyCode::KeyV);
    let mut toggle_mode = keys.just_pressed(KeyCode::KeyM);
    let mut toggle_spin = keys.just_pressed(KeyCode::KeyO);
    let mut toggle_tree = keys.just_pressed(KeyCode::KeyT)
        || (view.show_tech_tree && keys.just_pressed(KeyCode::Escape));
    let mut toggle_audio = false;
    let mut xray_pressed_now = false;
    let mut mode_pressed_now = false;
    let mut spin_pressed_now = false;
    let mut audio_pressed_now = false;
    let mut pressed_btn_now: Option<HangarButton> = None;
    let mut start = !view.show_tech_tree
        && (keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space));

    for (btn, interaction, _, _) in &btns {
        if *interaction != Interaction::None {
            ui_hovered = true;
        }
        if *interaction == Interaction::Pressed {
            if *btn != HangarButton::Noop || pressed_btn_now.is_none() {
                pressed_btn_now = Some(*btn);
            }
            match *btn {
                HangarButton::ToggleMode => mode_pressed_now = true,
                HangarButton::ToggleXray => xray_pressed_now = true,
                HangarButton::ToggleSpin => spin_pressed_now = true,
                HangarButton::ToggleAudio => audio_pressed_now = true,
                HangarButton::StartBattle if view.suppress_click_frames == 0 => start = true,
                _ => {}
            }
        }
    }

    let suppress_now = view.suppress_click_frames > 0;
    if view.suppress_click_frames > 0 {
        view.suppress_click_frames -= 1;
    }

    if !suppress_now {
        if xray_pressed_now && !view.prev_xray_btn {
            toggle_xray = true;
        }
        if mode_pressed_now && !view.prev_mode_btn {
            toggle_mode = true;
        }
        if spin_pressed_now && !view.prev_spin_btn {
            toggle_spin = true;
        }
        if audio_pressed_now && !view.prev_audio_btn {
            toggle_audio = true;
        }
    }
    view.prev_xray_btn = xray_pressed_now;
    view.prev_mode_btn = mode_pressed_now;
    view.prev_spin_btn = spin_pressed_now;
    view.prev_audio_btn = audio_pressed_now;

    let just_pressed_btn = if suppress_now {
        None
    } else {
        pressed_btn_now.filter(|b| *b != HangarButton::Noop && view.prev_pressed_btn != Some(*b))
    };
    view.prev_pressed_btn = pressed_btn_now;

    let mut clicked = false;
    if let Some(btn) = just_pressed_btn {
        match btn {
            HangarButton::SelectTank(g_idx) if g_idx < all_tanks.len() => {
                new_global_idx = Some(g_idx);
            }
            HangarButton::SelectRandomMap => new_map_choice = Some(0),
            HangarButton::SelectMap(i) if i < maps.len() => new_map_choice = Some(i + 1),
            HangarButton::ToggleTechTree => toggle_tree = true,
            HangarButton::SelectFilter(next_filter) => {
                if view.filter_tab != next_filter {
                    view.filter_tab = next_filter;
                    match next_filter {
                        HangarFilter::AllOwned => {
                            if !is_owned(&prog_snapshot, &view.preview_tank_id) {
                                view.preview_tank_id = selected_tank.0.0.clone();
                            }
                        }
                        HangarFilter::Nation(n) => {
                            let same_nation = load_tank_spec(&view.preview_tank_id)
                                .is_some_and(|s| s.nation_id == n);
                            if !same_nation
                                && let Some(best) = all_tanks
                                    .iter()
                                    .filter(|t| t.nation_id == n && is_owned(&prog_snapshot, &t.id))
                                    .max_by_key(|t| t.tier)
                                    .or_else(|| all_tanks.iter().find(|t| t.nation_id == n))
                            {
                                view.preview_tank_id = best.id.clone();
                                if is_owned(&prog_snapshot, &best.id) {
                                    selected_tank.0 = TankId(best.id.clone());
                                }
                            }
                        }
                    }
                    view.notice = None;
                    ui_dirty = true;
                    clicked = true;
                }
            }
            HangarButton::InspectTank(g_idx) if g_idx < all_tanks.len() => {
                let target = &all_tanks[g_idx];
                view.preview_tank_id = target.id.clone();
                if is_owned(&prog_snapshot, &target.id) {
                    selected_tank.0 = TankId(target.id.clone());
                }
                view.filter_tab = HangarFilter::Nation(target.nation_id);
                view.show_tech_tree = false;
                view.notice = None;
                ui_dirty = true;
                clicked = true;
            }
            HangarButton::SelectOwnedAndClose(g_idx) if g_idx < all_tanks.len() => {
                let target = &all_tanks[g_idx];
                view.preview_tank_id = target.id.clone();
                if is_owned(&prog_snapshot, &target.id) {
                    selected_tank.0 = TankId(target.id.clone());
                }
                view.show_tech_tree = false;
                view.notice = None;
                ui_dirty = true;
                clicked = true;
            }
            HangarButton::ResearchTank(g_idx) if g_idx < all_tanks.len() => {
                let target = &all_tanks[g_idx];
                if let Some(ref mut prog) = prog_pair.0 {
                    match research_tank(prog, target) {
                        Ok(()) => {
                            view.preview_tank_id = target.id.clone();
                            view.notice = Some((
                                format!("Исследован {}! Доступно {} оп.", target.name, prog.xp),
                                Color::srgb(0.44, 0.88, 1.0),
                            ));
                        }
                        Err(err) => {
                            view.notice = Some((
                                format!("{}: {err}", target.name),
                                Color::srgb(0.98, 0.45, 0.36),
                            ));
                        }
                    }
                    ui_dirty = true;
                    view.suppress_click_frames = 2;
                    clicked = true;
                }
            }
            HangarButton::BuyTank(g_idx) if g_idx < all_tanks.len() => {
                let target = &all_tanks[g_idx];
                if let Some(ref mut prog) = prog_pair.0 {
                    match buy_tank(prog, target) {
                        Ok(()) => {
                            view.preview_tank_id = target.id.clone();
                            selected_tank.0 = TankId(target.id.clone());
                            view.notice = Some((
                                format!("Куплен {} и выбран для боя!", target.name),
                                Color::srgb(0.45, 0.94, 0.52),
                            ));
                        }
                        Err(err) => {
                            view.notice = Some((
                                format!("{}: {err}", target.name),
                                Color::srgb(0.98, 0.45, 0.36),
                            ));
                        }
                    }
                    ui_dirty = true;
                    view.suppress_click_frames = 2;
                    clicked = true;
                }
            }
            _ => {}
        }
    }

    if let Some(g_idx) = new_global_idx
        && g_idx < all_tanks.len()
    {
        let target = &all_tanks[g_idx];
        let latest_prog = prog_pair.0.as_deref().unwrap_or(&default_prog);
        let owned = is_owned(latest_prog, &target.id);
        let changed_preview = view.preview_tank_id != target.id;
        let changed_combat = owned && selected_tank.0.0 != target.id;
        if changed_preview || changed_combat {
            view.preview_tank_id = target.id.clone();
            if owned {
                selected_tank.0 = TankId(target.id.clone());
            }
            view.notice = None;
            ui_dirty = true;
            clicked = true;
        }
    }

    if let Some(choice) = new_map_choice {
        let next_map_id = if choice == 0 {
            RANDOM_MAP_ID.to_string()
        } else {
            maps[choice - 1].id.clone()
        };
        if next_map_id != selected_map.0 {
            selected_map.0 = next_map_id;
            clicked = true;
        }
    }
    if toggle_tree {
        view.show_tech_tree = !view.show_tech_tree;
        ui_dirty = true;
        view.suppress_click_frames = 2;
        clicked = true;
    }
    if toggle_mode {
        *mode = match *mode {
            DamageRulesMode::ArcadeHpAndModules => DamageRulesMode::ModulesOnlyWt,
            DamageRulesMode::ModulesOnlyWt | DamageRulesMode::PrototypeTargets => {
                DamageRulesMode::ArcadeHpAndModules
            }
        };
        clicked = true;
    }
    if toggle_xray {
        view.xray_armor = !view.xray_armor;
        clicked = true;
    }
    if toggle_spin {
        view.auto_spin = !view.auto_spin;
        clicked = true;
    }
    if toggle_audio && let Some(ref mut a) = audio_pair.0 {
        a.muted = !a.muted;
        if !a.muted {
            clicked = true;
        }
    }

    if start {
        let latest_prog = prog_pair.0.as_deref().unwrap_or(&default_prog);
        if !is_owned(latest_prog, &selected_tank.0.0) {
            selected_tank.0 = TankId("ms_1".into());
        }
        if let Some(ref mut msgs) = audio_pair.1 {
            msgs.write(UiSoundMessage(SoundPreset::BattleStart));
        }
        next_state.set(GameState::InBattle);
        return;
    }
    if clicked && let Some(ref mut msgs) = audio_pair.1 {
        msgs.write(UiSoundMessage(SoundPreset::UiClick));
    }

    // Орбита камеры мышью (отключена поверх UI и внутри окна дерева исследований).
    let dragging = !view.show_tech_tree
        && (mouse_btn.pressed(MouseButton::Right)
            || (mouse_btn.pressed(MouseButton::Left) && !ui_hovered));
    if dragging && motion.delta != Vec2::ZERO {
        view.yaw -= motion.delta.x * 0.008;
        view.pitch = (view.pitch + motion.delta.y * 0.007).clamp(0.06, 1.12);
    }
    if !view.show_tech_tree && scroll.delta.y.abs() > 0.0 {
        view.distance = (view.distance - scroll.delta.y * 0.65).clamp(5.4, 14.5);
    }
    for mut cam_tf in &mut scene_q.p3() {
        let pos = hangar_camera_pos(&view);
        *cam_tf = Transform::from_translation(pos).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y);
    }

    let latest_prog = prog_pair.0.as_deref().unwrap_or(&default_prog);
    let combat_spec = owned_combat_spec(&selected_tank, latest_prog);
    let preview_spec = load_tank_spec(&view.preview_tank_id).unwrap_or_else(|| combat_spec.clone());
    let meta = tank_wot_meta(&preview_spec);
    let ratings = compute_tank_ratings(&preview_spec);

    let is_random_map = selected_map.0 == RANDOM_MAP_ID;
    let active_map_idx = if is_random_map {
        None
    } else {
        maps.iter().position(|m| m.id == selected_map.0).or(Some(0))
    };
    let (brief_title, brief_body, _) = map_brief_text(&selected_map.0, &maps, &all_tanks);

    let muted = audio_pair.0.as_ref().is_some_and(|a| a.muted);
    let wt_mode = *mode == DamageRulesMode::ModulesOnlyWt;

    // Переспавн 3D-макета и X-Ray оверлея при смене осматриваемого танка.
    if view.spawned_preview_id != preview_spec.id {
        for (old, _) in &scene_q.p1() {
            commands.entity(old).despawn();
        }
        for (old_x, _) in &scene_q.p2() {
            commands.entity(old_x).despawn();
        }
        if let Some((tt_ent, _)) = scene_q.p0().iter().next() {
            let new_prev = spawn_preview_tank(
                &mut commands,
                &mut assets_pair.0,
                &assets_pair.2,
                &preview_spec,
            );
            if view.xray_armor {
                commands.entity(new_prev).insert(Visibility::Hidden);
            }
            let new_xray = spawn_xray_overlay(
                &mut commands,
                &mut assets_pair.0,
                &mut assets_pair.1,
                &preview_spec,
                view.xray_armor,
            );
            commands.entity(tt_ent).add_children(&[new_prev, new_xray]);
            view.spawned_preview_id = preview_spec.id.clone();
        }
    } else {
        for (_, mut vis) in &mut scene_q.p1() {
            *vis = if view.xray_armor {
                Visibility::Hidden
            } else {
                Visibility::Visible
            };
        }
        for (_, mut vis) in &mut scene_q.p2() {
            *vis = if view.xray_armor {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
        }
    }

    for (_, mut tf) in &mut scene_q.p0() {
        if view.auto_spin && !dragging {
            tf.rotate_y(time.delta_secs() * 0.30);
        }
    }

    if ui_dirty {
        for old_ui in &ui_roots {
            commands.entity(old_ui).despawn();
        }
        let default_last = LastBattleReward::default();
        let last_reward = prog_pair.1.as_deref().unwrap_or(&default_last);
        spawn_hangar_ui(
            &mut commands,
            &all_tanks,
            &maps,
            &selected_tank,
            &selected_map,
            *mode,
            &view,
            latest_prog,
            last_reward,
            &preview_spec,
            muted,
        );
        view.suppress_click_frames = view.suppress_click_frames.max(1);
        return;
    }

    // Подсветка кнопок карусели, меню и верхней панели.
    for (btn, interaction, mut bg, mut border) in &mut btns {
        match *btn {
            HangarButton::SelectTank(g_idx) => {
                if let Some(t) = all_tanks.get(g_idx) {
                    let owned = is_owned(latest_prog, &t.id);
                    let researched = is_researched(latest_prog, &t.id);
                    let is_combat = t.id == combat_spec.id;
                    let is_preview = t.id == preview_spec.id;
                    let nation_accent = tank_wot_meta(t).accent;
                    *bg = if is_combat && is_preview {
                        BackgroundColor(Color::srgba(0.20, 0.24, 0.14, 0.96))
                    } else if is_preview {
                        BackgroundColor(Color::srgba(0.14, 0.20, 0.28, 0.96))
                    } else if *interaction == Interaction::Hovered {
                        BackgroundColor(Color::srgba(0.13, 0.17, 0.22, 0.94))
                    } else if owned {
                        BackgroundColor(Color::srgba(0.07, 0.10, 0.12, 0.92))
                    } else if researched {
                        BackgroundColor(Color::srgba(0.13, 0.12, 0.08, 0.90))
                    } else {
                        BackgroundColor(Color::srgba(0.045, 0.055, 0.075, 0.85))
                    };
                    *border = BorderColor::all(if is_preview {
                        Color::srgb(0.96, 0.64, 0.20)
                    } else if is_combat {
                        Color::srgb(0.42, 0.86, 0.44)
                    } else if *interaction == Interaction::Hovered {
                        Color::srgb(0.68, 0.74, 0.82)
                    } else if owned {
                        nation_accent
                    } else if researched {
                        Color::srgb(0.76, 0.62, 0.26)
                    } else {
                        Color::srgba(0.22, 0.26, 0.34, 0.8)
                    });
                }
            }
            HangarButton::SelectRandomMap => {
                *bg = if is_random_map {
                    BackgroundColor(Color::srgba(0.22, 0.18, 0.08, 0.94))
                } else if *interaction == Interaction::Hovered {
                    BackgroundColor(Color::srgba(0.12, 0.14, 0.18, 0.90))
                } else {
                    BackgroundColor(Color::srgba(0.06, 0.08, 0.11, 0.78))
                };
                *border = BorderColor::all(if is_random_map {
                    Color::srgb(0.95, 0.76, 0.36)
                } else {
                    Color::srgba(0.24, 0.30, 0.36, 0.75)
                });
            }
            HangarButton::SelectMap(i) => {
                let active = active_map_idx == Some(i);
                let accent = maps
                    .get(i)
                    .map(|m| biome_accent(m.biome))
                    .unwrap_or(Color::srgb(0.45, 0.75, 0.95));
                *bg = if active {
                    BackgroundColor(Color::srgba(0.14, 0.24, 0.34, 0.94))
                } else if *interaction == Interaction::Hovered {
                    BackgroundColor(Color::srgba(0.10, 0.14, 0.20, 0.90))
                } else {
                    BackgroundColor(Color::srgba(0.06, 0.08, 0.11, 0.78))
                };
                *border = BorderColor::all(if active {
                    accent
                } else {
                    Color::srgba(0.24, 0.30, 0.36, 0.75)
                });
            }
            HangarButton::ToggleMode => {
                *bg = if *interaction == Interaction::Hovered {
                    if wt_mode {
                        BackgroundColor(Color::srgb(0.56, 0.30, 0.12))
                    } else {
                        BackgroundColor(Color::srgb(0.24, 0.40, 0.20))
                    }
                } else if wt_mode {
                    BackgroundColor(Color::srgb(0.42, 0.22, 0.08))
                } else {
                    BackgroundColor(Color::srgb(0.18, 0.32, 0.16))
                };
                *border = BorderColor::all(if wt_mode {
                    Color::srgb(0.95, 0.60, 0.20)
                } else {
                    Color::srgb(0.52, 0.82, 0.40)
                });
            }
            HangarButton::ToggleXray => {
                *bg = if *interaction == Interaction::Hovered {
                    BackgroundColor(Color::srgb(0.18, 0.28, 0.40))
                } else if view.xray_armor {
                    BackgroundColor(Color::srgb(0.20, 0.36, 0.48))
                } else {
                    BackgroundColor(Color::srgb(0.11, 0.17, 0.25))
                };
            }
            HangarButton::ToggleSpin => {
                *bg = if *interaction == Interaction::Hovered {
                    BackgroundColor(Color::srgb(0.20, 0.28, 0.36))
                } else if view.auto_spin {
                    BackgroundColor(Color::srgb(0.18, 0.28, 0.36))
                } else {
                    BackgroundColor(Color::srgba(0.08, 0.11, 0.15, 0.85))
                };
            }
            HangarButton::ToggleAudio => {
                *bg = if *interaction == Interaction::Hovered {
                    if muted {
                        BackgroundColor(Color::srgb(0.42, 0.18, 0.18))
                    } else {
                        BackgroundColor(Color::srgb(0.16, 0.32, 0.26))
                    }
                } else if muted {
                    BackgroundColor(Color::srgb(0.32, 0.14, 0.14))
                } else {
                    BackgroundColor(Color::srgb(0.12, 0.24, 0.20))
                };
                *border = BorderColor::all(if muted {
                    Color::srgb(0.84, 0.38, 0.36)
                } else {
                    Color::srgb(0.42, 0.78, 0.62)
                });
            }
            HangarButton::StartBattle => {
                *bg = if *interaction == Interaction::Hovered {
                    BackgroundColor(Color::srgb(0.96, 0.36, 0.13))
                } else {
                    BackgroundColor(Color::srgb(0.84, 0.29, 0.10))
                };
            }
            _ => {}
        }
    }

    // Обновление прогресс-баров ТТХ.
    for (bar, mut node) in &mut bars {
        let ratio = match *bar {
            HangarBar::Firepower => ratings.firepower,
            HangarBar::Survivability => ratings.survivability,
            HangarBar::Armor => ratings.armor,
            HangarBar::Mobility => ratings.mobility,
        };
        node.width = Val::Percent((ratio * 100.0).clamp(4.0, 100.0));
    }

    // Обновление текстовых меток.
    for (lbl, mut txt) in &mut labels {
        match *lbl {
            HangarLabel::HeroTitle => {
                txt.0 = format!(
                    "{} {} {}   {}   •   {}",
                    meta.tier,
                    meta.class_icon,
                    meta.class_short,
                    preview_spec.name,
                    meta.nation_short
                );
            }
            HangarLabel::HeroSubtitle => {
                txt.0 = format!(
                    "{} • SolidAP {:.0} мм • {:.0} HP",
                    meta.gun_short,
                    penetration_at(&preview_spec.shell, 100.0),
                    preview_spec.max_hp
                );
            }
            HangarLabel::HeaderStatus => {
                txt.0 = format!("• {}", mode.ru_label());
            }
            HangarLabel::MapBriefTitle => {
                txt.0 = brief_title.clone();
            }
            HangarLabel::MapBriefBody => {
                txt.0 = brief_body.clone();
            }
            HangarLabel::Info => {
                txt.0 = format_hangar_ttx_compact(&preview_spec);
            }
            HangarLabel::ShellSlot => {
                txt.0 = format!("[1] {:.0}мм SolidAP", preview_spec.shell.caliber_mm);
            }
            HangarLabel::ToggleMode => {
                txt.0 = format!("Режим [M]: {}", mode.ru_label());
            }
            HangarLabel::ToggleXray => {
                txt.0 = if view.xray_armor {
                    "Броня/экипаж [V]: ВКЛ".to_string()
                } else {
                    "Броня/экипаж [V]".to_string()
                };
            }
            HangarLabel::ToggleSpin => {
                txt.0 = if view.auto_spin {
                    "Поворот [O]: ВКЛ".to_string()
                } else {
                    "Поворот [O]: ВЫКЛ".to_string()
                };
            }
            HangarLabel::ToggleAudio => {
                txt.0 = if muted {
                    "Звук [N]: ВЫКЛ".to_string()
                } else {
                    "Звук [N]: ВКЛ".to_string()
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hangar_tank_ratings_and_meta_cover_all_fifteen_vehicles() {
        let tanks = available_tanks();
        assert_eq!(tanks.len(), 18);
        for spec in &tanks {
            let r = compute_tank_ratings(spec);
            assert!((0.0..=1.0).contains(&r.firepower));
            assert!((0.0..=1.0).contains(&r.survivability));
            assert!((0.0..=1.0).contains(&r.armor));
            assert!((0.0..=1.0).contains(&r.mobility));

            let meta = tank_wot_meta(spec);
            assert!(!meta.tier.is_empty());
            assert!(!meta.gun_short.is_empty());
        }

        let prog = PlayerProgression::default();
        let owned = filtered_carousel_indices(&tanks, &prog, HangarFilter::AllOwned);
        assert_eq!(owned.len(), 3);
        assert_eq!(
            filtered_carousel_indices(&tanks, &prog, HangarFilter::Nation(Nation::Ussr)).len(),
            7
        );
        assert_eq!(
            filtered_carousel_indices(&tanks, &prog, HangarFilter::Nation(Nation::Germany)).len(),
            6
        );
        assert_eq!(
            filtered_carousel_indices(&tanks, &prog, HangarFilter::Nation(Nation::Usa)).len(),
            5
        );
    }
}
