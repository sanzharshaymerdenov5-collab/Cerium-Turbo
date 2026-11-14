//! Greedy-мешинг вокселей через `block-mesh` и конвертация в Bevy `Mesh`.

use bevy::asset::RenderAssetUsages;
use bevy::math::{IVec3, UVec3, Vec3};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use block_mesh::ndshape::{RuntimeShape, Shape};
use block_mesh::{
    GreedyQuadsBuffer, MergeVoxel, RIGHT_HANDED_Y_UP_CONFIG, Voxel, VoxelVisibility,
    greedy_quads,
};

use super::grid::VoxelGrid;
use super::material::Material;

#[derive(Clone, Copy, PartialEq, Eq)]
struct V(u8);

impl Voxel for V {
    fn get_visibility(&self) -> VoxelVisibility {
        if self.0 == 0 {
            VoxelVisibility::Empty
        } else {
            VoxelVisibility::Opaque
        }
    }
}

impl MergeVoxel for V {
    type MergeValue = u8;
    fn merge_value(&self) -> u8 {
        self.0
    }
}

/// Геометрия, готовая к превращению в `Mesh`.
#[derive(Debug, Default, Clone)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

impl MeshData {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn quad_count(&self) -> usize {
        self.indices.len() / 6
    }

    pub fn into_mesh(self) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
        .with_inserted_indices(Indices::U32(self.indices))
    }
}

/// Сколько квадов дал бы «наивный» мешинг (по одной грани на каждую открытую
/// грань вокселя) — нужен для проверки выигрыша greedy-алгоритма.
pub fn naive_face_count(grid: &VoxelGrid, min: IVec3, dim: UVec3) -> usize {
    let mut n = 0;
    let dirs = [
        IVec3::X,
        IVec3::NEG_X,
        IVec3::Y,
        IVec3::NEG_Y,
        IVec3::Z,
        IVec3::NEG_Z,
    ];
    for y in 0..dim.y as i32 {
        for z in 0..dim.z as i32 {
            for x in 0..dim.x as i32 {
                let p = min + IVec3::new(x, y, z);
                if !grid.is_solid(p) {
                    continue;
                }
                for d in dirs {
                    if !grid.is_solid(p + d) {
                        n += 1;
                    }
                }
            }
        }
    }
    n
}

fn color_of(grid: &VoxelGrid, raw: u8) -> [f32; 4] {
    match &grid.palette {
        Some(p) => p.get(raw as usize).copied().unwrap_or([1.0, 0.0, 1.0, 1.0]),
        None => Material::from_u8(raw).linear_rgba(),
    }
}

/// Мешит область `[min, min+dim)` сетки. Вершины — **относительно `grid.origin`**:
/// `индекс * voxel_size` (сущность с мешем размещается в `grid.origin`).
pub fn mesh_region(grid: &VoxelGrid, min: IVec3, dim: UVec3) -> MeshData {
    let padded = [dim.x + 2, dim.y + 2, dim.z + 2];
    let shape = RuntimeShape::<u32, 3>::new(padded);
    let mut voxels = vec![V(0); shape.size() as usize];
    for i in 0..shape.size() {
        let [x, y, z] = shape.delinearize(i);
        let p = min + IVec3::new(x as i32 - 1, y as i32 - 1, z as i32 - 1);
        voxels[i as usize] = V(grid.get_raw(p));
    }

    let mut buffer = GreedyQuadsBuffer::new(voxels.len());
    greedy_quads(
        &voxels,
        &shape,
        [0; 3],
        [padded[0] - 1, padded[1] - 1, padded[2] - 1],
        &RIGHT_HANDED_Y_UP_CONFIG.faces,
        &mut buffer,
    );

    let mut data = MeshData::default();
    let s = grid.voxel_size;
    let shift = Vec3::new(min.x as f32 - 1.0, min.y as f32 - 1.0, min.z as f32 - 1.0);
    for (group, face) in buffer
        .quads
        .groups
        .iter()
        .zip(RIGHT_HANDED_Y_UP_CONFIG.faces.iter())
    {
        for quad in group.iter() {
            let start = data.positions.len() as u32;
            let corners = face.quad_mesh_positions(quad, 1.0);
            let normals = face.quad_mesh_normals();
            let raw = voxels[shape.linearize(quad.minimum) as usize].0;
            let color = color_of(grid, raw);
            for (c, n) in corners.iter().zip(normals.iter()) {
                let p = (Vec3::from(*c) + shift) * s;
                data.positions.push(p.to_array());
                data.normals.push(*n);
                data.colors.push(color);
            }
            data.indices.extend_from_slice(&face.quad_mesh_indices(start));
        }
    }
    data
}

/// Мешит всю сетку целиком (для танков и небольших моделей).
pub fn mesh_grid(grid: &VoxelGrid) -> MeshData {
    mesh_region(grid, IVec3::ZERO, grid.dims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_voxel_gives_six_quads_at_right_place() {
        let mut g = VoxelGrid::new(UVec3::new(4, 4, 4), 0.5, Vec3::new(1.0, 2.0, 3.0));
        g.set(IVec3::new(1, 1, 1), Material::Brick);
        let m = mesh_grid(&g);
        assert_eq!(m.quad_count(), 6);
        assert_eq!(m.positions.len(), 24);
        // Воксель (1,1,1) при шаге 0.5 занимает [0.5,1.0] по каждой оси (относительно origin).
        for p in &m.positions {
            for c in p {
                assert!(*c >= 0.5 - 1e-4 && *c <= 1.0 + 1e-4, "coord={c}");
            }
        }
    }

    #[test]
    fn greedy_merges_flat_slab_into_few_quads() {
        let mut g = VoxelGrid::new(UVec3::new(32, 8, 32), 0.5, Vec3::ZERO);
        g.fill_box(IVec3::new(0, 0, 0), IVec3::new(31, 3, 31), Material::Soil);
        let greedy = mesh_grid(&g).quad_count();
        let naive = naive_face_count(&g, IVec3::ZERO, g.dims);
        assert_eq!(greedy, 6, "плита должна слиться в 6 больших квадов");
        assert!(naive > greedy * 50, "naive={naive} greedy={greedy}");
    }

    #[test]
    fn different_materials_are_not_merged_and_carry_colors() {
        let mut g = VoxelGrid::new(UVec3::new(4, 2, 2), 1.0, Vec3::ZERO);
        g.set(IVec3::new(0, 0, 0), Material::Brick);
        g.set(IVec3::new(1, 0, 0), Material::Concrete);
        let m = mesh_grid(&g);
        let brick = Material::Brick.linear_rgba();
        let concrete = Material::Concrete.linear_rgba();
        assert!(m.colors.contains(&brick));
        assert!(m.colors.contains(&concrete));
        // 2 вокселя разных материалов: 5+5 внешних граней (общая грань скрыта).
        assert_eq!(m.quad_count(), 10);
    }

    #[test]
    fn region_meshing_sees_neighbour_across_chunk_border() {
        let mut g = VoxelGrid::new(UVec3::new(64, 4, 4), 0.5, Vec3::ZERO);
        g.fill_box(IVec3::new(0, 0, 0), IVec3::new(63, 1, 3), Material::Soil);
        // Левый чанк x 0..32: на границе x=31|32 грань между вокселями не должна появляться.
        let left = mesh_region(&g, IVec3::ZERO, UVec3::new(32, 4, 4));
        let has_face_on_border = left
            .positions
            .iter()
            .zip(left.normals.iter())
            .any(|(p, n)| n[0] > 0.5 && (p[0] - 16.0).abs() < 1e-4);
        assert!(!has_face_on_border, "внутренняя грань на границе чанка");
    }

    #[test]
    fn palette_overrides_material_color() {
        let mut g = VoxelGrid::new(UVec3::new(2, 2, 2), 1.0, Vec3::ZERO);
        g.palette = Some(vec![[0.0; 4], [0.1, 0.2, 0.3, 1.0]]);
        g.set_raw(IVec3::new(0, 0, 0), 1);
        let m = mesh_grid(&g);
        assert!(m.colors.iter().all(|c| *c == [0.1, 0.2, 0.3, 1.0]));
    }
}
