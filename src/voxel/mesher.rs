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

/// Множитель яркости вершины по числу открытых соседей угла (0 = глубокий угол, 3 = открыто).
const AO_CURVE: [f32; 4] = [0.32, 0.54, 0.77, 1.0];

#[derive(Clone, Copy, PartialEq, Eq)]
struct AoFaceKey {
    raw: u8,
    ao: [u8; 4],
}

/// AO для 4 углов грани: `q` — воксель перед гранью (со стороны нормали),
/// `du`/`dv` — оси грани. Порядок углов: (-,-), (+,-), (+,+), (-,+).
fn face_corner_ao(grid: &VoxelGrid, q: IVec3, du: IVec3, dv: IVec3) -> [u8; 4] {
    let solid = |p: IVec3| grid.is_solid(p) as u8;
    let mut out = [3u8; 4];
    for (k, (a, b)) in [(-1, -1), (1, -1), (1, 1), (-1, 1)].into_iter().enumerate() {
        let s1 = solid(q + du * a);
        let s2 = solid(q + dv * b);
        let c = solid(q + du * a + dv * b);
        out[k] = if s1 == 1 && s2 == 1 { 0 } else { 3 - s1 - s2 - c };
    }
    out
}

/// Greedy-мешинг с вершинным Ambient Occlusion для небольших моделей (танки, мишени, реквизит).
///
/// Соседние грани сливаются только при совпадении материала и AO-узора,
/// поэтому плоские участки остаются крупными квадами, а стыки, ниши и углы
/// получают мягкое затенение. Диагональ квада выбирается по AO (без «анизотропных» артефактов).
/// Вершины — относительно `grid.origin`, как и в [`mesh_region`].
pub fn mesh_grid_ao(grid: &VoxelGrid) -> MeshData {
    let dims = grid.dims.as_ivec3();
    let s = grid.voxel_size;
    let mut data = MeshData::default();

    for d in 0..3usize {
        let u = (d + 1) % 3;
        let v = (d + 2) % 3;
        let mut n = IVec3::ZERO;
        n[d] = 1;
        let mut du = IVec3::ZERO;
        du[u] = 1;
        let mut dv = IVec3::ZERO;
        dv[v] = 1;
        let nu = dims[u].max(0) as usize;
        let nv = dims[v].max(0) as usize;
        let mut mask: Vec<Option<AoFaceKey>> = vec![None; nu * nv];

        for positive in [true, false] {
            let normal = if positive { n } else { -n };
            let normal_f = normal.as_vec3().to_array();
            for slice in 0..dims[d] {
                for j in 0..nv {
                    for i in 0..nu {
                        let mut p = IVec3::ZERO;
                        p[d] = slice;
                        p[u] = i as i32;
                        p[v] = j as i32;
                        let raw = grid.get_raw(p);
                        mask[j * nu + i] = if raw != 0 && !grid.is_solid(p + normal) {
                            Some(AoFaceKey {
                                raw,
                                ao: face_corner_ao(grid, p + normal, du, dv),
                            })
                        } else {
                            None
                        };
                    }
                }

                for j in 0..nv {
                    let mut i = 0;
                    while i < nu {
                        let Some(key) = mask[j * nu + i] else {
                            i += 1;
                            continue;
                        };
                        let mut w = 1;
                        while i + w < nu && mask[j * nu + i + w] == Some(key) {
                            w += 1;
                        }
                        let mut h = 1;
                        'grow: while j + h < nv {
                            for k in 0..w {
                                if mask[(j + h) * nu + i + k] != Some(key) {
                                    break 'grow;
                                }
                            }
                            h += 1;
                        }
                        for jj in 0..h {
                            for k in 0..w {
                                mask[(j + jj) * nu + i + k] = None;
                            }
                        }

                        let mut base = IVec3::ZERO;
                        base[d] = slice + positive as i32;
                        base[u] = i as i32;
                        base[v] = j as i32;
                        let wu = du * w as i32;
                        let hv = dv * h as i32;
                        let corners = [base, base + wu, base + wu + hv, base + hv];
                        let color = color_of(grid, key.raw);
                        let start = data.positions.len() as u32;
                        for (c, ao) in corners.iter().zip(key.ao) {
                            data.positions.push((c.as_vec3() * s).to_array());
                            data.normals.push(normal_f);
                            let f = AO_CURVE[ao as usize];
                            data.colors.push([color[0] * f, color[1] * f, color[2] * f, color[3]]);
                        }
                        let a = key.ao;
                        let flip = (a[0] as u16 + a[2] as u16) < (a[1] as u16 + a[3] as u16);
                        let tri: [u32; 6] = match (positive, flip) {
                            (true, false) => [0, 1, 2, 0, 2, 3],
                            (true, true) => [1, 2, 3, 1, 3, 0],
                            (false, false) => [0, 2, 1, 0, 3, 2],
                            (false, true) => [1, 3, 2, 1, 0, 3],
                        };
                        data.indices.extend(tri.iter().map(|t| start + t));
                        i += w;
                    }
                }
            }
        }
    }
    data
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

    /// Каждый треугольник AO-меша обходится против часовой стрелки относительно своей нормали.
    fn assert_ccw_winding(m: &MeshData) {
        for tri in m.indices.chunks(3) {
            let a = Vec3::from(m.positions[tri[0] as usize]);
            let b = Vec3::from(m.positions[tri[1] as usize]);
            let c = Vec3::from(m.positions[tri[2] as usize]);
            let n = Vec3::from(m.normals[tri[0] as usize]);
            assert!((b - a).cross(c - a).dot(n) > 0.0, "неверный обход треугольника");
        }
    }

    #[test]
    fn ao_mesher_single_voxel_has_six_lit_ccw_quads() {
        let mut g = VoxelGrid::new(UVec3::new(3, 3, 3), 0.5, Vec3::ZERO);
        g.set(IVec3::new(1, 1, 1), Material::TankOlive);
        let m = mesh_grid_ao(&g);
        assert_eq!(m.quad_count(), 6);
        assert_ccw_winding(&m);
        let base = Material::TankOlive.linear_rgba();
        assert!(m.colors.iter().all(|c| *c == base), "одиночный воксель не затеняется");
    }

    #[test]
    fn ao_mesher_merges_flat_slab_and_darkens_inner_corner() {
        let mut g = VoxelGrid::new(UVec3::new(16, 4, 16), 0.1, Vec3::ZERO);
        g.fill_box(IVec3::ZERO, IVec3::new(15, 0, 15), Material::TankOlive);
        let flat = mesh_grid_ao(&g);
        assert_eq!(flat.quad_count(), 6, "ровная плита должна слиться в 6 квадов");
        assert_ccw_winding(&flat);

        // Ставим на плиту блок: в углу примыкания на плите появляется затенение.
        g.fill_box(IVec3::new(6, 1, 6), IVec3::new(9, 2, 9), Material::TankDark);
        let m = mesh_grid_ao(&g);
        assert_ccw_winding(&m);
        let base = Material::TankOlive.linear_rgba();
        let darkened = m
            .colors
            .iter()
            .any(|c| c[0] < base[0] * 0.9 && c[0] > 0.0 && c[1] < base[1] * 0.9);
        assert!(darkened, "у основания блока должен быть AO");
        assert!(m.quad_count() < naive_face_count(&g, IVec3::ZERO, g.dims));
    }
}
