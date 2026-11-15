//! Баллистика: типы снарядов (только AP), пробитие, геометрия, снаряды.

pub mod geometry;
pub mod penetration;
pub mod projectile;
pub mod shell;

pub use projectile::{
    ArmorImpactMessage, ArmorPlate, BallisticsPlugin, HitKind, HitLog, Projectile, TargetInfo,
    TerrainImpactMessage, spawn_projectile,
};
pub use shell::{ShellSpec, ShellType};
