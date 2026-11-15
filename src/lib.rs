//! Cerium-Turbo — воксельная танковая игра на Rust (Bevy 0.20 + Avian3D 0.8).
//!
//! # Правило AP-Only
//! На этапах 1–6 в игре существует **только сплошной бронебойный снаряд**
//! ([`ballistics::ShellType::SolidAP`]). Остальные типы снарядов добавляются
//! только после завершения Этапа 6.

pub mod ai;
pub mod audio;
pub mod ballistics;
pub mod core;
pub mod damage;
pub mod maps;
pub mod progression;
pub mod tank;
pub mod ui;
pub mod vfx;
pub mod voxel;

use avian3d::prelude::*;
use bevy::prelude::*;

/// Подключает физику и все игровые плагины приложения.
pub fn configure_game_plugins(app: &mut App) {
    app.add_plugins(PhysicsPlugins::default()).add_plugins((
        core::CorePlugin,
        voxel::VoxelPlugin,
        maps::MapPlugin,
        tank::TankPlugin,
        ballistics::BallisticsPlugin,
        damage::DamagePlugin,
        progression::ProgressionPlugin,
        ai::AiPlugin,
        vfx::VfxPlugin,
        audio::GameAudioPlugin,
        ui::UiPlugin,
    ));
}

/// Собирает полноценное приложение игры (окно, физика, воксели, танк, UI).
pub fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Cerium-Turbo v1.3.1".into(),
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
    );
    configure_game_plugins(&mut app);
    app
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::{
        RenderPlugin,
        settings::{RenderCreation, WgpuSettings},
    };
    use bevy::winit::WinitPlugin;

    #[test]
    fn app_runs_hangar_battle_and_exit_without_missing_resources() {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .build()
                .disable::<WinitPlugin>()
                .disable::<bevy::log::LogPlugin>()
                .set(WindowPlugin {
                    primary_window: Some(Window::default()),
                    exit_condition: bevy::window::ExitCondition::DontExit,
                    ..default()
                })
                .set(RenderPlugin {
                    render_creation: RenderCreation::Automatic(Box::new(WgpuSettings {
                        backends: None,
                        ..default()
                    })),
                    ..default()
                }),
        );
        configure_game_plugins(&mut app);
        app.finish();
        app.cleanup();

        // Несколько кадров в ангаре (проверка StateTransition, First, PreUpdate, Update, PostUpdate).
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(
            *app.world().resource::<State<core::GameState>>().get(),
            core::GameState::Hangar
        );

        // Переход в бой.
        app.world_mut()
            .resource_mut::<NextState<core::GameState>>()
            .set(core::GameState::InBattle);
        for _ in 0..5 {
            app.update();
        }
        assert_eq!(
            *app.world().resource::<State<core::GameState>>().get(),
            core::GameState::InBattle
        );
        assert!(app.world().contains_resource::<voxel::VoxelWorld>());

        // Выстрел + шаг физики (проверка projectile/tank/turret/camera/hud систем).
        {
            let mut q = app.world_mut().query::<&mut tank::TankState>();
            for mut st in q.iter_mut(app.world_mut()) {
                st.throttle = 1.0;
                st.fire_queued = true;
            }
        }
        for _ in 0..3 {
            app.update();
        }

        // Возврат в ангар и очистка мира.
        app.world_mut()
            .resource_mut::<NextState<core::GameState>>()
            .set(core::GameState::Hangar);
        for _ in 0..3 {
            app.update();
        }
        assert!(!app.world().contains_resource::<voxel::VoxelWorld>());

        // Переключение режима на War Thunder (ModulesOnlyWt) клавишей M в ангаре.
        assert_eq!(
            *app.world().resource::<core::DamageRulesMode>(),
            core::DamageRulesMode::ArcadeHpAndModules
        );
        app.world_mut().write_message(bevy::input::keyboard::KeyboardInput {
            key_code: KeyCode::KeyM,
            logical_key: bevy::input::keyboard::Key::Character("m".into()),
            state: bevy::input::ButtonState::Pressed,
            text: None,
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
        app.update();
        app.world_mut().write_message(bevy::input::keyboard::KeyboardInput {
            key_code: KeyCode::KeyM,
            logical_key: bevy::input::keyboard::Key::Character("m".into()),
            state: bevy::input::ButtonState::Released,
            text: None,
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
        app.update();
        assert_eq!(
            *app.world().resource::<core::DamageRulesMode>(),
            core::DamageRulesMode::ModulesOnlyWt
        );

        // Вход в бой в режиме WT + проверка наличия экипажа у всех 10 танков 5v5 и работы XRay Hit-Cam.
        app.world_mut()
            .resource_mut::<NextState<core::GameState>>()
            .set(core::GameState::InBattle);
        for _ in 0..5 {
            app.update();
        }
        assert_eq!(
            *app.world().resource::<State<core::GameState>>().get(),
            core::GameState::InBattle
        );
        assert!(app.world().contains_resource::<damage::XRayHitReport>());
        {
            let mut q = app.world_mut().query::<&damage::TankCrew>();
            let crew_count = q.iter(app.world()).count();
            assert!(
                crew_count >= 10,
                "у игрока, 4 союзников и 5 врагов должен быть TankCrew: {crew_count}"
            );
        }
        app.world_mut().write_message(bevy::input::keyboard::KeyboardInput {
            key_code: KeyCode::Tab,
            logical_key: bevy::input::keyboard::Key::Tab,
            state: bevy::input::ButtonState::Pressed,
            text: None,
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
        app.update();
        app.world_mut().write_message(bevy::input::keyboard::KeyboardInput {
            key_code: KeyCode::Tab,
            logical_key: bevy::input::keyboard::Key::Tab,
            state: bevy::input::ButtonState::Released,
            text: None,
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
        app.update();
        assert_eq!(
            *app.world().resource::<State<core::GameState>>().get(),
            core::GameState::Hangar
        );
    }
}
