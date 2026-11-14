//! Ангар: выбор из 5 танков, мышиный орбит/зум камеры, 3D-рентген брони и модулей,
//! карточка подробных ТТХ и кнопка «В БОЙ».

#![allow(deprecated)]

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::math::{IVec3, UVec3};
use bevy::prelude::*;

use crate::ballistics::penetration::penetration_at;
use crate::core::{DamageRulesMode, GameState, SelectedMap, SelectedTank, TankId};
use crate::damage::{ModuleKind, TankCrew, TankModules};
use crate::maps::{MapSpec, available_maps, load_map_spec};
use crate::tank::armor::{hull_plate_specs, turret_plate_specs};
use crate::tank::{
    HangarPreviewTank, TankSpec, available_tanks, load_tank_spec, spawn_preview_tank,
};
use crate::voxel::{Material, VoxelAssets, VoxelGrid, voxel_model_bundle};

/// Состояние камеры и предпросмотра в ангаре.
#[derive(Resource, Debug)]
pub struct HangarViewState {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub auto_spin: bool,
    pub xray_armor: bool,
    pub prev_xray_btn: bool,
    pub prev_mode_btn: bool,
}

impl Default for HangarViewState {
    fn default() -> Self {
        Self {
            yaw: 0.62,
            pitch: 0.34,
            distance: 9.2,
            auto_spin: true,
            xray_armor: false,
            prev_xray_btn: false,
            prev_mode_btn: false,
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
    StartBattle,
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum HangarLabel {
    Info,
    ToggleXray,
    ToggleMode,
}

fn format_hangar_info(
    spec: &TankSpec,
    map_spec: &MapSpec,
    mode: DamageRulesMode,
    xray: bool,
) -> String {
    let p_100 = penetration_at(&spec.shell, 100.0);
    let p_500 = penetration_at(&spec.shell, 500.0);
    let hp_per_t = spec.engine_power_kw / (spec.mass_kg / 1000.0).max(1.0);
    let yaw_str = match spec.yaw_limits_deg {
        Some((a, b)) => format!(
            "ПТ-САУ (УГН {a:+.0}°..{b:+.0}°, {:.0}°/с)",
            spec.turret_traverse_dps
        ),
        None => format!("Круговая ({:.0}°/с)", spec.turret_traverse_dps),
    };
    let armor = &spec.armor;
    format!(
        "ТАНК: {}  [{}]\n\
         Прочность: {:.0} HP  |  Масса: {:.1} т  |  Уд. мощн.: {:.1} кВт/т\n\
         Скорость: +{:.0} / -{:.0} км/ч  |  Разворот: {:.0}°/с\n\
         Наведение: {}  |  УВН: -{:.0}°..+{:.0}°\n\
         Орудие {:.0} мм • ТОЛЬКО сплошной SolidAP ({:.0} м/с)\n\
         Урон AP (WoT): {:.0} HP  |  Перезарядка: {:.1} с\n\
         Пробитие: {:.0} мм (100 м) / {:.0} мм (500 м)  |  Рикошет >{:.0}°\n\
         Броня корпуса: ВЛД {:.0}@{:.0}° / НЛД {:.0} / Борт {:.0} / Корма {:.0} мм\n\
         Броня башни/рубки: Лоб {:.0} / Борт {:.0} / Корма {:.0} мм\n\
         Режим: {}  |  Рентген [V]: {}\n\
         КАРТА [Q/E]: {} [{}]\n\
         {}",
        spec.name,
        spec.nation,
        spec.max_hp,
        spec.mass_kg / 1000.0,
        hp_per_t,
        spec.max_speed_kmh,
        spec.reverse_speed_kmh,
        spec.max_yaw_rate_dps,
        yaw_str,
        spec.gun_depression_deg,
        spec.gun_elevation_deg,
        spec.shell.caliber_mm,
        spec.shell.muzzle_velocity,
        spec.shell.arcade_damage,
        spec.reload_s,
        p_100,
        p_500,
        spec.shell.ricochet_deg,
        armor.hull_front_upper_mm,
        armor.vld_slope_deg,
        armor.hull_front_lower_mm,
        armor.hull_side_mm,
        armor.hull_rear_mm,
        armor.turret_front_mm,
        armor.turret_side_mm,
        armor.turret_rear_mm,
        mode.ru_label(),
        if xray { "ВКЛ" } else { "ВЫКЛ" },
        map_spec.name,
        map_spec.biome.ru_name(),
        map_spec.description,
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

    // Прозрачные бронеплиты корпуса.
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
                        .with_scale(Vec3::new(plate.half_extents.x * 2.0, plate.half_extents.y * 2.0, 0.04)),
                ))
                .id(),
        );
    }

    let turret_yaw = if spec.yaw_limits_deg.is_some() { 0.0 } else { 0.25 };
    let turret_tf = Transform::from_translation(Vec3::from_array(spec.profile.turret_pivot))
        .with_rotation(Quat::from_rotation_y(turret_yaw));

    // Прозрачные бронеплиты башни/рубки.
    for plate in turret_plate_specs(spec) {
        let p_tf = turret_tf
            * Transform::from_translation(plate.local_pos).with_rotation(plate.local_rot);
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

    // Ствол в рентгене (ориентир направления орудия).
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

    // Внутренние и ходовые модули (яркие непрозрачные блоки внутри прозрачной брони).
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

    // Экипаж внутри боевого отделения (золотисто-лаймовые 3D-фигуры для режима War Thunder).
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

#[allow(clippy::too_many_arguments)]
pub fn setup_hangar(
    mut commands: Commands,
    selected_tank: Res<SelectedTank>,
    selected_map: Res<SelectedMap>,
    mode: Res<DamageRulesMode>,
    view: Res<HangarViewState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    assets: Res<VoxelAssets>,
) {
    commands.insert_resource(ClearColor(Color::srgb(0.11, 0.13, 0.16)));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.85, 0.90, 1.0),
        brightness: 450.0,
        ..default()
    });

    let mut plinth = VoxelGrid::new(UVec3::new(34, 2, 34), 0.3, Vec3::new(-5.1, -0.6, -5.1));
    plinth.fill_box(IVec3::ZERO, IVec3::new(33, 1, 33), Material::Concrete);

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
    let plinth_ent = commands
        .spawn(voxel_model_bundle(&plinth, &mut meshes, &assets))
        .id();

    commands
        .spawn((
            Name::new("HangarTurntable"),
            HangarTurntable,
            Transform::from_rotation(Quat::from_rotation_y(0.45)),
            Visibility::default(),
            DespawnOnExit(GameState::Hangar),
        ))
        .add_children(&[preview, xray_root, plinth_ent]);

    commands.spawn((
        DirectionalLight {
            illuminance: 9_500.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(8.0, 12.0, 10.0).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
        DespawnOnExit(GameState::Hangar),
    ));

    let cam_pos = hangar_camera_pos(&view);
    commands.spawn((
        HangarCamera,
        Camera3d::default(),
        Transform::from_translation(cam_pos).looking_at(Vec3::new(-0.9, 1.0, 0.0), Vec3::Y),
        DespawnOnExit(GameState::Hangar),
    ));

    let tanks = available_tanks();
    let maps = available_maps();
    let map_spec = load_map_spec(&selected_map.0)
        .or_else(|| maps.first().cloned())
        .expect("default map");
    let info = format_hangar_info(&spec, &map_spec, *mode, view.xray_armor);

    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::SpaceBetween,
                padding: UiRect::all(Val::Px(20.0)),
                ..default()
            },
            DespawnOnExit(GameState::Hangar),
        ))
        .with_children(|root| {
            root.spawn(Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|top| {
                top.spawn((
                    Node {
                        justify_content: JustifyContent::SpaceBetween,
                        align_items: AlignItems::Center,
                        padding: UiRect::axes(Val::Px(16.0), Val::Px(10.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        column_gap: Val::Px(10.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.06, 0.08, 0.10, 0.88)),
                    BorderColor::all(Color::srgb(0.35, 0.48, 0.32)),
                ))
                .with_children(|p| {
                    p.spawn((
                        Text::new(
                            "CERIUM-TURBO  •  ЭТАП 6: 5 ТАНКОВ, 5 КАРТ, WoT / WAR THUNDER (только SolidAP)",
                        ),
                        TextFont::from_font_size(17.5),
                        TextColor(Color::srgb(0.92, 0.95, 0.88)),
                    ));

                    p.spawn(Node {
                        column_gap: Val::Px(8.0),
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|btns_r| {
                        btns_r
                            .spawn((
                                Interaction::default(),
                                HangarButton::ToggleMode,
                                Node {
                                    padding: UiRect::axes(Val::Px(14.0), Val::Px(6.0)),
                                    border: UiRect::all(Val::Px(1.5)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(0.22, 0.34, 0.18)),
                                BorderColor::all(Color::srgb(0.55, 0.82, 0.42)),
                            ))
                            .with_children(|btn| {
                                btn.spawn((
                                    HangarLabel::ToggleMode,
                                    Text::new(format!("Режим [M]: {}", mode.ru_label())),
                                    TextFont::from_font_size(13.5),
                                    TextColor(Color::WHITE),
                                ));
                            });

                        btns_r
                            .spawn((
                                Interaction::default(),
                                HangarButton::ToggleXray,
                                Node {
                                    padding: UiRect::axes(Val::Px(14.0), Val::Px(6.0)),
                                    border: UiRect::all(Val::Px(1.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(0.14, 0.20, 0.28)),
                                BorderColor::all(Color::srgb(0.45, 0.65, 0.85)),
                            ))
                            .with_children(|btn| {
                                btn.spawn((
                                    HangarLabel::ToggleXray,
                                    Text::new("Броня/экипаж [V]"),
                                    TextFont::from_font_size(13.5),
                                    TextColor(Color::srgb(0.86, 0.94, 1.0)),
                                ));
                            });
                    });
                });

                // Кнопки выбора 5 танков.
                top.spawn(Node {
                    width: Val::Percent(100.0),
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: Val::Px(8.0),
                    row_gap: Val::Px(6.0),
                    ..default()
                })
                .with_children(|row| {
                    for (i, t) in tanks.iter().enumerate() {
                        let active = t.id == selected_tank.0.0;
                        row.spawn((
                            Interaction::default(),
                            HangarButton::SelectTank(i),
                            Node {
                                padding: UiRect::axes(Val::Px(12.0), Val::Px(7.0)),
                                border: UiRect::all(Val::Px(if active { 2.0 } else { 1.0 })),
                                ..default()
                            },
                            BackgroundColor(if active {
                                Color::srgb(0.20, 0.34, 0.18)
                            } else {
                                Color::srgba(0.06, 0.08, 0.10, 0.86)
                            }),
                            BorderColor::all(if active {
                                Color::srgb(0.55, 0.86, 0.45)
                            } else {
                                Color::srgb(0.28, 0.34, 0.40)
                            }),
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new(format!(
                                    "[{}] {} ({:.0}мм)",
                                    i + 1,
                                    t.name,
                                    t.shell.caliber_mm
                                )),
                                TextFont::from_font_size(14.0),
                                TextColor(Color::srgb(0.90, 0.94, 0.88)),
                            ));
                        });
                    }
                });

                // Кнопки выбора 5 карт.
                top.spawn(Node {
                    width: Val::Percent(100.0),
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: Val::Px(8.0),
                    row_gap: Val::Px(6.0),
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|row| {
                    for (i, m) in maps.iter().enumerate() {
                        let active = m.id == selected_map.0;
                        row.spawn((
                            Interaction::default(),
                            HangarButton::SelectMap(i),
                            Node {
                                padding: UiRect::axes(Val::Px(11.0), Val::Px(6.0)),
                                border: UiRect::all(Val::Px(if active { 2.0 } else { 1.0 })),
                                ..default()
                            },
                            BackgroundColor(if active {
                                Color::srgb(0.16, 0.28, 0.38)
                            } else {
                                Color::srgba(0.06, 0.08, 0.10, 0.84)
                            }),
                            BorderColor::all(if active {
                                Color::srgb(0.45, 0.78, 0.96)
                            } else {
                                Color::srgb(0.26, 0.32, 0.38)
                            }),
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new(format!("Карта: {}", m.name)),
                                TextFont::from_font_size(13.0),
                                TextColor(Color::srgb(0.86, 0.93, 0.98)),
                            ));
                        });
                    }
                });
            });

            root.spawn((
                Node {
                    max_width: Val::Px(530.0),
                    padding: UiRect::all(Val::Px(14.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.06, 0.08, 0.10, 0.88)),
                BorderColor::all(Color::srgb(0.30, 0.36, 0.42)),
            ))
            .with_children(|p| {
                p.spawn((
                    HangarLabel::Info,
                    Text::new(info),
                    TextFont::from_font_size(13.5),
                    TextColor(Color::srgb(0.86, 0.90, 0.94)),
                ));
            });

            root.spawn(Node {
                width: Val::Percent(100.0),
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|bar| {
                bar.spawn((
                    Node {
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.06, 0.08, 0.10, 0.82)),
                    BorderColor::all(Color::srgb(0.25, 0.30, 0.36)),
                ))
                .with_children(|h| {
                    h.spawn((
                        Text::new(
                            "ПКМ/ЛКМ: камера  |  Колёсико: зум  |  1-5 / ←→: танк  |  Q / E: карта  |  M: режим (WoT/WT)  |  V: рентген",
                        ),
                        TextFont::from_font_size(12.5),
                        TextColor(Color::srgb(0.75, 0.80, 0.86)),
                    ));
                });

                bar.spawn((
                    Interaction::default(),
                    HangarButton::StartBattle,
                    Node {
                        padding: UiRect::axes(Val::Px(38.0), Val::Px(14.0)),
                        border: UiRect::all(Val::Px(2.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.24, 0.46, 0.22)),
                    BorderColor::all(Color::srgb(0.55, 0.82, 0.48)),
                ))
                .with_children(|btn| {
                    btn.spawn((
                        Text::new("В БОЙ  [Enter / Пробел / ЛКМ]"),
                        TextFont::from_font_size(20.0),
                        TextColor(Color::WHITE),
                    ));
                });
            });
        });
}

fn hangar_camera_pos(view: &HangarViewState) -> Vec3 {
    let focus = Vec3::new(-0.9, 1.0, 0.0);
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
    keys: Res<ButtonInput<KeyCode>>,
    mouse_btn: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut next_state: ResMut<NextState<GameState>>,
    mut selected_tank: ResMut<SelectedTank>,
    mut selected_map: ResMut<SelectedMap>,
    mut mode: ResMut<DamageRulesMode>,
    mut view: ResMut<HangarViewState>,
    mut assets_pair: (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>, Res<VoxelAssets>),
    mut scene_q: ParamSet<(
        Query<(Entity, &mut Transform), With<HangarTurntable>>,
        Query<(Entity, &mut Visibility), With<HangarPreviewTank>>,
        Query<(Entity, &mut Visibility), With<HangarXrayRoot>>,
        Query<&mut Transform, With<HangarCamera>>,
    )>,
    mut labels: Query<(&HangarLabel, &mut Text)>,
    mut btns: Query<(&HangarButton, &Interaction, &mut BackgroundColor, &mut BorderColor)>,
) {
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
    let mut xray_pressed_now = false;
    let mut mode_pressed_now = false;
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
    view.prev_xray_btn = xray_pressed_now;
    view.prev_mode_btn = mode_pressed_now;

    if let Some(i) = new_idx
        && tanks[i].id != selected_tank.0.0
    {
        selected_tank.0 = TankId(tanks[i].id.clone());
    }
    if let Some(i) = new_map_idx
        && maps[i].id != selected_map.0
    {
        selected_map.0 = maps[i].id.clone();
    }
    if toggle_mode {
        *mode = match *mode {
            DamageRulesMode::ArcadeHpAndModules => DamageRulesMode::ModulesOnlyWt,
            DamageRulesMode::ModulesOnlyWt | DamageRulesMode::PrototypeTargets => {
                DamageRulesMode::ArcadeHpAndModules
            }
        };
    }
    if toggle_xray {
        view.xray_armor = !view.xray_armor;
    }
    if start {
        next_state.set(GameState::InBattle);
        return;
    }

    // Орбита камеры мышью.
    let dragging = mouse_btn.pressed(MouseButton::Right)
        || (mouse_btn.pressed(MouseButton::Left) && !ui_hovered);
    if dragging && motion.delta != Vec2::ZERO {
        view.yaw -= motion.delta.x * 0.008;
        view.pitch = (view.pitch + motion.delta.y * 0.007).clamp(0.08, 1.15);
    }
    if scroll.delta.y.abs() > 0.0 {
        view.distance = (view.distance - scroll.delta.y * 0.65).clamp(5.2, 16.0);
    }
    for mut cam_tf in &mut scene_q.p3() {
        let pos = hangar_camera_pos(&view);
        *cam_tf = Transform::from_translation(pos).looking_at(Vec3::new(-0.9, 1.0, 0.0), Vec3::Y);
    }

    let active_idx = tanks
        .iter()
        .position(|t| t.id == selected_tank.0.0)
        .unwrap_or(0);
    let spec = tanks
        .get(active_idx)
        .cloned()
        .unwrap_or_else(|| load_tank_spec("t34_85").expect("t34_85"));

    let active_map_idx = maps
        .iter()
        .position(|m| m.id == selected_map.0)
        .unwrap_or(0);
    let map_spec = maps
        .get(active_map_idx)
        .cloned()
        .or_else(|| load_map_spec("proving_grounds"))
        .expect("default map");

    // Подсветка кнопок.
    let wt_mode = *mode == DamageRulesMode::ModulesOnlyWt;
    for (btn, interaction, mut bg, mut border) in &mut btns {
        match *btn {
            HangarButton::SelectTank(i) => {
                let active = i == active_idx;
                *bg = if active {
                    BackgroundColor(Color::srgb(0.20, 0.34, 0.18))
                } else if *interaction == Interaction::Hovered {
                    BackgroundColor(Color::srgb(0.14, 0.18, 0.22))
                } else {
                    BackgroundColor(Color::srgba(0.06, 0.08, 0.10, 0.86))
                };
                *border = BorderColor::all(if active {
                    Color::srgb(0.55, 0.86, 0.45)
                } else {
                    Color::srgb(0.28, 0.34, 0.40)
                });
            }
            HangarButton::SelectMap(i) => {
                let active = i == active_map_idx;
                *bg = if active {
                    BackgroundColor(Color::srgb(0.16, 0.28, 0.38))
                } else if *interaction == Interaction::Hovered {
                    BackgroundColor(Color::srgb(0.13, 0.18, 0.24))
                } else {
                    BackgroundColor(Color::srgba(0.06, 0.08, 0.10, 0.84))
                };
                *border = BorderColor::all(if active {
                    Color::srgb(0.45, 0.78, 0.96)
                } else {
                    Color::srgb(0.26, 0.32, 0.38)
                });
            }
            HangarButton::ToggleMode => {
                *bg = if *interaction == Interaction::Hovered {
                    if wt_mode {
                        BackgroundColor(Color::srgb(0.56, 0.32, 0.12))
                    } else {
                        BackgroundColor(Color::srgb(0.26, 0.42, 0.22))
                    }
                } else if wt_mode {
                    BackgroundColor(Color::srgb(0.44, 0.24, 0.08))
                } else {
                    BackgroundColor(Color::srgb(0.20, 0.34, 0.18))
                };
                *border = BorderColor::all(if wt_mode {
                    Color::srgb(0.95, 0.62, 0.22)
                } else {
                    Color::srgb(0.55, 0.82, 0.42)
                });
            }
            HangarButton::ToggleXray => {
                *bg = if *interaction == Interaction::Hovered {
                    BackgroundColor(Color::srgb(0.20, 0.30, 0.42))
                } else if view.xray_armor {
                    BackgroundColor(Color::srgb(0.22, 0.38, 0.50))
                } else {
                    BackgroundColor(Color::srgb(0.14, 0.20, 0.28))
                };
            }
            HangarButton::StartBattle => {
                *bg = if *interaction == Interaction::Hovered {
                    BackgroundColor(Color::srgb(0.32, 0.58, 0.28))
                } else {
                    BackgroundColor(Color::srgb(0.24, 0.46, 0.22))
                };
            }
        }
    }

    for (lbl, mut txt) in &mut labels {
        match *lbl {
            HangarLabel::Info => {
                txt.0 = format_hangar_info(&spec, &map_spec, *mode, view.xray_armor);
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
