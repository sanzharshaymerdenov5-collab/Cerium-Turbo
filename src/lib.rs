//! Cerium-Turbo — воксельная танковая игра на Rust (Bevy 0.20 + Avian3D 0.8).
//!
//! # Правило AP-Only
//! На этапах 1–6 в игре существует **только сплошной бронебойный снаряд**
//! ([`ballistics::ShellType::SolidAP`]). Остальные типы снарядов добавляются
//! только после завершения Этапа 6.

pub mod ballistics;
pub mod core;
pub mod maps;
pub mod tank;
pub mod ui;
pub mod voxel;

use avian3d::prelude::*;
use bevy::prelude::*;

/// Собирает полноценное приложение игры (окно, физика, воксели, танк, UI).
pub fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Cerium-Turbo — Этап 1: прототип".into(),
                    canvas: Some("#bevy".into()),
                    fit_canvas_to_parent: true,
                    prevent_default_event_handling: true,
                    ..default()
                }),
                ..default()
            })
            .set(bevy::log::LogPlugin {
                level: bevy::log::Level::INFO,
                filter: "wgpu=error,naga=warn,bevy_render=warn,bevy_app=warn".into(),
                ..default()
            }),
    )
    .add_plugins(PhysicsPlugins::default())
    .add_plugins((
        core::CorePlugin,
        voxel::VoxelPlugin,
        maps::MapPlugin,
        tank::TankPlugin,
        ballistics::BallisticsPlugin,
        ui::UiPlugin,
    ));
    app
}
