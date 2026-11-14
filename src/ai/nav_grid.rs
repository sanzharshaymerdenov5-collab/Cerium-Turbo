//! 2.5D Навигационная сетка (`VoxelNavGrid`) и алгоритм A* с обновлением при разрушениях.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use bevy::math::{IVec2, IVec3, Vec2, Vec3};
use bevy::prelude::*;

use super::perception::has_line_of_sight;
use crate::voxel::{Material, VoxelGrid, VoxelWorld};

/// Высота неразрушаемого рельефа (без построек) в мировой точке `(x, z)`.
pub fn terrain_only_height(grid: &VoxelGrid, x: f32, z: f32) -> f32 {
    let v = grid.world_to_voxel(Vec3::new(x, grid.origin.y, z));
    let vx = v.x.clamp(0, grid.dims.x as i32 - 1);
    let vz = v.z.clamp(0, grid.dims.z as i32 - 1);
    for y in (0..grid.dims.y as i32).rev() {
        let m = grid.get(IVec3::new(vx, y, vz));
        if !m.is_air()
            && !m.is_destructible()
            && m != Material::TargetSteel
            && m != Material::TargetPost
        {
            return grid.origin.y + (y + 1) as f32 * grid.voxel_size;
        }
    }
    grid.origin.y + grid.voxel_size
}

/// 2.5D навигационная сетка для ИИ-танков.
#[derive(Resource, Debug, Clone)]
pub struct VoxelNavGrid {
    pub cell_size: f32,
    pub cols: usize,
    pub rows: usize,
    pub origin_xz: Vec2,
    pub heights: Vec<f32>,
    pub terrain_blocked: Vec<bool>,
    pub structure_blocked: Vec<bool>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct OpenNode {
    idx: usize,
    f_cost: u32,
}

impl Ord for OpenNode {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .f_cost
            .cmp(&self.f_cost)
            .then_with(|| self.idx.cmp(&other.idx))
    }
}

impl PartialOrd for OpenNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl VoxelNavGrid {
    pub const DEFAULT_CELL_M: f32 = 2.0;

    /// Строит навигационную сетку целиком по воксельному миру.
    pub fn from_grid(grid: &VoxelGrid, cell_size: f32) -> Self {
        let size = grid.size_m();
        let cols = ((size.x / cell_size).ceil() as usize).max(4);
        let rows = ((size.z / cell_size).ceil() as usize).max(4);
        let total = cols * rows;
        let mut nav = Self {
            cell_size,
            cols,
            rows,
            origin_xz: Vec2::new(grid.origin.x, grid.origin.z),
            heights: vec![grid.origin.y; total],
            terrain_blocked: vec![false; total],
            structure_blocked: vec![false; total],
        };

        for cz in 0..rows {
            for cx in 0..cols {
                let idx = cz * cols + cx;
                let p = nav.cell_center_xz(cx, cz);
                nav.heights[idx] = terrain_only_height(grid, p.x, p.y);
            }
        }
        nav.recompute_terrain_slopes();
        nav.update_cell_rect(grid, 0, cols - 1, 0, rows - 1);
        nav
    }

    #[inline]
    pub fn in_bounds(&self, c: IVec2) -> bool {
        c.x >= 0 && c.y >= 0 && (c.x as usize) < self.cols && (c.y as usize) < self.rows
    }

    #[inline]
    pub fn idx(&self, cx: usize, cz: usize) -> usize {
        cz * self.cols + cx
    }

    #[inline]
    pub fn cell_center_xz(&self, cx: usize, cz: usize) -> Vec2 {
        self.origin_xz + Vec2::new((cx as f32 + 0.5) * self.cell_size, (cz as f32 + 0.5) * self.cell_size)
    }

    pub fn world_to_cell(&self, pos: Vec3) -> IVec2 {
        let rel = Vec2::new(pos.x, pos.z) - self.origin_xz;
        IVec2::new(
            (rel.x / self.cell_size).floor() as i32,
            (rel.y / self.cell_size).floor() as i32,
        )
    }

    pub fn cell_to_world(&self, c: IVec2) -> Vec3 {
        let cx = c.x.clamp(0, self.cols as i32 - 1) as usize;
        let cz = c.y.clamp(0, self.rows as i32 - 1) as usize;
        let xz = self.cell_center_xz(cx, cz);
        Vec3::new(xz.x, self.heights[self.idx(cx, cz)] + 0.25, xz.y)
    }

    #[inline]
    pub fn is_walkable(&self, c: IVec2) -> bool {
        if !self.in_bounds(c) {
            return false;
        }
        let i = self.idx(c.x as usize, c.y as usize);
        !self.terrain_blocked[i] && !self.structure_blocked[i]
    }

    fn recompute_terrain_slopes(&mut self) {
        let max_step = self.cell_size * 0.72;
        for cz in 0..self.rows {
            for cx in 0..self.cols {
                let i = self.idx(cx, cz);
                if cx <= 1 || cz <= 1 || cx + 2 >= self.cols || cz + 2 >= self.rows {
                    self.terrain_blocked[i] = true;
                    continue;
                }
                let h = self.heights[i];
                let mut steep = false;
                for (dx, dz) in [(1isize, 0), (-1, 0), (0, 1), (0, -1)] {
                    let ni = self.idx((cx as isize + dx) as usize, (cz as isize + dz) as usize);
                    if (self.heights[ni] - h).abs() > max_step {
                        steep = true;
                        break;
                    }
                }
                self.terrain_blocked[i] = steep;
            }
        }
    }

    /// Инкрементально пересчитывает проходимость построек в окрестности `(center, radius_m)`.
    pub fn update_region(&mut self, grid: &VoxelGrid, center: Vec3, radius_m: f32) {
        let r = radius_m + 3.0;
        let min_c = self.world_to_cell(center - Vec3::new(r, 0.0, r));
        let max_c = self.world_to_cell(center + Vec3::new(r, 0.0, r));
        let x0 = min_c.x.clamp(0, self.cols as i32 - 1) as usize;
        let x1 = max_c.x.clamp(0, self.cols as i32 - 1) as usize;
        let z0 = min_c.y.clamp(0, self.rows as i32 - 1) as usize;
        let z1 = max_c.y.clamp(0, self.rows as i32 - 1) as usize;
        self.update_cell_rect(grid, x0.min(x1), x0.max(x1), z0.min(z1), z0.max(z1));
    }

    fn update_cell_rect(
        &mut self,
        grid: &VoxelGrid,
        x0: usize,
        x1: usize,
        z0: usize,
        z1: usize,
    ) {
        let vs = grid.voxel_size;
        // Габарит полуширины танка (1.35 м), чтобы маршрут не цеплял стены углом корпуса.
        let half_footprint = 1.35;
        let clearance_h = (2.4 / vs).ceil() as i32;

        for cz in z0..=z1 {
            for cx in x0..=x1 {
                let i = self.idx(cx, cz);
                let c_xz = self.cell_center_xz(cx, cz);
                let h = self.heights[i];
                let base_vy = ((h - grid.origin.y) / vs).round() as i32;

                let vmin = grid.world_to_voxel(Vec3::new(
                    c_xz.x - half_footprint,
                    0.0,
                    c_xz.y - half_footprint,
                ));
                let vmax = grid.world_to_voxel(Vec3::new(
                    c_xz.x + half_footprint,
                    0.0,
                    c_xz.y + half_footprint,
                ));

                let mut blocked = false;
                'scan: for vz in vmin.z.max(0)..=vmax.z.min(grid.dims.z as i32 - 1) {
                    for vx in vmin.x.max(0)..=vmax.x.min(grid.dims.x as i32 - 1) {
                        for vy in (base_vy + 1)..=(base_vy + clearance_h).min(grid.dims.y as i32 - 1) {
                            let m = grid.get(IVec3::new(vx, vy, vz));
                            if m.is_destructible()
                                || m == Material::TargetSteel
                                || m == Material::TargetPost
                            {
                                blocked = true;
                                break 'scan;
                            }
                        }
                    }
                }
                self.structure_blocked[i] = blocked;
            }
        }
    }

    /// Находит ближайшую проходимую клетку (если исходная оказалась в препятствии).
    pub fn nearest_walkable(&self, world_pos: Vec3, max_radius_cells: i32) -> Option<IVec2> {
        let start = self.world_to_cell(world_pos);
        if self.is_walkable(start) {
            return Some(start);
        }
        let mut best: Option<(i32, IVec2)> = None;
        for dz in -max_radius_cells..=max_radius_cells {
            for dx in -max_radius_cells..=max_radius_cells {
                let c = start + IVec2::new(dx, dz);
                if self.is_walkable(c) {
                    let d2 = dx * dx + dz * dz;
                    if best.is_none_or(|(bd, _)| d2 < bd) {
                        best = Some((d2, c));
                    }
                }
            }
        }
        best.map(|(_, c)| c)
    }

    /// Проверяет видимость/прямолинейную проходимость между двумя клетками сетки.
    fn nav_segment_clear(&self, a: IVec2, b: IVec2) -> bool {
        let steps = ((a.x - b.x).abs().max((a.y - b.y).abs()) * 2).max(1);
        for s in 0..=steps {
            let t = s as f32 / steps as f32;
            let fx = a.x as f32 + (b.x - a.x) as f32 * t;
            let fz = a.y as f32 + (b.y - a.y) as f32 * t;
            let c = IVec2::new(fx.round() as i32, fz.round() as i32);
            if !self.is_walkable(c) {
                return false;
            }
        }
        true
    }

    /// Поиск пути A* с оптимизацией сглаживания (в мировых координатах, без начальной клетки).
    pub fn find_path(&self, from_world: Vec3, to_world: Vec3) -> Vec<Vec3> {
        let Some(start) = self.nearest_walkable(from_world, 5) else {
            return Vec::new();
        };
        let Some(goal) = self.nearest_walkable(to_world, 5) else {
            return Vec::new();
        };
        if start == goal {
            return vec![self.cell_to_world(goal)];
        }

        let total = self.cols * self.rows;
        let mut g_score = vec![u32::MAX; total];
        let mut parent = vec![usize::MAX; total];
        let mut closed = vec![false; total];
        let mut heap = BinaryHeap::new();

        let start_idx = self.idx(start.x as usize, start.y as usize);
        let goal_idx = self.idx(goal.x as usize, goal.y as usize);
        let heuristic = |c: IVec2| -> u32 {
            let dx = (c.x - goal.x).abs() as u32;
            let dz = (c.y - goal.y).abs() as u32;
            let diag = dx.min(dz);
            let straight = dx.max(dz) - diag;
            diag * 14 + straight * 10
        };

        g_score[start_idx] = 0;
        heap.push(OpenNode {
            idx: start_idx,
            f_cost: heuristic(start),
        });

        let dirs: [(i32, i32, u32); 8] = [
            (1, 0, 10),
            (-1, 0, 10),
            (0, 1, 10),
            (0, -1, 10),
            (1, 1, 14),
            (1, -1, 14),
            (-1, 1, 14),
            (-1, -1, 14),
        ];

        let mut iters = 0usize;
        let mut found = false;
        while let Some(cur) = heap.pop() {
            if cur.idx == goal_idx {
                found = true;
                break;
            }
            if closed[cur.idx] {
                continue;
            }
            closed[cur.idx] = true;
            iters += 1;
            if iters > 25_000 {
                break;
            }

            let cx = (cur.idx % self.cols) as i32;
            let cz = (cur.idx / self.cols) as i32;
            let cur_g = g_score[cur.idx];

            for (dx, dz, step_cost) in dirs {
                let nx = cx + dx;
                let nz = cz + dz;
                let nc = IVec2::new(nx, nz);
                if !self.is_walkable(nc) {
                    continue;
                }
                if dx != 0 && dz != 0 {
                    if !self.is_walkable(IVec2::new(cx + dx, cz))
                        || !self.is_walkable(IVec2::new(cx, cz + dz))
                    {
                        continue;
                    }
                }
                let ni = self.idx(nx as usize, nz as usize);
                if closed[ni] {
                    continue;
                }
                // Лёгкий штраф за крутизну подъёма.
                let dh = (self.heights[ni] - self.heights[cur.idx]).abs();
                let slope_penalty = (dh * 4.0) as u32;
                let ng = cur_g.saturating_add(step_cost + slope_penalty);
                if ng < g_score[ni] {
                    g_score[ni] = ng;
                    parent[ni] = cur.idx;
                    heap.push(OpenNode {
                        idx: ni,
                        f_cost: ng.saturating_add(heuristic(nc)),
                    });
                }
            }
        }

        if !found {
            return Vec::new();
        }

        let mut raw_cells = Vec::new();
        let mut cur = goal_idx;
        while cur != usize::MAX && cur != start_idx {
            raw_cells.push(IVec2::new((cur % self.cols) as i32, (cur / self.cols) as i32));
            cur = parent[cur];
        }
        raw_cells.push(start);
        raw_cells.reverse();

        // Сглаживание (string-pulling).
        let mut smoothed = Vec::new();
        let mut anchor = 0usize;
        while anchor + 1 < raw_cells.len() {
            let mut furthest = anchor + 1;
            for test in (anchor + 2)..raw_cells.len() {
                if self.nav_segment_clear(raw_cells[anchor], raw_cells[test]) {
                    furthest = test;
                }
            }
            smoothed.push(self.cell_to_world(raw_cells[furthest]));
            anchor = furthest;
        }
        if smoothed.is_empty() {
            smoothed.push(self.cell_to_world(goal));
        }
        smoothed
    }

    /// Ищет соседнюю точку укрытия (нет LoS до `threat_pos`) в радиусе `search_radius_m`.
    pub fn find_cover_near(
        &self,
        grid: &VoxelGrid,
        bot_pos: Vec3,
        threat_pos: Vec3,
        search_radius_m: f32,
    ) -> Option<Vec3> {
        let base = self.world_to_cell(bot_pos);
        let r_cells = (search_radius_m / self.cell_size).ceil() as i32;
        let threat_eye = threat_pos + Vec3::Y * 1.6;
        let mut best: Option<(f32, Vec3)> = None;

        for dz in (-r_cells..=r_cells).step_by(2) {
            for dx in (-r_cells..=r_cells).step_by(2) {
                let c = base + IVec2::new(dx, dz);
                if !self.is_walkable(c) {
                    continue;
                }
                let wp = self.cell_to_world(c);
                let dist_bot = wp.distance(bot_pos);
                if dist_bot < 5.0 || dist_bot > search_radius_m {
                    continue;
                }
                let cover_eye = wp + Vec3::Y * 1.5;
                if !has_line_of_sight(grid, cover_eye, threat_eye) {
                    // Предпочитаем близкое укрытие, чуть дальше от угрозы.
                    let dist_threat = wp.distance(threat_pos);
                    let score = dist_bot - dist_threat * 0.25;
                    if best.is_none_or(|(bs, _)| score < bs) {
                        best = Some((score, wp));
                    }
                }
            }
        }
        best.map(|(_, p)| p)
    }
}

/// Синхронизирует `VoxelNavGrid` при разрушении воксельных построек в бою.
pub fn sync_nav_grid_with_destruction(
    mut world: ResMut<VoxelWorld>,
    mut nav: ResMut<VoxelNavGrid>,
) {
    if world.carved_regions.is_empty() {
        return;
    }
    let regions: Vec<(Vec3, f32)> = world.carved_regions.drain(..).collect();
    for (center, radius) in regions {
        nav.update_region(&world.grid, center, radius);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::UVec3;

    fn path_length_xz(start: Vec3, path: &[Vec3]) -> f32 {
        let mut total = 0.0;
        let mut prev = Vec2::new(start.x, start.z);
        for p in path {
            let cur = Vec2::new(p.x, p.z);
            total += prev.distance(cur);
            prev = cur;
        }
        total
    }

    #[test]
    fn nav_grid_detours_around_wall_and_repaths_after_destruction() {
        // 128x16x128 вокселей по 0.5 м -> поле 64x64 м ([-32..32]).
        let mut g = VoxelGrid::new(UVec3::new(128, 16, 128), 0.5, Vec3::new(-32.0, 0.0, -32.0));
        g.fill_box(IVec3::new(0, 0, 0), IVec3::new(127, 3, 127), Material::Grass);
        // Кирпичная стена на z=0 (воксели z=63..64), от x=-14 до +14 м (воксели x=36..92).
        g.fill_box(IVec3::new(36, 4, 63), IVec3::new(92, 9, 64), Material::Brick);

        let mut world = VoxelWorld::new(g);
        let mut nav = VoxelNavGrid::from_grid(&world.grid, 2.0);

        let start = Vec3::new(0.0, 2.5, 14.0);
        let goal = Vec3::new(0.0, 2.5, -14.0);
        assert!(!has_line_of_sight(&world.grid, start, goal));

        let detour = nav.find_path(start, goal);
        assert!(!detour.is_empty(), "путь в обход стены должен существовать");
        let detour_len = path_length_xz(start, &detour);
        // Прямой путь 28 м; в обход стены x=±14 м будет заметно длиннее (>34 м).
        assert!(detour_len > 34.0, "detour_len={detour_len}");

        // Пробиваем проём шириной ~5 м в центре стены (x=0, z=0).
        world.carve_sphere(Vec3::new(0.0, 2.8, 0.0), 3.0);
        world.carve_sphere(Vec3::new(-1.5, 2.8, 0.0), 2.2);
        world.carve_sphere(Vec3::new(1.5, 2.8, 0.0), 2.2);
        for (c, r) in world.carved_regions.drain(..) {
            nav.update_region(&world.grid, c, r);
        }

        assert!(has_line_of_sight(&world.grid, start, goal));
        let direct = nav.find_path(start, goal);
        assert!(!direct.is_empty());
        let direct_len = path_length_xz(start, &direct);
        assert!(
            direct_len < detour_len - 5.0,
            "после разрушения стены путь через проём ({direct_len:.1}) должен стать короче обхода ({detour_len:.1})"
        );
    }
}
