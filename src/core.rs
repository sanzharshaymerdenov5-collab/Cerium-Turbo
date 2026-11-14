//! Базовые типы ядра: состояния игры, идентификаторы, режим правил урона.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Глобальные состояния игры.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GameState {
    /// Ангар: выбор танка и старт боя.
    #[default]
    Hangar,
    /// Бой на карте.
    InBattle,
}

/// Режим правил урона. Этап 1–2 используют `PrototypeTargets`;
/// остальные режимы появятся на Этапах 3 и 6 (переключатель).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Resource)]
pub enum DamageRulesMode {
    /// Этапы 1–2: пробития/рикошеты фиксируются на мишенях.
    #[default]
    PrototypeTargets,
    /// Этапы 3–5: общее ХП + ХП модулей (стиль WoT).
    ArcadeHpAndModules,
    /// Этап 6: только модули и экипаж (стиль War Thunder).
    ModulesOnlyWt,
}

/// Идентификатор танка (имя `.ron`-файла без расширения).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TankId(pub String);

/// Какой танк выбран в ангаре.
#[derive(Resource, Debug, Clone)]
pub struct SelectedTank(pub TankId);

/// Какая карта выбрана.
#[derive(Resource, Debug, Clone)]
pub struct SelectedMap(pub String);

pub struct CorePlugin;

impl Plugin for CorePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<GameState>()
            .init_resource::<DamageRulesMode>()
            .insert_resource(SelectedTank(TankId("t34_85".into())))
            .insert_resource(SelectedMap("proving_grounds".into()));
    }
}
