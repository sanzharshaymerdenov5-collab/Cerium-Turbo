//! Базовые типы ядра: состояния игры, команды, исход боя, режим правил урона.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Глобальные состояния игры.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GameState {
    /// Ангар: выбор танка, карты, режима и старт боя.
    #[default]
    Hangar,
    /// Бой на карте.
    InBattle,
}

/// Сторона конфликта.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Team {
    Player,
    Enemy,
}

/// Текущий исход боя.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BattleStatus {
    #[default]
    Ongoing,
    Victory,
    Defeat,
}

/// Режим правил урона.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Resource)]
pub enum DamageRulesMode {
    /// Этапы 1–2: пробития/рикошеты фиксируются по счётчику попаданий.
    PrototypeTargets,
    /// Этапы 3–5: общее ХП + ХП модулей (стиль WoT).
    #[default]
    ArcadeHpAndModules,
    /// Этап 6: только модули и экипаж (стиль War Thunder).
    ModulesOnlyWt,
}

impl DamageRulesMode {
    pub fn ru_label(self) -> &'static str {
        match self {
            Self::PrototypeTargets => "Прототип",
            Self::ArcadeHpAndModules => "WoT (ХП + Модули)",
            Self::ModulesOnlyWt => "War Thunder (Только модули)",
        }
    }
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

/// Порядок инициализации боя при входе в `GameState::InBattle`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BattleSetupSet {
    ResolveMap,
    SpawnWorld,
    SpawnHud,
}

pub struct CorePlugin;

impl Plugin for CorePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<GameState>()
            .init_resource::<DamageRulesMode>()
            .init_resource::<BattleStatus>()
            .insert_resource(SelectedTank(TankId("ms_1".into())))
            .insert_resource(SelectedMap("proving_grounds".into()))
            .configure_sets(
                OnEnter(GameState::InBattle),
                (
                    BattleSetupSet::ResolveMap,
                    BattleSetupSet::SpawnWorld,
                    BattleSetupSet::SpawnHud,
                )
                    .chain(),
            );
    }
}
