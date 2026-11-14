//! Модуль танка: описания, воксельный билдер, спавн, подвеска, башня, камера.

pub mod builder;
pub mod camera;
pub mod controller;
pub mod spawn;
pub mod spec;
pub mod turret;

use bevy::prelude::*;

use crate::core::GameState;
pub use camera::{AimTarget, TankCamera, spawn_battle_camera};
pub use spawn::{
    GunPivot, MuzzlePoint, PlayerTank, Suspension, TankRoot, TankState, TurretPivot,
    spawn_preview_tank, spawn_tank,
};
pub use spec::{
    ProceduralTankProfile, T34_85_RON, TankSpec, VoxelVisualSource, available_tanks,
    load_tank_spec,
};

pub struct TankPlugin;

impl Plugin for TankPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AimTarget>()
            .add_systems(
                Update,
                (
                    controller::read_player_input,
                    camera::update_tank_camera,
                    turret::aim_turret_and_gun,
                    controller::tank_fire_system,
                )
                    .chain()
                    .run_if(in_state(GameState::InBattle)),
            )
            .add_systems(
                FixedUpdate,
                controller::tank_suspension_and_drive.run_if(in_state(GameState::InBattle)),
            )
            .add_systems(OnExit(GameState::InBattle), camera::release_cursor_on_exit);
    }
}
