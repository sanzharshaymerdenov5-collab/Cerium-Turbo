//! Модуль танка: описания, воксельный билдер, спавн, подвеска, башня, камера.

pub mod armor;
pub mod builder;
pub mod camera;
pub mod controller;
pub mod spawn;
pub mod spec;
pub mod turret;

use bevy::prelude::*;

use crate::core::GameState;
pub use armor::{ArmorOwner, TankCombat};
pub use camera::{AimPlateInfo, AimTarget, CameraShake, TankCamera, spawn_battle_camera};
pub use controller::TankFiredMessage;
pub use spawn::{
    AimCommand, BarrelMesh, GunPivot, HangarPreviewTank, MuzzlePoint, PlayerTank, Suspension,
    TankRoot, TankState, TurretPivot, spawn_preview_tank, spawn_tank,
};
pub use spec::{
    ArmorScheme, ArmorZone, BT_7_RON, HullShape, IS_3_RON, KV_1_RON, M24_CHAFFEE_RON,
    M26_PERSHING_RON, M2A4_RON, M3_STUART_RON, M4A3E8_RON, MS_1_RON, Nation, PANTHER_RON,
    PZ_2_C_RON, PZ_3_RON, PZ_4_RON, ProceduralTankProfile, SU_100_RON, T34_76_RON, T34_85_RON,
    TIGER_1_RON, TIGER_2_RON, TankClass, TankSpec, TankVisualModel, TurretShape,
    VoxelVisualSource, available_tanks, load_tank_spec,
};

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TankAimSet;

pub struct TankPlugin;

impl Plugin for TankPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AimTarget>()
            .init_resource::<CameraShake>()
            .add_message::<TankFiredMessage>()
            .add_systems(
                Update,
                (
                    controller::read_player_input,
                    camera::update_tank_camera,
                    turret::aim_turret_and_gun,
                    camera::update_gun_aim_point,
                    controller::tank_fire_system,
                )
                    .chain()
                    .in_set(TankAimSet)
                    .run_if(in_state(GameState::InBattle)),
            )
            .add_systems(
                FixedUpdate,
                controller::tank_suspension_and_drive.run_if(in_state(GameState::InBattle)),
            )
            .add_systems(OnExit(GameState::InBattle), camera::release_cursor_on_exit);
    }
}
