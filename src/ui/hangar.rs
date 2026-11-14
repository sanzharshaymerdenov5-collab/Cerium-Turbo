//! Ангар: вращающийся 3D-макет Т-34-85 на подиуме, карточка ТТХ и кнопка «В БОЙ».

use bevy::math::{IVec3, UVec3};
use bevy::prelude::*;

use crate::core::{GameState, SelectedMap, SelectedTank};
use crate::tank::{load_tank_spec, spawn_preview_tank};
use crate::voxel::{Material, VoxelAssets, VoxelGrid, voxel_model_bundle};

#[derive(Component)]
pub struct HangarTurntable;

#[derive(Component)]
pub struct StartBattleButton;

pub fn setup_hangar(
    mut commands: Commands,
    selected_tank: Res<SelectedTank>,
    selected_map: Res<SelectedMap>,
    mut meshes: ResMut<Assets<Mesh>>,
    assets: Res<VoxelAssets>,
) {
    commands.insert_resource(ClearColor(Color::srgb(0.11, 0.13, 0.16)));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.85, 0.90, 1.0),
        brightness: 450.0,
        ..default()
    });

    // Подиум из вокселей.
    let mut plinth = VoxelGrid::new(UVec3::new(32, 2, 32), 0.3, Vec3::new(-4.8, -0.6, -4.8));
    plinth.fill_box(IVec3::ZERO, IVec3::new(31, 1, 31), Material::Concrete);

    let spec = load_tank_spec(&selected_tank.0.0)
        .unwrap_or_else(|| load_tank_spec("t34_85").expect("t34_85"));
    let preview = spawn_preview_tank(&mut commands, &mut meshes, &assets, &spec);
    let plinth_ent = commands
        .spawn(voxel_model_bundle(&plinth, &mut meshes, &assets))
        .id();

    commands
        .spawn((
            Name::new("HangarTurntable"),
            HangarTurntable,
            Transform::from_rotation(Quat::from_rotation_y(0.55)),
            Visibility::default(),
            DespawnOnExit(GameState::Hangar),
        ))
        .add_children(&[preview, plinth_ent]);

    commands.spawn((
        DirectionalLight {
            illuminance: 9_500.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(8.0, 12.0, 10.0).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
        DespawnOnExit(GameState::Hangar),
    ));

    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(5.8, 3.3, 7.6).looking_at(Vec3::new(-0.8, 1.1, 0.0), Vec3::Y),
        DespawnOnExit(GameState::Hangar),
    ));

    // UI ангара.
    let info = format!(
        "ТАНК: {} ({})\n\
         Масса: {:.0} т  |  Мощность: {:.0} кВт\n\
         Скорость: +{:.0} / -{:.0} км/ч\n\
         Орудие: {:.0} мм  |  Перезарядка: {:.1} с\n\
         УВН: -{:.0}° .. +{:.0}°  |  Башня: {:.0}°/с\n\
         Снаряд: ТОЛЬКО сплошной AP ({:.0} м/с)\n\
         Пробитие (100 м, 0°): {:.0} мм  |  Рикошет: >{:.0}°\n\
         Карта: Полигон ({})",
        spec.name,
        spec.nation,
        spec.mass_kg / 1000.0,
        spec.engine_power_kw,
        spec.max_speed_kmh,
        spec.reverse_speed_kmh,
        spec.shell.caliber_mm,
        spec.reload_s,
        spec.gun_depression_deg,
        spec.gun_elevation_deg,
        spec.turret_traverse_dps,
        spec.shell.muzzle_velocity,
        spec.shell.pen_at_100m_mm,
        spec.shell.ricochet_deg,
        selected_map.0,
    );

    #[allow(deprecated)]
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::SpaceBetween,
                padding: UiRect::all(Val::Px(24.0)),
                ..default()
            },
            DespawnOnExit(GameState::Hangar),
        ))
        .with_children(|root| {
            // Заголовок.
            root.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(18.0), Val::Px(12.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.06, 0.08, 0.10, 0.88)),
                BorderColor::all(Color::srgb(0.35, 0.48, 0.32)),
            ))
            .with_children(|p| {
                p.spawn((
                    Text::new(
                        "CERIUM-TURBO  •  ЭТАП 2: БОЙ С ИИ-ВРАГАМИ (только сплошной AP)",
                    ),
                    TextFont::from_font_size(20.0),
                    TextColor(Color::srgb(0.92, 0.95, 0.88)),
                ));
            });

            // Карточка ТТХ слева.
            root.spawn((
                Node {
                    max_width: Val::Px(460.0),
                    padding: UiRect::all(Val::Px(18.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.06, 0.08, 0.10, 0.88)),
                BorderColor::all(Color::srgb(0.30, 0.36, 0.42)),
            ))
            .with_children(|p| {
                p.spawn((
                    Text::new(info),
                    TextFont::from_font_size(16.0),
                    TextColor(Color::srgb(0.86, 0.90, 0.94)),
                ));
            });

            // Нижняя панель с кнопкой «В БОЙ».
            root.spawn(Node {
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            })
            .with_children(|bar| {
                bar.spawn((
                    Interaction::default(),
                    StartBattleButton,
                    Node {
                        padding: UiRect::axes(Val::Px(42.0), Val::Px(16.0)),
                        border: UiRect::all(Val::Px(2.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.24, 0.46, 0.22)),
                    BorderColor::all(Color::srgb(0.55, 0.82, 0.48)),
                ))
                .with_children(|btn| {
                    btn.spawn((
                        Text::new("В БОЙ  [Enter / Пробел / ЛКМ]"),
                        TextFont::from_font_size(22.0),
                        TextColor(Color::WHITE),
                    ));
                });
            });
        });
}

#[allow(deprecated)]
pub fn update_hangar(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut next_state: ResMut<NextState<GameState>>,
    mut turntables: Query<&mut Transform, With<HangarTurntable>>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<StartBattleButton>),
    >,
) {
    for mut tf in &mut turntables {
        tf.rotate_y(time.delta_secs() * 0.35);
    }

    let mut start = keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space);
    for (interaction, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                start = true;
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgb(0.32, 0.58, 0.28));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgb(0.24, 0.46, 0.22));
            }
        }
    }
    if start {
        next_state.set(GameState::InBattle);
    }
}
