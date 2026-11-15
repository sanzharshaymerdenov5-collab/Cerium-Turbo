//! Ангар в стиле World of Tanks (Обновление 1.2):
//! - Промышленный 3D-цех со смотровой ямой, рельсами, кран-балкой, трубами, прожекторами и дымкой;
//! - Верхняя панель с крупной кнопкой «В БОЙ!», левое меню режимов и карт, правые карточки ТТХ,
//!   плавающая полоса экипажа и снаряжения и нижняя карусель из 5 исторических танков;
//! - Свободный 360°-обзор камерой и 3D-рентген брони, модулей и экипажа `[V]`.

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
use crate::maps::{MapBiome, MapSpec, available_maps, load_map_spec};
use crate::tank::armor::{hull_plate_specs, turret_plate_specs};
use crate::tank::{
    HangarPreviewTank, TankSpec, available_tanks, load_tank_spec, spawn_preview_tank,
};
use crate::voxel::{Material, VoxelAssets, VoxelGrid, voxel_model_bundle};

/// Настройки и текущее состояние камеры и предпросмотра в ангаре.
#[derive(Resource, Debug)]
pub struct HangarViewState {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub auto_spin: bool,
    pub xray_armor: bool,
    pub prev_xray_btn: bool,
    pub prev_mode_btn: bool,
    pub prev_spin_btn: bool,
    pub prev_audio_btn: bool,
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
            prev_xray_btn: false,
            prev_mode_btn: false,
            prev_spin_btn: false,
            prev_audio_btn: false,
        }
    }
}

#[derive(Component)]
pub struct HangarTurntable;

#[derive(Component)]
pub struct HangarXrayRoot;

#[derive(Component)]
pub struct HangarCamera;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum HangarButton {
    SelectTank(usize),
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

/// Классификация и декоративные метаданные техники в стиле WoT.
#[derive(Debug, Clone, Copy)]
pub struct TankWotMeta {
    pub tier: &'static str,
    pub class_icon: &'static str,
    pub class_short: &'static str,
    pub nation_short: &'static str,
    pub gun_short: &'static str,
    pub accent: Color,
}

pub fn tank_wot_meta(spec: &TankSpec) -> TankWotMeta {
    match spec.id.as_str() {
        "m24_chaffee" => TankWotMeta {
            tier: "V",
            class_icon: "◆",
            class_short: "ЛТ",
            nation_short: "США",
            gun_short: "75мм M6",
            accent: Color::srgb(0.46, 0.72, 0.34),
        },
        "t34_85" => TankWotMeta {
            tier: "VI",
            class_icon: "◈",
            class_short: "СТ",
            nation_short: "СССР",
            gun_short: "85мм ЗиС-С-53",
            accent: Color::srgb(0.84, 0.28, 0.24),
        },
        "su_100" => TankWotMeta {
            tier: "VI",
            class_icon: "▼",
            class_short: "ПТ",
            nation_short: "СССР",
            gun_short: "100мм Д-10С",
            accent: Color::srgb(0.84, 0.28, 0.24),
        },
        "tiger_1" => TankWotMeta {
            tier: "VII",
            class_icon: "■",
            class_short: "ТТ",
            nation_short: "ГЕРМ.",
            gun_short: "88мм KwK 36",
            accent: Color::srgb(0.76, 0.64, 0.36),
        },
        "is_3" => TankWotMeta {
            tier: "VIII",
            class_icon: "■",
            class_short: "ТТ",
            nation_short: "СССР",
            gun_short: "122мм Д-25Т",
            accent: Color::srgb(0.84, 0.28, 0.24),
        },
        _ => TankWotMeta {
            tier: "VI",
            class_icon: "◈",
            class_short: "СТ",
            nation_short: "ТАНК",
            gun_short: "AP",
            accent: Color::srgb(0.65, 0.72, 0.80),
        },
    }
}

pub fn compute_tank_ratings(spec: &TankSpec) -> TankRatings {
    let pen = penetration_at(&spec.shell, 100.0);
    let dpm_proxy = (spec.shell.arcade_damage / spec.reload_s.max(1.0)) * 2.1;
    let firepower = ((pen * 0.56 + dpm_proxy * 0.62) / 195.0).clamp(0.28, 0.98);
    let survivability = (spec.max_hp / 1600.0).clamp(0.28, 0.98);

    let eff_vld = effective_armor_mm(
        &spec.shell,
        spec.armor.hull_front_upper_mm,
        spec.armor.vld_slope_deg,
    );
    let armor = ((eff_vld * 0.55 + spec.armor.turret_front_mm * 0.65) / 260.0).clamp(0.20, 0.98);

    let hp_per_t = spec.engine_power_kw / (spec.mass_kg / 1000.0).max(1.0);
    let mobility =
        ((spec.max_speed_kmh / 60.0) * 0.65 + (hp_per_t / 15.0) * 0.35).clamp(0.24, 0.98);

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

fn enemy_lineup_ru(map_spec: &MapSpec) -> String {
    if map_spec.enemy_spawns.is_empty() {
        return "Мишени полигона (без ИИ)".to_string();
    }
    let names: Vec<String> = map_spec
        .enemy_spawns
        .iter()
        .map(|s| match s.tank_id.as_deref() {
            Some("m24_chaffee") => "M24".to_string(),
            Some("t34_85") => "Т-34-85".to_string(),
            Some("tiger_1") => "Tiger I".to_string(),
            Some("su_100") => "СУ-100".to_string(),
            Some("is_3") => "ИС-3".to_string(),
            _ => "Танк ИИ".to_string(),
        })
        .collect();
    format!("Противник ({}): {}", names.len(), names.join(", "))
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

#[allow(clippy::too_many_arguments)]
pub fn setup_hangar(
    mut commands: Commands,
    selected_tank: Res<SelectedTank>,
    selected_map: Res<SelectedMap>,
    mode: Res<DamageRulesMode>,
    view: Res<HangarViewState>,
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

    spawn_hangar_environment(&mut commands, &mut meshes, &mut materials, &assets);

    let spec = load_tank_spec(&selected_tank.0.0)
        .unwrap_or_else(|| load_tank_spec("t34_85").expect("t34_85"));
    let preview = spawn_preview_tank(&mut commands, &mut meshes, &assets, &spec);
    if view.xray_armor {
        commands.entity(preview).insert(Visibility::Hidden);
    }
    let xray_root = spawn_xray_overlay(
        &mut commands,
        &mut meshes,
        &mut materials,
        &spec,
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

    let tanks = available_tanks();
    let maps = available_maps();
    let map_spec = load_map_spec(&selected_map.0)
        .or_else(|| maps.first().cloned())
        .expect("default map");
    let meta = tank_wot_meta(&spec);
    let ratings = compute_tank_ratings(&spec);
    let muted = audio.as_ref().is_some_and(|a| a.muted);
    let wt_mode = *mode == DamageRulesMode::ModulesOnlyWt;

    // ─── Корневой интерфейс WoT-ангара ───
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::SpaceBetween,
                padding: UiRect::all(Val::Px(12.0)),
                row_gap: Val::Px(6.0),
                ..default()
            },
            DespawnOnExit(GameState::Hangar),
        ))
        .with_children(|root| {
            // 1. ВЕРХНЯЯ ПАНЕЛЬ + КРУПНАЯ КНОПКА «В БОЙ!» + ТИТУЛ МАШИНЫ
            root.spawn(Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(6.0),
                ..default()
            })
            .with_children(|header| {
                // Тёмная верхняя строка навигации WoT.
                header
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            padding: UiRect::axes(Val::Px(14.0), Val::Px(6.0)),
                            border: UiRect::bottom(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(4.0)),
                            column_gap: Val::Px(10.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.04, 0.055, 0.072, 0.90)),
                        BorderColor::all(Color::srgba(0.32, 0.38, 0.46, 0.7)),
                    ))
                    .with_children(|bar| {
                        // Лево: бренд + бейдж «Обновление 1.2» + быстрый режим.
                        bar.spawn(Node {
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(8.0),
                            ..default()
                        })
                        .with_children(|left| {
                            left.spawn((
                                Text::new("CERIUM-TURBO"),
                                TextFont::from_font_size(16.0),
                                TextColor(Color::srgb(0.95, 0.96, 0.98)),
                            ));
                            left.spawn((
                                Node {
                                    padding: UiRect::axes(Val::Px(7.0), Val::Px(2.0)),
                                    border: UiRect::all(Val::Px(1.0)),
                                    border_radius: BorderRadius::all(Val::Px(3.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.68, 0.30, 0.08, 0.90)),
                                BorderColor::all(Color::srgb(0.96, 0.56, 0.22)),
                            ))
                            .with_children(|b| {
                                b.spawn((
                                    Text::new("v1.2"),
                                    TextFont::from_font_size(12.0),
                                    TextColor(Color::srgb(1.0, 0.92, 0.78)),
                                ));
                            });
                            left.spawn((
                                HangarLabel::HeaderStatus,
                                Text::new(format!("• Режим: {}", mode.ru_label())),
                                TextFont::from_font_size(12.0),
                                TextColor(Color::srgb(0.72, 0.80, 0.88)),
                            ));
                        });

                        // Центр: фирменная оранжево-красная кнопка «В БОЙ!».
                        bar.spawn((
                            Interaction::default(),
                            HangarButton::StartBattle,
                            Node {
                                padding: UiRect::axes(Val::Px(34.0), Val::Px(8.0)),
                                border: UiRect::all(Val::Px(2.0)),
                                border_radius: BorderRadius::all(Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.84, 0.29, 0.10)),
                            BorderColor::all(Color::srgb(1.0, 0.56, 0.26)),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                Text::new("В БОЙ!"),
                                TextFont::from_font_size(18.5),
                                TextColor(Color::WHITE),
                            ));
                        });

                        // Право: сводка и кнопка звука `[N]`.
                        bar.spawn(Node {
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(8.0),
                            ..default()
                        })
                        .with_children(|right| {
                            right.spawn((
                                Text::new("5 машин • 5 карт • AP-Only"),
                                TextFont::from_font_size(11.5),
                                TextColor(Color::srgb(0.68, 0.75, 0.82)),
                            ));
                            right
                                .spawn((
                                    Interaction::default(),
                                    HangarButton::ToggleAudio,
                                    Node {
                                        padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)),
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
                                        TextFont::from_font_size(12.0),
                                        TextColor(Color::srgb(0.90, 0.96, 0.94)),
                                    ));
                                });
                        });
                    });

                // Центрированный заголовок выбранного танка прямо под кнопкой «В БОЙ!».
                header
                    .spawn((
                        Node {
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Center,
                            padding: UiRect::axes(Val::Px(22.0), Val::Px(4.0)),
                            border_radius: BorderRadius::all(Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.03, 0.045, 0.06, 0.56)),
                    ))
                    .with_children(|hero| {
                        hero.spawn((
                            HangarLabel::HeroTitle,
                            Text::new(format!(
                                "{} {} {}   {}   •   {}",
                                meta.tier, meta.class_icon, meta.class_short, spec.name, meta.nation_short
                            )),
                            TextFont::from_font_size(19.0),
                            TextColor(Color::srgb(0.96, 0.97, 0.98)),
                        ));
                        hero.spawn((
                            HangarLabel::HeroSubtitle,
                            Text::new(format!(
                                "{} • SolidAP {:.0} мм • {:.0} HP",
                                meta.gun_short,
                                penetration_at(&spec.shell, 100.0),
                                spec.max_hp
                            )),
                            TextFont::from_font_size(12.0),
                            TextColor(Color::srgb(0.76, 0.82, 0.88)),
                        ));
                    });
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
                        padding: UiRect::all(Val::Px(10.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(4.0)),
                        row_gap: Val::Px(6.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.035, 0.05, 0.068, 0.80)),
                    BorderColor::all(Color::srgba(0.30, 0.36, 0.44, 0.65)),
                ))
                .with_children(|left| {
                    left.spawn((
                        Text::new("РЕЖИМЫ И ИНСПЕКЦИЯ"),
                        TextFont::from_font_size(11.0),
                        TextColor(Color::srgb(0.64, 0.72, 0.80)),
                    ));

                    // Кнопка переключения режима WoT / War Thunder.
                    left.spawn((
                        Interaction::default(),
                        HangarButton::ToggleMode,
                        Node {
                            padding: UiRect::axes(Val::Px(10.0), Val::Px(7.0)),
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
                            TextFont::from_font_size(12.5),
                            TextColor(Color::WHITE),
                        ));
                    });

                    // Кнопка X-Ray брони и экипажа.
                    left.spawn((
                        Interaction::default(),
                        HangarButton::ToggleXray,
                        Node {
                            padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
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
                            TextFont::from_font_size(12.5),
                            TextColor(Color::srgb(0.86, 0.94, 1.0)),
                        ));
                    });

                    // Кнопка автовращения танка на стенде.
                    left.spawn((
                        Interaction::default(),
                        HangarButton::ToggleSpin,
                        Node {
                            padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)),
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
                            TextFont::from_font_size(12.0),
                            TextColor(Color::srgb(0.80, 0.87, 0.94)),
                        ));
                    });

                    left.spawn((
                        Node {
                            margin: UiRect::top(Val::Px(4.0)),
                            ..default()
                        },
                        Text::new("КАРТА БОЯ [Q / E]"),
                        TextFont::from_font_size(11.0),
                        TextColor(Color::srgb(0.64, 0.72, 0.80)),
                    ));

                    for (i, m) in maps.iter().enumerate() {
                        let active = m.id == selected_map.0;
                        left.spawn((
                            Interaction::default(),
                            HangarButton::SelectMap(i),
                            Node {
                                padding: UiRect::axes(Val::Px(9.0), Val::Px(5.0)),
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
                                TextFont::from_font_size(12.5),
                                TextColor(Color::srgb(0.92, 0.95, 0.98)),
                            ));
                            b.spawn((
                                Text::new(biome_short_ru(m.biome)),
                                TextFont::from_font_size(10.5),
                                TextColor(Color::srgb(0.64, 0.74, 0.84)),
                            ));
                        });
                    }
                });

                // Правая панель WoT: карточка карты/противников + карточка ТТХ со шкалами.
                mid.spawn(Node {
                    width: Val::Px(308.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.0),
                    ..default()
                })
                .with_children(|right| {
                    // Карточка брифинга карты.
                    right
                        .spawn((
                            Node {
                                padding: UiRect::all(Val::Px(10.0)),
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
                            BorderColor::all(biome_accent(map_spec.biome)),
                        ))
                        .with_children(|mc| {
                            mc.spawn((
                                HangarLabel::MapBriefTitle,
                                Text::new(format!("{}  •  {}", map_spec.name, biome_short_ru(map_spec.biome))),
                                TextFont::from_font_size(13.5),
                                TextColor(Color::srgb(0.96, 0.90, 0.66)),
                            ));
                            mc.spawn((
                                HangarLabel::MapBriefBody,
                                Text::new(format!(
                                    "{}\n{}",
                                    map_spec.description,
                                    enemy_lineup_ru(&map_spec)
                                )),
                                TextFont::from_font_size(11.5),
                                TextColor(Color::srgb(0.78, 0.84, 0.90)),
                            ));
                        });

                    // Карточка характеристик машины со шкалами в стиле WoT.
                    right
                        .spawn((
                            Node {
                                padding: UiRect::all(Val::Px(10.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                border_radius: BorderRadius::all(Val::Px(4.0)),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(6.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.04, 0.055, 0.075, 0.86)),
                            BorderColor::all(Color::srgba(0.32, 0.38, 0.46, 0.72)),
                        ))
                        .with_children(|ttx| {
                            ttx.spawn((
                                Text::new("ХАРАКТЕРИСТИКИ МАШИНЫ"),
                                TextFont::from_font_size(11.5),
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
                                        TextFont::from_font_size(11.0),
                                        TextColor(Color::srgb(0.76, 0.82, 0.88)),
                                    ));
                                    row.spawn((
                                        Node {
                                            width: Val::Percent(100.0),
                                            height: Val::Px(6.0),
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
                                Text::new(format_hangar_ttx_compact(&spec)),
                                TextFont::from_font_size(11.0),
                                TextColor(Color::srgb(0.80, 0.86, 0.92)),
                            ));
                        });
                });
            });

            // 3. НИЖНИЙ БЛОК: ЭКИПАЖ И БОЕКОМПЛЕКТ НАД КАРУСЕЛЬЮ + КАРУСЕЛЬ ИЗ 5 ТАНКОВ + ПОДСКАЗКИ
            root.spawn(Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(6.0),
                ..default()
            })
            .with_children(|bottom| {
                // Полоса экипажа (4 танкиста) и боекомплекта/расходников прямо под танком, как в WoT.
                bottom
                    .spawn((
                        Node {
                            padding: UiRect::axes(Val::Px(12.0), Val::Px(5.0)),
                            border: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(4.0)),
                            column_gap: Val::Px(10.0),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.04, 0.055, 0.075, 0.88)),
                        BorderColor::all(Color::srgba(0.34, 0.40, 0.48, 0.7)),
                    ))
                    .with_children(|strip| {
                        // 4 слота экипажа.
                        for role in [
                            CrewRole::Commander,
                            CrewRole::Gunner,
                            CrewRole::Driver,
                            CrewRole::Loader,
                        ] {
                            strip
                                .spawn((
                                    Node {
                                        padding: UiRect::axes(Val::Px(7.0), Val::Px(3.0)),
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
                                        TextFont::from_font_size(11.0),
                                        TextColor(Color::srgb(0.88, 0.92, 0.80)),
                                    ));
                                });
                        }

                        strip.spawn((
                            Text::new("│"),
                            TextFont::from_font_size(14.0),
                            TextColor(Color::srgba(0.40, 0.46, 0.54, 0.8)),
                        ));

                        // Слот единственного снаряда (AP-Only до следующих обновлений).
                        strip
                            .spawn((
                                Node {
                                    padding: UiRect::axes(Val::Px(9.0), Val::Px(3.0)),
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
                                    Text::new(format!("[1] {:.0}мм SolidAP", spec.shell.caliber_mm)),
                                    TextFont::from_font_size(11.5),
                                    TextColor(Color::srgb(1.0, 0.90, 0.68)),
                                ));
                            });

                        // Слоты расходников `[5]` и `[6]`.
                        for label in ["[5] Ремк. ×2", "[6] Огнет. ×2"] {
                            strip
                                .spawn((
                                    Node {
                                        padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
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
                                        TextFont::from_font_size(11.0),
                                        TextColor(Color::srgb(0.82, 0.92, 0.98)),
                                    ));
                                });
                        }
                    });

                // Нижняя карусель 5 танков в стиле World of Tanks.
                bottom
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            padding: UiRect::all(Val::Px(6.0)),
                            border: UiRect::top(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(4.0)),
                            column_gap: Val::Px(8.0),
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.03, 0.042, 0.056, 0.92)),
                        BorderColor::all(Color::srgba(0.32, 0.38, 0.46, 0.72)),
                    ))
                    .with_children(|carousel| {
                        for (i, t) in tanks.iter().enumerate() {
                            let active = t.id == selected_tank.0.0;
                            let tm = tank_wot_meta(t);
                            carousel
                                .spawn((
                                    Interaction::default(),
                                    HangarButton::SelectTank(i),
                                    Node {
                                        flex_grow: 1.0,
                                        min_width: Val::Px(150.0),
                                        max_width: Val::Px(245.0),
                                        padding: UiRect::axes(Val::Px(10.0), Val::Px(7.0)),
                                        border: UiRect {
                                            top: Val::Px(3.0),
                                            left: Val::Px(if active { 2.0 } else { 1.0 }),
                                            right: Val::Px(if active { 2.0 } else { 1.0 }),
                                            bottom: Val::Px(if active { 2.0 } else { 1.0 }),
                                        },
                                        border_radius: BorderRadius::all(Val::Px(4.0)),
                                        flex_direction: FlexDirection::Column,
                                        row_gap: Val::Px(3.0),
                                        ..default()
                                    },
                                    BackgroundColor(if active {
                                        Color::srgba(0.20, 0.24, 0.14, 0.96)
                                    } else {
                                        Color::srgba(0.07, 0.09, 0.12, 0.90)
                                    }),
                                    BorderColor::all(if active {
                                        Color::srgb(0.96, 0.64, 0.20)
                                    } else {
                                        tm.accent
                                    }),
                                ))
                                .with_children(|card| {
                                    // Верхняя строчка карточки: [клавиша], уровень/класс и нация.
                                    card.spawn(Node {
                                        width: Val::Percent(100.0),
                                        justify_content: JustifyContent::SpaceBetween,
                                        ..default()
                                    })
                                    .with_children(|top_row| {
                                        top_row.spawn((
                                            Text::new(format!("[{}] {} {} {}", i + 1, tm.tier, tm.class_icon, tm.class_short)),
                                            TextFont::from_font_size(11.0),
                                            TextColor(Color::srgb(0.96, 0.80, 0.42)),
                                        ));
                                        top_row.spawn((
                                            Text::new(tm.nation_short),
                                            TextFont::from_font_size(10.5),
                                            TextColor(Color::srgb(0.68, 0.75, 0.82)),
                                        ));
                                    });
                                    // Имя танка.
                                    card.spawn((
                                        Text::new(t.name.clone()),
                                        TextFont::from_font_size(13.5),
                                        TextColor(Color::srgb(0.96, 0.97, 0.99)),
                                    ));
                                    // Краткие боевые характеристики.
                                    card.spawn((
                                        Text::new(format!(
                                            "{:.0}мм • {:.0} HP • {:.0} км/ч",
                                            t.shell.caliber_mm, t.max_hp, t.max_speed_kmh
                                        )),
                                        TextFont::from_font_size(11.0),
                                        TextColor(Color::srgb(0.72, 0.78, 0.86)),
                                    ));
                                });
                        }
                    });

                // Сервисная строка управления в самом низу.
                bottom.spawn((
                    Text::new(
                        "ЛКМ/ПКМ: 360° осмотр  •  Колёсико: зум  •  1–5 / ←→: танк  •  Q / E: карта  •  M: режим  •  V: рентген  •  O: поворот  •  N: звук  •  Enter: в бой",
                    ),
                    TextFont::from_font_size(10.5),
                    TextColor(Color::srgb(0.62, 0.68, 0.76)),
                ));
            });
        });
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
    let tanks = available_tanks();
    let cur_idx = tanks
        .iter()
        .position(|t| t.id == selected_tank.0.0)
        .unwrap_or(0);
    let mut new_idx: Option<usize> = None;

    let maps = available_maps();
    let cur_map_idx = maps
        .iter()
        .position(|m| m.id == selected_map.0)
        .unwrap_or(0);
    let mut new_map_idx: Option<usize> = None;

    let digit_keys = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
    ];
    for (i, &k) in digit_keys.iter().enumerate() {
        if keys.just_pressed(k) && i < tanks.len() {
            new_idx = Some(i);
        }
    }
    if keys.just_pressed(KeyCode::ArrowRight) && !tanks.is_empty() {
        new_idx = Some((cur_idx + 1) % tanks.len());
    }
    if keys.just_pressed(KeyCode::ArrowLeft) && !tanks.is_empty() {
        new_idx = Some((cur_idx + tanks.len() - 1) % tanks.len());
    }
    if keys.just_pressed(KeyCode::KeyE) && !maps.is_empty() {
        new_map_idx = Some((cur_map_idx + 1) % maps.len());
    }
    if keys.just_pressed(KeyCode::KeyQ) && !maps.is_empty() {
        new_map_idx = Some((cur_map_idx + maps.len() - 1) % maps.len());
    }

    let mut ui_hovered = false;
    let mut toggle_xray = keys.just_pressed(KeyCode::KeyV);
    let mut toggle_mode = keys.just_pressed(KeyCode::KeyM);
    let mut toggle_spin = keys.just_pressed(KeyCode::KeyO);
    let mut toggle_audio = false;
    let mut xray_pressed_now = false;
    let mut mode_pressed_now = false;
    let mut spin_pressed_now = false;
    let mut audio_pressed_now = false;
    let mut start = keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space);

    for (btn, interaction, _, _) in &btns {
        if *interaction != Interaction::None {
            ui_hovered = true;
        }
        if *interaction == Interaction::Pressed {
            match *btn {
                HangarButton::SelectTank(i) if i < tanks.len() => new_idx = Some(i),
                HangarButton::SelectMap(i) if i < maps.len() => new_map_idx = Some(i),
                HangarButton::ToggleMode => mode_pressed_now = true,
                HangarButton::ToggleXray => xray_pressed_now = true,
                HangarButton::ToggleSpin => spin_pressed_now = true,
                HangarButton::ToggleAudio => audio_pressed_now = true,
                HangarButton::StartBattle => start = true,
                _ => {}
            }
        }
    }
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
    view.prev_xray_btn = xray_pressed_now;
    view.prev_mode_btn = mode_pressed_now;
    view.prev_spin_btn = spin_pressed_now;
    view.prev_audio_btn = audio_pressed_now;

    let mut clicked = false;
    if let Some(i) = new_idx
        && tanks[i].id != selected_tank.0.0
    {
        selected_tank.0 = TankId(tanks[i].id.clone());
        clicked = true;
    }
    if let Some(i) = new_map_idx
        && maps[i].id != selected_map.0
    {
        selected_map.0 = maps[i].id.clone();
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
        if let Some(ref mut msgs) = audio_pair.1 {
            msgs.write(UiSoundMessage(SoundPreset::BattleStart));
        }
        next_state.set(GameState::InBattle);
        return;
    }
    if clicked && let Some(ref mut msgs) = audio_pair.1 {
        msgs.write(UiSoundMessage(SoundPreset::UiClick));
    }

    // Орбита камеры мышью.
    let dragging = mouse_btn.pressed(MouseButton::Right)
        || (mouse_btn.pressed(MouseButton::Left) && !ui_hovered);
    if dragging && motion.delta != Vec2::ZERO {
        view.yaw -= motion.delta.x * 0.008;
        view.pitch = (view.pitch + motion.delta.y * 0.007).clamp(0.06, 1.12);
    }
    if scroll.delta.y.abs() > 0.0 {
        view.distance = (view.distance - scroll.delta.y * 0.65).clamp(5.4, 14.5);
    }
    for mut cam_tf in &mut scene_q.p3() {
        let pos = hangar_camera_pos(&view);
        *cam_tf = Transform::from_translation(pos).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y);
    }

    let active_idx = tanks
        .iter()
        .position(|t| t.id == selected_tank.0.0)
        .unwrap_or(0);
    let spec = tanks
        .get(active_idx)
        .cloned()
        .unwrap_or_else(|| load_tank_spec("t34_85").expect("t34_85"));
    let meta = tank_wot_meta(&spec);
    let ratings = compute_tank_ratings(&spec);

    let active_map_idx = maps
        .iter()
        .position(|m| m.id == selected_map.0)
        .unwrap_or(0);
    let map_spec = maps
        .get(active_map_idx)
        .cloned()
        .or_else(|| load_map_spec("proving_grounds"))
        .expect("default map");

    let muted = audio_pair.0.as_ref().is_some_and(|a| a.muted);
    let wt_mode = *mode == DamageRulesMode::ModulesOnlyWt;

    // Подсветка кнопок карусели, меню и верхней панели.
    for (btn, interaction, mut bg, mut border) in &mut btns {
        match *btn {
            HangarButton::SelectTank(i) => {
                let active = i == active_idx;
                let nation_accent = tanks
                    .get(i)
                    .map(|t| tank_wot_meta(t).accent)
                    .unwrap_or(Color::srgb(0.45, 0.50, 0.56));
                *bg = if active {
                    BackgroundColor(Color::srgba(0.20, 0.24, 0.14, 0.96))
                } else if *interaction == Interaction::Hovered {
                    BackgroundColor(Color::srgba(0.13, 0.17, 0.22, 0.94))
                } else {
                    BackgroundColor(Color::srgba(0.07, 0.09, 0.12, 0.90))
                };
                *border = BorderColor::all(if active {
                    Color::srgb(0.96, 0.64, 0.20)
                } else if *interaction == Interaction::Hovered {
                    Color::srgb(0.68, 0.74, 0.82)
                } else {
                    nation_accent
                });
            }
            HangarButton::SelectMap(i) => {
                let active = i == active_map_idx;
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
                    meta.tier, meta.class_icon, meta.class_short, spec.name, meta.nation_short
                );
            }
            HangarLabel::HeroSubtitle => {
                txt.0 = format!(
                    "{} • SolidAP {:.0} мм • {:.0} HP",
                    meta.gun_short,
                    penetration_at(&spec.shell, 100.0),
                    spec.max_hp
                );
            }
            HangarLabel::HeaderStatus => {
                txt.0 = format!("• Режим: {}", mode.ru_label());
            }
            HangarLabel::MapBriefTitle => {
                txt.0 = format!("{}  •  {}", map_spec.name, biome_short_ru(map_spec.biome));
            }
            HangarLabel::MapBriefBody => {
                txt.0 = format!(
                    "{}\n{}",
                    map_spec.description,
                    enemy_lineup_ru(&map_spec)
                );
            }
            HangarLabel::Info => {
                txt.0 = format_hangar_ttx_compact(&spec);
            }
            HangarLabel::ShellSlot => {
                txt.0 = format!("[1] {:.0}мм SolidAP", spec.shell.caliber_mm);
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

    // Переспавн 3D-макета и X-Ray оверлея при смене танка.
    if selected_tank.is_changed() {
        for (old, _) in &scene_q.p1() {
            commands.entity(old).despawn();
        }
        for (old_x, _) in &scene_q.p2() {
            commands.entity(old_x).despawn();
        }
        if let Some((tt_ent, _)) = scene_q.p0().iter().next() {
            let new_prev =
                spawn_preview_tank(&mut commands, &mut assets_pair.0, &assets_pair.2, &spec);
            if view.xray_armor {
                commands.entity(new_prev).insert(Visibility::Hidden);
            }
            let new_xray = spawn_xray_overlay(
                &mut commands,
                &mut assets_pair.0,
                &mut assets_pair.1,
                &spec,
                view.xray_armor,
            );
            commands.entity(tt_ent).add_children(&[new_prev, new_xray]);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hangar_tank_ratings_and_meta_cover_all_five_vehicles() {
        let tanks = available_tanks();
        assert_eq!(tanks.len(), 5);
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
    }
}
