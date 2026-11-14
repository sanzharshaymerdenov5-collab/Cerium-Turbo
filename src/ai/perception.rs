//! Восприятие ИИ: проверка прямой видимости (LoS) по воксельной сетке мира.

use bevy::math::Vec3;

use crate::voxel::VoxelGrid;

/// Проверяет, нет ли вокселей рельефа или неразрушенных построек на отрезке `[from, to]`.
pub fn has_line_of_sight(grid: &VoxelGrid, from: Vec3, to: Vec3) -> bool {
    let delta = to - from;
    let dist = delta.length();
    if dist < 1.5 {
        return true;
    }
    let dir = delta / dist;
    let check_dist = (dist - 1.5).max(0.1);
    grid.raycast(from, dir, check_dist).is_none()
}
