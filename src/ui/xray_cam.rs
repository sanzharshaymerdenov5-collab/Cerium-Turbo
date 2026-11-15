//! WT X-Ray Hit-Cam (Этап 6): окно рентгена последнего попадания в режиме `ModulesOnlyWt`.
//! Показывает точку входа сплошного снаряда `SolidAP`, конус кинетических осколков,
//! остаточное пробитие, повреждённые модули и состояние экипажа цели.

use bevy::prelude::*;

use crate::core::{DamageRulesMode, GameState};
use crate::damage::{CrewState, ModuleState, XRayHitReport};

#[derive(Component)]
pub struct XRayHitCamPanel;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum XRayHitCamText {
    Title,
    Details,
    Schematic,
    DamagedList,
}

/// Строит компактную 2D-схему входа сплошного AP и разлёта конуса осколков внутри корпуса цели.
pub fn render_xray_schematic(report: &XRayHitReport) -> String {
    const W: usize = 15;
    const H: usize = 7;
    let mut grid = [[' '; W]; H];

    for (r, row) in grid.iter_mut().enumerate() {
        for (c, ch) in row.iter_mut().enumerate() {
            let border_x = c == 1 || c == W - 2;
            let border_y = r == 0 || r == H - 1;
            if border_y && (1..=W - 2).contains(&c) {
                *ch = '─';
            } else if border_x {
                *ch = '│';
            } else if (5..=9).contains(&c) && (2..=4).contains(&r) {
                *ch = '·';
            }
        }
    }
    grid[0][1] = '┌';
    grid[0][W - 2] = '┐';
    grid[H - 1][1] = '└';
    grid[H - 1][W - 2] = '┘';

    // Преобразование нормализованных координат [-1..1] в сетку (нос танка сверху r=0, корма снизу r=H-1).
    let to_cell = |p: Vec2| -> (usize, usize) {
        let col = (((p.x + 1.0) * 0.5) * (W - 3) as f32 + 1.0)
            .round()
            .clamp(0.0, (W - 1) as f32) as usize;
        let row = (((p.y + 1.0) * 0.5) * (H - 1) as f32)
            .round()
            .clamp(0.0, (H - 1) as f32) as usize;
        (row, col)
    };

    let (er, ec) = to_cell(report.entry_norm_xz);
    grid[er][ec] = 'X';

    if report.spall_ray_count > 0 && report.ray_norm_xz.length_squared() > 1e-4 {
        let d = report.ray_norm_xz.normalize();
        let perp = Vec2::new(-d.y, d.x);
        for step in 1..=5 {
            let dist = step as f32 * 0.28;
            let core_p = report.entry_norm_xz + d * dist;
            let (cr, cc) = to_cell(core_p);
            if grid[cr][cc] != 'X' {
                grid[cr][cc] = '═';
            }
            if step >= 2 {
                let spread = (step as f32 - 1.0) * 0.22;
                for sgn in [-1.0f32, 1.0] {
                    let sp_p = core_p + perp * (sgn * spread);
                    let (sr, sc) = to_cell(sp_p);
                    if grid[sr][sc] != 'X' && grid[sr][sc] != '═' {
                        grid[sr][sc] = '*';
                    }
                }
            }
        }
    }

    let mut out = String::from("   [НОС]\n");
    for row in &grid {
        for &c in row {
            out.push(c);
        }
        out.push('\n');
    }
    out.push_str("  [КОРМА]");
    out
}

/// Спавнит виджет WT X-Ray Hit-Cam в правом верхнем/среднем углу боевого экрана.
pub fn setup_xray_hit_cam(mut commands: Commands) {
    commands
        .spawn((
            XRayHitCamPanel,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(14.0),
                top: Val::Px(92.0),
                width: Val::Px(342.0),
                padding: UiRect::all(Val::Px(12.0)),
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
            BackgroundColor(Color::srgba(0.04, 0.055, 0.075, 0.90)),
            BorderColor::all(Color::srgb(0.88, 0.54, 0.18)),
            Visibility::Hidden,
            DespawnOnExit(GameState::InBattle),
        ))
        .with_children(|p| {
            p.spawn((
                XRayHitCamText::Title,
                Text::new("WT X-RAY HIT-CAM"),
                TextFont::from_font_size(14.5),
                TextColor(Color::srgb(1.0, 0.84, 0.36)),
            ));
            p.spawn((
                XRayHitCamText::Details,
                Text::new(""),
                TextFont::from_font_size(12.0),
                TextColor(Color::srgb(0.86, 0.92, 0.96)),
            ));
            p.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(3.0)),
                    align_self: AlignSelf::FlexStart,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.03, 0.045, 0.88)),
                BorderColor::all(Color::srgba(0.28, 0.36, 0.44, 0.78)),
            ))
            .with_children(|box_node| {
                box_node.spawn((
                    XRayHitCamText::Schematic,
                    Text::new(""),
                    TextFont::from_font_size(12.0),
                    TextColor(Color::srgb(0.76, 0.95, 0.82)),
                ));
            });
            p.spawn((
                XRayHitCamText::DamagedList,
                Text::new(""),
                TextFont::from_font_size(12.0),
                TextColor(Color::srgb(1.0, 0.78, 0.62)),
            ));
        });
}

pub fn update_xray_hit_cam(
    mode: Res<DamageRulesMode>,
    report: Res<XRayHitReport>,
    mut panel_q: Query<(&mut Visibility, &mut BorderColor), With<XRayHitCamPanel>>,
    mut texts: Query<(&XRayHitCamText, &mut Text, &mut TextColor)>,
) {
    let show = *mode == DamageRulesMode::ModulesOnlyWt && report.active;
    for (mut vis, mut border) in &mut panel_q {
        *vis = if show {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if show {
            *border = BorderColor::all(if report.victim_killed {
                Color::srgb(0.98, 0.28, 0.22)
            } else if report.outcome_label == "ПРОБИТИЕ" {
                Color::srgb(0.95, 0.68, 0.18)
            } else {
                Color::srgb(0.45, 0.52, 0.60)
            });
        }
    }

    if !show {
        return;
    }

    let title_str = if let Some(ref reason) = report.kill_reason {
        format!("WT X-RAY: {} • {}", report.victim_name, reason)
    } else {
        format!("WT X-RAY: {} [{}]", report.victim_name, report.outcome_label)
    };

    let details_str = format!(
        "Зона: {} ({:.0}мм -> эфф. {:.0}мм)\nОст. проб. AP: {:.0}мм  |  Осколки: {} ({:.0}°)",
        report.armor_zone_ru,
        report.nominal_mm,
        report.effective_mm,
        report.residual_pen_mm,
        report.spall_ray_count,
        report.spall_half_angle_deg,
    );

    let schematic_str = render_xray_schematic(&report);

    let mut lines = Vec::new();
    if !report.module_hits.is_empty() {
        let mods: Vec<String> = report
            .module_hits
            .iter()
            .map(|(k, s)| {
                let st = match s {
                    ModuleState::Destroyed => "выбит",
                    ModuleState::Damaged => "повр.",
                    ModuleState::Intact => "цел",
                };
                format!("{}({})", k.ru_short(), st)
            })
            .collect();
        lines.push(format!("Модули: {}", mods.join(", ")));
    }
    if !report.crew_hits.is_empty() {
        let crew: Vec<String> = report
            .crew_hits
            .iter()
            .map(|(r, s)| {
                let st = match s {
                    CrewState::Unconscious => "выведен",
                    CrewState::Moved => "пересел",
                    CrewState::Alive => "ранен",
                };
                format!("{}({})", r.ru_short(), st)
            })
            .collect();
        lines.push(format!("Экипаж: {}", crew.join(", ")));
    }
    lines.push(format!(
        "Боеспособных танкистов: {}/{}",
        report.alive_crew_after, report.total_crew
    ));
    let damaged_str = lines.join("\n");

    for (tag, mut txt, mut col) in &mut texts {
        match *tag {
            XRayHitCamText::Title => {
                txt.0 = title_str.clone();
                col.0 = if report.victim_killed {
                    Color::srgb(1.0, 0.38, 0.30)
                } else if report.outcome_label == "ПРОБИТИЕ" {
                    Color::srgb(1.0, 0.84, 0.36)
                } else {
                    Color::srgb(0.80, 0.86, 0.92)
                };
            }
            XRayHitCamText::Details => txt.0 = details_str.clone(),
            XRayHitCamText::Schematic => txt.0 = schematic_str.clone(),
            XRayHitCamText::DamagedList => txt.0 = damaged_str.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::{CrewRole, ModuleKind};

    #[test]
    fn xray_schematic_marks_entry_and_spall_fan() {
        let report = XRayHitReport {
            active: true,
            age: 0.0,
            victim_name: "Т-34-85".into(),
            victim_killed: false,
            outcome_label: "ПРОБИТИЕ".into(),
            armor_zone_ru: "ВЛД".into(),
            nominal_mm: 45.0,
            effective_mm: 82.0,
            residual_pen_mm: 48.0,
            spall_ray_count: 18,
            spall_half_angle_deg: 28.0,
            entry_norm_xz: Vec2::new(0.0, -0.9),
            ray_norm_xz: Vec2::new(0.0, 1.0),
            module_hits: vec![(ModuleKind::Engine, ModuleState::Damaged)],
            crew_hits: vec![(CrewRole::Driver, CrewState::Unconscious)],
            alive_crew_after: 3,
            total_crew: 4,
            kill_reason: None,
        };
        let txt = render_xray_schematic(&report);
        assert!(txt.contains('X'));
        assert!(txt.contains('*'));
        assert!(txt.contains("[НОС]"));
    }
}
