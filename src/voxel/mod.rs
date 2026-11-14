//! Воксели: сетка, материалы, мешинг, чанки-сущности, обломки, `.vox`-загрузчик.

pub mod grid;
pub mod material;
pub mod mesher;
pub mod vox_loader;

use std::collections::HashMap;

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::core::GameState;
pub use grid::{CHUNK, CarveResult, VoxelGrid, VoxelHit, VoxelWorld};
pub use material::{ALL_MATERIALS, Material, VoxelDestructibility};

/// Максимум одновременно живущих обломков.
pub const MAX_DEBRIS: usize = 400;

/// Маркер сущности-чанка мира (координата чанка).
#[derive(Component, Debug, Clone, Copy)]
pub struct VoxelChunk(pub IVec3);

/// Чанк → сущность.
#[derive(Resource, Default, Debug)]
pub struct ChunkEntities(pub HashMap<IVec3, Entity>);

/// Общие ассеты вокселей (материал с вертексными цветами, куб для обломков).
#[derive(Resource)]
pub struct VoxelAssets {
    pub material: Handle<StandardMaterial>,
    pub cube: Handle<Mesh>,
    pub tracer: Handle<StandardMaterial>,
    pub debris: HashMap<Material, Handle<StandardMaterial>>,
}

/// Простой xorshift-генератор (без внешних зависимостей, детерминированный).
#[derive(Debug, Clone)]
pub struct Rng(pub u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    /// Равномерно в `[0, 1)`.
    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    /// Равномерно в `[-1, 1)`.
    pub fn signed(&mut self) -> f32 {
        self.f32() * 2.0 - 1.0
    }
}

/// Обломок разрушенного вокселя (лёгкая кинематика без Avian).
#[derive(Component, Debug)]
pub struct Debris {
    pub velocity: Vec3,
    pub life: f32,
    pub resting: bool,
}

pub struct VoxelPlugin;

impl Plugin for VoxelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ChunkEntities>()
            .add_systems(
                Update,
                rebuild_dirty_chunks
                    .run_if(in_state(GameState::InBattle))
                    .run_if(resource_exists::<VoxelWorld>),
            )
            .add_systems(Update, update_debris)
            .add_systems(OnExit(GameState::InBattle), cleanup_world);

        if app.world().contains_resource::<Assets<StandardMaterial>>()
            && app.world().contains_resource::<Assets<Mesh>>()
        {
            app.init_resource::<VoxelAssets>();
        }
    }

    fn finish(&self, app: &mut App) {
        if !app.world().contains_resource::<VoxelAssets>()
            && app.world().contains_resource::<Assets<StandardMaterial>>()
            && app.world().contains_resource::<Assets<Mesh>>()
        {
            app.init_resource::<VoxelAssets>();
        }
    }
}

impl FromWorld for VoxelAssets {
    fn from_world(world: &mut World) -> Self {
        let (material, tracer, debris) = {
            let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
            let material = materials.add(StandardMaterial {
                base_color: Color::WHITE,
                perceptual_roughness: 0.95,
                metallic: 0.0,
                reflectance: 0.08,
                ..default()
            });
            let tracer = materials.add(StandardMaterial {
                base_color: Color::srgb(1.0, 0.85, 0.3),
                emissive: LinearRgba::new(6.0, 4.0, 1.0, 1.0),
                unlit: true,
                ..default()
            });
            let mut debris = HashMap::new();
            for m in ALL_MATERIALS {
                let c = m.linear_rgba();
                debris.insert(
                    m,
                    materials.add(StandardMaterial {
                        base_color: Color::linear_rgb(c[0], c[1], c[2]),
                        perceptual_roughness: 1.0,
                        ..default()
                    }),
                );
            }
            (material, tracer, debris)
        };
        let cube = world
            .resource_mut::<Assets<Mesh>>()
            .add(Cuboid::new(1.0, 1.0, 1.0));
        Self {
            material,
            cube,
            tracer,
            debris,
        }
    }
}

/// Удаляет мир при выходе из боя.
fn cleanup_world(mut commands: Commands, mut chunks: ResMut<ChunkEntities>) {
    chunks.0.clear();
    commands.remove_resource::<VoxelWorld>();
}

/// Коллайдер чанка: только поверхностные воксели (есть соседний воздух).
/// Воксель `c` занимает куб `[c*s, (c+1)*s]` — как и в [`VoxelGrid`]
/// (проверено по исходникам parry: `voxel_center = (c + 0.5) * size`).
pub fn chunk_collider(grid: &VoxelGrid, chunk: IVec3) -> Option<Collider> {
    let min = chunk * CHUNK;
    let max = (min + IVec3::splat(CHUNK)).min(grid.dims.as_ivec3());
    let mut coords = Vec::new();
    let dirs = [
        IVec3::X,
        IVec3::NEG_X,
        IVec3::Y,
        IVec3::NEG_Y,
        IVec3::Z,
        IVec3::NEG_Z,
    ];
    for y in min.y..max.y {
        for z in min.z..max.z {
            for x in min.x..max.x {
                let p = IVec3::new(x, y, z);
                if grid.is_solid(p) && dirs.iter().any(|d| !grid.is_solid(p + *d)) {
                    coords.push(p);
                }
            }
        }
    }
    if coords.is_empty() {
        None
    } else {
        Some(Collider::voxels(Vec3::splat(grid.voxel_size), &coords))
    }
}

/// Меш чанка (вершины относительно `grid.origin`).
pub fn chunk_mesh(grid: &VoxelGrid, chunk: IVec3) -> mesher::MeshData {
    let min = chunk * CHUNK;
    let dims = grid.dims.as_ivec3();
    let dim = (min + IVec3::splat(CHUNK)).min(dims) - min;
    if dim.cmple(IVec3::ZERO).any() {
        return mesher::MeshData::default();
    }
    mesher::mesh_region(grid, min, dim.as_uvec3())
}

/// Создаёт все чанки мира (меш + статический коллайдер).
pub fn spawn_world_chunks(
    commands: &mut Commands,
    world: &VoxelWorld,
    meshes: &mut Assets<Mesh>,
    assets: &VoxelAssets,
    chunks: &mut ChunkEntities,
) {
    chunks.0.clear();
    let cd = world.chunk_dims();
    for cy in 0..cd.y {
        for cz in 0..cd.z {
            for cx in 0..cd.x {
                let c = IVec3::new(cx, cy, cz);
                if let Some(e) = spawn_chunk(commands, &world.grid, c, meshes, assets) {
                    chunks.0.insert(c, e);
                }
            }
        }
    }
}

fn spawn_chunk(
    commands: &mut Commands,
    grid: &VoxelGrid,
    c: IVec3,
    meshes: &mut Assets<Mesh>,
    assets: &VoxelAssets,
) -> Option<Entity> {
    let data = chunk_mesh(grid, c);
    if data.is_empty() {
        return None;
    }
    let mut e = commands.spawn((
        Name::new(format!("Chunk {c}")),
        VoxelChunk(c),
        Mesh3d(meshes.add(data.into_mesh())),
        MeshMaterial3d(assets.material.clone()),
        Transform::from_translation(grid.origin),
        RigidBody::Static,
        DespawnOnExit(GameState::InBattle),
    ));
    if let Some(col) = chunk_collider(grid, c) {
        e.insert(col);
    }
    Some(e.id())
}

/// Максимум пересобираемых чанков за один кадр (плавный FPS при разрушениях в WASM).
pub const MAX_CHUNKS_PER_FRAME: usize = 6;

fn rebuild_dirty_chunks(
    mut commands: Commands,
    mut world: ResMut<VoxelWorld>,
    mut chunks: ResMut<ChunkEntities>,
    mut meshes: ResMut<Assets<Mesh>>,
    assets: Res<VoxelAssets>,
) {
    if world.dirty.is_empty() {
        return;
    }
    let dirty: Vec<IVec3> = world.dirty.iter().take(MAX_CHUNKS_PER_FRAME).copied().collect();
    for c in &dirty {
        world.dirty.remove(c);
    }
    for c in dirty {
        let data = chunk_mesh(&world.grid, c);
        let collider = chunk_collider(&world.grid, c);
        match (chunks.0.get(&c).copied(), data.is_empty()) {
            (Some(e), true) => {
                commands.entity(e).despawn();
                chunks.0.remove(&c);
            }
            (Some(e), false) => {
                let mut ec = commands.entity(e);
                ec.insert(Mesh3d(meshes.add(data.into_mesh())));
                match collider {
                    Some(col) => {
                        ec.insert(col);
                    }
                    None => {
                        ec.remove::<Collider>();
                    }
                }
            }
            (None, false) => {
                if let Some(e) = spawn_chunk(&mut commands, &world.grid, c, &mut meshes, &assets) {
                    chunks.0.insert(c, e);
                }
            }
            (None, true) => {}
        }
    }
}

/// Бандл визуальной модели из сетки: сущность смещена в `grid.origin`
/// относительно родителя (меш строится относительно origin).
pub fn voxel_model_bundle(
    grid: &VoxelGrid,
    meshes: &mut Assets<Mesh>,
    assets: &VoxelAssets,
) -> impl Bundle {
    let data = mesher::mesh_grid(grid);
    (
        Mesh3d(meshes.add(data.into_mesh())),
        MeshMaterial3d(assets.material.clone()),
        Transform::from_translation(grid.origin),
    )
}

/// Всплеск обломков. `normal` — направление вылета.
pub fn spawn_debris_burst(
    commands: &mut Commands,
    assets: &VoxelAssets,
    existing: usize,
    pos: Vec3,
    normal: Vec3,
    material: Material,
    count: usize,
    voxel_size: f32,
    rng: &mut Rng,
) {
    let Some(mat) = assets.debris.get(&material) else {
        return;
    };
    let room = MAX_DEBRIS.saturating_sub(existing);
    for _ in 0..count.min(room) {
        let jitter = Vec3::new(rng.signed(), rng.signed().abs(), rng.signed());
        let velocity = (normal + jitter).normalize_or_zero() * (2.0 + rng.f32() * 6.0);
        let size = voxel_size * (0.25 + rng.f32() * 0.35);
        commands.spawn((
            Debris {
                velocity,
                life: 1.5 + rng.f32() * 2.0,
                resting: false,
            },
            Mesh3d(assets.cube.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(pos + jitter * voxel_size * 0.3)
                .with_scale(Vec3::splat(size)),
            DespawnOnExit(GameState::InBattle),
        ));
    }
}

fn update_debris(
    mut commands: Commands,
    time: Res<Time>,
    world: Option<Res<VoxelWorld>>,
    mut q: Query<(Entity, &mut Debris, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (e, mut d, mut tf) in &mut q {
        d.life -= dt;
        if d.life <= 0.0 {
            commands.entity(e).despawn();
            continue;
        }
        if d.resting {
            continue;
        }
        d.velocity.y -= 9.81 * dt;
        tf.translation += d.velocity * dt;
        tf.rotate_local_x(d.velocity.length() * dt * 2.0);
        if let Some(w) = &world {
            let g = &w.grid;
            if g.is_solid(g.world_to_voxel(tf.translation)) {
                tf.translation.y = g
                    .surface_height_at(tf.translation.x, tf.translation.z)
                    .unwrap_or(tf.translation.y)
                    + tf.scale.y * 0.5;
                d.velocity = Vec3::ZERO;
                d.resting = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_and_in_range() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            let x = a.f32();
            assert_eq!(x, b.f32());
            assert!((0.0..1.0).contains(&x));
        }
    }

    #[test]
    fn chunk_collider_only_for_surface_voxels() {
        let mut g = VoxelGrid::new(UVec3::new(8, 8, 8), 0.5, Vec3::ZERO);
        g.fill_box(IVec3::ZERO, IVec3::splat(7), Material::Rock);
        assert!(chunk_collider(&g, IVec3::ZERO).is_some());
        let empty = VoxelGrid::new(UVec3::new(8, 8, 8), 0.5, Vec3::ZERO);
        assert!(chunk_collider(&empty, IVec3::ZERO).is_none());
    }

    #[test]
    fn chunk_mesh_is_origin_relative() {
        let mut g = VoxelGrid::new(UVec3::new(8, 8, 8), 0.5, Vec3::new(100.0, 0.0, 100.0));
        g.set(IVec3::new(0, 0, 0), Material::Soil);
        let m = chunk_mesh(&g, IVec3::ZERO);
        assert!(!m.is_empty());
        assert!(m.positions.iter().all(|p| p[0] <= 0.5 + 1e-4));
    }
}
