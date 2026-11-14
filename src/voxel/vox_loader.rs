//! Загрузчик `.vox` (MagicaVoxel) через `dot_vox` — для полировки визуала
//! поверх процедурно сгенерированных моделей.

use bevy::math::{IVec3, UVec3, Vec3};

use super::grid::VoxelGrid;

/// Загружает первую модель `.vox` в [`VoxelGrid`] с цветовой палитрой.
///
/// MagicaVoxel использует правостороннюю систему Z-вверх; конвертируем в Y-вверх
/// (`x -> x`, `z_vox -> y`, `y_vox -> -z`), чтобы передняя часть модели смотрела в `-Z`.
/// `origin` задаётся так, чтобы центр основания модели оказался в нуле.
pub fn load_vox(bytes: &[u8], voxel_size: f32) -> Result<VoxelGrid, String> {
    let data = dot_vox::load_bytes(bytes).map_err(|e| e.to_string())?;
    let model = data
        .models
        .first()
        .ok_or_else(|| "в .vox нет моделей".to_string())?;

    let (sx, sy, sz) = (model.size.x, model.size.y, model.size.z);
    let dims = UVec3::new(sx, sz, sy);
    let origin = Vec3::new(
        -(sx as f32) * 0.5 * voxel_size,
        0.0,
        -(sy as f32) * 0.5 * voxel_size,
    );
    let mut grid = VoxelGrid::new(dims, voxel_size, origin);

    // Палитра: индекс сетки = индекс палитры .vox + 1 (0 зарезервирован под воздух).
    let mut palette = vec![[0.0, 0.0, 0.0, 0.0]];
    for c in data.palette.iter().take(255) {
        palette.push([
            super::material::srgb_to_linear(c.r as f32 / 255.0),
            super::material::srgb_to_linear(c.g as f32 / 255.0),
            super::material::srgb_to_linear(c.b as f32 / 255.0),
            1.0,
        ]);
    }
    grid.palette = Some(palette);

    for v in &model.voxels {
        let p = IVec3::new(v.x as i32, v.z as i32, sy as i32 - 1 - v.y as i32);
        grid.set_raw(p, v.i.saturating_add(1));
    }
    Ok(grid)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Собирает минимальный валидный `.vox` с одной моделью.
    fn make_vox(size: (u32, u32, u32), voxels: &[(u8, u8, u8, u8)]) -> Vec<u8> {
        let mut size_chunk = Vec::new();
        size_chunk.extend_from_slice(b"SIZE");
        size_chunk.extend_from_slice(&12u32.to_le_bytes());
        size_chunk.extend_from_slice(&0u32.to_le_bytes());
        for v in [size.0, size.1, size.2] {
            size_chunk.extend_from_slice(&v.to_le_bytes());
        }
        let mut xyzi = Vec::new();
        xyzi.extend_from_slice(b"XYZI");
        xyzi.extend_from_slice(&(4 + 4 * voxels.len() as u32).to_le_bytes());
        xyzi.extend_from_slice(&0u32.to_le_bytes());
        xyzi.extend_from_slice(&(voxels.len() as u32).to_le_bytes());
        for (x, y, z, i) in voxels {
            xyzi.extend_from_slice(&[*x, *y, *z, *i]);
        }
        let children = (size_chunk.len() + xyzi.len()) as u32;
        let mut out = Vec::new();
        out.extend_from_slice(b"VOX ");
        out.extend_from_slice(&150u32.to_le_bytes());
        out.extend_from_slice(b"MAIN");
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&children.to_le_bytes());
        out.extend_from_slice(&size_chunk);
        out.extend_from_slice(&xyzi);
        out
    }

    #[test]
    fn loads_voxels_and_converts_axes() {
        // Модель 4x3x2 (x, y, z_up); вокселей: (0,0,0) и (3,2,1).
        let bytes = make_vox((4, 3, 2), &[(0, 0, 0, 1), (3, 2, 1, 5)]);
        let g = load_vox(&bytes, 0.1).expect("vox должен загрузиться");
        assert_eq!(g.dims, UVec3::new(4, 2, 3), "Z-вверх -> Y-вверх");
        assert_eq!(g.count_solid(), 2);
        // (0,0,0) -> (0,0, sy-1-0 = 2); (3,2,1) -> (3,1,0).
        assert!(g.is_solid(IVec3::new(0, 0, 2)));
        assert!(g.is_solid(IVec3::new(3, 1, 0)));
        assert!(g.palette.is_some());
    }

    #[test]
    fn rejects_garbage() {
        assert!(load_vox(b"not a vox file", 0.1).is_err());
    }
}
