//! Боевой HUD: телеметрия танка, ХП и кукла модулей (Этап 3), цветной маркер пробития,
//! всплывающий фид урона, дальномер, снайперский прицел и лог попаданий.

use bevy::ecs::message::MessageCursor;
use bevy::prelude::*;

use crate::audio::{AudioSettings, SoundPreset, UiSoundMessage};
use crate::ballistics::penetration::{ImpactOutcome, effective_armor_mm, penetration_at};
use crate::ballistics::{ArmorImpactMessage, HitLog};
use crate::core::{BattleStatus, DamageRulesMode, GameState, Team};
use crate::damage::{
    CrewRole, CrewState, DamageFeed, ModuleKind, ModuleState, PenIndicator, TankCrew,
    TankDestroyedMessage, TankHealth, TankModules, classify_penetration,
};
use crate::tank::{AimTarget, PlayerTank, TankCamera, TankCombat, TankRoot, TankState};

const MAX_ENEMY_MARKERS: usize = 8;

/// Кратковременная визуальная вспышка хит-маркера на перекрестии при попадании.
#[derive(Resource, Debug, Default)]
pub struct HudHitMarker {
    pub timer: f32,
    pub is_pen: bool,
    pub is_kill: bool,
    pub badge: String,
}

const ALL_MODULES: [ModuleKind; 7] = [
    ModuleKind::LeftTrack,
    ModuleKind::RightTrack,
    ModuleKind::Engine,
    ModuleKind::TurretRing,
    ModuleKind::GunBreech,
    ModuleKind::AmmoRack,
    ModuleKind::FuelTank,
];

const ALL_CREW: [CrewRole; 4] = [
    CrewRole::Driver,
    CrewRole::Gunner,
    CrewRole::Commander,
    CrewRole::Loader,
];

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum HudText {
    Telemetry,
    Controls,
    Crosshair,
    HitFlash,
    Aim,
    DamageFeed,
    HitLog,
    HpHeader,
    WtCrewHeader,
    Consumables,
    BattleBanner,
    EnemyMarker(usize),
    ModuleBadge(ModuleKind),
    CrewBadge(CrewRole),
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum HudPanel {
    HullHp,
    WtStatus,
    HpBarFill,
    ReloadBarFill,
    FireAlert,
    WtFireAlert,
    ModuleBox(ModuleKind),
    CrewBox(CrewRole),
    BattleBanner,
    EnemyMarker(usize),
    SniperOverlay,
}

pub fn setup_hud(mut commands: Commands) {
    commands.insert_resource(HudHitMarker::default());
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::SpaceBetween,
                padding: UiRect::all(Val::Px(16.0)),
                ..default()
            },
            DespawnOnExit(GameState::InBattle),
        ))
        .with_children(|root| {
            // Верхняя строка: слева телеметрия + шкала перезарядки, справа подсказка управления.
            root.spawn(Node {
                width: Val::Percent(100.0),
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::FlexStart,
                ..default()
            })
            .with_children(|top| {
                top.spawn((
                    Node {
                        padding: UiRect::all(Val::Px(12.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(6.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.05, 0.07, 0.09, 0.78)),
                    BorderColor::all(Color::srgb(0.32, 0.44, 0.30)),
                ))
                .with_children(|p| {
                    p.spawn((
                        HudText::Telemetry,
                        Text::new("Т-34-85"),
                        TextFont::from_font_size(15.0),
                        TextColor(Color::srgb(0.90, 0.95, 0.88)),
                    ));
                    // Полоса прогресса перезарядки орудия.
                    p.spawn((
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Px(6.0),
                            border: UiRect::all(Val::Px(1.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.10, 0.12, 0.14, 0.90)),
                        BorderColor::all(Color::srgb(0.24, 0.30, 0.34)),
                    ))
                    .with_children(|bar| {
                        bar.spawn((
                            HudPanel::ReloadBarFill,
                            Node {
                                width: Val::Percent(100.0),
                                height: Val::Percent(100.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.28, 0.84, 0.40)),
                            Visibility::Inherited,
                        ));
                    });
                });

                top.spawn((
                    Node {
                        padding: UiRect::all(Val::Px(12.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.05, 0.07, 0.09, 0.72)),
                    BorderColor::all(Color::srgb(0.28, 0.32, 0.38)),
                ))
                .with_children(|p| {
                    p.spawn((
                        HudText::Controls,
                        Text::new(
                            "WASD: ход/поворот  |  Мышь: башня/прицел  |  ЛКМ/Пробел: огонь\n\
                             Shift: снайперский  |  5: Ремкомпл.  |  6: Огнетуш.  |  R: на гусеницы  |  N: звук  |  Tab: ангар",
                        ),
                        TextFont::from_font_size(12.5),
                        TextColor(Color::srgb(0.78, 0.83, 0.88)),
                    ));
                });
            });

            // Центр: цветное перекрестие, хит-маркер, дальномер и всплывающий фид урона.
            root.spawn(Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(4.0),
                ..default()
            })
            .with_children(|center| {
                center.spawn((
                    HudText::Crosshair,
                    Text::new("+"),
                    TextFont::from_font_size(30.0),
                    TextColor(Color::srgba(0.95, 1.0, 0.4, 0.92)),
                ));
                center.spawn((
                    HudText::HitFlash,
                    Text::new(""),
                    TextFont::from_font_size(13.5),
                    TextColor(Color::srgb(1.0, 0.72, 0.24)),
                ));
                center.spawn((
                    HudText::Aim,
                    Text::new(""),
                    TextFont::from_font_size(15.0),
                    TextColor(Color::srgb(0.95, 0.96, 0.72)),
                ));
                center.spawn((
                    HudText::DamageFeed,
                    Text::new(""),
                    TextFont::from_font_size(16.0),
                    TextColor(Color::srgb(1.0, 0.72, 0.25)),
                ));
            });

            // Нижняя строка: слева панель ХП/модулей (WoT) или панель экипажа/модулей (WT), справа журнал попаданий.
            root.spawn(Node {
                width: Val::Percent(100.0),
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::FlexEnd,
                column_gap: Val::Px(12.0),
                ..default()
            })
            .with_children(|bottom| {
                // 1. Панель ХП и кукла модулей (режим WoT — Этап 3).
                bottom
                    .spawn((
                        HudPanel::HullHp,
                        Node {
                            width: Val::Px(360.0),
                            padding: UiRect::all(Val::Px(10.0)),
                            border: UiRect::all(Val::Px(1.0)),
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(6.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.05, 0.07, 0.09, 0.82)),
                        BorderColor::all(Color::srgb(0.32, 0.46, 0.30)),
                        Visibility::Visible,
                    ))
                    .with_children(|hp| {
                        hp.spawn((
                            HudText::HpHeader,
                            Text::new("ПРОЧНОСТЬ: 850 / 850 HP"),
                            TextFont::from_font_size(15.0),
                            TextColor(Color::srgb(0.88, 0.96, 0.86)),
                        ));

                        // Полоса прочности.
                        hp.spawn((
                            Node {
                                width: Val::Percent(100.0),
                                height: Val::Px(14.0),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.12, 0.05, 0.05, 0.9)),
                            BorderColor::all(Color::srgb(0.25, 0.30, 0.35)),
                        ))
                        .with_children(|bar| {
                            bar.spawn((
                                HudPanel::HpBarFill,
                                Node {
                                    width: Val::Percent(100.0),
                                    height: Val::Percent(100.0),
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(0.24, 0.80, 0.34)),
                                Visibility::Inherited,
                            ));
                        });

                        // Предупреждение о пожаре.
                        hp.spawn((
                            HudPanel::FireAlert,
                            Node {
                                display: Display::None,
                                padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.55, 0.12, 0.06, 0.94)),
                            BorderColor::all(Color::srgb(1.0, 0.45, 0.20)),
                            Visibility::Hidden,
                        ))
                        .with_children(|f| {
                            f.spawn((
                                Text::new("ПОЖАР! Нажмите [6] Огнетушитель"),
                                TextFont::from_font_size(13.0),
                                TextColor(Color::srgb(1.0, 0.92, 0.80)),
                            ));
                        });

                        // Сетка модулей (кукла повреждений).
                        hp.spawn(Node {
                            width: Val::Percent(100.0),
                            flex_wrap: FlexWrap::Wrap,
                            column_gap: Val::Px(5.0),
                            row_gap: Val::Px(5.0),
                            ..default()
                        })
                        .with_children(|grid| {
                            for kind in ALL_MODULES {
                                grid.spawn((
                                    HudPanel::ModuleBox(kind),
                                    Node {
                                        padding: UiRect::axes(Val::Px(7.0), Val::Px(3.0)),
                                        border: UiRect::all(Val::Px(1.0)),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgba(0.08, 0.16, 0.10, 0.85)),
                                    BorderColor::all(Color::srgb(0.26, 0.56, 0.30)),
                                    Visibility::Inherited,
                                ))
                                .with_children(|b| {
                                    b.spawn((
                                        HudText::ModuleBadge(kind),
                                        Text::new(kind.ru_short()),
                                        TextFont::from_font_size(12.0),
                                        TextColor(Color::srgb(0.80, 0.96, 0.82)),
                                    ));
                                });
                            }
                        });

                        hp.spawn((
                            HudText::Consumables,
                            Text::new("[5] Ремкомплект (2)  |  [6] Огнетушитель (2)"),
                            TextFont::from_font_size(12.0),
                            TextColor(Color::srgb(0.76, 0.82, 0.88)),
                        ));
                    });

                // 2. Панель экипажа и модулей без общего ХП (режим War Thunder — Этап 6).
                bottom
                    .spawn((
                        HudPanel::WtStatus,
                        Node {
                            display: Display::None,
                            width: Val::Px(390.0),
                            padding: UiRect::all(Val::Px(10.0)),
                            border: UiRect::all(Val::Px(1.5)),
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(6.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.05, 0.07, 0.09, 0.86)),
                        BorderColor::all(Color::srgb(0.86, 0.54, 0.18)),
                        Visibility::Hidden,
                    ))
                    .with_children(|wt| {
                        wt.spawn((
                            HudText::WtCrewHeader,
                            Text::new("ЭКИПАЖ: 4 / 4  •  WAR THUNDER (БЕЗ ХП)"),
                            TextFont::from_font_size(14.5),
                            TextColor(Color::srgb(1.0, 0.86, 0.44)),
                        ));

                        wt.spawn((
                            HudPanel::WtFireAlert,
                            Node {
                                display: Display::None,
                                padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.55, 0.12, 0.06, 0.94)),
                            BorderColor::all(Color::srgb(1.0, 0.45, 0.20)),
                            Visibility::Hidden,
                        ))
                        .with_children(|f| {
                            f.spawn((
                                Text::new("ПОЖАР! Выводит экипаж — [6] Огнетуш."),
                                TextFont::from_font_size(12.5),
                                TextColor(Color::srgb(1.0, 0.92, 0.80)),
                            ));
                        });

                        // Сетка состояния экипажа (< 2 живых = уничтожен).
                        wt.spawn(Node {
                            width: Val::Percent(100.0),
                            flex_wrap: FlexWrap::Wrap,
                            column_gap: Val::Px(5.0),
                            row_gap: Val::Px(5.0),
                            ..default()
                        })
                        .with_children(|crew_row| {
                            for role in ALL_CREW {
                                crew_row
                                    .spawn((
                                        HudPanel::CrewBox(role),
                                        Node {
                                            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                                            border: UiRect::all(Val::Px(1.0)),
                                            ..default()
                                        },
                                        BackgroundColor(Color::srgba(0.08, 0.18, 0.12, 0.90)),
                                        BorderColor::all(Color::srgb(0.32, 0.72, 0.36)),
                                        Visibility::Inherited,
                                    ))
                                    .with_children(|b| {
                                        b.spawn((
                                            HudText::CrewBadge(role),
                                            Text::new(role.ru_short()),
                                            TextFont::from_font_size(12.0),
                                            TextColor(Color::srgb(0.84, 0.98, 0.86)),
                                        ));
                                    });
                            }
                        });

                        // Сетка внутренних и ходовых модулей.
                        wt.spawn(Node {
                            width: Val::Percent(100.0),
                            flex_wrap: FlexWrap::Wrap,
                            column_gap: Val::Px(5.0),
                            row_gap: Val::Px(5.0),
                            ..default()
                        })
                        .with_children(|grid| {
                            for kind in ALL_MODULES {
                                grid.spawn((
                                    HudPanel::ModuleBox(kind),
                                    Node {
                                        padding: UiRect::axes(Val::Px(7.0), Val::Px(3.0)),
                                        border: UiRect::all(Val::Px(1.0)),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgba(0.08, 0.16, 0.10, 0.85)),
                                    BorderColor::all(Color::srgb(0.26, 0.56, 0.30)),
                                    Visibility::Inherited,
                                ))
                                .with_children(|b| {
                                    b.spawn((
                                        HudText::ModuleBadge(kind),
                                        Text::new(kind.ru_short()),
                                        TextFont::from_font_size(12.0),
                                        TextColor(Color::srgb(0.80, 0.96, 0.82)),
                                    ));
                                });
                            }
                        });

                        wt.spawn((
                            HudText::Consumables,
                            Text::new("[5] Ремкомплект (2)  |  [6] Огнетушитель (2)"),
                            TextFont::from_font_size(12.0),
                            TextColor(Color::srgb(0.76, 0.82, 0.88)),
                        ));
                    });

                // Журнал попаданий.
                bottom
                    .spawn((
                        Node {
                            min_width: Val::Px(420.0),
                            max_width: Val::Px(640.0),
                            padding: UiRect::all(Val::Px(12.0)),
                            border: UiRect::all(Val::Px(1.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.05, 0.07, 0.09, 0.78)),
                        BorderColor::all(Color::srgb(0.30, 0.36, 0.42)),
                    ))
                    .with_children(|p| {
                        p.spawn((
                            HudText::HitLog,
                            Text::new("Журнал попаданий (уничтожьте врагов ИИ и испытайте бронестенды)"),
                            TextFont::from_font_size(14.0),
                            TextColor(Color::srgb(0.88, 0.91, 0.95)),
                        ));
                    });
            });
        });

    // Баннер победы/поражения по центру.
    commands
        .spawn((
            HudPanel::BattleBanner,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(20.0),
                left: Val::Percent(25.0),
                width: Val::Percent(50.0),
                padding: UiRect::axes(Val::Px(24.0), Val::Px(16.0)),
                border: UiRect::all(Val::Px(2.0)),
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.06, 0.08, 0.10, 0.90)),
            BorderColor::all(Color::srgb(0.90, 0.75, 0.25)),
            Visibility::Hidden,
            DespawnOnExit(GameState::InBattle),
        ))
        .with_children(|b| {
            b.spawn((
                HudText::BattleBanner,
                Text::new(""),
                TextFont::from_font_size(24.0),
                TextColor(Color::WHITE),
            ));
        });

    // Пул 2D-маркеров над вражескими танками.
    for i in 0..MAX_ENEMY_MARKERS {
        commands
            .spawn((
                HudPanel::EnemyMarker(i),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(0.0),
                    padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.18, 0.05, 0.04, 0.76)),
                BorderColor::all(Color::srgb(0.92, 0.35, 0.20)),
                Visibility::Hidden,
                DespawnOnExit(GameState::InBattle),
            ))
            .with_children(|m| {
                m.spawn((
                    HudText::EnemyMarker(i),
                    Text::new(""),
                    TextFont::from_font_size(12.0),
                    TextColor(Color::srgb(1.0, 0.80, 0.72)),
                ));
            });
    }

    // Рамка снайперского прицела (по умолчанию скрыта).
    commands.spawn((
        HudPanel::SniperOverlay,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            border: UiRect::all(Val::Px(48.0)),
            ..default()
        },
        BorderColor::all(Color::srgba(0.02, 0.03, 0.04, 0.82)),
        Visibility::Hidden,
        DespawnOnExit(GameState::InBattle),
    ));
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn update_hud(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut next_state: ResMut<NextState<GameState>>,
    state: (
        Res<DamageRulesMode>,
        Res<AimTarget>,
        Res<HitLog>,
        Res<DamageFeed>,
        Res<BattleStatus>,
    ),
    mut hit_marker: ResMut<HudHitMarker>,
    mut msg_resources: (
        Option<Res<Messages<ArmorImpactMessage>>>,
        Option<Res<Messages<TankDestroyedMessage>>>,
        Option<ResMut<Messages<UiSoundMessage>>>,
        Option<Res<AudioSettings>>,
    ),
    mut cursors: (
        Local<MessageCursor<ArmorImpactMessage>>,
        Local<MessageCursor<TankDestroyedMessage>>,
    ),
    player_q: Query<
        (
            &GlobalTransform,
            &TankRoot,
            &TankState,
            &TankCombat,
            Option<&TankHealth>,
            Option<&TankModules>,
            Option<&TankCrew>,
        ),
        With<PlayerTank>,
    >,
    all_combat: Query<(&GlobalTransform, &TankCombat, Option<&TankHealth>)>,
    cams: Query<(&Camera, &GlobalTransform, &TankCamera), With<Camera3d>>,
    mut texts: Query<(&HudText, &mut Text, Option<&mut TextColor>)>,
    mut panels: Query<(
        &HudPanel,
        &mut Node,
        &mut Visibility,
        Option<&mut BackgroundColor>,
        Option<&mut BorderColor>,
    )>,
) {
    let (mode, aim, log, feed, status) = state;
    if keys.just_pressed(KeyCode::Tab) {
        if let Some(ref mut ui_s) = msg_resources.2 {
            ui_s.write(UiSoundMessage(SoundPreset::UiClick));
        }
        next_state.set(GameState::Hangar);
        return;
    }

    let dt = time.delta_secs();
    if hit_marker.timer > 0.0 {
        hit_marker.timer = (hit_marker.timer - dt).max(0.0);
    }

    if let Some(ref msgs) = msg_resources.0 {
        for msg in cursors.0.read(msgs) {
            if msg.shooter_team == Some(Team::Player) && msg.owner_tank.is_some() {
                match msg.outcome {
                    ImpactOutcome::Penetration { .. } => {
                        hit_marker.timer = 0.45;
                        hit_marker.is_pen = true;
                        hit_marker.is_kill = false;
                        hit_marker.badge = "ПРОБИТИЕ!".to_string();
                    }
                    ImpactOutcome::Ricochet => {
                        hit_marker.timer = 0.35;
                        hit_marker.is_pen = false;
                        hit_marker.is_kill = false;
                        hit_marker.badge = "РИКОШЕТ".to_string();
                    }
                    ImpactOutcome::NoPenetration { .. } => {
                        hit_marker.timer = 0.35;
                        hit_marker.is_pen = false;
                        hit_marker.is_kill = false;
                        hit_marker.badge = "НЕ ПРОБИТ".to_string();
                    }
                }
            }
        }
    }
    if let Some(ref msgs) = msg_resources.1 {
        for msg in cursors.1.read(msgs) {
            if msg.team == Team::Enemy {
                hit_marker.timer = 0.85;
                hit_marker.is_pen = true;
                hit_marker.is_kill = true;
                hit_marker.badge = if msg.ammo_detonated {
                    "ПОДРЫВ БК!".to_string()
                } else {
                    "УНИЧТОЖЕН!".to_string()
                };
            }
        }
    }

    let cam_opt = cams.iter().next();
    let sniper = cam_opt.is_some_and(|(_, _, c)| c.sniper);

    let mut enemy_total = 0usize;
    let mut enemy_alive = 0usize;
    let mut alive_enemies = Vec::new();
    for (gt, c, h) in &all_combat {
        if c.team == Team::Enemy {
            enemy_total += 1;
            if c.alive {
                enemy_alive += 1;
                alive_enemies.push((gt.translation(), c, h.copied()));
            }
        }
    }

    let player = player_q.single().ok();
    let player_pos = player.map(|(gt, _, _, _, _, _, _)| gt.translation());
    let pen_ind = match (player, aim.target_plate) {
        (Some((_, root, _, _, _, _, _)), Some(plate)) => classify_penetration(
            &root.spec.shell,
            aim.distance,
            plate.thickness_mm,
            plate.angle_deg,
        ),
        _ => PenIndicator::None,
    };

    // Подготовка текстов.
    let telemetry_str = if let Some((_, root, st, combat, health_opt, mods_opt, crew_opt)) = player {
        let gun_status = if !combat.alive {
            "ТАНК УНИЧТОЖЕН".to_string()
        } else if combat.gun_jammed {
            if crew_opt.is_some_and(|cr| cr.state(CrewRole::Gunner) != CrewState::Alive)
                && *mode == DamageRulesMode::ModulesOnlyWt
            {
                "НАВОДЧИК ВЫБИТ!".to_string()
            } else {
                "КАЗЁННИК ВЫБИТ!".to_string()
            }
        } else if st.reload_timer <= 0.01 {
            "ГОТОВО К ВЫСТРЕЛУ".to_string()
        } else {
            format!("ПЕРЕЗАРЯДКА {:.1} с", st.reload_timer)
        };
        let scope_str = if sniper { "СНАЙПЕР (x3)" } else { "АРКАДНЫЙ" };
        let hp_line = match (*mode, health_opt) {
            (DamageRulesMode::ArcadeHpAndModules, Some(h)) => {
                format!("Прочность: {:.0}/{:.0} HP", h.hp, h.max_hp)
            }
            (DamageRulesMode::ModulesOnlyWt, _) => {
                let alive_c = crew_opt.map_or(4, |c| c.alive_count());
                let total_c = crew_opt.map_or(4, |c| c.total_count());
                format!("Режим WT (без общего ХП)  •  Экипаж: {alive_c}/{total_c}")
            }
            _ => {
                let left = combat.prototype_max_hits.saturating_sub(combat.prototype_hits);
                format!("Прочность (прототип): {}/{}", left, combat.prototype_max_hits)
            }
        };
        let ice_warn = if st.surface_traction > 0.0 && st.surface_traction < 0.5 {
            "  •  ГОЛОЛЁД!"
        } else {
            ""
        };
        let fire_warn = if mods_opt.is_some_and(|m| m.fire_timer > 0.0) {
            "  •  ПОЖАР!"
        } else {
            ""
        };
        format!(
            "{} [{}]  |  Скорость: {:+.1} км/ч  |  Катки: {}/8{}{}\n\
             {}  |  Враги: {}/{}\n\
             Орудие {:.0} мм: {}  |  Прицел: {}\n\
             Снаряд: ТОЛЬКО сплошной SolidAP • Выстрелов: {} (пробитий: {})",
            root.spec.name,
            mode.ru_label(),
            st.speed_kmh,
            st.grounded_wheels,
            ice_warn,
            fire_warn,
            hp_line,
            enemy_alive,
            enemy_total,
            root.spec.shell.caliber_mm,
            gun_status,
            scope_str,
            log.shots,
            log.penetrations,
        )
    } else {
        String::new()
    };

    let controls_str = format!(
        "WASD: ход/поворот  |  Мышь: башня/прицел  |  ЛКМ/Пробел: огонь\n\
         Shift: снайпер  |  5: Ремкомпл.  |  6: Огнетуш.  |  R: на гусеницы  |  Звук [N]: {}  |  Tab: ангар",
        if msg_resources.3.as_ref().is_some_and(|a| a.muted) {
            "ВЫКЛ"
        } else {
            "ВКЛ"
        }
    );

    let aim_str = {
        let err = aim.point.distance(aim.gun_point);
        let spread = if err < 0.8 { "СВЕДЁН" } else { "НАВОДКА..." };
        match (&aim.target_name, aim.target_plate, player) {
            (Some(name), Some(plate), Some((_, root, _, _, _, _, _))) => {
                let eff = effective_armor_mm(&root.spec.shell, plate.thickness_mm, plate.angle_deg);
                let pen = penetration_at(&root.spec.shell, aim.distance);
                format!(
                    "{:.0} м  •  {} ({:.0}мм @ {:.0}° -> эфф. {:.0}мм / проб. {:.0}мм)  [{}]",
                    aim.distance, name, plate.thickness_mm, plate.angle_deg, eff, pen, spread
                )
            }
            (Some(name), _, _) => format!("{:.0} м  •  {}  [{}]", aim.distance, name, spread),
            (None, _, _) => format!("{:.0} м  [{}]", aim.distance, spread),
        }
    };

    let (feed_str, feed_crit) = if feed.items.is_empty() {
        (String::new(), false)
    } else {
        let crit = feed.items.iter().any(|i| i.critical);
        let s = feed
            .items
            .iter()
            .map(|i| i.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        (s, crit)
    };

    let log_str = if log.entries.is_empty() {
        "Журнал попаданий: ищите и уничтожайте врагов или проверьте бронестенды".to_string()
    } else {
        let mut s = String::from("Журнал попаданий:\n");
        for e in log.entries.iter().rev() {
            s.push_str("• ");
            s.push_str(&e.text);
            s.push('\n');
        }
        s.trim_end().to_string()
    };

    let hp_opt = player.and_then(|(_, _, _, _, h, _, _)| h);
    let mods_opt = player.and_then(|(_, _, _, _, _, m, _)| m);
    let crew_opt = player.and_then(|(_, _, _, _, _, _, c)| c);
    let reload_info = player.map(|(_, root, st, combat, _, _, _)| {
        let total = root.spec.reload_s.max(0.1);
        let progress = if !combat.alive || combat.gun_jammed {
            0.0
        } else if st.reload_timer <= 0.01 {
            1.0
        } else {
            (1.0 - st.reload_timer / total).clamp(0.0, 1.0)
        };
        (progress, combat.alive && !combat.gun_jammed && st.reload_timer <= 0.01, combat.gun_jammed || !combat.alive)
    });

    // Обновление текстовых элементов.
    for (tag, mut txt, color_opt) in &mut texts {
        match *tag {
            HudText::Telemetry => txt.0 = telemetry_str.clone(),
            HudText::Controls => txt.0 = controls_str.clone(),
            HudText::Crosshair => {
                if hit_marker.timer > 0.0 {
                    txt.0 = if hit_marker.is_pen { "×" } else { "◈" }.to_string();
                    if let Some(mut c) = color_opt {
                        c.0 = if hit_marker.is_kill {
                            Color::srgb(1.0, 0.25, 0.20)
                        } else if hit_marker.is_pen {
                            Color::srgb(1.0, 0.84, 0.22)
                        } else {
                            Color::srgb(0.72, 0.88, 1.0)
                        };
                    }
                } else {
                    txt.0 = "+".to_string();
                    if let Some(mut c) = color_opt {
                        c.0 = match pen_ind {
                            PenIndicator::Green => Color::srgb(0.28, 0.96, 0.36),
                            PenIndicator::Yellow => Color::srgb(0.98, 0.82, 0.22),
                            PenIndicator::Red => Color::srgb(0.96, 0.28, 0.26),
                            PenIndicator::None => Color::srgba(0.95, 1.0, 0.4, 0.92),
                        };
                    }
                }
            }
            HudText::HitFlash => {
                if hit_marker.timer > 0.0 {
                    txt.0 = hit_marker.badge.clone();
                    if let Some(mut c) = color_opt {
                        c.0 = if hit_marker.is_kill {
                            Color::srgb(1.0, 0.36, 0.26)
                        } else if hit_marker.is_pen {
                            Color::srgb(1.0, 0.88, 0.32)
                        } else {
                            Color::srgb(0.76, 0.88, 1.0)
                        };
                    }
                } else {
                    txt.0.clear();
                }
            }
            HudText::Aim => txt.0 = aim_str.clone(),
            HudText::DamageFeed => {
                txt.0 = feed_str.clone();
                if let Some(mut c) = color_opt {
                    c.0 = if feed_crit {
                        Color::srgb(1.0, 0.56, 0.20)
                    } else {
                        Color::srgb(0.88, 0.96, 0.84)
                    };
                }
            }
            HudText::HitLog => txt.0 = log_str.clone(),
            HudText::HpHeader => {
                if let Some(h) = hp_opt {
                    txt.0 = format!("ПРОЧНОСТЬ: {:.0} / {:.0} HP", h.hp, h.max_hp);
                }
            }
            HudText::WtCrewHeader => {
                if let Some(crew) = crew_opt {
                    let alive_c = crew.alive_count();
                    let total_c = crew.total_count();
                    txt.0 = if let Some(rep_role) = crew.replacing {
                        format!(
                            "ЭКИПАЖ: {alive_c}/{total_c}  •  ПЕРЕСАДКА {} ({:.1}с)",
                            rep_role.ru_short(),
                            crew.replacement_timer.max(0.0)
                        )
                    } else {
                        format!("ЭКИПАЖ: {alive_c} / {total_c}  •  WAR THUNDER (БЕЗ ХП)")
                    };
                }
            }
            HudText::Consumables => {
                if let Some(mods) = mods_opt {
                    txt.0 = format!(
                        "[5] Ремкомплект ({})  |  [6] Огнетушитель ({})",
                        mods.repair_charges, mods.extinguisher_charges
                    );
                }
            }
            HudText::BattleBanner => match *status {
                BattleStatus::Ongoing => {}
                BattleStatus::Victory => {
                    txt.0 = "ПОБЕДА! Все ИИ-танки уничтожены • [Tab] — вернуться в ангар"
                        .to_string();
                }
                BattleStatus::Defeat => {
                    txt.0 = "ПОРАЖЕНИЕ! Ваш танк подбит • [Tab] — вернуться в ангар".to_string();
                }
            },
            HudText::ModuleBadge(kind) => {
                if let Some(mods) = mods_opt {
                    let m = mods.modules.iter().find(|m| m.kind == kind);
                    let st = m.map_or(ModuleState::Intact, |x| x.state);
                    let rem = m.map_or(0.0, |x| x.repair_timer);
                    txt.0 = match st {
                        ModuleState::Intact => kind.ru_short().to_string(),
                        ModuleState::Damaged => format!("{} !", kind.ru_short()),
                        ModuleState::Destroyed => {
                            if rem > 0.05 {
                                format!("{} {:.1}с", kind.ru_short(), rem)
                            } else {
                                format!("{} X", kind.ru_short())
                            }
                        }
                    };
                    if let Some(mut c) = color_opt {
                        c.0 = match st {
                            ModuleState::Intact => Color::srgb(0.80, 0.96, 0.82),
                            ModuleState::Damaged => Color::srgb(1.0, 0.88, 0.35),
                            ModuleState::Destroyed => Color::srgb(1.0, 0.55, 0.52),
                        };
                    }
                }
            }
            HudText::CrewBadge(role) => {
                if let Some(crew) = crew_opt
                    && let Some(member) = crew.member(role)
                {
                    txt.0 = match member.state {
                        CrewState::Alive if member.is_backup => {
                            format!("{}*", role.ru_short())
                        }
                        CrewState::Alive => role.ru_short().to_string(),
                        CrewState::Moved => format!("{} ->", role.ru_short()),
                        CrewState::Unconscious => format!("{} X", role.ru_short()),
                    };
                    if let Some(mut c) = color_opt {
                        c.0 = match member.state {
                            CrewState::Alive if member.is_backup => {
                                Color::srgb(0.65, 0.98, 0.86)
                            }
                            CrewState::Alive => Color::srgb(0.84, 0.98, 0.86),
                            CrewState::Moved => Color::srgb(1.0, 0.86, 0.42),
                            CrewState::Unconscious => Color::srgb(1.0, 0.52, 0.50),
                        };
                    }
                }
            }
            HudText::EnemyMarker(idx) => {
                if let Some(&(epos, ec, eh)) = alive_enemies.get(idx) {
                    let dist = player_pos.map_or(0.0, |p| p.distance(epos));
                    txt.0 = match (*mode, eh) {
                        (DamageRulesMode::ArcadeHpAndModules, Some(h)) => {
                            format!("▼ {} [{:.0}/{:.0} HP] {:.0}м", ec.tank_name, h.hp, h.max_hp, dist)
                        }
                        (DamageRulesMode::ModulesOnlyWt, _) => {
                            format!("▼ {}  {:.0}м", ec.tank_name, dist)
                        }
                        _ => {
                            let left = ec.prototype_max_hits.saturating_sub(ec.prototype_hits);
                            format!("▼ {} [{}/{}] {:.0}м", ec.tank_name, left, ec.prototype_max_hits, dist)
                        }
                    };
                }
            }
        }
    }

    // Обновление панелей и маркеров.
    let show_hp_panel = *mode == DamageRulesMode::ArcadeHpAndModules;
    let show_wt_panel = *mode == DamageRulesMode::ModulesOnlyWt;
    let on_fire = mods_opt.is_some_and(|m| m.fire_timer > 0.0);
    let fire_pulse = (time.elapsed_secs() * 12.0).sin() * 0.5 + 0.5;

    for (panel, mut node, mut vis, bg_opt, border_opt) in &mut panels {
        match *panel {
            HudPanel::SniperOverlay => {
                *vis = if sniper {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                };
            }
            HudPanel::HullHp => {
                node.display = if show_hp_panel {
                    Display::Flex
                } else {
                    Display::None
                };
                *vis = if show_hp_panel {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                };
            }
            HudPanel::WtStatus => {
                node.display = if show_wt_panel {
                    Display::Flex
                } else {
                    Display::None
                };
                *vis = if show_wt_panel {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                };
            }
            HudPanel::HpBarFill => {
                if let Some(h) = hp_opt {
                    let ratio = h.ratio();
                    node.width = Val::Percent(ratio * 100.0);
                    if let Some(mut bg) = bg_opt {
                        bg.0 = if ratio > 0.6 {
                            Color::srgb(0.24, 0.80, 0.34)
                        } else if ratio > 0.25 {
                            Color::srgb(0.92, 0.72, 0.16)
                        } else {
                            Color::srgb(0.90, 0.22, 0.20)
                        };
                    }
                }
            }
            HudPanel::ReloadBarFill => {
                if let Some((progress, ready, broken)) = reload_info {
                    node.width = Val::Percent(if broken { 100.0 } else { progress * 100.0 });
                    if let Some(mut bg) = bg_opt {
                        bg.0 = if broken {
                            Color::srgb(0.88, 0.24, 0.20)
                        } else if ready {
                            Color::srgb(0.26, 0.86, 0.42)
                        } else {
                            Color::srgb(0.96, 0.72, 0.22)
                        };
                    }
                }
            }
            HudPanel::FireAlert => {
                let show = show_hp_panel && on_fire;
                node.display = if show { Display::Flex } else { Display::None };
                *vis = if show {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                };
                if show && let Some(mut border) = border_opt {
                    *border = BorderColor::all(Color::srgb(
                        1.0,
                        0.28 + fire_pulse * 0.45,
                        0.12 + fire_pulse * 0.25,
                    ));
                }
            }
            HudPanel::WtFireAlert => {
                let show = show_wt_panel && on_fire;
                node.display = if show { Display::Flex } else { Display::None };
                *vis = if show {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                };
                if show && let Some(mut border) = border_opt {
                    *border = BorderColor::all(Color::srgb(
                        1.0,
                        0.28 + fire_pulse * 0.45,
                        0.12 + fire_pulse * 0.25,
                    ));
                }
            }
            HudPanel::ModuleBox(kind) => {
                if let Some(mods) = mods_opt {
                    let st = mods.state(kind);
                    if let Some(mut bg) = bg_opt {
                        bg.0 = match st {
                            ModuleState::Intact => Color::srgba(0.08, 0.16, 0.10, 0.85),
                            ModuleState::Damaged => Color::srgba(0.24, 0.18, 0.04, 0.90),
                            ModuleState::Destroyed => Color::srgba(0.28, 0.06, 0.06, 0.92),
                        };
                    }
                    if let Some(mut border) = border_opt {
                        *border = match st {
                            ModuleState::Intact => BorderColor::all(Color::srgb(0.26, 0.56, 0.30)),
                            ModuleState::Damaged => BorderColor::all(Color::srgb(0.95, 0.72, 0.16)),
                            ModuleState::Destroyed => BorderColor::all(Color::srgb(0.95, 0.25, 0.22)),
                        };
                    }
                }
            }
            HudPanel::CrewBox(role) => {
                if let Some(crew) = crew_opt
                    && let Some(member) = crew.member(role)
                {
                    if let Some(mut bg) = bg_opt {
                        bg.0 = match member.state {
                            CrewState::Alive if member.is_backup => {
                                Color::srgba(0.06, 0.20, 0.18, 0.90)
                            }
                            CrewState::Alive => Color::srgba(0.08, 0.18, 0.12, 0.90),
                            CrewState::Moved => Color::srgba(0.24, 0.18, 0.04, 0.90),
                            CrewState::Unconscious => Color::srgba(0.30, 0.06, 0.06, 0.92),
                        };
                    }
                    if let Some(mut border) = border_opt {
                        *border = match member.state {
                            CrewState::Alive if member.is_backup => {
                                BorderColor::all(Color::srgb(0.30, 0.86, 0.68))
                            }
                            CrewState::Alive => BorderColor::all(Color::srgb(0.32, 0.72, 0.36)),
                            CrewState::Moved => BorderColor::all(Color::srgb(0.95, 0.72, 0.16)),
                            CrewState::Unconscious => {
                                BorderColor::all(Color::srgb(0.96, 0.26, 0.22))
                            }
                        };
                    }
                }
            }
            HudPanel::BattleBanner => match *status {
                BattleStatus::Ongoing => *vis = Visibility::Hidden,
                BattleStatus::Victory => {
                    *vis = Visibility::Visible;
                    if let Some(mut border) = border_opt {
                        *border = BorderColor::all(Color::srgb(0.35, 0.88, 0.40));
                    }
                }
                BattleStatus::Defeat => {
                    *vis = Visibility::Visible;
                    if let Some(mut border) = border_opt {
                        *border = BorderColor::all(Color::srgb(0.92, 0.28, 0.24));
                    }
                }
            },
            HudPanel::EnemyMarker(idx) => {
                let Some(&(epos, _, _)) = alive_enemies.get(idx) else {
                    *vis = Visibility::Hidden;
                    continue;
                };
                let Some((cam, cam_gt, _)) = cam_opt else {
                    *vis = Visibility::Hidden;
                    continue;
                };
                if let Ok(vp) = cam.world_to_viewport(cam_gt, epos + Vec3::Y * 2.9) {
                    node.left = Val::Px((vp.x - 56.0).max(4.0));
                    node.top = Val::Px((vp.y - 26.0).max(4.0));
                    *vis = Visibility::Visible;
                } else {
                    *vis = Visibility::Hidden;
                }
            }
        }
    }
}
