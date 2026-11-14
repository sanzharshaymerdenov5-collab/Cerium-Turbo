//! Боевой HUD: телеметрия танка, перезарядка, дальномер, перекрестие, снайперский прицел, лог попаданий.

use bevy::prelude::*;

use crate::ballistics::HitLog;
use crate::core::GameState;
use crate::tank::{AimTarget, PlayerTank, TankCamera, TankRoot, TankState};

#[derive(Component)]
pub struct TelemetryText;

#[derive(Component)]
pub struct AimText;

#[derive(Component)]
pub struct HitLogText;

#[derive(Component)]
pub struct SniperOverlay;

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
                            "WASD: ход  |  Мышь: башня/прицел  |  ЛКМ/Пробел: огонь\n\
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
                    max_width: Val::Px(680.0),
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
                    Text::new("Журнал попаданий (стреляйте по мишеням 50/150/250 м или постройкам)"),
                    TextFont::from_font_size(15.0),
                    TextColor(Color::srgb(0.88, 0.91, 0.95)),
                ));
            });
        });

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
    tanks: Query<(&TankRoot, &TankState), With<PlayerTank>>,
    cams: Query<&TankCamera>,
    mut telemetry_q: Query<&mut Text, (With<TelemetryText>, Without<AimText>, Without<HitLogText>)>,
    mut aim_q: Query<&mut Text, (With<AimText>, Without<TelemetryText>, Without<HitLogText>)>,
    mut log_q: Query<&mut Text, (With<HitLogText>, Without<TelemetryText>, Without<AimText>)>,
    mut sniper_q: Query<&mut Visibility, With<SniperOverlay>>,
) {
    if keys.just_pressed(KeyCode::Tab) {
        next_state.set(GameState::Hangar);
        return;
    }

    let sniper = cams.iter().next().is_some_and(|c| c.sniper);
    for mut v in &mut sniper_q {
        *v = if sniper {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }

    if let (Ok((root, st)), Ok(mut txt)) = (tanks.single(), telemetry_q.single_mut()) {
        let gun_status = if st.reload_timer <= 0.01 {
            "ГОТОВО К ВЫСТРЕЛУ".to_string()
        } else {
            format!("ПЕРЕЗАРЯДКА {:.1} с", st.reload_timer)
        };
        let mode = if sniper { "СНАЙПЕР (x3)" } else { "АРКАДНЫЙ" };
        txt.0 = format!(
            "{}  |  Скорость: {:+.1} км/ч  |  Катки: {}/8\n\
             Орудие {:.0} мм: {}  |  Прицел: {}\n\
             Снаряд: сплошной бронебойный (SolidAP) • Выстрелов: {} (пробитий: {})",
            root.spec.name,
            st.speed_kmh,
            st.grounded_wheels,
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
            txt.0 = "Журнал попаданий: стреляйте по бронеплитам (50 / 150 / 250 м) или постройкам"
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
}
