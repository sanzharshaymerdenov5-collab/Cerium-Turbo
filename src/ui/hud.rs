//! Боевой HUD (Обновление 1.4.1):
//! - Верхняя тактическая панель: спидометр/перезарядка, счёт команд 5v5, управление;
//! - Многокомпонентный прицел с индикатором бронепробития, хит-маркером и снайперской оптикой;
//! - Нижние панели живучести (WoT ХП + модули / WT экипаж + модули) и слоты БК/снаряжения;
//! - Реал-тайм тактическая мини-карта (игрок, союзники, враги), журнал попаданий и 3D-маркеры.

use bevy::ecs::message::MessageCursor;
use bevy::prelude::*;

use crate::audio::{AudioSettings, SoundPreset, UiSoundMessage};
use crate::ballistics::penetration::{ImpactOutcome, effective_armor_mm, penetration_at};
use crate::ballistics::{ArmorImpactMessage, HitLog};
use crate::core::{
    BattleStatus, DamageRulesMode, GameState, SelectedMap, SelectedTank, Team,
};
use crate::damage::{
    CrewRole, CrewState, DamageFeed, ModuleKind, ModuleState, PenIndicator, TankCrew,
    TankDestroyedMessage, TankHealth, TankModules, classify_penetration,
};
use crate::maps::{
    ActiveBattleMap, MapBiome, MapSpec, PROVING_GROUNDS_RON, load_map_spec, resolve_map_spec,
    spec::StructureSpec,
};
use crate::progression::{
    BattleRewardTracker, LastBattleReward, PlayerProgression, finalize_battle_reward,
};
use crate::tank::{AimTarget, PlayerTank, TankCamera, TankCombat, TankRoot, TankState, load_tank_spec};

const MAX_ALLY_MARKERS: usize = 6;
const MAX_ENEMY_MARKERS: usize = 8;
const MINIMAP_PX: f32 = 168.0;

/// Кратковременная визуальная вспышка хит-маркера на перекрестии при попадании.
#[derive(Resource, Debug, Default)]
pub struct HudHitMarker {
    pub timer: f32,
    pub is_pen: bool,
    pub is_kill: bool,
    pub badge: String,
}

/// Мировые границы карты для проекции на тактическую мини-карту.
#[derive(Resource, Debug, Clone)]
pub struct HudMapBounds {
    pub map_name: String,
    pub half_w: f32,
    pub half_l: f32,
    pub map_px: f32,
}

impl Default for HudMapBounds {
    fn default() -> Self {
        Self {
            map_name: "Полигон".into(),
            half_w: 64.0,
            half_l: 144.0,
            map_px: MINIMAP_PX,
        }
    }
}

/// Проецирует мировую позицию `(x, z)` в пиксельные координаты `[0..map_px]` внутри мини-карты.
pub fn world_to_minimap(world_pos: Vec3, bounds: &HudMapBounds) -> Vec2 {
    let hw = bounds.half_w.max(1.0);
    let hl = bounds.half_l.max(1.0);
    let u = ((world_pos.x / hw) * 0.5 + 0.5).clamp(0.03, 0.97);
    let v = ((world_pos.z / hl) * 0.5 + 0.5).clamp(0.03, 0.97);
    Vec2::new(u * bounds.map_px, v * bounds.map_px)
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
    TelemetryTitle,
    Telemetry,
    ReloadStatus,
    TeamStrip,
    Controls,
    Crosshair,
    CrosshairBracketL,
    CrosshairBracketR,
    HitFlash,
    Aim,
    DamageFeed,
    HitLog,
    HpHeader,
    WtCrewHeader,
    SlotShell,
    SlotRepair,
    SlotExtinguisher,
    BattleBanner,
    AllyMarker(usize),
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
    SlotRepair,
    SlotExtinguisher,
    ModuleBox(ModuleKind),
    CrewBox(CrewRole),
    BattleBanner,
    AllyMarker(usize),
    AllyMarkerHpBar(usize),
    AllyMarkerHpFill(usize),
    EnemyMarker(usize),
    EnemyMarkerHpBar(usize),
    EnemyMarkerHpFill(usize),
    SniperOverlay,
    CrosshairOverlay,
    CrosshairTick(u8),
    GunReticle,
    MinimapPlayerDot,
    MinimapAimDot,
    MinimapAllyDot(usize),
    MinimapEnemyDot(usize),
}

fn minimap_bg_color(biome: MapBiome) -> Color {
    match biome {
        MapBiome::Temperate => Color::srgba(0.09, 0.14, 0.09, 0.90),
        MapBiome::City => Color::srgba(0.11, 0.12, 0.14, 0.90),
        MapBiome::Desert => Color::srgba(0.17, 0.14, 0.09, 0.90),
        MapBiome::Winter => Color::srgba(0.14, 0.18, 0.23, 0.90),
        MapBiome::Industrial => Color::srgba(0.09, 0.10, 0.13, 0.90),
        MapBiome::Highland => Color::srgba(0.12, 0.16, 0.17, 0.90),
    }
}

pub fn setup_hud(
    mut commands: Commands,
    active_map: Option<Res<ActiveBattleMap>>,
    selected_map: Option<Res<SelectedMap>>,
    selected_tank: Option<Res<SelectedTank>>,
) {
    commands.insert_resource(HudHitMarker::default());

    let map_spec = active_map
        .map(|m| m.0.clone())
        .or_else(|| {
            selected_map
                .as_ref()
                .map(|s| resolve_map_spec(&s.0, 1))
                .or_else(|| load_map_spec("proving_grounds"))
        })
        .unwrap_or_else(|| MapSpec::from_ron(PROVING_GROUNDS_RON).expect("default map"));
    let tank_spec = selected_tank
        .as_ref()
        .and_then(|t| load_tank_spec(&t.0.0))
        .or_else(|| load_tank_spec("ms_1"))
        .or_else(|| load_tank_spec("t34_85"));

    let bounds = HudMapBounds {
        map_name: map_spec.name.clone(),
        half_w: (map_spec.dims.0 as f32 * map_spec.voxel_size * 0.5).max(10.0),
        half_l: (map_spec.dims.2 as f32 * map_spec.voxel_size * 0.5).max(10.0),
        map_px: MINIMAP_PX,
    };
    commands.insert_resource(bounds.clone());

    let initial_caliber = tank_spec.as_ref().map_or(37.0, |s| s.shell.caliber_mm);
    let initial_name = tank_spec.as_ref().map_or("МС-1 (Т-18)", |s| s.name.as_str());

    // Рамка снайперского прицела и оптическая сетка (спавним первой, чтобы не перекрывать HUD).
    commands
        .spawn((
            HudPanel::SniperOverlay,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                border: UiRect::all(Val::Px(46.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor::all(Color::srgba(0.01, 0.015, 0.02, 0.86)),
            Visibility::Hidden,
            DespawnOnExit(GameState::InBattle),
        ))
        .with_children(|scope| {
            // Строго симметричные горизонтальные и нижняя штриховые линии ровно по оптическому центру (50%, 50%).
            scope
                .spawn(Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(42.0),
                    ..default()
                })
                .with_children(|stadia| {
                    stadia.spawn((
                        Node {
                            width: Val::Px(110.0),
                            height: Val::Px(1.5),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.75, 0.90, 0.78, 0.45)),
                    ));
                    stadia.spawn((
                        Node {
                            width: Val::Px(110.0),
                            height: Val::Px(1.5),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.75, 0.90, 0.78, 0.45)),
                    ));
                });

            scope
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        bottom: Val::Px(14.0),
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(3.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(3.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.04, 0.06, 0.08, 0.86)),
                    BorderColor::all(Color::srgba(0.45, 0.72, 0.45, 0.7)),
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new("ОПТИЧЕСКИЙ ПРИЦЕЛ  •  3.0x"),
                        TextFont::from_font_size(11.5),
                        TextColor(Color::srgb(0.78, 0.94, 0.78)),
                    ));
                });
        });

    // Кольцо реального сведения орудия (проецируется по `aim.gun_point` в экранные координаты).
    commands.spawn((
        HudPanel::GunReticle,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Px(26.0),
            height: Val::Px(26.0),
            border: UiRect::all(Val::Px(1.8)),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        BackgroundColor(Color::NONE),
        BorderColor::all(Color::srgba(0.95, 1.0, 0.45, 0.85)),
        Visibility::Hidden,
        DespawnOnExit(GameState::InBattle),
    ));

    // Центральный перекрест прицела — строго в геометрическом центре экрана `(50%, 50%)`,
    // полностью независим от размеров верхней/нижней панелей и ленты сообщений.
    commands
        .spawn((
            HudPanel::CrosshairOverlay,
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
            DespawnOnExit(GameState::InBattle),
        ))
        .with_children(|overlay| {
            overlay
                .spawn(Node {
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    column_gap: Val::Px(6.0),
                    ..default()
                })
                .with_children(|ret| {
                    ret.spawn((
                        HudText::CrosshairBracketL,
                        Text::new("["),
                        TextFont::from_font_size(20.0),
                        TextColor(Color::srgba(0.95, 1.0, 0.45, 0.78)),
                    ));

                    ret.spawn(Node {
                        position_type: PositionType::Relative,
                        width: Val::Px(28.0),
                        height: Val::Px(28.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|pip_box| {
                        // Центральная точка (2x2 px) ровно на (13..15, 13..15) внутри 28x28 px.
                        pip_box.spawn((
                            HudPanel::CrosshairTick(0),
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px(13.0),
                                top: Val::Px(13.0),
                                width: Val::Px(2.0),
                                height: Val::Px(2.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.95, 1.0, 0.45, 0.96)),
                        ));
                        // Левый, правый, верхний и нижний штрихи перекрестия.
                        pip_box.spawn((
                            HudPanel::CrosshairTick(1),
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px(3.0),
                                top: Val::Px(13.0),
                                width: Val::Px(6.0),
                                height: Val::Px(2.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.95, 1.0, 0.45, 0.94)),
                        ));
                        pip_box.spawn((
                            HudPanel::CrosshairTick(2),
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px(19.0),
                                top: Val::Px(13.0),
                                width: Val::Px(6.0),
                                height: Val::Px(2.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.95, 1.0, 0.45, 0.94)),
                        ));
                        pip_box.spawn((
                            HudPanel::CrosshairTick(3),
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px(13.0),
                                top: Val::Px(3.0),
                                width: Val::Px(2.0),
                                height: Val::Px(6.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.95, 1.0, 0.45, 0.94)),
                        ));
                        pip_box.spawn((
                            HudPanel::CrosshairTick(4),
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px(13.0),
                                top: Val::Px(19.0),
                                width: Val::Px(2.0),
                                height: Val::Px(6.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.95, 1.0, 0.45, 0.94)),
                        ));
                        // Хит-маркер (`×` / `◈`) поверх перекрестия при попадании.
                        pip_box.spawn((
                            HudText::Crosshair,
                            Text::new(""),
                            TextFont::from_font_size(24.0),
                            TextColor(Color::srgba(0.95, 1.0, 0.45, 0.94)),
                        ));
                    });

                    ret.spawn((
                        HudText::CrosshairBracketR,
                        Text::new("]"),
                        TextFont::from_font_size(20.0),
                        TextColor(Color::srgba(0.95, 1.0, 0.45, 0.78)),
                    ));
                });
        });

    // Подприцельный блок (дальномер, хит-бейдж, лента урона) строго под прицелом, не сдвигающий перекрест.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Percent(53.5),
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(4.0),
                ..default()
            },
            DespawnOnExit(GameState::InBattle),
        ))
        .with_children(|center| {
            center.spawn((
                HudText::HitFlash,
                Text::new(""),
                TextFont::from_font_size(14.0),
                TextColor(Color::srgb(1.0, 0.74, 0.24)),
            ));
            center.spawn((
                HudText::Aim,
                Text::new(""),
                TextFont::from_font_size(14.0),
                TextColor(Color::srgb(0.94, 0.96, 0.78)),
            ));
            center.spawn((
                HudText::DamageFeed,
                Text::new(""),
                TextFont::from_font_size(15.0),
                TextColor(Color::srgb(1.0, 0.72, 0.25)),
            ));
        });

    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::SpaceBetween,
                padding: UiRect::all(Val::Px(14.0)),
                ..default()
            },
            DespawnOnExit(GameState::InBattle),
        ))
        .with_children(|root| {
            // 1. ВЕРХНЯЯ СТРОКА: ТЕЛЕМЕТРИЯ + ПЕРЕЗАРЯДКА | СЧЁТ КОМАНД | УПРАВЛЕНИЕ
            root.spawn(Node {
                width: Val::Percent(100.0),
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::FlexStart,
                column_gap: Val::Px(10.0),
                ..default()
            })
            .with_children(|top| {
                // Верх-лево: спидометр, статус орудия и полоса перезарядки.
                top.spawn((
                    Node {
                        min_width: Val::Px(285.0),
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(9.0)),
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
                    BackgroundColor(Color::srgba(0.04, 0.055, 0.075, 0.84)),
                    BorderColor::all(Color::srgb(0.38, 0.56, 0.34)),
                ))
                .with_children(|p| {
                    p.spawn((
                        HudText::TelemetryTitle,
                        Text::new(format!("{initial_name}  •  {initial_caliber:.0}мм SolidAP")),
                        TextFont::from_font_size(14.0),
                        TextColor(Color::srgb(0.95, 0.97, 0.92)),
                    ));
                    p.spawn((
                        HudText::Telemetry,
                        Text::new("+0.0 км/ч  •  Катки 8/8"),
                        TextFont::from_font_size(12.5),
                        TextColor(Color::srgb(0.76, 0.84, 0.88)),
                    ));
                    p.spawn((
                        HudText::ReloadStatus,
                        Text::new("ГОТОВО К ВЫСТРЕЛУ"),
                        TextFont::from_font_size(12.5),
                        TextColor(Color::srgb(0.45, 0.95, 0.52)),
                    ));
                    // Полоса прогресса перезарядки орудия.
                    p.spawn((
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Px(8.0),
                            border: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(3.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.08, 0.10, 0.12, 0.94)),
                        BorderColor::all(Color::srgb(0.24, 0.30, 0.36)),
                    ))
                    .with_children(|bar| {
                        bar.spawn((
                            HudPanel::ReloadBarFill,
                            Node {
                                width: Val::Percent(100.0),
                                height: Val::Percent(100.0),
                                border_radius: BorderRadius::all(Val::Px(2.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.28, 0.86, 0.42)),
                            Visibility::Inherited,
                        ));
                    });
                });

                // Верх-центр: тактическая шапка счёта команд.
                top.spawn((
                    Node {
                        padding: UiRect::axes(Val::Px(16.0), Val::Px(6.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(4.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.04, 0.055, 0.075, 0.84)),
                    BorderColor::all(Color::srgba(0.36, 0.42, 0.50, 0.75)),
                ))
                .with_children(|score| {
                    score.spawn((
                        HudText::TeamStrip,
                        Text::new(format!(
                            "СОЮЗ ■ ■ ■ ■ ■  |  {}  •  v1.4.2  |  ■ ■ ■ ■ ■ ВРАГ",
                            map_spec.name
                        )),
                        TextFont::from_font_size(13.0),
                        TextColor(Color::srgb(0.92, 0.94, 0.96)),
                    ));
                });

                // Верх-право: компактная подсказка клавиш и статуса звука.
                top.spawn((
                    Node {
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(4.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.04, 0.055, 0.075, 0.78)),
                    BorderColor::all(Color::srgba(0.28, 0.34, 0.42, 0.72)),
                ))
                .with_children(|p| {
                    p.spawn((
                        HudText::Controls,
                        Text::new(
                            "WASD: ход • Мышь: башня • ЛКМ/Пробел: огонь\n\
                             Shift: оптика • R: гусен. • N: звук ВКЛ • Tab: ангар",
                        ),
                        TextFont::from_font_size(11.5),
                        TextColor(Color::srgb(0.76, 0.82, 0.88)),
                    ));
                });
            });

            // 2. НИЖНЯЯ СТРОКА: ЛЕВО (ЖИВУЧЕСТЬ + СЛОТЫ БК/СНАРЯЖЕНИЯ) | ПРАВО (ЖУРНАЛ + МИНИ-КАРТА)
            root.spawn(Node {
                width: Val::Percent(100.0),
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::FlexEnd,
                column_gap: Val::Px(12.0),
                ..default()
            })
            .with_children(|bottom| {
                // Левый нижний стек: панель режима + слоты снаряда и расходников.
                bottom
                    .spawn(Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(6.0),
                        ..default()
                    })
                    .with_children(|bl| {
                        // 1А. Панель ХП и куклы модулей (режим WoT).
                        bl.spawn((
                            HudPanel::HullHp,
                            Node {
                                width: Val::Px(374.0),
                                padding: UiRect::all(Val::Px(10.0)),
                                border: UiRect {
                                    left: Val::Px(3.0),
                                    top: Val::Px(1.0),
                                    right: Val::Px(1.0),
                                    bottom: Val::Px(1.0),
                                },
                                border_radius: BorderRadius::all(Val::Px(4.0)),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(6.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.04, 0.055, 0.075, 0.86)),
                            BorderColor::all(Color::srgb(0.36, 0.56, 0.34)),
                            Visibility::Visible,
                        ))
                        .with_children(|hp| {
                            hp.spawn((
                                HudText::HpHeader,
                                Text::new("ПРОЧНОСТЬ: 850 / 850 HP"),
                                TextFont::from_font_size(15.0),
                                TextColor(Color::srgb(0.90, 0.97, 0.88)),
                            ));

                            hp.spawn((
                                Node {
                                    width: Val::Percent(100.0),
                                    height: Val::Px(16.0),
                                    border: UiRect::all(Val::Px(1.0)),
                                    border_radius: BorderRadius::all(Val::Px(3.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.12, 0.05, 0.05, 0.92)),
                                BorderColor::all(Color::srgb(0.28, 0.34, 0.40)),
                            ))
                            .with_children(|bar| {
                                bar.spawn((
                                    HudPanel::HpBarFill,
                                    Node {
                                        width: Val::Percent(100.0),
                                        height: Val::Percent(100.0),
                                        border_radius: BorderRadius::all(Val::Px(2.0)),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgb(0.24, 0.82, 0.36)),
                                    Visibility::Inherited,
                                ));
                            });

                            hp.spawn((
                                HudPanel::FireAlert,
                                Node {
                                    display: Display::None,
                                    padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
                                    border: UiRect::all(Val::Px(1.5)),
                                    border_radius: BorderRadius::all(Val::Px(3.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.58, 0.12, 0.06, 0.95)),
                                BorderColor::all(Color::srgb(1.0, 0.45, 0.20)),
                                Visibility::Hidden,
                            ))
                            .with_children(|f| {
                                f.spawn((
                                    Text::new("! ПОЖАР ! Нажмите [6] Огнетушитель"),
                                    TextFont::from_font_size(12.5),
                                    TextColor(Color::srgb(1.0, 0.94, 0.82)),
                                ));
                            });

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
                                            border_radius: BorderRadius::all(Val::Px(3.0)),
                                            ..default()
                                        },
                                        BackgroundColor(Color::srgba(0.08, 0.16, 0.10, 0.88)),
                                        BorderColor::all(Color::srgb(0.26, 0.56, 0.30)),
                                        Visibility::Inherited,
                                    ))
                                    .with_children(|b| {
                                        b.spawn((
                                            HudText::ModuleBadge(kind),
                                            Text::new(kind.ru_short()),
                                            TextFont::from_font_size(11.5),
                                            TextColor(Color::srgb(0.82, 0.96, 0.84)),
                                        ));
                                    });
                                }
                            });
                        });

                        // 1Б. Панель экипажа и модулей без общего ХП (режим War Thunder).
                        bl.spawn((
                            HudPanel::WtStatus,
                            Node {
                                display: Display::None,
                                width: Val::Px(386.0),
                                padding: UiRect::all(Val::Px(10.0)),
                                border: UiRect {
                                    left: Val::Px(3.0),
                                    top: Val::Px(1.0),
                                    right: Val::Px(1.0),
                                    bottom: Val::Px(1.0),
                                },
                                border_radius: BorderRadius::all(Val::Px(4.0)),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(6.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.04, 0.055, 0.075, 0.88)),
                            BorderColor::all(Color::srgb(0.88, 0.54, 0.18)),
                            Visibility::Hidden,
                        ))
                        .with_children(|wt| {
                            wt.spawn((
                                HudText::WtCrewHeader,
                                Text::new("ЭКИПАЖ: 4 / 4  •  WAR THUNDER (БЕЗ ХП)"),
                                TextFont::from_font_size(14.0),
                                TextColor(Color::srgb(1.0, 0.86, 0.44)),
                            ));

                            wt.spawn((
                                HudPanel::WtFireAlert,
                                Node {
                                    display: Display::None,
                                    padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
                                    border: UiRect::all(Val::Px(1.5)),
                                    border_radius: BorderRadius::all(Val::Px(3.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.58, 0.12, 0.06, 0.95)),
                                BorderColor::all(Color::srgb(1.0, 0.45, 0.20)),
                                Visibility::Hidden,
                            ))
                            .with_children(|f| {
                                f.spawn((
                                    Text::new("! ПОЖАР ! Угроза экипажу — [6] Огнетуш."),
                                    TextFont::from_font_size(12.0),
                                    TextColor(Color::srgb(1.0, 0.94, 0.82)),
                                ));
                            });

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
                                                border_radius: BorderRadius::all(Val::Px(3.0)),
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
                                                TextFont::from_font_size(11.5),
                                                TextColor(Color::srgb(0.84, 0.98, 0.86)),
                                            ));
                                        });
                                }
                            });

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
                                            border_radius: BorderRadius::all(Val::Px(3.0)),
                                            ..default()
                                        },
                                        BackgroundColor(Color::srgba(0.08, 0.16, 0.10, 0.88)),
                                        BorderColor::all(Color::srgb(0.26, 0.56, 0.30)),
                                        Visibility::Inherited,
                                    ))
                                    .with_children(|b| {
                                        b.spawn((
                                            HudText::ModuleBadge(kind),
                                            Text::new(kind.ru_short()),
                                            TextFont::from_font_size(11.5),
                                            TextColor(Color::srgb(0.82, 0.96, 0.84)),
                                        ));
                                    });
                                }
                            });
                        });

                        // 1В. Нижние боевые слоты снаряда `[1]` и расходников `[5]`/`[6]`.
                        bl.spawn(Node {
                            column_gap: Val::Px(6.0),
                            ..default()
                        })
                        .with_children(|slots| {
                            slots
                                .spawn((
                                    Node {
                                        padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)),
                                        border: UiRect::all(Val::Px(1.5)),
                                        border_radius: BorderRadius::all(Val::Px(4.0)),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgba(0.26, 0.17, 0.07, 0.92)),
                                    BorderColor::all(Color::srgb(0.95, 0.64, 0.22)),
                                ))
                                .with_children(|b| {
                                    b.spawn((
                                        HudText::SlotShell,
                                        Text::new(format!("[1] {initial_caliber:.0}мм SolidAP")),
                                        TextFont::from_font_size(12.0),
                                        TextColor(Color::srgb(1.0, 0.90, 0.68)),
                                    ));
                                });

                            slots
                                .spawn((
                                    HudPanel::SlotRepair,
                                    Node {
                                        padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)),
                                        border: UiRect::all(Val::Px(1.0)),
                                        border_radius: BorderRadius::all(Val::Px(4.0)),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgba(0.06, 0.13, 0.16, 0.90)),
                                    BorderColor::all(Color::srgb(0.32, 0.68, 0.72)),
                                ))
                                .with_children(|b| {
                                    b.spawn((
                                        HudText::SlotRepair,
                                        Text::new("[5] Ремк. ×2"),
                                        TextFont::from_font_size(12.0),
                                        TextColor(Color::srgb(0.84, 0.96, 0.98)),
                                    ));
                                });

                            slots
                                .spawn((
                                    HudPanel::SlotExtinguisher,
                                    Node {
                                        padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)),
                                        border: UiRect::all(Val::Px(1.0)),
                                        border_radius: BorderRadius::all(Val::Px(4.0)),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgba(0.07, 0.13, 0.17, 0.90)),
                                    BorderColor::all(Color::srgb(0.34, 0.56, 0.72)),
                                ))
                                .with_children(|b| {
                                    b.spawn((
                                        HudText::SlotExtinguisher,
                                        Text::new("[6] Огнет. ×2"),
                                        TextFont::from_font_size(12.0),
                                        TextColor(Color::srgb(0.86, 0.94, 0.98)),
                                    ));
                                });
                        });
                    });

                // Правый нижний кластер: Журнал попаданий + Тактическая мини-карта.
                bottom
                    .spawn(Node {
                        align_items: AlignItems::FlexEnd,
                        column_gap: Val::Px(10.0),
                        ..default()
                    })
                    .with_children(|br| {
                        // Журнал попаданий.
                        br.spawn((
                            Node {
                                min_width: Val::Px(310.0),
                                max_width: Val::Px(420.0),
                                padding: UiRect::all(Val::Px(10.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                border_radius: BorderRadius::all(Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.04, 0.055, 0.075, 0.82)),
                            BorderColor::all(Color::srgba(0.30, 0.36, 0.44, 0.78)),
                        ))
                        .with_children(|p| {
                            p.spawn((
                                HudText::HitLog,
                                Text::new("Боевой журнал:\n• Готов к бою"),
                                TextFont::from_font_size(12.5),
                                TextColor(Color::srgb(0.88, 0.91, 0.95)),
                            ));
                        });

                        // Тактическая мини-карта (`168 x 168 px`).
                        br.spawn((
                            Node {
                                flex_direction: FlexDirection::Column,
                                align_items: AlignItems::Center,
                                padding: UiRect::all(Val::Px(5.0)),
                                border: UiRect::all(Val::Px(1.5)),
                                border_radius: BorderRadius::all(Val::Px(4.0)),
                                row_gap: Val::Px(3.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.035, 0.048, 0.065, 0.90)),
                            BorderColor::all(Color::srgb(0.34, 0.42, 0.50)),
                        ))
                        .with_children(|mm_card| {
                            mm_card.spawn((
                                Text::new(format!("{} • N↑", map_spec.name)),
                                TextFont::from_font_size(10.5),
                                TextColor(Color::srgb(0.76, 0.84, 0.90)),
                            ));

                            mm_card
                                .spawn((
                                    Node {
                                        width: Val::Px(MINIMAP_PX),
                                        height: Val::Px(MINIMAP_PX),
                                        border: UiRect::all(Val::Px(1.0)),
                                        overflow: Overflow::clip(),
                                        ..default()
                                    },
                                    BackgroundColor(minimap_bg_color(map_spec.biome)),
                                    BorderColor::all(Color::srgba(0.28, 0.36, 0.44, 0.85)),
                                ))
                                .with_children(|canvas| {
                                    // Сетка 3x3 на мини-карте.
                                    for frac in [0.333f32, 0.666] {
                                        canvas.spawn((
                                            Node {
                                                position_type: PositionType::Absolute,
                                                left: Val::Px(MINIMAP_PX * frac),
                                                top: Val::Px(0.0),
                                                width: Val::Px(1.0),
                                                height: Val::Px(MINIMAP_PX),
                                                ..default()
                                            },
                                            BackgroundColor(Color::srgba(0.55, 0.65, 0.75, 0.16)),
                                        ));
                                        canvas.spawn((
                                            Node {
                                                position_type: PositionType::Absolute,
                                                left: Val::Px(0.0),
                                                top: Val::Px(MINIMAP_PX * frac),
                                                width: Val::Px(MINIMAP_PX),
                                                height: Val::Px(1.0),
                                                ..default()
                                            },
                                            BackgroundColor(Color::srgba(0.55, 0.65, 0.75, 0.16)),
                                        ));
                                    }

                                    // Ледяные зоны (Зимний укрепрайон).
                                    for ice in &map_spec.ice_zones {
                                        let p_min = world_to_minimap(
                                            Vec3::new(ice.min_xz.0, 0.0, ice.min_xz.1),
                                            &bounds,
                                        );
                                        let p_max = world_to_minimap(
                                            Vec3::new(ice.max_xz.0, 0.0, ice.max_xz.1),
                                            &bounds,
                                        );
                                        canvas.spawn((
                                            Node {
                                                position_type: PositionType::Absolute,
                                                left: Val::Px(p_min.x.min(p_max.x)),
                                                top: Val::Px(p_min.y.min(p_max.y)),
                                                width: Val::Px((p_max.x - p_min.x).abs().max(4.0)),
                                                height: Val::Px((p_max.y - p_min.y).abs().max(4.0)),
                                                ..default()
                                            },
                                            BackgroundColor(Color::srgba(0.45, 0.78, 0.96, 0.28)),
                                        ));
                                    }

                                    // Разрушаемые постройки и стены на мини-карте.
                                    for st in &map_spec.structures {
                                        let (min_xz, max_xz, col) = match st {
                                            StructureSpec::Wall { min_xz, max_xz, .. } => (
                                                *min_xz,
                                                *max_xz,
                                                Color::srgba(0.70, 0.64, 0.56, 0.65),
                                            ),
                                            StructureSpec::House {
                                                center_xz, size_xz, ..
                                            } => (
                                                (
                                                    center_xz.0 - size_xz.0 * 0.5,
                                                    center_xz.1 - size_xz.1 * 0.5,
                                                ),
                                                (
                                                    center_xz.0 + size_xz.0 * 0.5,
                                                    center_xz.1 + size_xz.1 * 0.5,
                                                ),
                                                Color::srgba(0.58, 0.54, 0.48, 0.78),
                                            ),
                                        };
                                        let p_min = world_to_minimap(
                                            Vec3::new(min_xz.0, 0.0, min_xz.1),
                                            &bounds,
                                        );
                                        let p_max = world_to_minimap(
                                            Vec3::new(max_xz.0, 0.0, max_xz.1),
                                            &bounds,
                                        );
                                        canvas.spawn((
                                            Node {
                                                position_type: PositionType::Absolute,
                                                left: Val::Px(p_min.x.min(p_max.x)),
                                                top: Val::Px(p_min.y.min(p_max.y)),
                                                width: Val::Px((p_max.x - p_min.x).abs().max(3.0)),
                                                height: Val::Px((p_max.y - p_min.y).abs().max(3.0)),
                                                ..default()
                                            },
                                            BackgroundColor(col),
                                        ));
                                    }

                                    // Метка направления взгляда/башни игрока (золотая точка).
                                    canvas.spawn((
                                        HudPanel::MinimapAimDot,
                                        Node {
                                            position_type: PositionType::Absolute,
                                            left: Val::Px(0.0),
                                            top: Val::Px(0.0),
                                            width: Val::Px(4.0),
                                            height: Val::Px(4.0),
                                            border_radius: BorderRadius::all(Val::Px(2.0)),
                                            ..default()
                                        },
                                        BackgroundColor(Color::srgb(0.98, 0.84, 0.28)),
                                        Visibility::Inherited,
                                    ));

                                    // Метка игрока (зелёная точка с белой рамкой).
                                    canvas.spawn((
                                        HudPanel::MinimapPlayerDot,
                                        Node {
                                            position_type: PositionType::Absolute,
                                            left: Val::Px(0.0),
                                            top: Val::Px(0.0),
                                            width: Val::Px(8.0),
                                            height: Val::Px(8.0),
                                            border: UiRect::all(Val::Px(1.0)),
                                            border_radius: BorderRadius::all(Val::Px(4.0)),
                                            ..default()
                                        },
                                        BackgroundColor(Color::srgb(0.28, 0.94, 0.42)),
                                        BorderColor::all(Color::WHITE),
                                        Visibility::Inherited,
                                    ));

                                    // Пул меток союзников (бирюзово-синие точки).
                                    for i in 0..MAX_ALLY_MARKERS {
                                        canvas.spawn((
                                            HudPanel::MinimapAllyDot(i),
                                            Node {
                                                position_type: PositionType::Absolute,
                                                left: Val::Px(0.0),
                                                top: Val::Px(0.0),
                                                width: Val::Px(7.0),
                                                height: Val::Px(7.0),
                                                border: UiRect::all(Val::Px(1.0)),
                                                border_radius: BorderRadius::all(Val::Px(3.5)),
                                                ..default()
                                            },
                                            BackgroundColor(Color::srgb(0.24, 0.72, 0.98)),
                                            BorderColor::all(Color::srgb(0.76, 0.94, 1.0)),
                                            Visibility::Hidden,
                                        ));
                                    }

                                    // Пул меток врагов (красные ромбы/квадраты).
                                    for i in 0..MAX_ENEMY_MARKERS {
                                        canvas.spawn((
                                            HudPanel::MinimapEnemyDot(i),
                                            Node {
                                                position_type: PositionType::Absolute,
                                                left: Val::Px(0.0),
                                                top: Val::Px(0.0),
                                                width: Val::Px(7.0),
                                                height: Val::Px(7.0),
                                                border: UiRect::all(Val::Px(1.0)),
                                                border_radius: BorderRadius::all(Val::Px(2.0)),
                                                ..default()
                                            },
                                            BackgroundColor(Color::srgb(0.96, 0.26, 0.22)),
                                            BorderColor::all(Color::srgb(1.0, 0.78, 0.72)),
                                            Visibility::Hidden,
                                        ));
                                    }
                                });
                        });
                    });
            });
        });

    // Итоговый баннер победы/поражения или наблюдателя по центру.
    commands
        .spawn((
            HudPanel::BattleBanner,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(20.0),
                left: Val::Percent(22.0),
                width: Val::Percent(56.0),
                padding: UiRect::axes(Val::Px(24.0), Val::Px(16.0)),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.04, 0.055, 0.075, 0.92)),
            BorderColor::all(Color::srgb(0.90, 0.75, 0.25)),
            Visibility::Hidden,
            DespawnOnExit(GameState::InBattle),
        ))
        .with_children(|b| {
            b.spawn((
                HudText::BattleBanner,
                Text::new(""),
                TextFont::from_font_size(20.0),
                TextColor(Color::WHITE),
            ));
        });

    // Пул 3D->2D маркеров над союзниками.
    for i in 0..MAX_ALLY_MARKERS {
        commands
            .spawn((
                HudPanel::AllyMarker(i),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(0.0),
                    min_width: Val::Px(96.0),
                    padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(3.0)),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.03, 0.09, 0.16, 0.76)),
                BorderColor::all(Color::srgb(0.28, 0.72, 0.96)),
                Visibility::Hidden,
                DespawnOnExit(GameState::InBattle),
            ))
            .with_children(|m| {
                m.spawn((
                    HudText::AllyMarker(i),
                    Text::new(""),
                    TextFont::from_font_size(11.0),
                    TextColor(Color::srgb(0.80, 0.94, 1.0)),
                ));
                m.spawn((
                    HudPanel::AllyMarkerHpBar(i),
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Px(3.5),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.06, 0.14, 0.22, 0.9)),
                    Visibility::Inherited,
                ))
                .with_children(|bar| {
                    bar.spawn((
                        HudPanel::AllyMarkerHpFill(i),
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.28, 0.78, 0.96)),
                        Visibility::Inherited,
                    ));
                });
            });
    }

    // Пул 3D->2D маркеров над вражескими танками со встроенной мини-полосой ХП.
    for i in 0..MAX_ENEMY_MARKERS {
        commands
            .spawn((
                HudPanel::EnemyMarker(i),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(0.0),
                    min_width: Val::Px(104.0),
                    padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(3.0)),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.16, 0.04, 0.04, 0.80)),
                BorderColor::all(Color::srgb(0.92, 0.32, 0.22)),
                Visibility::Hidden,
                DespawnOnExit(GameState::InBattle),
            ))
            .with_children(|m| {
                m.spawn((
                    HudText::EnemyMarker(i),
                    Text::new(""),
                    TextFont::from_font_size(11.5),
                    TextColor(Color::srgb(1.0, 0.86, 0.78)),
                ));
                m.spawn((
                    HudPanel::EnemyMarkerHpBar(i),
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Px(4.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.25, 0.06, 0.06, 0.9)),
                    Visibility::Inherited,
                ))
                .with_children(|bar| {
                    bar.spawn((
                        HudPanel::EnemyMarkerHpFill(i),
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.94, 0.28, 0.22)),
                        Visibility::Inherited,
                    ));
                });
            });
    }
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
    mut progression: (
        Option<ResMut<PlayerProgression>>,
        Option<ResMut<BattleRewardTracker>>,
        Option<ResMut<LastBattleReward>>,
    ),
    map_bounds: Option<Res<HudMapBounds>>,
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
            Entity,
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
    all_combat: Query<(Entity, &GlobalTransform, &TankCombat, Option<&TankHealth>)>,
    cams: Query<(&Camera, &Transform, &TankCamera), With<Camera3d>>,
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
        if let (Some(mut prog), Some(mut tracker), Some(mut last_reward)) = (
            progression.0.take(),
            progression.1.take(),
            progression.2.take(),
        ) {
            let victory = *status == BattleStatus::Victory;
            let early_exit = *status == BattleStatus::Ongoing;
            finalize_battle_reward(&mut prog, &mut tracker, &mut last_reward, victory, early_exit);
        }
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

    let player = player_q.single().ok();
    let human_ent = player.map(|(e, ..)| e);

    if let Some(ref msgs) = msg_resources.0 {
        for msg in cursors.0.read(msgs) {
            let by_human = human_ent.is_some()
                && msg.shooter == human_ent
                && msg.owner_tank.is_some()
                && msg.owner_tank != human_ent;
            if by_human {
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
            if msg.team == Team::Enemy && human_ent.is_some() && msg.killer == human_ent {
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
    let cam_proj = cam_opt.map(|(cam, cam_tf, _)| (cam, GlobalTransform::from(*cam_tf)));
    let sniper = cam_opt.is_some_and(|(_, _, c)| c.sniper);

    let mut ally_total = 0usize;
    let mut ally_alive = 0usize;
    let mut enemy_total = 0usize;
    let mut enemy_alive = 0usize;
    let mut alive_allies = Vec::new();
    let mut alive_enemies = Vec::new();
    for (ent, gt, c, h) in &all_combat {
        match c.team {
            Team::Player => {
                ally_total += 1;
                if c.alive {
                    ally_alive += 1;
                    if Some(ent) != human_ent {
                        alive_allies.push((gt.translation(), c, h.copied()));
                    }
                }
            }
            Team::Enemy => {
                enemy_total += 1;
                if c.alive {
                    enemy_alive += 1;
                    alive_enemies.push((gt.translation(), c, h.copied()));
                }
            }
        }
    }

    let player_pos = player.map(|(_, gt, ..)| gt.translation());
    let player_alive = player.is_some_and(|(_, _, _, _, c, ..)| c.alive);
    let pen_ind = match (player, aim.target_plate) {
        (Some((_, _, root, ..)), Some(plate)) => classify_penetration(
            &root.spec.shell,
            aim.distance,
            plate.thickness_mm,
            plate.angle_deg,
        ),
        _ => PenIndicator::None,
    };

    let (title_str, telemetry_str, reload_str, reload_ready, reload_broken) =
        if let Some((_, _, root, st, combat, _, mods_opt, crew_opt)) = player {
            let (gun_status, ready, broken) = if !combat.alive {
                ("ТАНК УНИЧТОЖЕН".to_string(), false, true)
            } else if combat.gun_jammed {
                if crew_opt.is_some_and(|cr| cr.state(CrewRole::Gunner) != CrewState::Alive)
                    && *mode == DamageRulesMode::ModulesOnlyWt
                {
                    ("НАВОДЧИК ВЫБИТ!".to_string(), false, true)
                } else {
                    ("КАЗЁННИК ВЫБИТ!".to_string(), false, true)
                }
            } else if st.reload_timer <= 0.01 {
                ("ГОТОВО К ВЫСТРЕЛУ".to_string(), true, false)
            } else {
                (format!("ПЕРЕЗАРЯДКА {:.1} с", st.reload_timer), false, false)
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
            let scope_tag = if sniper { "ОПТИКА 3x" } else { "3-е ЛИЦО" };
            (
                format!(
                    "{}  •  {:.0}мм SolidAP  •  {}",
                    root.spec.name, root.spec.shell.caliber_mm, scope_tag
                ),
                format!(
                    "Скорость: {:+.1} км/ч  •  Катки {}/8  •  Проб.: {}/{}{}{}",
                    st.speed_kmh,
                    st.grounded_wheels,
                    log.penetrations,
                    log.shots,
                    ice_warn,
                    fire_warn
                ),
                gun_status,
                ready,
                broken,
            )
        } else {
            (
                "Танк".into(),
                String::new(),
                "ТАНК УНИЧТОЖЕН".into(),
                false,
                true,
            )
        };

    let map_title = map_bounds
        .as_ref()
        .map_or("Бой", |b| b.map_name.as_str());
    let mode_short = match *mode {
        DamageRulesMode::ArcadeHpAndModules => "WoT",
        DamageRulesMode::ModulesOnlyWt => "WT",
        DamageRulesMode::PrototypeTargets => "Прот.",
    };
    let ally_icons: String = (0..ally_total.max(1))
        .map(|i| if i < ally_alive { "■" } else { "□" })
        .collect::<Vec<_>>()
        .join(" ");
    let enemy_icons: String = (0..enemy_total)
        .map(|i| if i < enemy_alive { "■" } else { "□" })
        .collect::<Vec<_>>()
        .join(" ");
    let team_strip_str = if enemy_total == 0 {
        format!("СОЮЗ {ally_icons}  |  {map_title} ({mode_short})  •  v1.4.2  |  МИШЕНИ")
    } else {
        format!("СОЮЗ {ally_icons}  |  {map_title} ({mode_short})  •  v1.4.2  |  {enemy_icons} ВРАГ")
    };

    let controls_str = format!(
        "WASD: ход • Мышь: башня • ЛКМ/Пробел: огонь\n\
         Shift: оптика • R: гусен. • Звук [N]: {} • Tab: ангар",
        if msg_resources.3.as_ref().is_some_and(|a| a.muted) {
            "ВЫКЛ"
        } else {
            "ВКЛ"
        }
    );

    let gun_converged = aim.is_converged();
    let aim_str = {
        let spread = if gun_converged { "СВЕДЁН" } else { "НАВОДКА..." };
        match (&aim.target_name, aim.target_plate, player) {
            (Some(name), Some(plate), Some((_, _, root, ..))) => {
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
        "Боевой журнал:\n• Наведите прицел на врага или мишень".to_string()
    } else {
        let mut s = String::from("Боевой журнал:\n");
        for e in log.entries.iter().rev() {
            s.push_str("• ");
            s.push_str(&e.text);
            s.push('\n');
        }
        s.trim_end().to_string()
    };

    let hp_opt = player.and_then(|(_, _, _, _, _, h, ..)| h);
    let mods_opt = player.and_then(|(_, _, _, _, _, _, m, _)| m);
    let crew_opt = player.and_then(|(_, _, _, _, _, _, _, c)| c);
    let reload_info = player.map(|(_, _, root, st, combat, ..)| {
        let total = root.spec.reload_s.max(0.1);
        let progress = if !combat.alive || combat.gun_jammed {
            0.0
        } else if st.reload_timer <= 0.01 {
            1.0
        } else {
            (1.0 - st.reload_timer / total).clamp(0.0, 1.0)
        };
        (
            progress,
            combat.alive && !combat.gun_jammed && st.reload_timer <= 0.01,
            combat.gun_jammed || !combat.alive,
        )
    });

    let reticle_color = if hit_marker.timer > 0.0 {
        if hit_marker.is_kill {
            Color::srgb(1.0, 0.25, 0.20)
        } else if hit_marker.is_pen {
            Color::srgb(1.0, 0.84, 0.22)
        } else {
            Color::srgb(0.72, 0.88, 1.0)
        }
    } else {
        match pen_ind {
            PenIndicator::Green => Color::srgb(0.28, 0.96, 0.36),
            PenIndicator::Yellow => Color::srgb(0.98, 0.82, 0.22),
            PenIndicator::Red => Color::srgb(0.96, 0.28, 0.26),
            PenIndicator::None => Color::srgba(0.95, 1.0, 0.45, 0.92),
        }
    };

    let caliber_mm = player.map_or(37.0, |(_, _, r, ..)| r.spec.shell.caliber_mm);
    let repair_charges = mods_opt.map_or(0, |m| m.repair_charges);
    let ext_charges = mods_opt.map_or(0, |m| m.extinguisher_charges);

    // Обновление текстовых элементов.
    for (tag, mut txt, color_opt) in &mut texts {
        match *tag {
            HudText::TelemetryTitle => txt.0 = title_str.clone(),
            HudText::Telemetry => txt.0 = telemetry_str.clone(),
            HudText::ReloadStatus => {
                txt.0 = reload_str.clone();
                if let Some(mut c) = color_opt {
                    c.0 = if reload_broken {
                        Color::srgb(1.0, 0.38, 0.32)
                    } else if reload_ready {
                        Color::srgb(0.38, 0.94, 0.48)
                    } else {
                        Color::srgb(0.98, 0.78, 0.28)
                    };
                }
            }
            HudText::TeamStrip => txt.0 = team_strip_str.clone(),
            HudText::Controls => txt.0 = controls_str.clone(),
            HudText::Crosshair => {
                if hit_marker.timer > 0.0 {
                    txt.0 = if hit_marker.is_pen { "×" } else { "◈" }.to_string();
                } else {
                    txt.0.clear();
                }
                if let Some(mut c) = color_opt {
                    c.0 = reticle_color;
                }
            }
            HudText::CrosshairBracketL | HudText::CrosshairBracketR => {
                if let Some(mut c) = color_opt {
                    c.0 = reticle_color.with_alpha(0.75);
                }
            }
            HudText::HitFlash => {
                if hit_marker.timer > 0.0 {
                    txt.0 = hit_marker.badge.clone();
                    if let Some(mut c) = color_opt {
                        c.0 = reticle_color;
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
            HudText::SlotShell => {
                txt.0 = format!("[1] {caliber_mm:.0}мм SolidAP");
            }
            HudText::SlotRepair => {
                txt.0 = format!("[5] Ремк. ×{repair_charges}");
            }
            HudText::SlotExtinguisher => {
                txt.0 = format!("[6] Огнет. ×{ext_charges}");
            }
            HudText::BattleBanner => match *status {
                BattleStatus::Ongoing => {
                    if !player_alive && ally_alive > 0 {
                        txt.0 = format!(
                            "ВАШ ТАНК ПОДБИТ — ИДЁТ БОЙ СОЮЗНИКОВ (5v5)\nСоюзники в строю: {ally_alive}/{ally_total}  •  Противники: {enemy_alive}/{enemy_total}  •  [Tab] — выйти в ангар"
                        );
                    }
                }
                BattleStatus::Victory => {
                    if let Some(ref lr) = progression.2
                        && lr.active
                    {
                        let bonus = if lr.random_bonus {
                            " (+10% случ. карта)"
                        } else {
                            ""
                        };
                        txt.0 = format!(
                            "ПОБЕДА! Все враги уничтожены{bonus}\n\
                             Награда: +{} серебра  •  +{} опыта  |  Фраги: {}  •  Пробития: {}  •  Урон: {:.0}\n\
                             [Tab] — вернуться в ангар",
                            lr.credits_earned, lr.xp_earned, lr.kills, lr.penetrations, lr.damage
                        );
                    } else {
                        txt.0 = format!(
                            "ПОБЕДА! Все враги уничтожены\nВыстрелов: {} • Пробитий: {} • [Tab] — в ангар",
                            log.shots, log.penetrations
                        );
                    }
                }
                BattleStatus::Defeat => {
                    if let Some(ref lr) = progression.2
                        && lr.active
                    {
                        let bonus = if lr.random_bonus {
                            " (+10% случ. карта)"
                        } else {
                            ""
                        };
                        txt.0 = format!(
                            "ПОРАЖЕНИЕ! Союзная группа потеряна{bonus}\n\
                             Награда: +{} серебра  •  +{} опыта  |  Фраги: {}  •  Пробития: {}  •  Урон: {:.0}\n\
                             [Tab] — вернуться в ангар",
                            lr.credits_earned, lr.xp_earned, lr.kills, lr.penetrations, lr.damage
                        );
                    } else {
                        txt.0 = format!(
                            "ПОРАЖЕНИЕ! Союзная группа потеряна\nВыстрелов: {} • Пробитий: {} • [Tab] — в ангар",
                            log.shots, log.penetrations
                        );
                    }
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
                            ModuleState::Intact => Color::srgb(0.82, 0.96, 0.84),
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
                        CrewState::Alive if member.is_backup => format!("{}*", role.ru_short()),
                        CrewState::Alive => role.ru_short().to_string(),
                        CrewState::Moved => format!("{} ->", role.ru_short()),
                        CrewState::Unconscious => format!("{} X", role.ru_short()),
                    };
                    if let Some(mut c) = color_opt {
                        c.0 = match member.state {
                            CrewState::Alive if member.is_backup => Color::srgb(0.65, 0.98, 0.86),
                            CrewState::Alive => Color::srgb(0.84, 0.98, 0.86),
                            CrewState::Moved => Color::srgb(1.0, 0.86, 0.42),
                            CrewState::Unconscious => Color::srgb(1.0, 0.52, 0.50),
                        };
                    }
                }
            }
            HudText::AllyMarker(idx) => {
                if let Some(&(apos, ac, ah)) = alive_allies.get(idx) {
                    let dist = player_pos.map_or(0.0, |p| p.distance(apos));
                    txt.0 = match (*mode, ah) {
                        (DamageRulesMode::ArcadeHpAndModules, Some(h)) => {
                            format!("◆ {} {:.0}м [{:.0} HP]", ac.tank_name, dist, h.hp)
                        }
                        _ => {
                            format!("◆ {}  {:.0}м", ac.tank_name, dist)
                        }
                    };
                }
            }
            HudText::EnemyMarker(idx) => {
                if let Some(&(epos, ec, eh)) = alive_enemies.get(idx) {
                    let dist = player_pos.map_or(0.0, |p| p.distance(epos));
                    txt.0 = match (*mode, eh) {
                        (DamageRulesMode::ArcadeHpAndModules, Some(h)) => {
                            format!("▼ {} {:.0}м [{:.0} HP]", ec.tank_name, dist, h.hp)
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

    // Обновление панелей, мини-карты и 3D->2D маркеров.
    let show_hp_panel = *mode == DamageRulesMode::ArcadeHpAndModules;
    let show_wt_panel = *mode == DamageRulesMode::ModulesOnlyWt;
    let on_fire = mods_opt.is_some_and(|m| m.fire_timer > 0.0);
    let fire_pulse = (time.elapsed_secs() * 12.0).sin() * 0.5 + 0.5;
    let default_bounds = HudMapBounds::default();
    let bounds = map_bounds.as_deref().unwrap_or(&default_bounds);

    for (panel, mut node, mut vis, bg_opt, border_opt) in &mut panels {
        match *panel {
            HudPanel::SniperOverlay => {
                *vis = if sniper {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                };
            }
            HudPanel::CrosshairOverlay => {
                *vis = Visibility::Inherited;
            }
            HudPanel::CrosshairTick(_) => {
                if let Some(mut bg) = bg_opt {
                    bg.0 = reticle_color;
                }
            }
            HudPanel::GunReticle => {
                if !player_alive {
                    *vis = Visibility::Hidden;
                    continue;
                }
                let Some((cam, ref cam_gt)) = cam_proj else {
                    *vis = Visibility::Hidden;
                    continue;
                };
                if let Ok(vp) = cam.world_to_viewport(cam_gt, aim.gun_point) {
                    node.left = Val::Px(vp.x - 13.0);
                    node.top = Val::Px(vp.y - 13.0);
                    *vis = Visibility::Visible;
                    if let Some(mut border) = border_opt {
                        *border = if gun_converged {
                            BorderColor::all(reticle_color.with_alpha(0.92))
                        } else {
                            BorderColor::all(Color::srgba(1.0, 0.72, 0.24, 0.90))
                        };
                    }
                } else {
                    *vis = Visibility::Hidden;
                }
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
                            Color::srgb(0.24, 0.82, 0.36)
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
                            Color::srgb(0.26, 0.88, 0.44)
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
            HudPanel::SlotRepair => {
                if let Some(mut bg) = bg_opt {
                    bg.0 = if repair_charges > 0 {
                        Color::srgba(0.06, 0.14, 0.17, 0.90)
                    } else {
                        Color::srgba(0.08, 0.09, 0.10, 0.70)
                    };
                }
                if let Some(mut border) = border_opt {
                    *border = if repair_charges > 0 {
                        BorderColor::all(Color::srgb(0.32, 0.68, 0.72))
                    } else {
                        BorderColor::all(Color::srgba(0.25, 0.28, 0.30, 0.6))
                    };
                }
            }
            HudPanel::SlotExtinguisher => {
                if let Some(mut bg) = bg_opt {
                    bg.0 = if on_fire && ext_charges > 0 {
                        Color::srgba(0.56, 0.14 + fire_pulse * 0.15, 0.06, 0.95)
                    } else if ext_charges > 0 {
                        Color::srgba(0.07, 0.13, 0.17, 0.90)
                    } else {
                        Color::srgba(0.08, 0.09, 0.10, 0.70)
                    };
                }
                if let Some(mut border) = border_opt {
                    *border = if on_fire && ext_charges > 0 {
                        BorderColor::all(Color::srgb(1.0, 0.48 + fire_pulse * 0.35, 0.18))
                    } else if ext_charges > 0 {
                        BorderColor::all(Color::srgb(0.34, 0.56, 0.72))
                    } else {
                        BorderColor::all(Color::srgba(0.25, 0.28, 0.30, 0.6))
                    };
                }
            }
            HudPanel::ModuleBox(kind) => {
                if let Some(mods) = mods_opt {
                    let st = mods.state(kind);
                    if let Some(mut bg) = bg_opt {
                        bg.0 = match st {
                            ModuleState::Intact => Color::srgba(0.08, 0.16, 0.10, 0.88),
                            ModuleState::Damaged => Color::srgba(0.24, 0.18, 0.04, 0.92),
                            ModuleState::Destroyed => Color::srgba(0.28, 0.06, 0.06, 0.94),
                        };
                    }
                    if let Some(mut border) = border_opt {
                        *border = match st {
                            ModuleState::Intact => BorderColor::all(Color::srgb(0.26, 0.56, 0.30)),
                            ModuleState::Damaged => BorderColor::all(Color::srgb(0.95, 0.72, 0.16)),
                            ModuleState::Destroyed => {
                                BorderColor::all(Color::srgb(0.95, 0.25, 0.22))
                            }
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
                BattleStatus::Ongoing => {
                    if !player_alive && ally_alive > 0 {
                        node.top = Val::Percent(10.0);
                        *vis = Visibility::Visible;
                        if let Some(mut border) = border_opt {
                            *border = BorderColor::all(Color::srgb(0.95, 0.72, 0.24));
                        }
                    } else {
                        *vis = Visibility::Hidden;
                    }
                }
                BattleStatus::Victory => {
                    node.top = Val::Percent(20.0);
                    *vis = Visibility::Visible;
                    if let Some(mut border) = border_opt {
                        *border = BorderColor::all(Color::srgb(0.35, 0.88, 0.42));
                    }
                }
                BattleStatus::Defeat => {
                    node.top = Val::Percent(20.0);
                    *vis = Visibility::Visible;
                    if let Some(mut border) = border_opt {
                        *border = BorderColor::all(Color::srgb(0.92, 0.28, 0.24));
                    }
                }
            },
            HudPanel::AllyMarker(idx) => {
                let Some(&(apos, _, _)) = alive_allies.get(idx) else {
                    *vis = Visibility::Hidden;
                    continue;
                };
                let Some((cam, ref cam_gt)) = cam_proj else {
                    *vis = Visibility::Hidden;
                    continue;
                };
                if let Ok(vp) = cam.world_to_viewport(cam_gt, apos + Vec3::Y * 2.9) {
                    node.left = Val::Px((vp.x - 52.0).max(4.0));
                    node.top = Val::Px((vp.y - 26.0).max(4.0));
                    *vis = Visibility::Visible;
                } else {
                    *vis = Visibility::Hidden;
                }
            }
            HudPanel::AllyMarkerHpBar(idx) => {
                let show_bar = *mode == DamageRulesMode::ArcadeHpAndModules
                    && alive_allies.get(idx).and_then(|a| a.2).is_some();
                node.display = if show_bar {
                    Display::Flex
                } else {
                    Display::None
                };
                *vis = if show_bar {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
            HudPanel::AllyMarkerHpFill(idx) => {
                if let Some(&(_, _, h_opt)) = alive_allies.get(idx)
                    && *mode == DamageRulesMode::ArcadeHpAndModules
                    && let Some(h) = h_opt
                {
                    node.width = Val::Percent((h.ratio() * 100.0).clamp(0.0, 100.0));
                    *vis = Visibility::Inherited;
                } else {
                    node.width = Val::Percent(100.0);
                    *vis = Visibility::Hidden;
                }
            }
            HudPanel::EnemyMarker(idx) => {
                let Some(&(epos, _, _)) = alive_enemies.get(idx) else {
                    *vis = Visibility::Hidden;
                    continue;
                };
                let Some((cam, ref cam_gt)) = cam_proj else {
                    *vis = Visibility::Hidden;
                    continue;
                };
                if let Ok(vp) = cam.world_to_viewport(cam_gt, epos + Vec3::Y * 3.0) {
                    node.left = Val::Px((vp.x - 56.0).max(4.0));
                    node.top = Val::Px((vp.y - 28.0).max(4.0));
                    *vis = Visibility::Visible;
                } else {
                    *vis = Visibility::Hidden;
                }
            }
            HudPanel::EnemyMarkerHpBar(idx) => {
                let show_bar = *mode == DamageRulesMode::ArcadeHpAndModules
                    && alive_enemies.get(idx).and_then(|e| e.2).is_some();
                node.display = if show_bar {
                    Display::Flex
                } else {
                    Display::None
                };
                *vis = if show_bar {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
            HudPanel::EnemyMarkerHpFill(idx) => {
                if let Some(&(_, _, h_opt)) = alive_enemies.get(idx)
                    && *mode == DamageRulesMode::ArcadeHpAndModules
                    && let Some(h) = h_opt
                {
                    node.width = Val::Percent((h.ratio() * 100.0).clamp(0.0, 100.0));
                    *vis = Visibility::Inherited;
                } else {
                    node.width = Val::Percent(100.0);
                    *vis = Visibility::Hidden;
                }
            }
            HudPanel::MinimapPlayerDot => {
                if let Some(ppos) = player_pos {
                    let mp = world_to_minimap(ppos, bounds);
                    node.left = Val::Px((mp.x - 4.0).clamp(0.0, bounds.map_px - 8.0));
                    node.top = Val::Px((mp.y - 4.0).clamp(0.0, bounds.map_px - 8.0));
                    *vis = Visibility::Inherited;
                } else {
                    *vis = Visibility::Hidden;
                }
            }
            HudPanel::MinimapAimDot => {
                if let Some(ppos) = player_pos {
                    let dir = (aim.gun_point - ppos).xz().normalize_or(Vec2::NEG_Y);
                    let mp = world_to_minimap(ppos, bounds);
                    let aim_mp = mp + dir * 13.0;
                    node.left = Val::Px((aim_mp.x - 2.0).clamp(0.0, bounds.map_px - 4.0));
                    node.top = Val::Px((aim_mp.y - 2.0).clamp(0.0, bounds.map_px - 4.0));
                    *vis = Visibility::Inherited;
                } else {
                    *vis = Visibility::Hidden;
                }
            }
            HudPanel::MinimapAllyDot(idx) => {
                if let Some(&(apos, _, _)) = alive_allies.get(idx) {
                    let mp = world_to_minimap(apos, bounds);
                    node.left = Val::Px((mp.x - 3.5).clamp(0.0, bounds.map_px - 7.0));
                    node.top = Val::Px((mp.y - 3.5).clamp(0.0, bounds.map_px - 7.0));
                    *vis = Visibility::Inherited;
                } else {
                    *vis = Visibility::Hidden;
                }
            }
            HudPanel::MinimapEnemyDot(idx) => {
                if let Some(&(epos, _, _)) = alive_enemies.get(idx) {
                    let mp = world_to_minimap(epos, bounds);
                    node.left = Val::Px((mp.x - 3.5).clamp(0.0, bounds.map_px - 7.0));
                    node.top = Val::Px((mp.y - 3.5).clamp(0.0, bounds.map_px - 7.0));
                    *vis = Visibility::Inherited;
                } else {
                    *vis = Visibility::Hidden;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::state::app::StatesPlugin;

    #[test]
    fn world_to_minimap_projects_and_clamps_bounds() {
        let b = HudMapBounds {
            map_name: "Test".into(),
            half_w: 100.0,
            half_l: 200.0,
            map_px: 168.0,
        };
        let center = world_to_minimap(Vec3::ZERO, &b);
        assert!((center.x - 84.0).abs() < 1e-3);
        assert!((center.y - 84.0).abs() < 1e-3);

        // Вражеская сторона (-Z) должна быть наверху (меньший y), старт игрока (+Z) внизу.
        let north = world_to_minimap(Vec3::new(0.0, 0.0, -150.0), &b);
        let south = world_to_minimap(Vec3::new(0.0, 0.0, 150.0), &b);
        assert!(north.y < center.y && south.y > center.y);

        // Выход за границы карты аккуратно ограничивается в пределах [0..map_px].
        let oob = world_to_minimap(Vec3::new(500.0, 0.0, -900.0), &b);
        assert!(oob.x <= b.map_px && oob.y >= 0.0);
    }

    #[test]
    fn crosshair_overlay_is_centered_independently_of_hud_panels() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .init_state::<GameState>()
            .add_systems(OnEnter(GameState::InBattle), setup_hud);
        app.world_mut()
            .resource_mut::<NextState<GameState>>()
            .set(GameState::InBattle);
        app.finish();
        app.cleanup();
        app.update();

        let mut panels_q = app.world_mut().query::<(&HudPanel, &Node)>();
        let crosshair_node = panels_q
            .iter(app.world())
            .find(|(p, _)| **p == HudPanel::CrosshairOverlay)
            .map(|(_, n)| n.clone())
            .expect("CrosshairOverlay должен создаваться в setup_hud");
        assert_eq!(crosshair_node.position_type, PositionType::Absolute);
        assert_eq!(crosshair_node.width, Val::Percent(100.0));
        assert_eq!(crosshair_node.height, Val::Percent(100.0));
        assert_eq!(crosshair_node.justify_content, JustifyContent::Center);
        assert_eq!(crosshair_node.align_items, AlignItems::Center);

        let gun_ring = panels_q
            .iter(app.world())
            .find(|(p, _)| **p == HudPanel::GunReticle)
            .map(|(_, n)| n.clone())
            .expect("GunReticle должен создаваться в setup_hud");
        assert_eq!(gun_ring.position_type, PositionType::Absolute);

        let mut texts_q = app.world_mut().query::<(&HudText, &Text)>();
        let strip = texts_q
            .iter(app.world())
            .find(|(t, _)| **t == HudText::TeamStrip)
            .map(|(_, txt)| txt.0.clone())
            .expect("TeamStrip");
        assert!(strip.contains("v1.4.2"), "ожидается v1.4.2: {strip}");
    }

    #[test]
    fn horizon_aim_target_with_depth_offset_counts_as_converged() {
        // При взгляде в пустой горизонт луч камеры (809.5 м) и луч ствола (800.2 м)
        // лежат на одной прямой с разницей лишь по глубине ~7 м — угловое сведение равно 0°.
        let horizon_aim = AimTarget {
            point: Vec3::new(0.0, 5.65, -658.0),
            distance: 809.5,
            target_name: None,
            target_plate: None,
            gun_point: Vec3::new(0.0, 5.65, -665.0),
            muzzle_origin: Vec3::new(0.0, 5.05, 135.2),
            muzzle_dir: (Vec3::new(0.0, 5.65, -658.0) - Vec3::new(0.0, 5.05, 135.2)).normalize(),
        };
        assert!(horizon_aim.is_converged());

        let turned_away = AimTarget {
            muzzle_dir: Vec3::new(0.25, 0.0, -0.968).normalize(),
            gun_point: Vec3::new(200.0, 5.05, -640.0),
            ..horizon_aim
        };
        assert!(!turned_away.is_converged());
    }
}
