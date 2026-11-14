//! Пользовательский интерфейс: ангар и боевой HUD.

pub mod hangar;
pub mod hud;

use bevy::prelude::*;

use crate::core::GameState;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Hangar), hangar::setup_hangar)
            .add_systems(Update, hangar::update_hangar.run_if(in_state(GameState::Hangar)))
            .add_systems(OnEnter(GameState::InBattle), hud::setup_hud)
            .add_systems(Update, hud::update_hud.run_if(in_state(GameState::InBattle)));
    }
}
