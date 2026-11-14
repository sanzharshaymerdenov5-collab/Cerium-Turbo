//! Плотная воксельная сетка, raycast (DDA) и разрушение построек.

use std::collections::HashSet;

use bevy::math::{IVec3, UVec3, Vec3};
use bevy::prelude::Resource;

use super::material::Material;

/// Размер чанка (в вокселях) по каждой оси.
pub const CHUNK: i32 = 32;

/// Результат попадания луча в воксель.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoxelHit {
    pub voxel: IVec3,
    /// Расстояние от начала луча (в метрах).
    pub distance: f32,
    /// Нормаль грани, через которую луч вошёл в воксель (в сторону луча).
    pub normal: Vec3,
    pub material: Material,
    pub point: Vec3,
}

/// Плотная сетка вокселей. Значение вокселя — [`Material`] как `u8`,
/// либо индекс палитры, если задана `palette` (для `.vox`-моделей).
#[derive(Debug, Clone)]
pub struct VoxelGrid {
    pub dims: UVec3,
    pub voxel_size: f32,
    /// Координаты (в метрах) минимального угла сетки.
    pub origin: Vec3,
    /// Необязательная палитра цветов (линейный RGBA) вместо таблицы материалов.
    pub palette: Option<Vec<[f32; 4]>>,
    data: Vec<u8>,
}

impl VoxelGrid {
    pub fn new(dims: UVec3, voxel_size: f32, origin: Vec3) -> Self {
        let len = (dims.x as usize) * (dims.y as usize) * (dims.z as usize);
        Self {
            dims,
            voxel_size,
            origin,
            palette: None,
            data: vec![0; len],
        }
    }

    #[inline]
    fn index(&self, p: IVec3) -> Option<usize> {
        if p.x < 0 || p.y < 0 || p.z < 0 {
            return None;
        }
        let (x, y, z) = (p.x as u32, p.y as u32, p.z as u32);
        if x >= self.dims.x || y >= self.dims.y || z >= self.dims.z {
            return None;
        }
        Some(((y * self.dims.z + z) * self.dims.x + x) as usize)
    }

    #[inline]
    pub fn in_bounds(&self, p: IVec3) -> bool {
        self.index(p).is_some()
    }

    /// Сырое значение вокселя (0 вне сетки).
    #[inline]
    pub fn get_raw(&self, p: IVec3) -> u8 {
        self.index(p).map(|i| self.data[i]).unwrap_or(0)
    }

    #[inline]
    pub fn get(&self, p: IVec3) -> Material {
        Material::from_u8(self.get_raw(p))
    }

    #[inline]
    pub fn is_solid(&self, p: IVec3) -> bool {
        self.get_raw(p) != 0
    }

    pub fn set_raw(&mut self, p: IVec3, v: u8) {
        if let Some(i) = self.index(p) {
            self.data[i] = v;
        }
    }

    pub fn set(&mut self, p: IVec3, m: Material) {
        self.set_raw(p, m as u8);
    }

    /// Заполняет бокс `[min, max]` (включительно) материалом.
    pub fn fill_box(&mut self, min: IVec3, max: IVec3, m: Material) {
        for y in min.y..=max.y {
            for z in min.z..=max.z {
                for x in min.x..=max.x {
                    self.set(IVec3::new(x, y, z), m);
                }
            }
        }
    }

    pub fn count_solid(&self) -> usize {
        self.data.iter().filter(|v| **v != 0).count()
    }

    /// Мировая точка -> координата вокселя.
    pub fn world_to_voxel(&self, p: Vec3) -> IVec3 {
        ((p - self.origin) / self.voxel_size).floor().as_ivec3()
    }

    /// Центр вокселя в мировых координатах.
    pub fn voxel_center(&self, v: IVec3) -> Vec3 {
        self.origin + (v.as_vec3() + Vec3::splat(0.5)) * self.voxel_size
    }

    /// Размер сетки в метрах.
    pub fn size_m(&self) -> Vec3 {
        self.dims.as_vec3() * self.voxel_size
    }

    /// Высота (координата Y в метрах) верхней поверхности колонны (x,z)
    /// на мировой точке; `None` если вне сетки или колонна пуста.
    pub fn surface_height_at(&self, x: f32, z: f32) -> Option<f32> {
        let v = self.world_to_voxel(Vec3::new(x, self.origin.y, z));
        if v.x < 0 || v.z < 0 || v.x >= self.dims.x as i32 || v.z >= self.dims.z as i32 {
            return None;
        }
        for y in (0..self.dims.y as i32).rev() {
            if self.is_solid(IVec3::new(v.x, y, v.z)) {
                return Some(self.origin.y + (y + 1) as f32 * self.voxel_size);
            }
        }
        None
    }

    /// Луч по сетке (Amanatides–Woo DDA). `dir` нормализуется внутри.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<VoxelHit> {
        let dir = dir.try_normalize()?;
        let s = self.voxel_size;
        let local = (origin - self.origin) / s;
        let dims = self.dims.as_vec3();
        let max_t = max_dist / s;

        // Обрезка по габаритам сетки (slab-метод), t — в единицах вокселей.
        let mut t_enter = 0.0f32;
        let mut t_exit = max_t;
        let mut enter_axis: Option<usize> = None;
        for a in 0..3 {
            let d = dir[a];
            if d.abs() < 1e-9 {
                if local[a] < 0.0 || local[a] >= dims[a] {
                    return None;
                }
            } else {
                let inv = 1.0 / d;
                let mut ta = (0.0 - local[a]) * inv;
                let mut tb = (dims[a] - local[a]) * inv;
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                if ta > t_enter {
                    t_enter = ta;
                    enter_axis = Some(a);
                }
                t_exit = t_exit.min(tb);
            }
        }
        if t_enter > t_exit {
            return None;
        }

        let start = local + dir * (t_enter + 1e-4);
        let mut cell = start.floor().as_ivec3();
        cell = cell.clamp(IVec3::ZERO, self.dims.as_ivec3() - IVec3::ONE);

        let mut step = [0i32; 3];
        let mut t_max = [f32::INFINITY; 3];
        let mut t_delta = [f32::INFINITY; 3];
        for a in 0..3 {
            let d = dir[a];
            if d > 1e-9 {
                step[a] = 1;
                t_max[a] = ((cell[a] + 1) as f32 - local[a]) / d;
                t_delta[a] = 1.0 / d;
            } else if d < -1e-9 {
                step[a] = -1;
                t_max[a] = (cell[a] as f32 - local[a]) / d;
                t_delta[a] = -1.0 / d;
            }
        }

        let mut t_cur = t_enter;
        let mut last_axis = enter_axis;
        loop {
            let m = self.get(cell);
            if !m.is_air() {
                let mut normal = Vec3::ZERO;
                let axis = last_axis.unwrap_or_else(|| {
                    // Луч стартовал внутри вокселя: нормаль против доминирующей оси.
                    let a = dir.abs();
                    if a.x >= a.y && a.x >= a.z {
                        0
                    } else if a.y >= a.z {
                        1
                    } else {
                        2
                    }
                });
                normal[axis] = if dir[axis] > 0.0 { -1.0 } else { 1.0 };
                let distance = (t_cur * s).min(max_dist);
                return Some(VoxelHit {
                    voxel: cell,
                    distance,
                    normal,
                    material: m,
                    point: origin + dir * distance,
                });
            }
            // Шаг к следующей ячейке.
            let axis = if t_max[0] <= t_max[1] && t_max[0] <= t_max[2] {
                0
            } else if t_max[1] <= t_max[2] {
                1
            } else {
                2
            };
            t_cur = t_max[axis];
            if t_cur > t_exit {
                return None;
            }
            cell[axis] += step[axis];
            if !self.in_bounds(cell) {
                return None;
            }
            t_max[axis] += t_delta[axis];
            last_axis = Some(axis);
        }
    }
}

/// Мировая воксельная сцена: сетка + набор «грязных» чанков для пересборки.
#[derive(Resource, Debug, Clone)]
pub struct VoxelWorld {
    pub grid: VoxelGrid,
    pub dirty: HashSet<IVec3>,
    /// Области недавних разрушений построек `(центр, радиус)` для обновления навигационной сетки ИИ.
    pub carved_regions: Vec<(Vec3, f32)>,
}

/// Результат разрушения.
#[derive(Debug, Default, Clone)]
pub struct CarveResult {
    /// Удалённые воксели (координата, материал).
    pub removed: Vec<(IVec3, Material)>,
    /// Сколько вокселей рельефа «защитили» сферу от разрушения.
    pub protected_terrain: usize,
}

impl VoxelWorld {
    pub fn new(grid: VoxelGrid) -> Self {
        Self {
            grid,
            dirty: HashSet::new(),
            carved_regions: Vec::new(),
        }
    }

    /// Размер сетки в чанках.
    pub fn chunk_dims(&self) -> IVec3 {
        let d = self.grid.dims.as_ivec3();
        (d + IVec3::splat(CHUNK - 1)) / CHUNK
    }

    pub fn mark_dirty_voxel(&mut self, v: IVec3) {
        let c = v.div_euclid(IVec3::splat(CHUNK));
        let cd = self.chunk_dims();
        let mut add = |c: IVec3| {
            if c.cmpge(IVec3::ZERO).all() && c.cmplt(cd).all() {
                self.dirty.insert(c);
            }
        };
        add(c);
        // Грани на границе чанка видны соседу — его тоже пересобираем.
        let local = v.rem_euclid(IVec3::splat(CHUNK));
        for a in 0..3 {
            if local[a] == 0 {
                let mut n = c;
                n[a] -= 1;
                add(n);
            }
            if local[a] == CHUNK - 1 {
                let mut n = c;
                n[a] += 1;
                add(n);
            }
        }
    }

    /// Вырезает сферу. Удаляются **только разрушаемые** воксели построек;
    /// рельеф остаётся нетронутым.
    pub fn carve_sphere(&mut self, center: Vec3, radius: f32) -> CarveResult {
        let mut result = CarveResult::default();
        let s = self.grid.voxel_size;
        let min = self.grid.world_to_voxel(center - Vec3::splat(radius));
        let max = self.grid.world_to_voxel(center + Vec3::splat(radius));
        for y in min.y..=max.y {
            for z in min.z..=max.z {
                for x in min.x..=max.x {
                    let v = IVec3::new(x, y, z);
                    let m = self.grid.get(v);
                    if m.is_air() {
                        continue;
                    }
                    let c = self.grid.voxel_center(v);
                    if c.distance(center) > radius + s * 0.5 {
                        continue;
                    }
                    if m.is_destructible() {
                        self.grid.set(v, Material::Air);
                        result.removed.push((v, m));
                    } else {
                        result.protected_terrain += 1;
                    }
                }
            }
        }
        if !result.removed.is_empty() {
            self.carved_regions.push((center, radius));
        }
        let removed: Vec<IVec3> = result.removed.iter().map(|(v, _)| *v).collect();
        for v in removed {
            self.mark_dirty_voxel(v);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_world() -> VoxelWorld {
        // 64x32x64, шаг 0.5 м; земля до y=7 включительно (высота 4 м); кирпичная стена выше.
        let mut g = VoxelGrid::new(UVec3::new(64, 32, 64), 0.5, Vec3::new(-16.0, 0.0, -16.0));
        g.fill_box(IVec3::new(0, 0, 0), IVec3::new(63, 7, 63), Material::Soil);
        g.fill_box(IVec3::new(20, 8, 30), IVec3::new(40, 15, 31), Material::Brick);
        VoxelWorld::new(g)
    }

    #[test]
    fn raycast_down_hits_ground_at_expected_distance() {
        let w = flat_world();
        let hit = w
            .grid
            .raycast(Vec3::new(0.0, 10.0, 0.0), Vec3::NEG_Y, 100.0)
            .expect("луч вниз должен попасть в землю");
        assert!((hit.distance - 6.0).abs() < 0.01, "distance={}", hit.distance);
        assert_eq!(hit.normal, Vec3::Y);
        assert_eq!(hit.material, Material::Soil);
    }

    #[test]
    fn raycast_horizontal_hits_wall_and_misses_when_short() {
        let w = flat_world();
        // Стена: x вокселей 20..40 -> мир x = -6..4; z вокселей 30..31 -> мир z = -1..0.
        let o = Vec3::new(0.0, 5.0, 10.0);
        let hit = w.grid.raycast(o, Vec3::NEG_Z, 100.0).unwrap();
        assert_eq!(hit.material, Material::Brick);
        assert!((hit.distance - 10.0).abs() < 0.01, "distance={}", hit.distance);
        assert_eq!(hit.normal, Vec3::Z);
        assert!(w.grid.raycast(o, Vec3::NEG_Z, 5.0).is_none());
    }

    #[test]
    fn raycast_from_outside_grid_and_parallel_miss() {
        let w = flat_world();
        // Снаружи сетки, летит над землёй мимо — промах по вертикали (выше стены).
        assert!(
            w.grid
                .raycast(Vec3::new(-50.0, 20.0, 0.0), Vec3::X, 200.0)
                .is_none()
        );
        // Снаружи, но попадает в землю.
        let hit = w
            .grid
            .raycast(Vec3::new(-50.0, 2.0, 0.0), Vec3::X, 200.0)
            .unwrap();
        assert!((hit.distance - 34.0).abs() < 0.01, "distance={}", hit.distance);
    }

    #[test]
    fn carve_removes_only_destructible_voxels_and_marks_dirty() {
        let mut w = flat_world();
        let terrain_before = w.grid.count_solid();
        // Сфера на стыке земли и стены: центр на высоте 4 м (граница), на стене.
        let res = w.carve_sphere(Vec3::new(-1.0, 4.5, -0.5), 1.5);
        assert!(!res.removed.is_empty(), "стена должна быть повреждена");
        assert!(res.protected_terrain > 0, "рельеф в сфере должен остаться");
        assert!(res.removed.iter().all(|(_, m)| *m == Material::Brick));
        // Земля цела: любой воксель рельефа остался.
        assert_eq!(w.grid.get(IVec3::new(25, 7, 30)), Material::Soil);
        assert_eq!(
            w.grid.count_solid(),
            terrain_before - res.removed.len(),
            "изменилось только число разрушаемых вокселей"
        );
        assert!(!w.dirty.is_empty());
    }

    #[test]
    fn terrain_only_sphere_changes_nothing() {
        let mut w = flat_world();
        let before = w.grid.count_solid();
        let res = w.carve_sphere(Vec3::new(10.0, 2.0, 10.0), 2.0);
        assert!(res.removed.is_empty());
        assert!(res.protected_terrain > 0);
        assert_eq!(w.grid.count_solid(), before);
        assert!(w.dirty.is_empty());
    }

    #[test]
    fn dirty_marks_neighbour_chunk_on_boundary() {
        let mut w = flat_world();
        w.mark_dirty_voxel(IVec3::new(31, 5, 5));
        assert!(w.dirty.contains(&IVec3::new(0, 0, 0)));
        assert!(w.dirty.contains(&IVec3::new(1, 0, 0)));
    }

    #[test]
    fn surface_height_matches_fill() {
        let w = flat_world();
        let h = w.grid.surface_height_at(0.0, 10.0).unwrap();
        assert!((h - 4.0).abs() < 1e-4);
        assert!(w.grid.surface_height_at(500.0, 0.0).is_none());
    }
}
