//! Боевой HUD: телеметрия танка, перезарядка, дальномер, перекрестие, снайперский прицел, лог попаданий.

use bevy::prelude::*;

use crate::ballistics::HitLog;
use crate::core::{BattleStatus, GameState, Team};
use crate::tank::{AimTarget, PlayerTank, TankCamera, TankCombat, TankRoot, TankState};

#[derive(Component)]
pub struct TelemetryText;

#[derive(Component)]
pub struct AimText;

#[derive(Component)]
pub struct HitLogText;

#[derive(Component)]
pub struct SniperOverlay;

#[derive(Component)]
pub struct BattleBanner;

#[derive(Component)]
pub struct BattleBannerText;

#[derive(Component)]
pub struct EnemyMarker(pub usize);

#[derive(Component)]
pub struct EnemyMarkerText(pub usize);

const MAX_ENEMY_MARKERS: usize = 8;

pub fn setup_hud(mut commands: Commands) {
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
            // Верхняя строка: слева телеметрия, справа управление.
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
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.05, 0.07, 0.09, 0.78)),
                    BorderColor::all(Color::srgb(0.32, 0.44, 0.30)),
                ))
                .with_children(|p| {
                    p.spawn((
                        TelemetryText,
                        Text::new("Т-34-85"),
                        TextFont::from_font_size(16.0),
                        TextColor(Color::srgb(0.90, 0.95, 0.88)),
                    ));
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
                        Text::new(
                            "WASD: ход и поворот  |  Мышь: башня/прицел  |  ЛКМ/Пробел: огонь\n\
                             Shift: снайперский прицел  |  R: на гусеницы  |  Tab: ангар  |  Esc: курсор",
                        ),
                        TextFont::from_font_size(14.0),
                        TextColor(Color::srgb(0.78, 0.83, 0.88)),
                    ));
                });
            });

            // Центр: прицел + дальномер.
            root.spawn(Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|center| {
                center.spawn((
                    Text::new("+"),
                    TextFont::from_font_size(28.0),
                    TextColor(Color::srgba(0.95, 1.0, 0.4, 0.92)),
                ));
                center.spawn((
                    AimText,
                    Text::new(""),
                    TextFont::from_font_size(15.0),
                    TextColor(Color::srgb(0.95, 0.96, 0.72)),
                ));
            });

            // Низ: журнал попаданий и разрушений.
            root.spawn((
                Node {
                    min_width: Val::Px(460.0),
                    max_width: Val::Px(700.0),
                    padding: UiRect::all(Val::Px(12.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.05, 0.07, 0.09, 0.78)),
                BorderColor::all(Color::srgb(0.30, 0.36, 0.42)),
            ))
            .with_children(|p| {
                p.spawn((
                    HitLogText,
                    Text::new("Журнал попаданий (уничтожьте врагов ИИ и испытайте бронестенды)"),
                    TextFont::from_font_size(15.0),
                    TextColor(Color::srgb(0.88, 0.91, 0.95)),
                ));
            });
        });

    // Баннер победы/поражения по центру.
    commands
        .spawn((
            BattleBanner,
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
                BattleBannerText,
                Text::new(""),
                TextFont::from_font_size(24.0),
                TextColor(Color::WHITE),
            ));
        });

    // Пул 2D-маркеров над вражескими танками.
    for i in 0..MAX_ENEMY_MARKERS {
        commands
            .spawn((
                EnemyMarker(i),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(0.0),
                    padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.18, 0.05, 0.04, 0.72)),
                BorderColor::all(Color::srgb(0.92, 0.35, 0.20)),
                Visibility::Hidden,
                DespawnOnExit(GameState::InBattle),
            ))
            .with_children(|m| {
                m.spawn((
                    EnemyMarkerText(i),
                    Text::new(""),
                    TextFont::from_font_size(12.0),
                    TextColor(Color::srgb(1.0, 0.78, 0.68)),
                ));
            });
    }

    // Рамка снайперского прицела (по умолчанию скрыта).
    commands.spawn((
        SniperOverlay,
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

#[allow(clippy::too_many_arguments)]
pub fn update_hud(
    keys: Res<ButtonInput<KeyCode>>,
    mut next_state: ResMut<NextState<GameState>>,
    aim: Res<AimTarget>,
    log: Res<HitLog>,
    status: Res<BattleStatus>,
    player_q: Query<(&GlobalTransform, &TankRoot, &TankState, &TankCombat), With<PlayerTank>>,
    all_combat: Query<(&GlobalTransform, &TankCombat)>,
    cams: Query<(&Camera, &GlobalTransform, &TankCamera), With<Camera3d>>,
    mut telemetry_q: Query<
        &mut Text,
        (
            With<TelemetryText>,
            Without<AimText>,
            Without<HitLogText>,
            Without<BattleBannerText>,
        ),
    >,
    mut aim_q: Query<
        &mut Text,
        (
            With<AimText>,
            Without<TelemetryText>,
            Without<HitLogText>,
            Without<BattleBannerText>,
        ),
    >,
    mut log_q: Query<
        &mut Text,
        (
            With<HitLogText>,
            Without<TelemetryText>,
            Without<AimText>,
            Without<BattleBannerText>,
        ),
    >,
    mut sniper_q: Query<&mut Visibility, (With<SniperOverlay>, Without<BattleBanner>, Without<EnemyMarker>)>,
    mut banner_q: Query<(&mut Visibility, &mut BorderColor), (With<BattleBanner>, Without<SniperOverlay>, Without<EnemyMarker>)>,
    mut banner_text_q: Query<&mut Text, (With<BattleBannerText>, Without<TelemetryText>, Without<AimText>, Without<HitLogText>)>,
    mut marker_nodes: Query<(&EnemyMarker, &mut Node, &mut Visibility), (Without<SniperOverlay>, Without<BattleBanner>)>,
    mut marker_texts: Query<
        (&EnemyMarkerText, &mut Text),
        (Without<TelemetryText>, Without<AimText>, Without<HitLogText>, Without<BattleBannerText>),
    >,
) {
    if keys.just_pressed(KeyCode::Tab) {
        next_state.set(GameState::Hangar);
        return;
    }

    let cam_opt = cams.iter().next();
    let sniper = cam_opt.is_some_and(|(_, _, c)| c.sniper);
    for mut v in &mut sniper_q {
        *v = if sniper {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }

    let mut enemy_total = 0usize;
    let mut enemy_alive = 0usize;
    let mut alive_enemies = Vec::new();
    for (gt, c) in &all_combat {
        if c.team == Team::Enemy {
            enemy_total += 1;
            if c.alive {
                enemy_alive += 1;
                alive_enemies.push((gt.translation(), c));
            }
        }
    }

    let player_pos = player_q.single().ok().map(|(gt, _, _, _)| gt.translation());
    if let (Ok((_, root, st, combat)), Ok(mut txt)) = (player_q.single(), telemetry_q.single_mut()) {
        let gun_status = if !combat.alive {
            "ТАНК УНИЧТОЖЕН".to_string()
        } else if st.reload_timer <= 0.01 {
            "ГОТОВО К ВЫСТРЕЛУ".to_string()
        } else {
            format!("ПЕРЕЗАРЯДКА {:.1} с", st.reload_timer)
        };
        let mode = if sniper { "СНАЙПЕР (x3)" } else { "АРКАДНЫЙ" };
        let dmg_left = combat.prototype_max_hits.saturating_sub(combat.prototype_hits);
        txt.0 = format!(
            "{}  |  Скорость: {:+.1} км/ч  |  Катки: {}/8\n\
             Прочность (прототип): {}/{} попаданий  |  Враги: {}/{}\n\
             Орудие {:.0} мм: {}  |  Прицел: {}\n\
             Снаряд: ТОЛЬКО сплошной SolidAP • Выстрелов: {} (пробитий: {})",
            root.spec.name,
            st.speed_kmh,
            st.grounded_wheels,
            dmg_left,
            combat.prototype_max_hits,
            enemy_alive,
            enemy_total,
            root.spec.shell.caliber_mm,
            gun_status,
            mode,
            log.shots,
            log.penetrations,
        );
    }

    if let Ok(mut txt) = aim_q.single_mut() {
        let err = aim.point.distance(aim.gun_point);
        let spread = if err < 0.8 { "СВЕДЁН" } else { "НАВОДКА..." };
        txt.0 = match &aim.target_name {
            Some(name) => format!("{:.0} м  •  {}  [{}]", aim.distance, name, spread),
            None => format!("{:.0} м  [{}]", aim.distance, spread),
        };
    }

    if let Ok(mut txt) = log_q.single_mut() {
        if log.entries.is_empty() {
            txt.0 = "Журнал попаданий: ищите и уничтожайте вражеские Т-34-85 или проверьте бронеплиты"
                .to_string();
        } else {
            let mut s = String::from("Журнал попаданий:\n");
            for e in log.entries.iter().rev() {
                s.push_str("• ");
                s.push_str(&e.text);
                s.push('\n');
            }
            txt.0 = s.trim_end().to_string();
        }
    }

    // Баннер победы/поражения.
    for (mut vis, mut border) in &mut banner_q {
        match *status {
            BattleStatus::Ongoing => {
                *vis = Visibility::Hidden;
            }
            BattleStatus::Victory => {
                *vis = Visibility::Visible;
                *border = BorderColor::all(Color::srgb(0.35, 0.88, 0.40));
                for mut bt in &mut banner_text_q {
                    bt.0 = "ПОБЕДА! Все ИИ-танки уничтожены • [Tab] — вернуться в ангар".to_string();
                }
            }
            BattleStatus::Defeat => {
                *vis = Visibility::Visible;
                *border = BorderColor::all(Color::srgb(0.92, 0.28, 0.24));
                for mut bt in &mut banner_text_q {
                    bt.0 = "ПОРАЖЕНИЕ! Ваш танк подбит • [Tab] — вернуться в ангар".to_string();
                }
            }
        }
    }

    // 2D-маркеры над врагами.
    for (slot, mut node, mut vis) in &mut marker_nodes {
        let Some(&(epos, ec)) = alive_enemies.get(slot.0) else {
            *vis = Visibility::Hidden;
            continue;
        };
        let Some((cam, cam_gt, _)) = cam_opt else {
            *vis = Visibility::Hidden;
            continue;
        };
        if let Ok(vp) = cam.world_to_viewport(cam_gt, epos + Vec3::Y * 2.9) {
            node.left = Val::Px((vp.x - 52.0).max(4.0));
            node.top = Val::Px((vp.y - 26.0).max(4.0));
            *vis = Visibility::Visible;
            let dist = player_pos.map_or(0.0, |p| p.distance(epos));
            let left = ec.prototype_max_hits.saturating_sub(ec.prototype_hits);
            for (tslot, mut txt) in &mut marker_texts {
                if tslot.0 == slot.0 {
                    txt.0 = format!("▼ {} [{}/{}] {:.0}м", ec.tank_name, left, ec.prototype_max_hits, dist);
                }
            }
        } else {
            *vis = Visibility::Hidden;
        }
    }
}
