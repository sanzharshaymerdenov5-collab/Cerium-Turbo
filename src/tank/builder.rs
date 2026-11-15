//! Процедурный конструктор детализированных воксельных сеток танка из [`ProceduralTankProfile`].

use std::f32::consts::TAU;

use bevy::math::{IVec3, UVec3, Vec2, Vec3};

use super::spec::{HullShape, ProceduralTankProfile, TankVisualModel, TurretShape};
use crate::core::Team;
use crate::voxel::{Material, VoxelGrid};

/// Цветовая схема вокселей танка по команде.
#[derive(Debug, Clone, Copy)]
pub struct TankPalette {
    pub armor: Material,
    pub armor_light: Material,
    pub dark: Material,
    pub mark: Material,
    pub camo: Option<Material>,
}

impl TankPalette {
    pub fn for_team(team: Team) -> Self {
        match team {
            Team::Player => Self {
                armor: Material::TankOlive,
                armor_light: Material::TankOliveLight,
                dark: Material::TankDark,
                mark: Material::TankStar,
                camo: None,
            },
            Team::Enemy => Self {
                armor: Material::EnemyArmor,
                armor_light: Material::EnemyArmorLight,
                dark: Material::EnemyDark,
                mark: Material::EnemyMark,
                camo: Some(Material::EnemyCamo),
            },
        }
    }

    /// Основная окраска броневого листа с учётом камуфляжа врага.
    #[inline]
    fn hull_paint(&self, p: Vec3, highlight: bool) -> Material {
        if let Some(camo) = self.camo
            && camo_noise_3d(p * 1.15 + Vec3::new(3.7, 1.3, 9.1)) > 0.58
        {
            return camo;
        }
        if highlight {
            self.armor_light
        } else {
            self.armor
        }
    }
}

/// Три части визуальной модели танка.
#[derive(Debug, Clone)]
pub struct TankVoxelParts {
    /// Корпус + ходовая часть (в кадре `TankRoot`: origin — центр низа гусениц).
    pub hull: VoxelGrid,
    /// Башня или рубка (в кадре `TurretPivot`).
    pub turret: VoxelGrid,
    /// Ствол с маской (в кадре `GunPivot`: тянется вдоль −Z).
    pub barrel: VoxelGrid,
}

/// Строит сетку по предикату `f(p) -> Option<Material>` на боксе `[min_m, max_m]`.
pub fn build_from_fn(
    min_m: Vec3,
    max_m: Vec3,
    voxel_size: f32,
    mut f: impl FnMut(Vec3) -> Option<Material>,
) -> VoxelGrid {
    let size = (max_m - min_m).max(Vec3::splat(voxel_size));
    let dims = (size / voxel_size).ceil().as_uvec3().max(UVec3::ONE);
    let mut grid = VoxelGrid::new(dims, voxel_size, min_m);
    for y in 0..dims.y as i32 {
        for z in 0..dims.z as i32 {
            for x in 0..dims.x as i32 {
                let v = IVec3::new(x, y, z);
                let p = grid.voxel_center(v);
                if let Some(m) = f(p) {
                    grid.set(v, m);
                }
            }
        }
    }
    grid
}

/// Строит воксельную сетку с нечётной зеркальной симметрией по X (центр x = 0 в середине столбца),
/// а при `center_y_zero` — и по Y (центр y = 0 в середине строки, удобно для полого канала ствола).
fn build_symmetric_grid(
    half_x: f32,
    y_range: (f32, f32),
    z_range: (f32, f32),
    voxel_size: f32,
    center_y_zero: bool,
    f: impl FnMut(Vec3) -> Option<Material>,
) -> VoxelGrid {
    let (min_y, max_y) = y_range;
    let (min_z, max_z) = z_range;
    let kx = (half_x / voxel_size).floor().max(1.0) as u32;
    let ox = -(kx as f32 + 0.5) * voxel_size;
    let hx = (kx as f32 + 0.5) * voxel_size;
    let (oy, ymax) = if center_y_zero {
        let ky = (max_y.abs().max(min_y.abs()) / voxel_size)
            .ceil()
            .max(1.0) as u32;
        let bound = (ky as f32 + 0.5) * voxel_size;
        (-bound, bound)
    } else {
        (min_y, max_y)
    };
    build_from_fn(
        Vec3::new(ox, oy, min_z),
        Vec3::new(hx, ymax, max_z),
        voxel_size,
        f,
    )
}

/// Разрешает конкретный исторический архетип танка (с fallback для `Auto`).
fn resolve_model(p: &ProceduralTankProfile) -> TankVisualModel {
    if p.visual_model != TankVisualModel::Auto {
        return p.visual_model;
    }
    match (p.hull_shape, p.turret_shape) {
        (HullShape::Casemate, _) | (_, TurretShape::FixedCasemate) => TankVisualModel::Su100,
        (HullShape::PikeNose, _) => TankVisualModel::Is3,
        (HullShape::SlopedT34, TurretShape::CompactLight) => TankVisualModel::Bt7,
        (HullShape::SlopedT34, TurretShape::RoundedDome) => TankVisualModel::M4A3E8,
        (HullShape::SlopedT34, _) => TankVisualModel::T34_85,
        (HullShape::LightBox, _) => TankVisualModel::M24Chaffee,
        (HullShape::HeavyBox, TurretShape::HexCast) => TankVisualModel::Kv1,
        (HullShape::HeavyBox, _) => {
            if p.hull_length >= 7.0 {
                TankVisualModel::Tiger2
            } else {
                TankVisualModel::Tiger1
            }
        }
    }
}

/// Строит воксельные части танка по профилю (палитра игрока по умолчанию).
pub fn build_tank_parts(p: &ProceduralTankProfile) -> TankVoxelParts {
    build_tank_parts_for_team(p, Team::Player)
}

/// Строит детализированные воксельные части танка с учётом команды, ходовой и исторического силуэта.
pub fn build_tank_parts_for_team(p: &ProceduralTankProfile, team: Team) -> TankVoxelParts {
    let ctx = TankBuildCtx::new(p, team);

    let half_w = match ctx.model {
        TankVisualModel::Pz4 | TankVisualModel::Panther | TankVisualModel::Tiger2 => {
            ctx.hw + p.voxel_size * 2.2
        }
        _ => ctx.hw + p.voxel_size * 1.4,
    };
    let min_z = -ctx.hl - 0.16;
    let max_z = if ctx.model == TankVisualModel::Ms1 {
        ctx.hl + 0.72
    } else {
        ctx.hl + 0.34
    };
    let max_y = p.deck_height + 0.38;
    let hull = build_symmetric_grid(
        half_w,
        (0.0, max_y),
        (min_z, max_z),
        p.voxel_size,
        false,
        |pt| ctx.sample_hull(pt),
    );

    let t_half_x = p.turret_radius_x + 0.36;
    let turret = build_symmetric_grid(
        t_half_x,
        (-p.voxel_size * 0.5, p.turret_height + 0.56),
        (-p.turret_radius_z - 0.34, p.turret_radius_z + 0.62),
        p.voxel_size,
        false,
        |pt| ctx.sample_turret(pt),
    );

    let b_half_x = if matches!(
        ctx.model,
        TankVisualModel::Tiger1
            | TankVisualModel::Tiger2
            | TankVisualModel::Panther
            | TankVisualModel::Pz4
            | TankVisualModel::Pz3
            | TankVisualModel::M26Pershing
            | TankVisualModel::M4A3E8
    ) {
        (p.turret_radius_x * 0.72).max(0.42)
    } else {
        (p.turret_radius_x * 0.52).max(0.32)
    };
    let b_half_y = (p.turret_height * 0.40).max(0.26);
    let barrel = build_symmetric_grid(
        b_half_x,
        (-b_half_y, b_half_y),
        (-p.barrel_length - 0.04, 0.42),
        p.gun_voxel_size,
        true,
        |pt| ctx.sample_barrel(pt),
    );

    TankVoxelParts {
        hull,
        turret,
        barrel,
    }
}

#[derive(Clone, Copy)]
struct RoadWheel {
    z: f32,
    y: f32,
    r: f32,
    x_min: f32,
    x_max: f32,
    spoked: bool,
    all_steel: bool,
}

#[derive(Clone, Copy)]
struct ReturnRoller {
    z: f32,
    y: f32,
    r: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BogieKind {
    None,
    Ms1Girder,
    Pz4Leaf,
    UsVvss,
    UsHvss,
}

struct GearSpec {
    road_wheels: Vec<RoadWheel>,
    rollers: Vec<ReturnRoller>,
    bogie_kind: BogieKind,
    bogie_centers: Vec<f32>,
    z_front: f32,
    y_front: f32,
    r_front: f32,
    front_drive: bool,
    z_rear: f32,
    y_rear: f32,
    r_rear: f32,
    y_track_bot: f32,
    y_track_top: f32,
    z_bot_start: f32,
    z_bot_end: f32,
    trailing_rear_idler: bool,
}

struct TankBuildCtx<'a> {
    p: &'a ProceduralTankProfile,
    model: TankVisualModel,
    team: Team,
    pal: TankPalette,
    hl: f32,
    hw: f32,
    inner_w: f32,
    clearance: f32,
    fender_y0: f32,
    fender_y1: f32,
    gear: GearSpec,
}

impl<'a> TankBuildCtx<'a> {
    fn new(p: &'a ProceduralTankProfile, team: Team) -> Self {
        let model = resolve_model(p);
        let vs = p.voxel_size;
        let hl = p.hull_length * 0.5;
        let hw = p.hull_width * 0.5;
        let inner_w = hw - p.track_width;
        let clearance = (p.deck_height * 0.25).clamp(0.24, 0.34);

        let top_row = ((p.track_height / vs).round() as i32 - 1).max(4);
        let y_track_top = (top_row as f32 + 0.5) * vs;
        let y_track_bot = 0.5 * vs;
        let fender_y0 = (top_row as f32 + 1.0) * vs - 0.005;
        let fender_y1 = (top_row as f32 + 2.0) * vs - 0.005;
        let belt_h = y_track_top - y_track_bot;

        let z_front = -(hl - (0.16_f32).min(hl * 0.08));
        let z_rear = hl - (0.18_f32).min(hl * 0.09);

        let gear = match model {
            TankVisualModel::Ms1 => {
                // МС-1: 6 малых стальных катков с балкой-подвеской, заднее ведущее, высоко поднятый ленивец.
                let s = hl / 1.825;
                let r_road = (belt_h * 0.20).clamp(0.11, 0.14);
                let y_w = y_track_bot + vs * 0.5 + r_road;
                let road_wheels = (0..6)
                    .map(|i| RoadWheel {
                        z: (-1.08 + i as f32 * 0.43) * s,
                        y: y_w,
                        r: r_road,
                        x_min: inner_w + 0.03,
                        x_max: hw - 0.02,
                        spoked: false,
                        all_steel: true,
                    })
                    .collect();
                let r_ret = (belt_h * 0.11).clamp(0.055, 0.08);
                let y_ret = y_track_top - vs * 0.45 - r_ret;
                let rollers = [-0.70, 0.05, 0.80]
                    .into_iter()
                    .map(|z| ReturnRoller {
                        z: z * s,
                        y: y_ret,
                        r: r_ret,
                    })
                    .collect();
                let r_front = (belt_h * 0.30).clamp(0.15, 0.21);
                let r_rear = (belt_h * 0.32).clamp(0.16, 0.22);
                GearSpec {
                    road_wheels,
                    rollers,
                    bogie_kind: BogieKind::Ms1Girder,
                    bogie_centers: vec![0.0],
                    z_front: -(hl - 0.18),
                    y_front: y_track_top - r_front - vs * 0.15,
                    r_front,
                    front_drive: false,
                    z_rear: hl - 0.16,
                    y_rear: y_track_top - r_rear - vs * 0.35,
                    r_rear,
                    y_track_bot,
                    y_track_top,
                    z_bot_start: -1.16 * s,
                    z_bot_end: 1.18 * s,
                    trailing_rear_idler: false,
                }
            }
            TankVisualModel::Bt7 => {
                // БТ-7: 4 гигантских спицованных катка Кристи; нос ванны выступает впереди гусениц.
                let s = hl / 2.775;
                let r_road = (belt_h - vs * 0.9) * 0.49;
                let y_w = y_track_bot + vs * 0.5 + r_road;
                let zs = [-1.52, -0.50, 0.52, 1.54].map(|z| z * s);
                let road_wheels = zs
                    .into_iter()
                    .map(|z| RoadWheel {
                        z,
                        y: y_w,
                        r: r_road,
                        x_min: inner_w + 0.03,
                        x_max: hw - 0.02,
                        spoked: true,
                        all_steel: false,
                    })
                    .collect();
                let r_front = (belt_h * 0.30).clamp(0.15, 0.22);
                let r_rear = (belt_h * 0.33).clamp(0.16, 0.24);
                GearSpec {
                    road_wheels,
                    rollers: Vec::new(),
                    bogie_kind: BogieKind::None,
                    bogie_centers: Vec::new(),
                    z_front: -2.18 * s,
                    y_front: y_track_top - r_front - vs * 0.3,
                    r_front,
                    front_drive: false,
                    z_rear: hl - 0.18,
                    y_rear: y_track_top - r_rear - vs * 0.3,
                    r_rear,
                    y_track_bot,
                    y_track_top,
                    z_bot_start: -1.62 * s,
                    z_bot_end: 1.62 * s,
                    trailing_rear_idler: false,
                }
            }
            TankVisualModel::T34_76 | TankVisualModel::T34_85 | TankVisualModel::Su100 => {
                // Т-34 / СУ-100: 5 крупных катков Кристи без роликов; заднее ведущее.
                let s = hl / 3.0;
                let r_road = (belt_h - vs * 1.1) * 0.49;
                let y_w = y_track_bot + vs * 0.5 + r_road;
                let zs = [-2.02, -1.03, -0.04, 0.96, 1.98].map(|z| z * s);
                let road_wheels = zs
                    .into_iter()
                    .map(|z| RoadWheel {
                        z,
                        y: y_w,
                        r: r_road,
                        x_min: inner_w + 0.04,
                        x_max: hw - 0.02,
                        spoked: true,
                        all_steel: false,
                    })
                    .collect();
                let r_front = (belt_h * 0.30).clamp(0.15, 0.25);
                let r_rear = (belt_h * 0.325).clamp(0.16, 0.27);
                GearSpec {
                    road_wheels,
                    rollers: Vec::new(),
                    bogie_kind: BogieKind::None,
                    bogie_centers: Vec::new(),
                    z_front,
                    y_front: y_track_top - r_front - vs * 0.3,
                    r_front,
                    front_drive: false,
                    z_rear,
                    y_rear: y_track_top - r_rear - vs * 0.3,
                    r_rear,
                    y_track_bot,
                    y_track_top,
                    z_bot_start: -2.08 * s,
                    z_bot_end: 2.04 * s,
                    trailing_rear_idler: false,
                }
            }
            TankVisualModel::Kv1 | TankVisualModel::Is3 => {
                // КВ-1 (цельнометаллические катки) и ИС-3 (обрезиненные штампованные): 6 катков + 3 ролика.
                let s = hl / 3.3;
                let is_kv = model == TankVisualModel::Kv1;
                let r_road = (belt_h * 0.285).clamp(0.17, 0.25);
                let y_w = y_track_bot + vs * 0.5 + r_road;
                let road_wheels = (0..6)
                    .map(|i| RoadWheel {
                        z: (-2.10 + i as f32 * 0.84) * s,
                        y: y_w,
                        r: r_road,
                        x_min: inner_w + 0.04,
                        x_max: hw - 0.02,
                        spoked: false,
                        all_steel: is_kv,
                    })
                    .collect();
                let r_ret = (belt_h * 0.125).clamp(0.07, 0.11);
                let y_ret = y_track_top - vs * 0.45 - r_ret;
                let rollers = [-1.35, 0.0, 1.35]
                    .into_iter()
                    .map(|z| ReturnRoller {
                        z: z * s,
                        y: y_ret,
                        r: r_ret,
                    })
                    .collect();
                let r_front = (belt_h * 0.285).clamp(0.16, 0.25);
                let r_rear = (belt_h * 0.31).clamp(0.17, 0.27);
                GearSpec {
                    road_wheels,
                    rollers,
                    bogie_kind: BogieKind::None,
                    bogie_centers: Vec::new(),
                    z_front: -(hl - (0.18_f32).min(hl * 0.08)),
                    y_front: y_track_top - r_front - vs * 0.3,
                    r_front,
                    front_drive: false,
                    z_rear: hl - (0.20_f32).min(hl * 0.09),
                    y_rear: y_track_top - r_rear - vs * 0.3,
                    r_rear,
                    y_track_bot,
                    y_track_top,
                    z_bot_start: -2.16 * s,
                    z_bot_end: 2.16 * s,
                    trailing_rear_idler: false,
                }
            }
            TankVisualModel::Pz2C => {
                // Pz. II Ausf. C: 5 средних катков + 4 поддерживающих ролика; ведущее спереди.
                let s = hl / 2.39;
                let r_road = (belt_h * 0.31).clamp(0.14, 0.22);
                let y_w = y_track_bot + vs * 0.5 + r_road;
                let road_wheels = [-1.42, -0.71, 0.0, 0.71, 1.42]
                    .into_iter()
                    .map(|z| RoadWheel {
                        z: z * s,
                        y: y_w,
                        r: r_road,
                        x_min: inner_w + 0.03,
                        x_max: hw - 0.02,
                        spoked: false,
                        all_steel: false,
                    })
                    .collect();
                let r_ret = (belt_h * 0.13).clamp(0.06, 0.095);
                let y_ret = y_track_top - vs * 0.45 - r_ret;
                let rollers = [-1.06, -0.35, 0.35, 1.06]
                    .into_iter()
                    .map(|z| ReturnRoller {
                        z: z * s,
                        y: y_ret,
                        r: r_ret,
                    })
                    .collect();
                let r_front = (belt_h * 0.33).clamp(0.15, 0.22);
                let r_rear = (belt_h * 0.30).clamp(0.14, 0.20);
                GearSpec {
                    road_wheels,
                    rollers,
                    bogie_kind: BogieKind::None,
                    bogie_centers: Vec::new(),
                    z_front,
                    y_front: y_track_top - r_front - vs * 0.3,
                    r_front,
                    front_drive: true,
                    z_rear,
                    y_rear: y_track_top - r_rear - vs * 0.3,
                    r_rear,
                    y_track_bot,
                    y_track_top,
                    z_bot_start: -1.48 * s,
                    z_bot_end: 1.48 * s,
                    trailing_rear_idler: false,
                }
            }
            TankVisualModel::Pz3 => {
                // Pz. III Ausf. J: 6 раздельных торсионных катков + 3 заметных ролика; ведущее спереди.
                let s = hl / 2.71;
                let r_road = (belt_h * 0.295).clamp(0.16, 0.23);
                let y_w = y_track_bot + vs * 0.5 + r_road;
                let road_wheels = (0..6)
                    .map(|i| RoadWheel {
                        z: (-1.75 + i as f32 * 0.70) * s,
                        y: y_w,
                        r: r_road,
                        x_min: inner_w + 0.04,
                        x_max: hw - 0.02,
                        spoked: false,
                        all_steel: false,
                    })
                    .collect();
                let r_ret = (belt_h * 0.13).clamp(0.07, 0.105);
                let y_ret = y_track_top - vs * 0.45 - r_ret;
                let rollers = [-1.18, 0.0, 1.18]
                    .into_iter()
                    .map(|z| ReturnRoller {
                        z: z * s,
                        y: y_ret,
                        r: r_ret,
                    })
                    .collect();
                let r_front = (belt_h * 0.32).clamp(0.17, 0.24);
                let r_rear = (belt_h * 0.30).clamp(0.16, 0.23);
                GearSpec {
                    road_wheels,
                    rollers,
                    bogie_kind: BogieKind::None,
                    bogie_centers: Vec::new(),
                    z_front,
                    y_front: y_track_top - r_front - vs * 0.3,
                    r_front,
                    front_drive: true,
                    z_rear,
                    y_rear: y_track_top - r_rear - vs * 0.3,
                    r_rear,
                    y_track_bot,
                    y_track_top,
                    z_bot_start: -1.82 * s,
                    z_bot_end: 1.82 * s,
                    trailing_rear_idler: false,
                }
            }
            TankVisualModel::Pz4 => {
                // Pz. IV Ausf. H: 8 малых катков в 4 рессорных тележках + 4 поддерживающих ролика.
                let s = hl / 2.96;
                let r_road = (belt_h * 0.255).clamp(0.14, 0.21);
                let y_w = y_track_bot + vs * 0.5 + r_road;
                let pairs = [-1.70_f32, -0.58, 0.54, 1.66];
                let mut road_wheels = Vec::with_capacity(8);
                let mut bogie_centers = Vec::with_capacity(4);
                for c in pairs {
                    bogie_centers.push(c * s);
                    road_wheels.push(RoadWheel {
                        z: (c - 0.24) * s,
                        y: y_w,
                        r: r_road,
                        x_min: inner_w + 0.04,
                        x_max: hw - 0.02,
                        spoked: false,
                        all_steel: false,
                    });
                    road_wheels.push(RoadWheel {
                        z: (c + 0.24) * s,
                        y: y_w,
                        r: r_road,
                        x_min: inner_w + 0.04,
                        x_max: hw - 0.02,
                        spoked: false,
                        all_steel: false,
                    });
                }
                let r_ret = (belt_h * 0.12).clamp(0.065, 0.095);
                let y_ret = y_track_top - vs * 0.45 - r_ret;
                let rollers = [-1.45, -0.48, 0.48, 1.45]
                    .into_iter()
                    .map(|z| ReturnRoller {
                        z: z * s,
                        y: y_ret,
                        r: r_ret,
                    })
                    .collect();
                let r_front = (belt_h * 0.31).clamp(0.17, 0.24);
                let r_rear = (belt_h * 0.29).clamp(0.16, 0.22);
                GearSpec {
                    road_wheels,
                    rollers,
                    bogie_kind: BogieKind::Pz4Leaf,
                    bogie_centers,
                    z_front,
                    y_front: y_track_top - r_front - vs * 0.3,
                    r_front,
                    front_drive: true,
                    z_rear,
                    y_rear: y_track_top - r_rear - vs * 0.3,
                    r_rear,
                    y_track_bot,
                    y_track_top,
                    z_bot_start: -2.02 * s,
                    z_bot_end: 2.00 * s,
                    trailing_rear_idler: false,
                }
            }
            TankVisualModel::Panther | TankVisualModel::Tiger1 | TankVisualModel::Tiger2 => {
                // Шахматные катки «Шактельлауфверк» (Panther, Tiger I, Tiger II).
                let s = hl / 3.15;
                let tw = (hw - inner_w).max(0.24);
                let r_road = (belt_h * 0.375).clamp(0.18, 0.34);
                let y_w = y_track_bot + vs * 0.5 + r_road;
                let mut road_wheels = Vec::with_capacity(8);
                for z in [-1.72_f32, -0.58, 0.56, 1.70] {
                    road_wheels.push(RoadWheel {
                        z: z * s,
                        y: y_w,
                        r: r_road,
                        x_min: inner_w + tw * 0.65,
                        x_max: hw - 0.02,
                        spoked: false,
                        all_steel: false,
                    });
                }
                for z in [-1.15_f32, -0.01, 1.13, 2.15] {
                    road_wheels.push(RoadWheel {
                        z: z * s,
                        y: y_w,
                        r: r_road,
                        x_min: inner_w + 0.04,
                        x_max: inner_w + tw * 0.60,
                        spoked: false,
                        all_steel: false,
                    });
                }
                let r_front = (belt_h * 0.307).clamp(0.16, 0.28);
                let r_rear = (belt_h * 0.273).clamp(0.15, 0.25);
                GearSpec {
                    road_wheels,
                    rollers: Vec::new(),
                    bogie_kind: BogieKind::None,
                    bogie_centers: Vec::new(),
                    z_front: -(hl - (0.20_f32).min(hl * 0.09)),
                    y_front: y_track_top - r_front - vs * 0.3,
                    r_front,
                    front_drive: true,
                    z_rear: hl - (0.22_f32).min(hl * 0.10),
                    y_rear: y_track_top - r_rear - vs * 0.3,
                    r_rear,
                    y_track_bot,
                    y_track_top,
                    z_bot_start: -1.80 * s,
                    z_bot_end: 2.18 * s,
                    trailing_rear_idler: false,
                }
            }
            TankVisualModel::M2A4 | TankVisualModel::M3Stuart => {
                // Американские VVSS (2 сдвоенные тележки): у M3 Stuart задний ленивец опущен на грунт!
                let s = hl / 2.24;
                let trailing = model == TankVisualModel::M3Stuart;
                let r_road = (belt_h * 0.28).clamp(0.14, 0.20);
                let y_w = y_track_bot + vs * 0.5 + r_road;
                let bogie_cs = if trailing { [-0.94, 0.32] } else { [-0.88, 0.58] };
                let mut road_wheels = Vec::with_capacity(4);
                let mut bogie_centers = Vec::with_capacity(2);
                for c in bogie_cs {
                    bogie_centers.push(c * s);
                    road_wheels.push(RoadWheel {
                        z: (c - 0.27) * s,
                        y: y_w,
                        r: r_road,
                        x_min: inner_w + 0.03,
                        x_max: hw - 0.02,
                        spoked: false,
                        all_steel: false,
                    });
                    road_wheels.push(RoadWheel {
                        z: (c + 0.27) * s,
                        y: y_w,
                        r: r_road,
                        x_min: inner_w + 0.03,
                        x_max: hw - 0.02,
                        spoked: false,
                        all_steel: false,
                    });
                }
                let r_ret = (belt_h * 0.12).clamp(0.055, 0.085);
                let y_ret = y_track_top - vs * 0.45 - r_ret;
                let rollers = [-0.94 * s, 0.32 * s]
                    .into_iter()
                    .map(|z| ReturnRoller { z, y: y_ret, r: r_ret })
                    .collect();
                let r_front = (belt_h * 0.32).clamp(0.15, 0.22);
                let r_rear = if trailing {
                    (r_road * 1.24).clamp(0.17, 0.25)
                } else {
                    (belt_h * 0.30).clamp(0.14, 0.20)
                };
                let y_rear = if trailing {
                    y_track_bot + vs * 0.5 + r_rear
                } else {
                    y_track_top - r_rear - vs * 0.3
                };
                let z_rear_end = hl - 0.19;
                GearSpec {
                    road_wheels,
                    rollers,
                    bogie_kind: BogieKind::UsVvss,
                    bogie_centers,
                    z_front,
                    y_front: y_track_top - r_front - vs * 0.3,
                    r_front,
                    front_drive: true,
                    z_rear: z_rear_end,
                    y_rear,
                    r_rear,
                    y_track_bot,
                    y_track_top,
                    z_bot_start: -1.32 * s,
                    z_bot_end: if trailing { z_rear_end } else { 0.96 * s },
                    trailing_rear_idler: trailing,
                }
            }
            TankVisualModel::M4A3E8 => {
                // M4A3E8 Sherman: 3 характерные горизонтальные тележки HVSS (6 сдвоенных катков) + 3 ролика.
                let s = hl / 2.95;
                let r_road = (belt_h * 0.285).clamp(0.16, 0.23);
                let y_w = y_track_bot + vs * 0.5 + r_road;
                let bogies = [-1.31_f32, 0.0, 1.31];
                let mut road_wheels = Vec::with_capacity(6);
                let mut bogie_centers = Vec::with_capacity(3);
                for c in bogies {
                    bogie_centers.push(c * s);
                    road_wheels.push(RoadWheel {
                        z: (c - 0.27) * s,
                        y: y_w,
                        r: r_road,
                        x_min: inner_w + 0.04,
                        x_max: hw - 0.02,
                        spoked: false,
                        all_steel: false,
                    });
                    road_wheels.push(RoadWheel {
                        z: (c + 0.27) * s,
                        y: y_w,
                        r: r_road,
                        x_min: inner_w + 0.04,
                        x_max: hw - 0.02,
                        spoked: false,
                        all_steel: false,
                    });
                }
                let r_ret = (belt_h * 0.125).clamp(0.065, 0.095);
                let y_ret = y_track_top - vs * 0.45 - r_ret;
                let rollers = bogies
                    .into_iter()
                    .map(|z| ReturnRoller {
                        z: z * s,
                        y: y_ret,
                        r: r_ret,
                    })
                    .collect();
                let r_front = (belt_h * 0.32).clamp(0.17, 0.25);
                let r_rear = (belt_h * 0.30).clamp(0.16, 0.24);
                GearSpec {
                    road_wheels,
                    rollers,
                    bogie_kind: BogieKind::UsHvss,
                    bogie_centers,
                    z_front,
                    y_front: y_track_top - r_front - vs * 0.3,
                    r_front,
                    front_drive: true,
                    z_rear,
                    y_rear: y_track_top - r_rear - vs * 0.3,
                    r_rear,
                    y_track_bot,
                    y_track_top,
                    z_bot_start: -1.68 * s,
                    z_bot_end: 1.68 * s,
                    trailing_rear_idler: false,
                }
            }
            TankVisualModel::M24Chaffee => {
                // M24 Chaffee: 5 торсионных катков + 3 ролика, переднее ведущее.
                let s = hl / 2.525;
                let r_road = (belt_h * 0.305).clamp(0.15, 0.23);
                let y_w = y_track_bot + vs * 0.5 + r_road;
                let zs = [-1.60, -0.80, 0.0, 0.80, 1.60].map(|z| z * s);
                let road_wheels = zs
                    .into_iter()
                    .map(|z| RoadWheel {
                        z,
                        y: y_w,
                        r: r_road,
                        x_min: inner_w + 0.04,
                        x_max: hw - 0.02,
                        spoked: false,
                        all_steel: false,
                    })
                    .collect();
                let r_ret = (belt_h * 0.135).clamp(0.06, 0.105);
                let y_ret = y_track_top - vs * 0.45 - r_ret;
                let rollers = [-1.05, 0.0, 1.05]
                    .into_iter()
                    .map(|z| ReturnRoller {
                        z: z * s,
                        y: y_ret,
                        r: r_ret,
                    })
                    .collect();
                let r_front = (belt_h * 0.32).clamp(0.15, 0.24);
                let r_rear = (belt_h * 0.29).clamp(0.14, 0.22);
                GearSpec {
                    road_wheels,
                    rollers,
                    bogie_kind: BogieKind::None,
                    bogie_centers: Vec::new(),
                    z_front,
                    y_front: y_track_top - r_front - vs * 0.3,
                    r_front,
                    front_drive: true,
                    z_rear,
                    y_rear: y_track_top - r_rear - vs * 0.3,
                    r_rear,
                    y_track_bot,
                    y_track_top,
                    z_bot_start: -1.68 * s,
                    z_bot_end: 1.66 * s,
                    trailing_rear_idler: false,
                }
            }
            TankVisualModel::M26Pershing => {
                // M26 Pershing: 6 торсионных катков + 5 поддерживающих роликов; ЗАДНЕЕ ведущее колесо.
                let s = hl / 3.175;
                let r_road = (belt_h * 0.295).clamp(0.17, 0.24);
                let y_w = y_track_bot + vs * 0.5 + r_road;
                let road_wheels = (0..6)
                    .map(|i| RoadWheel {
                        z: (-2.05 + i as f32 * 0.82) * s,
                        y: y_w,
                        r: r_road,
                        x_min: inner_w + 0.04,
                        x_max: hw - 0.02,
                        spoked: false,
                        all_steel: false,
                    })
                    .collect();
                let r_ret = (belt_h * 0.115).clamp(0.06, 0.095);
                let y_ret = y_track_top - vs * 0.45 - r_ret;
                let rollers = [-1.64, -0.82, 0.0, 0.82, 1.64]
                    .into_iter()
                    .map(|z| ReturnRoller {
                        z: z * s,
                        y: y_ret,
                        r: r_ret,
                    })
                    .collect();
                let r_front = (belt_h * 0.285).clamp(0.16, 0.23);
                let r_rear = (belt_h * 0.32).clamp(0.17, 0.26);
                GearSpec {
                    road_wheels,
                    rollers,
                    bogie_kind: BogieKind::None,
                    bogie_centers: Vec::new(),
                    z_front,
                    y_front: y_track_top - r_front - vs * 0.3,
                    r_front,
                    front_drive: false,
                    z_rear,
                    y_rear: y_track_top - r_rear - vs * 0.3,
                    r_rear,
                    y_track_bot,
                    y_track_top,
                    z_bot_start: -2.12 * s,
                    z_bot_end: 2.12 * s,
                    trailing_rear_idler: false,
                }
            }
            TankVisualModel::Auto => unreachable!("resolve_model eliminates Auto"),
        };

        Self {
            p,
            model,
            team,
            pal: TankPalette::for_team(team),
            hl,
            hw,
            inner_w,
            clearance,
            fender_y0,
            fender_y1,
            gear,
        }
    }

    fn sample_hull(&self, pt: Vec3) -> Option<Material> {
        let ax = pt.x.abs();

        if let Some(m) = self.hull_addons(pt, ax) {
            return Some(m);
        }

        if let Some(m) = self.sample_gear(pt, ax) {
            return Some(m);
        }

        if let Some(m) = self.sample_fenders(pt, ax) {
            return Some(m);
        }

        self.sample_hull_armor(pt, ax)
    }

    fn sample_gear(&self, pt: Vec3, ax: f32) -> Option<Material> {
        let g = &self.gear;
        let vs = self.p.voxel_size;
        let track_in = self.inner_w + 0.02;
        let track_out = self.hw;

        // 1. Оси балансиров и тележки подвески (связаны с корпусом через внутреннюю стенку).
        if ax >= self.inner_w - vs * 1.15 && ax <= track_out - 0.03 {
            // Специальные видимые элементы тележек подвески (VVSS / HVSS / Pz. IV / МС-1).
            match g.bogie_kind {
                BogieKind::UsHvss => {
                    for &cz in &g.bogie_centers {
                        let dz = (pt.z - cz).abs();
                        if ax >= self.inner_w - vs * 0.8
                            && dz <= 0.34
                            && pt.y >= g.y_track_bot + 0.14
                            && pt.y <= g.y_track_bot + 0.48
                        {
                            // Горизонтальная пружина HVSS в центре тележки.
                            let is_spring = dz <= 0.18
                                && pt.y >= g.y_track_bot + 0.24
                                && pt.y <= g.y_track_bot + 0.40;
                            return Some(if is_spring {
                                Material::TankSteel
                            } else {
                                self.pal.dark
                            });
                        }
                    }
                }
                BogieKind::UsVvss => {
                    for &cz in &g.bogie_centers {
                        let dz = (pt.z - cz).abs();
                        if ax >= self.inner_w - vs * 0.8
                            && dz <= 0.20
                            && pt.y >= g.y_track_bot + 0.12
                            && pt.y <= g.y_track_bot + 0.46
                        {
                            return Some(if dz <= 0.10 {
                                self.pal.dark
                            } else {
                                self.pal.armor
                            });
                        }
                    }
                }
                BogieKind::Pz4Leaf => {
                    for &cz in &g.bogie_centers {
                        let dz = (pt.z - cz).abs();
                        if ax >= self.inner_w - vs * 0.8
                            && dz <= 0.28
                            && pt.y >= g.y_track_bot + 0.12
                            && pt.y <= g.y_track_bot + 0.34
                        {
                            return Some(self.pal.dark);
                        }
                    }
                }
                BogieKind::Ms1Girder => {
                    if ax >= self.inner_w - vs * 0.8
                        && pt.z >= g.z_bot_start
                        && pt.z <= g.z_bot_end
                        && pt.y >= g.y_track_bot + 0.16
                        && pt.y <= g.y_track_bot + 0.30
                    {
                        return Some(self.pal.dark);
                    }
                }
                BogieKind::None => {}
            }

            for w in &g.road_wheels {
                if ax <= w.x_min + 0.05
                    && ((pt.z - w.z).hypot(pt.y - w.y) <= 0.11
                        || ((pt.z - w.z).abs() <= vs * 0.85
                            && pt.y >= w.y
                            && pt.y <= self.clearance + vs * 0.85))
                {
                    return Some(self.pal.dark);
                }
            }
            for r in &g.rollers {
                if ax <= track_out - 0.07 && (pt.z - r.z).hypot(pt.y - r.y) <= 0.08 {
                    return Some(self.pal.dark);
                }
            }
            if ax <= self.inner_w + 0.08
                && ((pt.z - g.z_front).hypot(pt.y - g.y_front) <= 0.10
                    || (pt.z - g.z_rear).hypot(pt.y - g.y_rear) <= 0.10)
            {
                return Some(Material::TankSteel);
            }
        }

        if ax < track_in || ax > track_out {
            return None;
        }

        let zy = Vec2::new(pt.z, pt.y);

        // 2. Ведущее колесо (с зубцами) и ленивец.
        if let Some(m) = self.wheel_end(
            ax,
            zy,
            Vec2::new(g.z_front, g.y_front),
            g.r_front,
            g.front_drive,
        ) {
            return Some(m);
        }
        if let Some(m) = self.wheel_end(
            ax,
            zy,
            Vec2::new(g.z_rear, g.y_rear),
            g.r_rear,
            !g.front_drive,
        ) {
            return Some(m);
        }

        // 3. Опорные катки (цельнометаллические для КВ-1 / МС-1, либо с резиновым бандажом).
        for w in &g.road_wheels {
            if ax < w.x_min || ax > w.x_max {
                continue;
            }
            let dz = pt.z - w.z;
            let dy = pt.y - w.y;
            let d = dz.hypot(dy);
            if d <= w.r {
                if d <= w.r * 0.28 {
                    return Some(Material::TankSteel);
                }
                if !w.all_steel && d > w.r * 0.72 {
                    return Some(Material::TankRubber);
                }
                if w.all_steel && d > w.r * 0.76 {
                    return Some(Material::TankSteel);
                }
                if ax <= w.x_max - vs * 0.7 {
                    if w.spoked && d >= w.r * 0.42 && d <= w.r * 0.65 {
                        let a = dy.atan2(dz).rem_euclid(TAU / 5.0) - TAU / 10.0;
                        if a.abs() < 0.22 {
                            return Some(self.pal.dark);
                        }
                    }
                    return Some(Material::TankWheel);
                }
            }
        }

        // 4. Поддерживающие ролики.
        let roller_mat = if matches!(self.model, TankVisualModel::Kv1 | TankVisualModel::Ms1) {
            Material::TankSteel
        } else {
            Material::TankRubber
        };
        for r in &g.rollers {
            if ax <= track_out - 0.04 && (pt.z - r.z).hypot(pt.y - r.y) <= r.r {
                return Some(roller_mat);
            }
        }

        // 5. Замкнутая гусеничная лента (с поддержкой низкого заднего ленивца у M3 Stuart).
        let rf = g.r_front + vs * 0.35;
        let rr = g.r_rear + vs * 0.35;
        let p_bot_f = Vec2::new(g.z_bot_start, g.y_track_bot);
        let p_bot_r = Vec2::new(g.z_bot_end, g.y_track_bot);
        let p_front_lo = Vec2::new(g.z_front - rf * 0.74, g.y_front - rf * 0.67);

        let d_bot = dist_seg2(zy, p_bot_f, p_bot_r);
        let d_ramp_f = dist_seg2(zy, p_bot_f, p_front_lo);
        let d_arc_f = {
            let v = zy - Vec2::new(g.z_front, g.y_front);
            if v.x <= 0.0 && v.y >= -rf * 0.72 {
                (v.length() - rf).abs()
            } else {
                f32::MAX
            }
        };

        let (d_top, d_ramp_r, d_arc_r) = if g.trailing_rear_idler {
            let top_rear_pt = Vec2::new(g.z_rear - 0.52, g.y_track_top);
            let idler_top_rear = Vec2::new(g.z_rear + rr * 0.48, g.y_rear + rr * 0.88);
            let d_t = dist_seg2(zy, Vec2::new(g.z_front, g.y_track_top), top_rear_pt);
            let d_rr = dist_seg2(zy, top_rear_pt, idler_top_rear);
            let d_ar = {
                let v = zy - Vec2::new(g.z_rear, g.y_rear);
                if v.x >= 0.0 {
                    (v.length() - rr).abs()
                } else {
                    f32::MAX
                }
            };
            (d_t, d_rr, d_ar)
        } else {
            let p_rear_lo = Vec2::new(g.z_rear + rr * 0.74, g.y_rear - rr * 0.67);
            let d_t = dist_seg2(
                zy,
                Vec2::new(g.z_front, g.y_track_top),
                Vec2::new(g.z_rear, g.y_track_top),
            );
            let d_rr = dist_seg2(zy, p_bot_r, p_rear_lo);
            let d_ar = {
                let v = zy - Vec2::new(g.z_rear, g.y_rear);
                if v.x >= 0.0 && v.y >= -rr * 0.72 {
                    (v.length() - rr).abs()
                } else {
                    f32::MAX
                }
            };
            (d_t, d_rr, d_ar)
        };

        let d_belt = d_bot.min(d_top).min(d_ramp_f).min(d_ramp_r).min(d_arc_f).min(d_arc_r);
        let half_t = vs * 0.72;
        if d_belt <= half_t {
            let shoe_idx = ((pt.z + pt.y * 0.8) / (vs * 2.0)).floor() as i32;
            let mat = if shoe_idx.rem_euclid(2) == 0 && ax > track_in + vs * 0.4 {
                Material::TankTrackLight
            } else {
                Material::TankTrack
            };
            return Some(mat);
        }
        None
    }

    fn wheel_end(
        &self,
        ax: f32,
        zy: Vec2,
        center: Vec2,
        r: f32,
        is_sprocket: bool,
    ) -> Option<Material> {
        let v = zy - center;
        let d = v.length();
        let outer_r = if is_sprocket { r * 1.04 } else { r };
        if d > outer_r || ax > self.hw - 0.02 {
            return None;
        }
        if d <= r * 0.30 {
            return Some(Material::TankSteel);
        }
        if is_sprocket && d >= r * 0.72 {
            let tooth_x_in = (self.hw - 0.22).max(self.inner_w + 0.04);
            let sector = (v.y.atan2(v.x).rem_euclid(TAU / 6.0) / (TAU / 6.0)) * 2.0 - 1.0;
            if sector.abs() < 0.46 && ax >= tooth_x_in {
                return Some(Material::TankSteel);
            }
            if d > r * 0.90 {
                return None;
            }
        }
        if ax <= self.hw - 0.07 {
            return Some(if is_sprocket {
                Material::TankWheel
            } else {
                self.pal.dark
            });
        }
        None
    }

    fn sample_fenders(&self, pt: Vec3, ax: f32) -> Option<Material> {
        let vs = self.p.voxel_size;
        let (fz0, fz1) = match self.model {
            TankVisualModel::Bt7 => (-self.hl * 0.82, self.hl - 0.14),
            _ => (-self.hl + 0.14, self.hl - 0.12),
        };

        // Бортовые экраны Schürzen у Panther и Pz. IV H (единая верхняя планка-рельса + сегментированные плиты).
        if matches!(self.model, TankVisualModel::Panther | TankVisualModel::Pz4)
            && pt.z >= fz0 + 0.25
            && pt.z <= fz1 - 0.22
        {
            let skirt_in = self.hw - 0.02;
            let skirt_out = self.hw + vs * 1.65;
            if ax >= skirt_in && ax <= skirt_out {
                let rail_bot = self.fender_y0 - 0.05;
                let skirt_bot = self.fender_y0 - 0.30;
                // Верхний крепёжный уголок-кронштейн (идёт сплошняком и стыкуется с полкой — гарантирует 1 компоненту).
                if pt.y >= rail_bot && pt.y <= self.fender_y1 + 0.02 {
                    return Some(self.pal.dark);
                }
                if pt.y >= skirt_bot && pt.y < rail_bot {
                    // Чёткие тёмные стыки 5/4 листов Schürzen без воздушного разрыва от рельсы.
                    let panels = if self.model == TankVisualModel::Panther {
                        5.0
                    } else {
                        4.0
                    };
                    let u = ((pt.z - (fz0 + 0.25)) / ((fz1 - fz0 - 0.47).max(1.0))) * panels;
                    let seam = (u - u.round()).abs() < 0.055;
                    return Some(if seam {
                        self.pal.dark
                    } else {
                        self.pal.hull_paint(pt, false)
                    });
                }
            }
        }

        if ax < self.inner_w - 0.02 || ax > self.hw + 0.04 {
            return None;
        }
        if pt.z >= fz0 && pt.z <= fz1 {
            if pt.y >= self.fender_y0 && pt.y <= self.fender_y1 {
                return Some(self.pal.hull_paint(pt, false));
            }
            if matches!(self.model, TankVisualModel::Tiger1 | TankVisualModel::Tiger2)
                && ax >= self.hw - 0.04
                && pt.y >= self.fender_y0 - vs * 0.8
                && pt.y <= self.fender_y0
            {
                return Some(self.pal.dark);
            }
        }
        // Передний изогнутый брызговик (толщина >= vs по вертикали для строгой 6-связности ступеньки).
        if pt.z < fz0 && pt.z >= fz0 - 0.24 {
            let drop = (fz0 - pt.z) * 0.48;
            if pt.y >= self.fender_y0 - drop - vs * 0.6 && pt.y <= self.fender_y1 - drop + vs * 0.6
            {
                return Some(self.pal.dark);
            }
        }
        // Задний брызговик.
        if pt.z > fz1 && pt.z <= fz1 + 0.22 {
            let drop = (pt.z - fz1) * 0.45;
            if pt.y >= self.fender_y0 - drop - vs * 0.6 && pt.y <= self.fender_y1 - drop + vs * 0.6
            {
                return Some(self.pal.dark);
            }
        }
        None
    }

    fn hull_front_z(&self, ax: f32, y: f32) -> f32 {
        let dh = self.p.deck_height;
        match self.model {
            TankVisualModel::M4A3E8 => {
                // M4A3E8 Sherman: выраженная выпуклая округлая литая трансмиссия НЛД + единая плита ВЛД 47°.
                let y_mid = dh * 0.56;
                if y >= y_mid {
                    -self.hl + 0.06 + (y - y_mid) * 1.07
                } else {
                    let frac = ((y - self.clearance) / (y_mid - self.clearance).max(0.1))
                        .clamp(0.0, 1.0);
                    let bulge = (frac * std::f32::consts::PI).sin() * 0.14;
                    -self.hl + 0.06 - bulge + (1.0 - frac) * 0.18
                }
            }
            TankVisualModel::M26Pershing => {
                // M26 Pershing: низкий широкий корпус, ступенчатая литая передача + гладкая 46° ВЛД.
                let y_mid = dh * 0.58;
                if y >= y_mid {
                    -self.hl + 0.06 + (y - y_mid) * 1.03
                } else {
                    let frac = ((y - self.clearance) / (y_mid - self.clearance).max(0.1))
                        .clamp(0.0, 1.0);
                    -self.hl + 0.04 + (1.0 - frac) * 0.22
                }
            }
            TankVisualModel::Panther => {
                // Panther: высокий строгий острый 55° немецкий клин ВЛД/НЛД.
                let y_mid = dh * 0.56;
                if y >= y_mid {
                    -self.hl + (y - y_mid) * 1.12
                } else {
                    -self.hl + (y_mid - y) * 0.74
                }
            }
            TankVisualModel::Tiger2 => {
                // Tiger II: длинная прямая наклонная ВЛД 50°.
                let y_mid = dh * 0.58;
                if y >= y_mid {
                    -self.hl + (y - y_mid) * 0.94
                } else {
                    -self.hl + (y_mid - y) * 0.68
                }
            }
            TankVisualModel::T34_76 | TankVisualModel::T34_85 => {
                let y_mid = dh * 0.565;
                if y >= y_mid {
                    -self.hl + (y - y_mid) * 1.18
                } else {
                    -self.hl + (y_mid - y) * 0.72
                }
            }
            TankVisualModel::Su100 => {
                let y_mid = dh * 0.60;
                if y >= y_mid {
                    -self.hl + (y - y_mid) * 0.98
                } else {
                    -self.hl + (y_mid - y) * 0.66
                }
            }
            TankVisualModel::Is3 => {
                let y_mid = dh * 0.610;
                let v = ax * 0.56;
                if y >= y_mid {
                    -self.hl + v + (y - y_mid) * 1.05
                } else {
                    -self.hl + v + (y_mid - y) * 0.70
                }
            }
            TankVisualModel::Kv1 => {
                // КВ-1: наклонная НЛД -> ровная передняя полка -> почти вертикальная бронеплита мехвода.
                let y_lo = dh * 0.52;
                let y_hi = dh * 0.66;
                if y >= y_hi {
                    -self.hl + 0.48 + (y - y_hi) * 0.36
                } else if y >= y_lo {
                    let frac = (y - y_lo) / (y_hi - y_lo).max(0.05);
                    -self.hl + 0.10 + frac * 0.38
                } else {
                    -self.hl + 0.10 + (y_lo - y) * 0.72
                }
            }
            TankVisualModel::Pz2C | TankVisualModel::Pz3 | TankVisualModel::Pz4 | TankVisualModel::Tiger1 => {
                // Классический немецкий ступенчатый нос.
                let s = (self.hl / 3.15).clamp(0.65, 1.15);
                let y_lo = dh * 0.580;
                let y_hi = dh * 0.696;
                let step_d = 0.12 * s;
                let step_w = 0.32 * s;
                if y >= y_hi {
                    -self.hl + step_d + step_w + (y - y_hi) * 0.14
                } else if y >= y_lo {
                    -self.hl + step_d + ((y - y_lo) / (y_hi - y_lo).max(0.05)) * step_w
                } else {
                    -self.hl + step_d + (y_lo - y) * 0.72
                }
            }
            TankVisualModel::Bt7 => {
                // БТ-7: удлиненный заострённый клиновидный «лодочный» нос с крутой крышкой мехвода.
                let y_mid = dh * 0.60;
                if y >= y_mid {
                    -self.hl + (y - y_mid) * 1.32
                } else {
                    -self.hl + (y_mid - y) * 0.58
                }
            }
            TankVisualModel::Ms1 => {
                // МС-1: короткий высокий заклёпанный нос со скосом вверх и вертикальной рубкой мехвода.
                let y_lo = dh * 0.58;
                let y_hi = dh * 0.70;
                if y >= y_hi {
                    -self.hl + 0.38 + (y - y_hi) * 0.25
                } else if y >= y_lo {
                    let frac = (y - y_lo) / (y_hi - y_lo).max(0.05);
                    -self.hl + 0.08 + frac * 0.30
                } else {
                    -self.hl + 0.08 + (y_lo - y) * 0.50
                }
            }
            TankVisualModel::M2A4 | TankVisualModel::M3Stuart => {
                let y_mid = dh * 0.60;
                if y >= y_mid {
                    -self.hl + 0.16 + (y - y_mid) * 0.72
                } else {
                    -self.hl + 0.16 + (y_mid - y) * 0.42
                }
            }
            TankVisualModel::M24Chaffee => {
                // M24 Chaffee: сильно скошенный низкий обтекаемый лоб 60°.
                let y_mid = dh * 0.55;
                if y >= y_mid {
                    -self.hl + (y - y_mid) * 1.25
                } else {
                    -self.hl + (y_mid - y) * 0.62
                }
            }
            TankVisualModel::Auto => unreachable!("resolve_model eliminates Auto"),
        }
    }

    fn hull_rear_z(&self, y: f32) -> f32 {
        let base_drop = (y - self.clearance).max(0.0);
        match self.model {
            TankVisualModel::Kv1 => {
                // КВ-1: изогнутый назад/вверх закруглённый нижний кормовой лист.
                let frac = ((y - self.clearance) / (self.p.deck_height - self.clearance).max(0.1))
                    .clamp(0.0, 1.0);
                self.hl - 0.10 - (1.0 - frac).powi(2) * 0.18
            }
            TankVisualModel::M4A3E8 | TankVisualModel::Tiger1 | TankVisualModel::Pz2C | TankVisualModel::Pz3 | TankVisualModel::Pz4 => {
                self.hl - base_drop * 0.08
            }
            TankVisualModel::M26Pershing | TankVisualModel::Panther | TankVisualModel::Tiger2 => {
                self.hl - base_drop * 0.22
            }
            TankVisualModel::Ms1 => {
                // Задняя моторная часть МС-1 скошена к хвосту.
                self.hl - base_drop * 0.30
            }
            _ => self.hl - base_drop * 0.25,
        }
    }

    fn hull_half_width(&self, y: f32, z: f32) -> f32 {
        let t = if self.p.deck_height > self.fender_y1 {
            ((y - self.fender_y1) / (self.p.deck_height - self.fender_y1)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if y <= self.fender_y1 {
            if self.model == TankVisualModel::Bt7 && z < -self.hl * 0.62 {
                // Зауженный в клин вытянутый нос БТ-7 перед гусеницами.
                let u = ((z - (-self.hl)) / (self.hl * 0.38)).clamp(0.0, 1.0);
                return self.inner_w * (0.28 + 0.72 * u);
            }
            return self.inner_w;
        }
        let tw = self.hw - self.inner_w;
        match self.model {
            TankVisualModel::M4A3E8 => {
                // Высокие почти вертикальные борта Шермана со слегка закруглённым верхним кантом.
                if t > 0.85 {
                    self.hw - 0.04 - (t - 0.85) * 0.45
                } else {
                    self.hw - 0.03
                }
            }
            TankVisualModel::M26Pershing => {
                // Низкий широкий корпус M26 с узкими скосами надгусеничных спонсонов.
                self.inner_w + tw * (0.86 - 0.24 * t)
            }
            TankVisualModel::Panther | TankVisualModel::Tiger2 => {
                // Сильно скошенные внутрь (~35°) борта Panther и Tiger II.
                (self.inner_w + tw * 0.90) * (1.0 - t) + (self.inner_w + tw * 0.26) * t
            }
            TankVisualModel::Tiger1 => self.hw - 0.04,
            TankVisualModel::Pz2C => {
                // Pz. II C: широкая рубка поверх полок для сдвинутого влево погона башни.
                if z <= self.hl * 0.42 {
                    self.inner_w + tw * (0.82 - 0.14 * t)
                } else {
                    self.inner_w + tw * (0.62 - 0.18 * t)
                }
            }
            TankVisualModel::Pz3 | TankVisualModel::Pz4 => {
                // Широкий спонсон немецких средних танков до края гусениц.
                self.inner_w + tw * (0.88 - 0.12 * t)
            }
            TankVisualModel::Kv1 => {
                // Строгий вертикальный тяжёлый бронекорпус КВ-1.
                self.inner_w + tw * (0.58 - 0.08 * t)
            }
            TankVisualModel::Is3 => {
                (self.inner_w + tw * 0.90) * (1.0 - t) + (self.inner_w + tw * 0.345) * t
            }
            TankVisualModel::Bt7 => {
                let base_w = (self.inner_w + tw * 0.65) * (1.0 - t) + (self.inner_w + tw * 0.28) * t;
                if z < -self.hl * 0.62 {
                    let u = ((z - (-self.hl)) / (self.hl * 0.38)).clamp(0.0, 1.0);
                    base_w * (0.34 + 0.66 * u)
                } else {
                    base_w
                }
            }
            TankVisualModel::T34_76 | TankVisualModel::T34_85 | TankVisualModel::Su100 => {
                (self.inner_w + tw * 0.84) * (1.0 - t) + (self.inner_w + tw * 0.32) * t
            }
            TankVisualModel::M24Chaffee => {
                // Обтекаемые наклонные спонсоны M24 Chaffee.
                self.inner_w + tw * (0.86 - 0.38 * t)
            }
            TankVisualModel::Ms1 | TankVisualModel::M2A4 | TankVisualModel::M3Stuart => {
                (self.inner_w + tw * 0.62) * (1.0 - t) + (self.inner_w + tw * 0.24) * t
            }
            TankVisualModel::Auto => unreachable!("resolve_model eliminates Auto"),
        }
    }

    fn sample_hull_armor(&self, pt: Vec3, ax: f32) -> Option<Material> {
        let vs = self.p.voxel_size;
        let dh = self.p.deck_height;
        if pt.y < self.clearance || pt.y > dh + 0.01 {
            return None;
        }

        let max_x = self.hull_half_width(pt.y, pt.z);
        if ax > max_x {
            return None;
        }

        let front_z = self.hull_front_z(ax, pt.y);
        let rear_z = self.hull_rear_z(pt.y);
        if pt.z < front_z || pt.z > rear_z {
            return None;
        }

        // Рельефная решётка МТО на кормовой крыше (круглые вентиляторы для Panther, прямоугольные жалюзи для остальных).
        let mto_z0 = self.hl * 0.24;
        let mto_z1 = rear_z - (0.18_f32).min(self.hl * 0.08);
        if pt.y > dh - vs * 1.15 && pt.z > mto_z0 && pt.z < mto_z1 && ax < self.inner_w * 0.74 {
            if matches!(self.model, TankVisualModel::Panther | TankVisualModel::Tiger2) {
                // Круглые немецкие вентиляторы по бокам кормы + центральная щель.
                let fan_r = self.inner_w * 0.24;
                let fan_z = (mto_z0 + mto_z1) * 0.50;
                if (ax - self.inner_w * 0.42).hypot(pt.z - fan_z) <= fan_r {
                    return Some(self.pal.dark);
                }
            } else {
                let slat = (pt.z / (vs * 2.0)).floor() as i32;
                if pt.y > dh - vs * 0.75 && slat % 2 == 0 && ax > self.inner_w * 0.12 {
                    return None;
                }
                return Some(self.pal.dark);
            }
        }

        // Сварной стык «щучьего носа» ИС-3.
        if self.model == TankVisualModel::Is3
            && ax < vs * 0.7
            && pt.z < front_z + vs * 1.4
            && pt.y >= dh * 0.60
        {
            return Some(self.pal.armor_light);
        }

        // Индивидуальные смотровые приборы мехвода на лобовой плите.
        if pt.y >= dh * 0.66 && pt.z < front_z + vs * 1.5 {
            match self.model {
                TankVisualModel::T34_76 | TankVisualModel::T34_85 => {
                    // Выпуклый прямоугольный люк мехвода Т-34 слева.
                    if (-self.inner_w * 0.62..=-self.inner_w * 0.14).contains(&pt.x)
                        && (dh * 0.73..=dh * 0.92).contains(&pt.y)
                    {
                        if (dh * 0.835..=dh * 0.905).contains(&pt.y)
                            && (pt.x + self.inner_w * 0.38).abs() <= self.inner_w * 0.16
                        {
                            return Some(Material::TankLens);
                        }
                        return Some(self.pal.armor_light);
                    }
                }
                TankVisualModel::Kv1 => {
                    // Центральный козырёк мехвода КВ-1.
                    if pt.x.abs() <= self.inner_w * 0.22 && (dh * 0.78..=dh * 0.88).contains(&pt.y) {
                        return Some(Material::TankLens);
                    }
                }
                TankVisualModel::Tiger1
                | TankVisualModel::Pz4
                | TankVisualModel::Pz3
                | TankVisualModel::Pz2C => {
                    if (dh * 0.80..=dh * 0.90).contains(&pt.y)
                        && (-self.inner_w * 0.70..=-self.inner_w * 0.32).contains(&pt.x)
                    {
                        return Some(Material::TankLens);
                    }
                }
                TankVisualModel::Panther | TankVisualModel::Tiger2
                    if (dh * 0.86..=dh * 0.94).contains(&pt.y)
                        && (-self.inner_w * 0.60..=-self.inner_w * 0.34).contains(&pt.x) =>
                {
                    // Узкая щель перископа / визора на левой стороне ВЛД Panther / Tiger II.
                    return Some(Material::TankLens);
                }
                _ => {}
            }
        }

        // Люки мехвода и стрелка-радиста на передней крыше / ВЛД.
        let is_hatch = match self.model {
            TankVisualModel::M4A3E8 | TankVisualModel::M26Pershing | TankVisualModel::M24Chaffee => {
                pt.y > dh - vs * 1.15
                    && (-self.hl * 0.72..=-self.hl * 0.36).contains(&pt.z)
                    && (self.inner_w * 0.22..=self.inner_w * 0.68).contains(&ax)
            }
            TankVisualModel::Panther
            | TankVisualModel::Tiger2
            | TankVisualModel::Tiger1
            | TankVisualModel::Pz4
            | TankVisualModel::Pz3
            | TankVisualModel::Pz2C
            | TankVisualModel::M2A4
            | TankVisualModel::M3Stuart => {
                pt.y > dh - vs * 1.1
                    && (-self.hl * 0.76..=-self.hl * 0.34).contains(&pt.z)
                    && (self.inner_w * 0.25..=self.inner_w * 0.72).contains(&ax)
            }
            _ => false,
        };

        // Задний круглый люк трансмиссии у Т-34 / СУ-100.
        let trans_y = dh * 0.66;
        let trans_r = (self.inner_w * 0.30).clamp(0.18, 0.32);
        if matches!(
            self.model,
            TankVisualModel::T34_76 | TankVisualModel::T34_85 | TankVisualModel::Su100
        ) && pt.z > rear_z - vs * 1.3
            && pt.x.hypot(pt.y - trans_y) <= trans_r
        {
            return Some(if pt.x.hypot(pt.y - trans_y) > trans_r * 0.73 {
                self.pal.dark
            } else {
                self.pal.armor_light
            });
        }

        Some(self.pal.hull_paint(pt, is_hatch))
    }

    fn hull_addons(&self, pt: Vec3, ax: f32) -> Option<Material> {
        let vs = self.p.voxel_size;
        let dh = self.p.deck_height;
        let tw = self.hw - self.inner_w;

        // 1. Погон башни на крыше (учитывает смещённые и кормовые башни!).
        if self.p.turret_shape != TurretShape::FixedCasemate {
            let tp = Vec3::from_array(self.p.turret_pivot);
            if pt.y >= dh - vs && pt.y <= dh + 0.03 {
                let d = (pt.x - tp.x).hypot(pt.z - tp.z);
                if d <= self.p.turret_radius_x * 0.94 && d >= self.p.turret_radius_x * 0.70 {
                    return Some(self.pal.dark);
                }
            }
        }

        // 2. Буксирные крюки на носу (учитывают сужение лодочного носа БТ-7 и прочих).
        let y_hook = dh * 0.56;
        let hook_z_probe = self.hull_front_z(0.0, y_hook);
        let max_hook_w = self.hull_half_width(y_hook, hook_z_probe);
        let hook_x = (self.inner_w * 0.60).min(max_hook_w * 0.55);
        if (ax - hook_x).abs() <= vs * 0.6
            && pt.y >= (y_hook - 0.12).max(self.clearance)
            && pt.y <= y_hook + 0.04
        {
            let fz = self.hull_front_z(ax, pt.y);
            if pt.z >= fz - 0.10 && pt.z <= fz + 0.14 {
                return Some(Material::TankSteel);
            }
        }

        // 3. Передние парные бугры-капюшоны люков мехвода/радиста на ВЛД M4A3E8 и M26 Pershing.
        if matches!(self.model, TankVisualModel::M4A3E8 | TankVisualModel::M26Pershing)
            && pt.y >= dh * 0.76
            && pt.y <= dh + 0.06
            && (ax - self.inner_w * 0.44).abs() <= self.inner_w * 0.24
        {
            let fz = self.hull_front_z(ax, pt.y.min(dh));
            if pt.z >= fz - 0.10 && pt.z <= fz + 0.14 {
                return Some(if pt.y > dh * 0.88 && (ax - self.inner_w * 0.44).abs() <= 0.12 {
                    Material::TankLens
                } else {
                    self.pal.hull_paint(pt, true)
                });
            }
        }

        // 4. Ящики-спонсоны вдоль бортовых полок на M26 Pershing (главный узнаваемый признак M26!).
        if self.model == TankVisualModel::M26Pershing
            && ax >= self.inner_w + 0.04
            && ax <= self.hw - 0.04
            && pt.y >= self.fender_y1
            && pt.y <= self.fender_y1 + 0.24
            && pt.z >= -self.hl * 0.66
            && pt.z <= self.hl * 0.64
        {
            let bin_seam = ((pt.z + self.hl * 0.66) / (self.hl * 0.44)).fract() < 0.08;
            return Some(if bin_seam || pt.y > self.fender_y1 + 0.18 {
                self.pal.dark
            } else {
                self.pal.hull_paint(pt, false)
            });
        }

        // 5. Длинный изогнутый вверх задний «хвост» противоопрокидывания на МС-1 (Т-18)!
        if self.model == TankVisualModel::Ms1
            && pt.z >= self.hl - 0.28
            && pt.z <= self.hl + 0.56
            && (ax - self.inner_w * 0.46).abs() <= 0.085
        {
            let u = ((pt.z - (self.hl - 0.14)) / 0.70).clamp(0.0, 1.0);
            let y_tail = self.clearance + 0.12 + u * u * 0.36;
            if (pt.y - y_tail).abs() <= 0.085 {
                return Some(self.pal.dark);
            }
        }
        if self.model == TankVisualModel::Ms1
            && pt.z >= self.hl + 0.40
            && pt.z <= self.hl + 0.56
            && ax <= self.inner_w * 0.52
            && (pt.y - (self.clearance + 0.46)).abs() <= 0.075
        {
            return Some(Material::TankSteel);
        }

        // 6. Задние цилиндрические баки / бидоны на полках у M3 Stuart!
        if self.model == TankVisualModel::M3Stuart
            && ax >= self.inner_w + 0.05
            && ax <= self.hw - 0.05
            && pt.z >= self.hl * 0.58
            && pt.z <= self.hl * 0.92
            && pt.y >= self.fender_y1
            && pt.y <= self.fender_y1 + 0.28
        {
            return Some(if pt.y > self.fender_y1 + 0.22 {
                self.pal.dark
            } else {
                self.pal.hull_paint(pt, true)
            });
        }

        // 7. Бортовые пулемётные спонсоны у раннего M2A4 Light!
        if self.model == TankVisualModel::M2A4
            && ax >= self.inner_w + 0.02
            && ax <= self.hw - 0.05
            && pt.z >= -self.hl * 0.42
            && pt.z <= -self.hl * 0.06
            && pt.y >= self.fender_y1
            && pt.y <= self.fender_y1 + 0.22
        {
            if pt.z <= -self.hl * 0.36 && (ax - (self.inner_w + tw * 0.50)).abs() <= 0.07 {
                return Some(Material::TankSteel);
            }
            return Some(self.pal.hull_paint(pt, false));
        }

        // 8. Запасные траки на ВЛД (Т-34, СУ-100, Panther, Tiger I, Pz IV, M4A3E8).
        if matches!(
            self.model,
            TankVisualModel::T34_76
                | TankVisualModel::T34_85
                | TankVisualModel::Su100
                | TankVisualModel::Panther
                | TankVisualModel::Tiger1
                | TankVisualModel::Pz4
                | TankVisualModel::M4A3E8
        ) && ax <= self.inner_w * 0.52
        {
            let rack_y = match self.model {
                TankVisualModel::Tiger1 | TankVisualModel::Pz4 => (dh * 0.58, dh * 0.68),
                TankVisualModel::Panther => (dh * 0.48, dh * 0.62),
                _ => (dh * 0.54, dh * 0.69),
            };
            if pt.y >= rack_y.0 && pt.y <= rack_y.1 {
                let fz = self.hull_front_z(ax, pt.y);
                if pt.z >= fz - 0.075 && pt.z <= fz + 0.09 {
                    let link = (pt.x / vs).floor() as i32;
                    return Some(if link % 2 == 0 {
                        Material::TankTrack
                    } else {
                        Material::TankTrackLight
                    });
                }
            }
        }

        // 9. Курсовой пулемёт справа на лбу (кроме безбашенных/чистых корпусов без курсового МГ).
        if !matches!(
            self.model,
            TankVisualModel::Su100 | TankVisualModel::Is3 | TankVisualModel::Ms1 | TankVisualModel::Bt7
        ) {
            let mg_x = self.inner_w * 0.44;
            let mg_y = match self.model {
                TankVisualModel::Tiger1 | TankVisualModel::Pz4 | TankVisualModel::Pz3 | TankVisualModel::Pz2C => dh * 0.855,
                TankVisualModel::M4A3E8 | TankVisualModel::M26Pershing | TankVisualModel::Panther | TankVisualModel::Tiger2 => dh * 0.78,
                _ => dh * 0.79,
            };
            let mg_z = self.hull_front_z(mg_x, mg_y) - 0.02;
            let barrel_len = (0.18_f32).min(self.hl * 0.09);
            let radial = Vec2::new(pt.x - mg_x, pt.y - mg_y).length();
            if radial <= 0.095 && pt.z >= mg_z - 0.08 && pt.z <= mg_z + 0.14 {
                return Some(self.pal.dark);
            }
            if radial <= (vs * 0.72).max(0.058) && pt.z >= mg_z - barrel_len && pt.z <= mg_z {
                return Some(Material::TankSteel);
            }
        }

        // 10. Перископический колпак мехвода ИС-3 в вершине «щучьего носа».
        if self.model == TankVisualModel::Is3 {
            let s = self.hl / 3.3;
            if ax <= self.inner_w * 0.22
                && pt.y >= dh - 0.02
                && pt.y <= dh + 0.12
                && pt.z >= -self.hl + 0.72 * s
                && pt.z <= -self.hl + 1.06 * s
            {
                if pt.z <= -self.hl + 0.82 * s && ax <= self.inner_w * 0.16 {
                    return Some(Material::TankLens);
                }
                return Some(self.pal.dark);
            }
        }

        // 11. Фара слева спереди.
        let lamp_pos = match self.model {
            TankVisualModel::Tiger1 | TankVisualModel::Pz4 | TankVisualModel::Pz3 => Vec3::new(
                -(self.inner_w + tw * 0.60),
                dh + 0.06,
                -self.hl + 0.56 * (self.hl / 3.15),
            ),
            TankVisualModel::Is3 => Vec3::new(
                -self.inner_w * 0.54,
                dh + 0.04,
                -self.hl + 0.75 * (self.hl / 3.3),
            ),
            TankVisualModel::Bt7 => Vec3::new(
                -(self.inner_w + tw * 0.42),
                self.fender_y1 + 0.06,
                -self.hl * 0.66,
            ),
            _ => Vec3::new(
                -(self.inner_w + tw * 0.36),
                self.fender_y1 + 0.07,
                -self.hl + (0.36_f32).min(self.hl * 0.16),
            ),
        };
        if (pt.x - lamp_pos.x).hypot(pt.y - lamp_pos.y) <= 0.085
            && pt.z >= lamp_pos.z - 0.10
            && pt.z <= lamp_pos.z + 0.10
        {
            return Some(if pt.z <= lamp_pos.z - 0.03 {
                Material::TankLens
            } else {
                self.pal.dark
            });
        }

        // 12. Наружные цилиндрические топливные баки ТОЛЬКО у советских танков (Т-34, СУ-100, ИС-3)!
        if matches!(
            self.model,
            TankVisualModel::T34_76
                | TankVisualModel::T34_85
                | TankVisualModel::Su100
                | TankVisualModel::Is3
        ) {
            let drum_r = (tw * 0.36).clamp(0.11, 0.18);
            let (drum_x, drum_y, z0, z1) = if self.model == TankVisualModel::Is3 {
                (
                    self.inner_w + tw * 0.69,
                    dh + drum_r - 0.12,
                    self.hl * 0.26,
                    self.hl * 0.83,
                )
            } else {
                (
                    self.inner_w + tw * 0.72,
                    self.fender_y1 + drum_r - 0.01,
                    self.hl * 0.35,
                    self.hl * 0.85,
                )
            };
            if pt.z >= z0 && pt.z <= z1 {
                let d = (ax - drum_x).hypot(pt.y - drum_y);
                let in_gap = (pt.z - (z0 + z1) * 0.5).abs() < 0.07;
                if !in_gap && d <= drum_r {
                    let strap = ((pt.z - z0) % 0.42).abs() < 0.06;
                    return Some(if strap {
                        self.pal.dark
                    } else {
                        self.pal.hull_paint(pt, false)
                    });
                }
            }
        }

        // 13. Бревно самовытаскивания у СУ-100 или длинный ящик-глушитель на левом крыле Pz. II C.
        if self.model == TankVisualModel::Su100
            && pt.x < -(self.inner_w + 0.04)
            && pt.x > -(self.hw - 0.04)
            && pt.z >= -self.hl * 0.22
            && pt.z <= self.hl * 0.28
            && (pt.y - (self.fender_y1 + 0.10)).abs() <= 0.10
        {
            return Some(Material::TankWood);
        }
        if self.model == TankVisualModel::Pz2C
            && pt.x < -(self.inner_w + 0.02)
            && pt.x > -(self.hw - 0.03)
            && pt.z >= 0.05
            && pt.z <= self.hl * 0.82
            && (pt.y - (self.fender_y1 + 0.08)).abs() <= 0.08
        {
            return Some(Material::TankExhaust);
        }

        // 14. Уникальные выхлопные системы на корме:
        //  - Tiger I: вертикальные фильтры Фейфеля с кожухами
        //  - Panther / Tiger II: парные косые выхлопные трубы + квадратные кормовые ящики Staukasten
        //  - Pz. III / Pz. IV: горизонтальный поперечный цилиндр-глушитель
        //  - США / СССР: парные кормовые патрубки / дефлекторы
        match self.model {
            TankVisualModel::Tiger1 => {
                let ex = self.inner_w * 0.58;
                let ey0 = dh * 0.52;
                let rz = self.hull_rear_z(pt.y.min(dh));
                if (ax - ex).abs() <= 0.14
                    && pt.z >= rz - 0.04
                    && pt.z <= self.hl + 0.24
                    && pt.y >= ey0
                    && pt.y <= dh + 0.24
                {
                    let inner = (ax - ex).abs() <= 0.06 && pt.y >= dh + 0.04;
                    return Some(if inner {
                        Material::TankExhaust
                    } else {
                        self.pal.dark
                    });
                }
            }
            TankVisualModel::Panther | TankVisualModel::Tiger2 => {
                let rz = self.hull_rear_z(pt.y.min(dh));
                // Наклонённые назад-вверх выхлопные патрубки.
                if (ax - self.inner_w * 0.30).abs() <= 0.09
                    && pt.y >= dh * 0.44
                    && pt.y <= dh * 0.96
                    && pt.z >= rz - 0.04
                    && pt.z <= rz + 0.22
                {
                    return Some(Material::TankExhaust);
                }
                // Пара квадратных кормовых ящиков (Staukasten) на Panther.
                if self.model == TankVisualModel::Panther
                    && ax >= self.inner_w * 0.48
                    && ax <= self.inner_w * 0.92
                    && pt.y >= dh * 0.46
                    && pt.y <= dh * 0.88
                    && pt.z >= rz - 0.04
                    && pt.z <= rz + 0.24
                {
                    return Some(self.pal.hull_paint(pt, false));
                }
            }
            TankVisualModel::Pz3 | TankVisualModel::Pz4 => {
                let ey = self.clearance + 0.24;
                let rz = self.hull_rear_z(ey);
                if ax <= self.inner_w * 0.68
                    && (pt.y - ey).hypot(pt.z - (rz + 0.08)) <= 0.11
                    && pt.z >= rz - 0.04
                {
                    return Some(Material::TankExhaust);
                }
            }
            _ => {
                let ex = self.inner_w * 0.54;
                let ey = dh * 0.72;
                let rz = self.hull_rear_z(ey);
                if (ax - ex).hypot(pt.y - ey) <= 0.085 && pt.z >= rz - 0.04 && pt.z <= rz + 0.20 {
                    return Some(Material::TankExhaust);
                }
            }
        }

        None
    }

    fn turret_side_limit(&self, pt_y: f32, pt_z: f32, rx: f32, rz: f32, th: f32) -> f32 {
        let t = (pt_y / th).clamp(0.0, 1.0);
        match self.model {
            TankVisualModel::Panther => {
                // Плоские сильно заваленные борта и суженный нос Panther Ausf. G!
                let z_taper = if pt_z < 0.0 {
                    1.0 - ((-pt_z) / rz.max(0.2)).clamp(0.0, 1.0) * 0.24
                } else {
                    1.0 - (pt_z / (rz * 1.1).max(0.2)).clamp(0.0, 1.0) * 0.12
                };
                rx * (1.0 - t * 0.34) * z_taper
            }
            TankVisualModel::Tiger2 => {
                // Длинный высокий Хеншель с крутым плоским завалом бортов 21°!
                let z_taper = if pt_z < 0.0 {
                    1.0 - ((-pt_z) / rz.max(0.2)).clamp(0.0, 1.0) * 0.30
                } else {
                    1.0 - (pt_z / (rz * 1.15).max(0.2)).clamp(0.0, 1.0) * 0.16
                };
                rx * (1.0 - t * 0.30) * z_taper
            }
            TankVisualModel::T34_76 => {
                // Компактная 6-гранная башня «Гайка».
                let k = 1.0 - t * 0.26;
                let z_fac = if pt_z < -rz * 0.15 {
                    let u = ((-pt_z - rz * 0.15) / (rz * 0.85)).clamp(0.0, 1.0);
                    1.0 - u * 0.38
                } else if pt_z > rz * 0.15 {
                    let u = ((pt_z - rz * 0.15) / (rz * 0.90)).clamp(0.0, 1.0);
                    1.0 - u * 0.28
                } else {
                    1.0
                };
                rx * k * z_fac
            }
            TankVisualModel::Kv1 => {
                // Высокие почти вертикальные борта КВ-1.
                let k = 1.0 - t * 0.10;
                rx * k
            }
            _ => rx * (1.0 - t * 0.16),
        }
    }

    fn turret_core_inside(&self, pt: Vec3, rx: f32, rz: f32, th: f32) -> bool {
        let t = (pt.y / th).clamp(0.0, 1.0);
        match self.model {
            TankVisualModel::Tiger1 => {
                let k = 1.0 - t * 0.06;
                if pt.z < 0.0 {
                    let nx = pt.x / (rx * k);
                    let nz = pt.z / (rz * k);
                    nx * nx + nz * nz <= 1.0
                } else {
                    pt.x.abs() <= rx * k && pt.z <= rz * k
                }
            }
            TankVisualModel::Tiger2 => {
                // Хеншель: плоский слегка наклонённый лоб (9°), плоские сильно скошенные борта (21°), длинная кормовая ниша.
                let front_z = -rz * 0.92 + t * (rz * 0.14);
                let rear_z = rz * 1.08 + t * 0.06;
                let side_x = self.turret_side_limit(pt.y, pt.z, rx, rz, th);
                pt.x.abs() <= side_x && pt.z >= front_z && pt.z <= rear_z
            }
            TankVisualModel::Panther => {
                // Пантера: плоские заваленные борта и плоская наклонная корма с люком (БЕЗ Rommelkiste).
                let front_z = -rz * 0.92 + t * (rz * 0.18);
                let rear_z = rz * 0.96 - t * (rz * 0.18);
                let side_x = self.turret_side_limit(pt.y, pt.z, rx, rz, th);
                pt.x.abs() <= side_x && pt.z >= front_z && pt.z <= rear_z
            }
            TankVisualModel::Pz4 | TankVisualModel::Pz3 | TankVisualModel::Pz2C => {
                // Граненные плоские борта со скосом к носу у Pz. II / III / IV.
                let k = 1.0 - t * 0.08;
                let half_x = if pt.z < -rz * 0.15 {
                    let u = ((-pt.z - rz * 0.15) / (rz * 0.85)).clamp(0.0, 1.0);
                    rx * k * (1.0 - 0.36 * u)
                } else if pt.z > rz * 0.55 {
                    let u = ((pt.z - rz * 0.55) / (rz * 0.45)).clamp(0.0, 1.0);
                    rx * k * (1.0 - 0.22 * u)
                } else {
                    rx * k
                };
                pt.x.abs() <= half_x && pt.z >= -rz * (1.0 - t * 0.12) && pt.z <= rz * k
            }
            TankVisualModel::M4A3E8 => {
                // Литая высокая башня T23: округлый корпус + выдающаяся назад кормовая радиониша!
                let k = 1.0 - t.powf(1.5) * 0.20;
                let body = {
                    let nx = pt.x / (rx * k);
                    let nz = (pt.z - 0.02) / (rz * k * 0.92);
                    nx * nx + nz * nz <= 1.0
                };
                let bustle = pt.z >= rz * 0.22
                    && pt.z <= rz * 1.24
                    && pt.y >= th * 0.22
                    && (pt.x / (rx * 0.68)).powi(2) + ((pt.z - rz * 0.72) / (rz * 0.54)).powi(2) <= 1.0;
                body || bustle
            }
            TankVisualModel::M26Pershing => {
                // Низкая широкая башня M26 с ОГРОМНОЙ сужающейся клиновидной кормовой противовесной нишей!
                let k = 1.0 - t * 0.18;
                let front_dome = {
                    let nx = pt.x / (rx * k);
                    let nz = (pt.z + rz * 0.12) / (rz * 0.72 * k);
                    nx * nx + nz * nz <= 1.0
                };
                let bustle = pt.z >= -rz * 0.08 && pt.z <= rz * 1.28 && {
                    let u = ((pt.z + rz * 0.08) / (rz * 1.36)).clamp(0.0, 1.0);
                    let bx = rx * k * (0.94 - 0.54 * u);
                    let y_min = th * 0.34 * u * u;
                    pt.x.abs() <= bx && pt.y >= y_min && pt.y <= th * (1.0 - 0.08 * u)
                };
                front_dome || bustle
            }
            TankVisualModel::T34_76 => {
                // Компактная 6-гранная башня «Гайка» Т-34-76 (резкий 20° завал и узкий силуэт).
                let side_x = self.turret_side_limit(pt.y, pt.z, rx, rz, th);
                let front_z = -rz + t * (rz * 0.24);
                let rear_z = rz - t * (rz * 0.16);
                pt.x.abs() <= side_x && pt.z >= front_z && pt.z <= rear_z
            }
            TankVisualModel::T34_85 => {
                // Раздутая литая 3-местная яйцевидная башня Т-34-85 с нависающей кормой.
                let k = 1.0 - t.powf(1.5) * 0.24;
                let nx = pt.x / (rx * k);
                let nz = (pt.z - 0.04) / (rz * (1.0 - t * 0.12));
                nx * nx + nz * nz <= 1.0
            }
            TankVisualModel::Kv1 => {
                // Прямоугольная скруглённая сзади высокая башня КВ-1.
                let k = 1.0 - t * 0.10;
                if pt.z <= rz * 0.20 {
                    pt.x.abs() <= rx * k * (1.0 - ((-pt.z - rz * 0.45).max(0.0) / rz) * 0.30)
                        && pt.z >= -rz * (1.0 - t * 0.08)
                } else {
                    let nx = pt.x / (rx * k);
                    let nz = (pt.z - rz * 0.20) / (rz * 0.85 * k);
                    nx * nx + nz * nz <= 1.0
                }
            }
            TankVisualModel::Is3 => {
                let base_pinch = if pt.y < self.p.voxel_size * 1.2 {
                    0.95
                } else {
                    1.0
                };
                let k = (1.0 - t.powf(1.9) * 0.42).max(0.46) * base_pinch;
                let nx = pt.x / (rx * k);
                let nz = (pt.z - 0.05) / (rz * k);
                nx * nx + nz * nz <= 1.0
            }
            TankVisualModel::Su100 => {
                let k = 1.0 - t * 0.22;
                let front_slope = -rz + t * (rz * 0.46);
                let right_blister = pt.x >= rx * 0.245
                    && pt.x <= rx * 0.96
                    && pt.z >= -rz * 0.36
                    && pt.z <= rz * 0.28
                    && t >= 0.45;
                right_blister || (pt.x.abs() <= rx * k && pt.z >= front_slope && pt.z <= rz)
            }
            TankVisualModel::Bt7 => {
                // Коническая башня БТ-7 с квадратной развитой кормовой радионишей.
                let k = 1.0 - t * 0.20;
                let cone = {
                    let nx = pt.x / (rx * k);
                    let nz = (pt.z + rz * 0.10) / (rz * 0.88 * k);
                    nx * nx + nz * nz <= 1.0
                };
                let radio_bustle = pt.z >= rz * 0.24
                    && pt.z <= rz * 1.12
                    && pt.x.abs() <= rx * 0.72 * k
                    && pt.y >= th * 0.14;
                cone || radio_bustle
            }
            TankVisualModel::Ms1 => {
                // Узкая гранёная клепаная башня МС-1.
                let k = 1.0 - t * 0.22;
                let nx = pt.x / (rx * k);
                let nz = (pt.z - 0.02) / (rz * k);
                (nx * nx + nz * nz <= 1.0) && (pt.x.abs() + (pt.z - 0.02).abs() * 0.55 <= rx * k * 1.25)
            }
            TankVisualModel::M2A4 => {
                // Высокая вертикальная 8-гранная клепаная башня M2A4 Light.
                let k = 1.0 - t * 0.08;
                pt.x.abs() <= rx * k
                    && pt.z.abs() <= rz * k
                    && (pt.x.abs() / rx + pt.z.abs() / rz) <= 1.48 * k
            }
            TankVisualModel::M3Stuart => {
                // Подковообразная закруглённая башня M3 Stuart D39273.
                let k = 1.0 - t * 0.10;
                if pt.z < 0.0 {
                    let nx = pt.x / (rx * k);
                    let nz = pt.z / (rz * k);
                    nx * nx + nz * nz <= 1.0
                } else {
                    let nx = pt.x / (rx * k * 0.94);
                    let nz = pt.z / (rz * k * 1.06);
                    nx * nx + nz * nz <= 1.0
                }
            }
            TankVisualModel::M24Chaffee => {
                // Обтекаемая низкая башня M24 Chaffee с вытянутым хвостом-противовесом.
                let k = 1.0 - t * 0.22;
                let nx = pt.x / (rx * k);
                let nz = (pt.z - 0.06) / (rz * k);
                let main = nx * nx + nz * nz <= 1.0;
                let bustle = pt.z >= rz * 0.30
                    && pt.z <= rz * 1.12
                    && pt.x.abs() <= rx * 0.68 * k
                    && pt.y >= th * 0.24;
                main || bustle
            }
            TankVisualModel::Auto => unreachable!("resolve_model eliminates Auto"),
        }
    }

    fn sample_turret(&self, pt: Vec3) -> Option<Material> {
        let vs = self.p.voxel_size;
        let rx = self.p.turret_radius_x;
        let rz = self.p.turret_radius_z;
        let th = self.p.turret_height;

        // 1. Утопленный погон башни (стыкует башню с крышей корпуса без просвета).
        if pt.y < 0.06 && pt.y >= -vs * 0.6 && self.p.turret_shape != TurretShape::FixedCasemate {
            let nx = pt.x / (rx * 0.90);
            let nz = (pt.z - 0.03) / (rz * 0.90);
            if nx * nx + nz * nz <= 1.0 {
                return Some(self.pal.dark);
            }
        }

        // 2. Навесные, бортовые (Schürzen / задний ДТ / кормовые ниши) и крышевые детали.
        if let Some(m) = self.turret_addons(pt, rx, rz, th, vs) {
            return Some(m);
        }

        if pt.y < 0.0 || pt.y > th {
            return None;
        }

        // 3. Основной броневой силуэт башни/рубки.
        if !self.turret_core_inside(pt, rx, rz, th) {
            return None;
        }

        // 4. Адаптивный опознавательный знак на скуле/борту башни (с учётом наклона борта!).
        let mark_z = match self.model {
            TankVisualModel::Su100 => rz * 0.16,
            TankVisualModel::M26Pershing | TankVisualModel::Tiger2 => -rz * 0.04,
            _ => rz * 0.02,
        };
        let mark_scale = (th / 0.72).clamp(0.55, 1.0);
        // Проверяем, что мы находимся на самом внешнем слое борта по горизонтали (±X):
        let outer_check = self.turret_core_inside(
            Vec3::new(pt.x + pt.x.signum() * vs * 1.15, pt.y, pt.z),
            rx,
            rz,
            th,
        );
        if pt.x.abs() > rx * 0.35
            && !outer_check
            && in_tactical_mark(
                self.team,
                (pt.z - mark_z) / mark_scale,
                (pt.y - th * 0.50) / mark_scale,
            )
        {
            return Some(self.pal.mark);
        }

        // Задняя дверь рубки СУ-100.
        if self.model == TankVisualModel::Su100
            && pt.z > rz - vs * 1.2
            && pt.x.abs() <= rx * 0.31
            && pt.y >= 0.08
            && pt.y <= th * 0.78
        {
            let border = pt.x.abs() > rx * 0.235 || pt.y > th * 0.70 || pt.y < 0.16;
            return Some(if border {
                self.pal.dark
            } else {
                self.pal.armor_light
            });
        }

        // Круглый кормовой эвакуационный люк у Panther и Tiger II!
        if matches!(self.model, TankVisualModel::Panther | TankVisualModel::Tiger2)
            && pt.z > rz * 0.68
            && !self.turret_core_inside(pt + Vec3::Z * vs * 1.2, rx, rz, th)
            && pt.x.hypot(pt.y - th * 0.46) <= (rx * 0.28).clamp(0.18, 0.28)
        {
            let r = pt.x.hypot(pt.y - th * 0.46);
            return Some(if r > rx * 0.21 {
                self.pal.dark
            } else {
                self.pal.armor_light
            });
        }

        let roof_rim = pt.y > th - vs * 0.8 && pt.x.abs() > rx * 0.50;
        Some(self.pal.hull_paint(pt, roof_rim))
    }

    fn turret_addons(
        &self,
        pt: Vec3,
        rx: f32,
        rz: f32,
        th: f32,
        vs: f32,
    ) -> Option<Material> {
        // 1. Уникальная П-образная экранировка башни (Turret Schürzen) у Pz. IV Ausf. H!
        if self.model == TankVisualModel::Pz4 {
            let skirt_x = rx + 0.19;
            let skirt_z = rz + 0.38;
            let in_y = pt.y >= 0.08 && pt.y <= th * 0.88;
            if in_y {
                // Боковые листы экрана и скошенные передние закрылки (капсула гарантирует 6-связность на диагонали).
                let on_side = pt.x.abs() >= skirt_x - vs * 0.60
                    && pt.x.abs() <= skirt_x + vs * 0.60
                    && pt.z >= -rz * 0.36
                    && pt.z <= skirt_z + vs * 0.5;
                let on_flap = pt.z >= -rz * 0.68
                    && pt.z <= -rz * 0.32
                    && dist_seg2(
                        Vec2::new(pt.x.abs(), pt.z),
                        Vec2::new(rx * 0.62, -rz * 0.64),
                        Vec2::new(skirt_x, -rz * 0.34),
                    ) <= vs * 0.90;
                // Задний дуговой экран позади Gepäckkasten.
                let on_rear = pt.z >= skirt_z - vs * 0.62
                    && pt.z <= skirt_z + vs * 0.62
                    && pt.x.abs() <= skirt_x + vs * 0.60;
                if on_side || on_flap || on_rear {
                    // Бортовой крест на Schürzen Pz. IV H!
                    if on_side
                        && in_tactical_mark(
                            self.team,
                            pt.z - (-rz * 0.04),
                            pt.y - th * 0.50,
                        )
                    {
                        return Some(self.pal.mark);
                    }
                    return Some(self.pal.hull_paint(pt, pt.y > th * 0.80));
                }
            }
            // Жёсткие боковые и кормовые кронштейны (от стенки башни прямо к экрану — 100% 6-связность!).
            if pt.y >= th * 0.40 && pt.y <= th * 0.58 {
                let side_strut = pt.x.abs() >= rx * 0.72
                    && pt.x.abs() <= skirt_x + vs * 0.4
                    && ((pt.z + rz * 0.20).abs() <= vs * 0.65
                        || (pt.z - rz * 0.30).abs() <= vs * 0.65
                        || (pt.z - rz * 0.82).abs() <= vs * 0.65);
                let rear_strut = pt.z >= rz * 0.72
                    && pt.z <= skirt_z + vs * 0.4
                    && (pt.x.abs() - rx * 0.48).abs() <= vs * 0.70;
                if side_strut || rear_strut {
                    return Some(self.pal.dark);
                }
            }
        }

        // 2. Выступающий назад башенный пулемёт ДТ в шаровой установке на корме КВ-1!
        if self.model == TankVisualModel::Kv1
            && pt.z >= rz * 0.86
            && pt.z <= rz + 0.16
            && pt.x.hypot(pt.y - th * 0.50) <= 0.11
        {
            if pt.z <= rz + 0.06 {
                return Some(self.pal.dark);
            }
            if pt.x.hypot(pt.y - th * 0.50) <= 0.055 {
                return Some(Material::TankSteel);
            }
        }

        // 3. Командирская башенка (нет у Т-34-76, уникальная высокая грибковая у МС-1, многогранная у M2A4).
        if self.model != TankVisualModel::T34_76 {
            let (cup_pos, cup_r, cup_h, has_slits) = match self.model {
                TankVisualModel::Su100 => (
                    Vec2::new(rx * 0.53, -rz * 0.05),
                    rx * 0.255,
                    th * 0.25,
                    true,
                ),
                TankVisualModel::Ms1 => (Vec2::new(0.0, 0.04), rx * 0.46, 0.20, false),
                TankVisualModel::M2A4 => (
                    Vec2::new(0.0, rz * 0.06),
                    rx * 0.44,
                    0.20,
                    true,
                ),
                TankVisualModel::M3Stuart => (
                    Vec2::new(-rx * 0.10, rz * 0.12),
                    rx * 0.36,
                    0.13,
                    true,
                ),
                TankVisualModel::Bt7 => (
                    Vec2::new(-rx * 0.28, rz * 0.08),
                    rx * 0.28,
                    0.07,
                    false,
                ),
                TankVisualModel::Pz2C => (
                    Vec2::new(-rx * 0.18, rz * 0.16),
                    rx * 0.32,
                    0.11,
                    false,
                ),
                TankVisualModel::Pz3 | TankVisualModel::Pz4 | TankVisualModel::Panther | TankVisualModel::Tiger1 | TankVisualModel::Tiger2 => (
                    Vec2::new(-rx * 0.35, rz * 0.20),
                    (rx * 0.29).clamp(0.18, 0.33),
                    (th * 0.32).clamp(0.17, 0.27),
                    true,
                ),
                TankVisualModel::M4A3E8 | TankVisualModel::M26Pershing | TankVisualModel::M24Chaffee => (
                    Vec2::new(-rx * 0.28, rz * 0.14),
                    (rx * 0.26).clamp(0.16, 0.26),
                    (th * 0.24).clamp(0.13, 0.19),
                    true,
                ),
                TankVisualModel::T34_85 => (
                    Vec2::new(-rx * 0.34, rz * 0.18),
                    (rx * 0.27).clamp(0.18, 0.27),
                    0.16,
                    true,
                ),
                TankVisualModel::Kv1 => (
                    Vec2::new(0.0, rz * 0.05),
                    rx * 0.24,
                    0.07,
                    false,
                ),
                TankVisualModel::Is3 => (
                    Vec2::new(-rx * 0.34, rz * 0.14),
                    (rx * 0.22).clamp(0.14, 0.24),
                    (th * 0.14).clamp(0.08, 0.12),
                    false,
                ),
                TankVisualModel::T34_76 | TankVisualModel::Auto => unreachable!(),
            };
            let d_cup = Vec2::new(pt.x - cup_pos.x, pt.z - cup_pos.y).length();
            let in_cup = if self.model == TankVisualModel::M2A4 {
                (pt.x - cup_pos.x).abs() <= cup_r
                    && (pt.z - cup_pos.y).abs() <= cup_r
                    && ((pt.x - cup_pos.x).abs() + (pt.z - cup_pos.y).abs()) <= cup_r * 1.44
            } else {
                d_cup <= cup_r
            };
            if pt.y >= th - 0.02 && pt.y <= th + cup_h && in_cup {
                if has_slits
                    && pt.y >= th + cup_h * 0.48
                    && pt.y <= th + cup_h * 0.80
                    && d_cup >= cup_r * 0.62
                {
                    return Some(Material::TankLens);
                }
                if pt.y > th + cup_h - vs * 0.9 {
                    return Some(self.pal.armor_light);
                }
                return Some(self.pal.dark);
            }
        }

        // 4. Знаменитые парные круглые люки («Микки Маус») на крыше Т-34-76!
        if self.model == TankVisualModel::T34_76
            && pt.y >= th - 0.02
            && pt.y <= th + 0.08
            && pt.z >= -rz * 0.18
            && pt.z <= rz * 0.38
        {
            for sx in [-rx * 0.30, rx * 0.30] {
                let d = (pt.x - sx).hypot(pt.z - rz * 0.10);
                if d <= rx * 0.28 {
                    return Some(if d > rx * 0.20 {
                        self.pal.dark
                    } else {
                        self.pal.armor_light
                    });
                }
            }
        }

        // 5. Люк заряжающего справа.
        if !matches!(
            self.model,
            TankVisualModel::T34_76 | TankVisualModel::Ms1 | TankVisualModel::Pz2C | TankVisualModel::M2A4 | TankVisualModel::M3Stuart
        ) {
            let load_x = if self.model == TankVisualModel::Su100 {
                -rx * 0.37
            } else {
                rx * 0.32
            };
            let load_r = (rx * 0.25).clamp(0.11, 0.22);
            let d_load = Vec2::new(pt.x - load_x, pt.z - rz * 0.14).length();
            if pt.y >= th - 0.02 && pt.y <= th + 0.08 && d_load <= load_r {
                return Some(if d_load > load_r * 0.70 {
                    self.pal.dark
                } else {
                    self.pal.armor_light
                });
            }
        }

        // 6. Два вентиляционных грибка на корме крыши Т-34-85.
        if self.model == TankVisualModel::T34_85 && pt.y >= th - 0.01 && pt.y <= th + 0.09 {
            let vent_r = (rx * 0.105).clamp(0.06, 0.10);
            for sx in [-rx * 0.27, rx * 0.27] {
                if (pt.x - sx).hypot(pt.z - rz * 0.56) <= vent_r {
                    return Some(self.pal.dark);
                }
            }
        }

        // 7. Кормовой ящик Rommelkiste / Gepäckkasten (ТОЛЬКО Pz. III, Pz. IV, Tiger I — НЕТ у Panther и Tiger II!).
        if matches!(
            self.model,
            TankVisualModel::Tiger1 | TankVisualModel::Pz4 | TankVisualModel::Pz3
        ) {
            let kiste_ext = (rz * 0.26).clamp(0.18, 0.34);
            if pt.y >= 0.12
                && pt.y <= th * 0.76
                && pt.x.abs() <= rx * 0.66
                && pt.z >= rz * 0.90
                && pt.z <= rz + kiste_ext
            {
                let strap = (pt.x.abs() - rx * 0.42).abs() < 0.05 || pt.y > th * 0.68;
                return Some(if strap {
                    self.pal.dark
                } else {
                    self.pal.hull_paint(pt, false)
                });
            }
        }

        // 8. Бортовые мортирки 3+3 на Tiger I и запасные траки на бортах Tiger I / Tiger II / Pz. IV / M4A3E8 / M26!
        if self.model == TankVisualModel::Tiger1 {
            let smoke_ext = (rx * 0.11).clamp(0.06, 0.12);
            if pt.y >= th * 0.45
                && pt.y <= th * 0.68
                && pt.x.abs() >= rx * 0.78
                && pt.x.abs() <= rx + smoke_ext
                && pt.z >= -rz * 0.54
                && pt.z <= -rz * 0.30
            {
                return Some(Material::TankSteel);
            }
        }
        if matches!(
            self.model,
            TankVisualModel::Tiger1
                | TankVisualModel::Tiger2
                | TankVisualModel::M4A3E8
                | TankVisualModel::M26Pershing
        ) && pt.y >= 0.16
            && pt.y <= th * 0.80
            && pt.z >= rz * 0.04
            && pt.z <= rz * 0.68
            && pt.x.abs() >= rx * 0.25
        {
            let inner = Vec3::new(pt.x - pt.x.signum() * vs * 1.02, pt.y, pt.z);
            if !self.turret_core_inside(pt, rx, rz, th) && self.turret_core_inside(inner, rx, rz, th)
            {
                let link = (pt.y / vs).floor() as i32;
                return Some(if link % 2 == 0 {
                    Material::TankTrack
                } else {
                    Material::TankTrackLight
                });
            }
        }

        // 9. Зенитный пулемёт на башнях США (M4A3E8, M26, M24, M3) и ДШК на ИС-3.
        if matches!(
            self.model,
            TankVisualModel::M4A3E8
                | TankVisualModel::M26Pershing
                | TankVisualModel::M24Chaffee
                | TankVisualModel::M3Stuart
                | TankVisualModel::Is3
        ) {
            let mount = Vec3::new(rx * 0.28, th, rz * 0.42);
            let mg_h = (th * 0.35).clamp(0.16, 0.24);
            let mg_len = (rz * 0.55).clamp(0.30, 0.62);
            let min_r = (vs * 0.74).max(0.06);
            let post = (pt.x - mount.x).hypot(pt.z - mount.z) <= min_r
                && pt.y >= th - vs * 1.6
                && pt.y <= th + mg_h;
            let body = (pt.x - mount.x).abs() <= 0.075
                && (pt.y - (th + mg_h)).abs() <= 0.065
                && pt.z >= mount.z - 0.14
                && pt.z <= mount.z + 0.16;
            let barrel = (pt.x - mount.x).hypot(pt.y - (th + mg_h)) <= min_r
                && pt.z >= mount.z - mg_len
                && pt.z <= mount.z - 0.04;
            if post || body || barrel {
                return Some(Material::TankSteel);
            }
        }

        // 10. Поручни на бортах башни Т-34-76, Т-34-85, ИС-3.
        if matches!(
            self.model,
            TankVisualModel::T34_76 | TankVisualModel::T34_85 | TankVisualModel::Is3
        ) && pt.x.abs() >= rx * 0.64
            && pt.x.abs() <= rx * 0.94
            && (pt.y - th * 0.50).abs() <= 0.045
            && pt.z >= rz * 0.16
            && pt.z <= rz * 0.56
        {
            return Some(Material::TankSteel);
        }

        // 11. База антенны на корме крыши.
        let ant = match self.model {
            TankVisualModel::Is3 => Vec2::new(-rx * 0.22, rz * 0.44),
            TankVisualModel::T34_85 | TankVisualModel::T34_76 => Vec2::new(-rx * 0.34, rz * 0.58),
            _ => Vec2::new(-rx * 0.44, rz * 0.62),
        };
        let ant_h = (th * 0.48).clamp(0.18, 0.32);
        if (pt.x - ant.x).hypot(pt.z - ant.y) <= (vs * 0.74).max(0.06)
            && pt.y >= th - vs * 0.8
            && pt.y <= th + ant_h
        {
            return Some(self.pal.dark);
        }

        None
    }

    fn sample_barrel(&self, pt: Vec3) -> Option<Material> {
        let br = self.p.barrel_radius;
        let bl = self.p.barrel_length;
        let gs = self.p.gun_voxel_size;
        let rx = self.p.turret_radius_x;
        let rz = self.p.turret_radius_z;
        let th = self.p.turret_height;
        let r_xy = pt.x.hypot(pt.y);

        // 1. Полый канал ствола у дульного среза (только 1x1 центр по X/Y).
        let bore_len = (bl * 0.06).clamp(0.10, 0.22);
        if pt.z <= -bl + bore_len
            && pt.z >= -bl - 0.02
            && pt.x.abs() < gs * 0.5
            && pt.y.abs() < gs * 0.5
        {
            return None;
        }

        // 2. Спаренный пулемёт MG34 справа от 20мм автопушки у Pz. II Ausf. C!
        if self.model == TankVisualModel::Pz2C {
            let coax_x = 0.16_f32;
            let coax_r = (gs * 1.15).max(0.046);
            if pt.z <= 0.06
                && pt.z >= -bl * 0.46
                && (pt.x - coax_x).hypot(pt.y) <= coax_r
            {
                return Some(Material::TankSteel);
            }
        }

        // 3. Дульный тормоз по историческому типу (щелевой американский/советский или двухкамерный немецкий).
        if self.p.has_muzzle_brake() {
            match self.model {
                TankVisualModel::M4A3E8 => {
                    // Однокамерный цилиндрический перфорированный тормоз 76мм M1A2.
                    let brake_len = (bl * 0.065).clamp(0.22, 0.30);
                    if pt.z >= -bl && pt.z <= -bl + brake_len {
                        let slot = pt.z >= -bl + brake_len * 0.28 && pt.z <= -bl + brake_len * 0.72;
                        if slot && pt.x.abs() >= br * 0.75 && pt.y.abs() <= br * 0.65 {
                            return None;
                        }
                        if r_xy <= br * 1.65 {
                            return Some(if slot { Material::TankSteel } else { self.pal.dark });
                        }
                    }
                }
                _ => {
                    // Двухкамерный плоский/грушевидный тормоз (Panther, Tiger I/II, Pz. IV H, M26, ИС-3).
                    let brake_len = (bl * 0.082).clamp(0.26, 0.42);
                    if pt.z >= -bl && pt.z <= -bl + brake_len {
                        let rx_b = if self.model == TankVisualModel::Panther {
                            br * 1.85
                        } else {
                            br * 1.95
                        };
                        let ry_b = br * 1.42;
                        let in_slot_1 =
                            pt.z >= -bl + brake_len * 0.20 && pt.z <= -bl + brake_len * 0.425;
                        let in_slot_2 =
                            pt.z >= -bl + brake_len * 0.55 && pt.z <= -bl + brake_len * 0.775;
                        if (in_slot_1 || in_slot_2)
                            && pt.x.abs() >= br * 0.85
                            && pt.y.abs() <= br * 0.72
                        {
                            return None;
                        }
                        if pt.x.abs() <= rx_b && pt.y.abs() <= ry_b {
                            let chamfer = (pt.x.abs() / rx_b).powi(2) + (pt.y.abs() / ry_b).powi(2);
                            if chamfer <= 1.42 {
                                return Some(if in_slot_1 || in_slot_2 {
                                    Material::TankSteel
                                } else {
                                    self.pal.dark
                                });
                            }
                        }
                    }
                }
            }
        } else {
            let ring_len = (bl * 0.04).clamp(0.08, 0.16);
            if pt.z >= -bl && pt.z <= -bl + ring_len && r_xy <= (br * 1.18).max(gs * 2.2) {
                return Some(Material::TankSteel);
            }
        }

        // 4. Откатный кожух / броневое корыто отката (Т-34-76, КВ-1, M24 Chaffee) или казённая муфта.
        if matches!(self.model, TankVisualModel::T34_76 | TankVisualModel::Kv1) {
            // Знаменитый нижний броневой жёлоб-корыто отката под стволом 76мм Ф-34 / ЗИС-5!
            if pt.z >= -0.66
                && pt.z <= 0.12
                && pt.x.abs() <= br * 1.45
                && pt.y >= -br * 2.15
                && pt.y <= br * 1.15
            {
                return Some(self.pal.hull_paint(pt, false));
            }
        } else if self.model == TankVisualModel::M24Chaffee
            && pt.z >= -0.48
            && pt.z <= 0.10
            && pt.x.abs() <= br * 1.10
            && pt.y >= -br * 1.95
            && pt.y <= 0.0
        {
            return Some(self.pal.dark);
        }

        let sleeve_len = match self.model {
            TankVisualModel::Tiger1 | TankVisualModel::Tiger2 | TankVisualModel::Panther => {
                (bl * 0.14).clamp(0.24, 0.62)
            }
            TankVisualModel::Pz2C | TankVisualModel::M2A4 | TankVisualModel::M3Stuart | TankVisualModel::Ms1 => {
                (bl * 0.22).clamp(0.20, 0.45)
            }
            _ => (bl * 0.17).clamp(0.22, 0.68),
        };
        if pt.z >= -sleeve_len && pt.z <= 0.0 && r_xy <= br * 1.45 {
            return Some(self.pal.dark);
        }

        // 5. Маска орудия по историческому архетипу!
        let mantlet = match self.model {
            TankVisualModel::Panther => {
                // Цилиндрическая «катушка» Topfblende поперёк лба башни Panther!
                let rad = (th * 0.255).clamp(0.16, 0.22);
                pt.x.abs() <= rx * 0.46 && pt.y.hypot(pt.z + 0.02) <= rad
            }
            TankVisualModel::Tiger2 => {
                // Узкая литая коническая маска «Saukopf» («свиное рыло») Tiger II!
                if pt.z >= -0.44 && pt.z <= 0.22 {
                    let u = ((0.22 - pt.z) / 0.66).clamp(0.0, 1.0);
                    let r_max = (th * 0.25).clamp(0.18, 0.23) * (1.0 - 0.42 * u);
                    pt.x.abs() <= r_max * 1.35 && pt.y.abs() <= r_max && pt.x.hypot(pt.y * 1.15) <= r_max * 1.38
                } else {
                    false
                }
            }
            TankVisualModel::Tiger1 => {
                // Широкая толстая ступенчатая маска Tiger I с прицелом и пулемётом.
                let mx = (rx * 0.64).clamp(0.45, 0.74);
                let my = (th * 0.26).clamp(0.15, 0.22);
                let mz_f = -0.24;
                let mz_b = 0.22;
                if pt.z >= mz_f && pt.z <= mz_b && pt.x.abs() <= mx && pt.y.abs() <= my {
                    let s = mx / 0.72;
                    if pt.z < mz_f * 0.54
                        && ((pt.x + 0.32 * s).abs() <= 0.07 * s || (pt.x + 0.44 * s).abs() <= 0.05 * s)
                        && pt.y.abs() <= 0.05
                    {
                        return Some(Material::TankLens);
                    }
                    if pt.z < mz_f * 0.38 && (pt.x - 0.30 * s).hypot(pt.y) <= 0.05 {
                        return Some(Material::TankSteel);
                    }
                    return Some(self.pal.hull_paint(pt, pt.y > my * 0.64));
                }
                false
            }
            TankVisualModel::Pz4 => {
                // Угловатая двойная маска KwK 40 (плоская основа + выступающий вперёд литой конус).
                let plate = pt.x.abs() <= rx * 0.44
                    && pt.y.abs() <= th * 0.24
                    && pt.z >= -0.18
                    && pt.z <= 0.18;
                let collar = pt.z >= -0.38
                    && pt.z < -0.05
                    && pt.x.abs() <= 0.18
                    && pt.y.abs() <= 0.14;
                plate || collar
            }
            TankVisualModel::Pz3 => {
                // Внутренняя маска + разнесённая наружная накладка Vorpanzer.
                let inner = pt.x.abs() <= rx * 0.42
                    && pt.y.abs() <= th * 0.24
                    && pt.z >= -0.16
                    && pt.z <= 0.18;
                let spaced = pt.x.abs() <= rx * 0.36
                    && pt.y.abs() <= th * 0.18
                    && pt.z >= -0.26
                    && pt.z <= -0.14;
                inner || spaced
            }
            TankVisualModel::Pz2C => {
                // Сдвинутый щит Pz. II C (держит и 20мм ствол, и спаренный MG34).
                pt.x >= -0.20 && pt.x <= 0.26 && pt.y.abs() <= 0.13 && pt.z >= -0.14 && pt.z <= 0.18
            }
            TankVisualModel::M4A3E8 | TankVisualModel::M26Pershing => {
                // Выгнутый широкий щит-ротор M62 / M67!
                let mx = if self.model == TankVisualModel::M26Pershing {
                    rx * 0.52
                } else {
                    rx * 0.48
                };
                let my = (th * 0.28).clamp(0.16, 0.22);
                let curve = -0.24 + (pt.y / my).powi(2) * 0.10;
                pt.x.abs() <= mx && pt.y.abs() <= my && pt.z >= curve && pt.z <= 0.22
            }
            TankVisualModel::Su100 => {
                let ex = (rx * 0.353).clamp(0.22, 0.38);
                let ey = (th * 0.36).clamp(0.16, 0.28);
                let ez = (rz * 0.288).clamp(0.20, 0.36);
                (pt.x / ex).powi(2) + (pt.y / ey).powi(2) + ((pt.z + 0.02) / ez).powi(2) <= 1.0
            }
            TankVisualModel::T34_85 | TankVisualModel::T34_76 | TankVisualModel::Kv1 | TankVisualModel::Bt7 => {
                let ex = (rx * 0.35).clamp(0.20, 0.34);
                let ey = (th * 0.30).clamp(0.15, 0.23);
                let ez = (rz * 0.26).clamp(0.18, 0.30);
                (pt.x / ex).powi(2) + (pt.y / ey).powi(2) + ((pt.z + 0.04) / ez).powi(2) <= 1.0
            }
            TankVisualModel::Is3 => {
                let mx = (rx * 0.287).clamp(0.20, 0.32);
                let my = (th * 0.328).clamp(0.15, 0.22);
                pt.z >= -0.30 && pt.z <= 0.24 && pt.x.abs() <= mx && pt.y.abs() <= my
            }
            TankVisualModel::M2A4 | TankVisualModel::M3Stuart | TankVisualModel::M24Chaffee | TankVisualModel::Ms1 | TankVisualModel::Auto => {
                let mx = (rx * 0.34).clamp(0.16, 0.28);
                let my = (th * 0.29).clamp(0.12, 0.19);
                pt.z >= -0.20 && pt.z <= 0.20 && pt.x.abs() <= mx && pt.y.abs() <= my
            }
        };
        if mantlet {
            return Some(self.pal.hull_paint(pt, false));
        }

        // 6. Основной конически сужающийся ствол орудия.
        if pt.z <= 0.08 && pt.z >= -bl {
            let frac = (-pt.z / bl.max(0.5)).clamp(0.0, 1.0);
            let r_taper = (br * (1.14 - 0.20 * frac)).max(gs * 2.15);
            if r_xy <= r_taper {
                if (pt.z + bl * 0.45).abs() <= 0.05 && r_xy >= r_taper * 0.85 {
                    return Some(self.pal.dark);
                }
                return Some(Material::TankSteel);
            }
        }

        None
    }
}

fn dist_seg2(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let denom = ab.length_squared();
    if denom <= 1e-6 {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / denom).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

fn in_tactical_mark(team: Team, dz: f32, dy: f32) -> bool {
    match team {
        Team::Player => {
            let r = dz.hypot(dy);
            let r_outer = 0.21;
            if r > r_outer {
                return false;
            }
            let angle = dz.atan2(dy);
            let sector = TAU / 5.0;
            let a = (angle.rem_euclid(sector) - sector * 0.5).abs();
            let t = a / (sector * 0.5);
            let limit = r_outer * (1.0 - t) + (r_outer * 0.42) * t;
            r <= limit
        }
        Team::Enemy => {
            (dz.abs() <= 0.055 && dy.abs() <= 0.17) || (dy.abs() <= 0.055 && dz.abs() <= 0.17)
        }
    }
}

fn hash3(x: i32, y: i32, z: i32) -> f32 {
    let mut n = (x as u32).wrapping_mul(374_761_393)
        ^ (y as u32).wrapping_mul(668_265_263)
        ^ (z as u32).wrapping_mul(2_147_483_647);
    n = (n ^ (n >> 13)).wrapping_mul(1_274_126_177);
    ((n ^ (n >> 16)) & 0x00ff_ffff) as f32 / 16_777_216.0
}

fn camo_noise_3d(p: Vec3) -> f32 {
    let f = p.floor();
    let t = p - f;
    let u = t * t * (Vec3::splat(3.0) - t * 2.0);
    let b = f.as_ivec3();
    let lerp = |a: f32, b: f32, s: f32| a + (b - a) * s;
    let x00 = lerp(hash3(b.x, b.y, b.z), hash3(b.x + 1, b.y, b.z), u.x);
    let x10 = lerp(hash3(b.x, b.y + 1, b.z), hash3(b.x + 1, b.y + 1, b.z), u.x);
    let x01 = lerp(hash3(b.x, b.y, b.z + 1), hash3(b.x + 1, b.y, b.z + 1), u.x);
    let x11 = lerp(
        hash3(b.x, b.y + 1, b.z + 1),
        hash3(b.x + 1, b.y + 1, b.z + 1),
        u.x,
    );
    let y0 = lerp(x00, x10, u.y);
    let y1 = lerp(x01, x11, u.y);
    lerp(y0, y1, u.z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tank::spec::{T34_85_RON, TankSpec, available_tanks, load_tank_spec};
    use crate::voxel::mesher::mesh_grid_ao;

    fn has_material(grid: &VoxelGrid, mat: Material) -> bool {
        for y in 0..grid.dims.y as i32 {
            for z in 0..grid.dims.z as i32 {
                for x in 0..grid.dims.x as i32 {
                    if grid.get(IVec3::new(x, y, z)) == mat {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn grid_z_bounds_m(grid: &VoxelGrid) -> (f32, f32) {
        let mut min_z = f32::MAX;
        let mut max_z = f32::MIN;
        for y in 0..grid.dims.y as i32 {
            for z in 0..grid.dims.z as i32 {
                for x in 0..grid.dims.x as i32 {
                    let v = IVec3::new(x, y, z);
                    if grid.is_solid(v) {
                        let p = grid.voxel_center(v);
                        min_z = min_z.min(p.z);
                        max_z = max_z.max(p.z);
                    }
                }
            }
        }
        (min_z, max_z)
    }

    fn grid_solid_extent_m(grid: &VoxelGrid) -> Vec3 {
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        for y in 0..grid.dims.y as i32 {
            for z in 0..grid.dims.z as i32 {
                for x in 0..grid.dims.x as i32 {
                    let v = IVec3::new(x, y, z);
                    if grid.is_solid(v) {
                        let p = grid.voxel_center(v);
                        min = min.min(p);
                        max = max.max(p);
                    }
                }
            }
        }
        max - min
    }

    #[test]
    fn t34_85_ron_parses_and_builds_detailed_ao_meshes() {
        let spec = TankSpec::from_ron(T34_85_RON).expect("t34_85.ron должен парситься");
        assert_eq!(spec.name, "Т-34-85");
        let parts = build_tank_parts(&spec.profile);
        assert!(parts.hull.count_solid() > 4_000, "hull={}", parts.hull.count_solid());
        assert!(parts.turret.count_solid() > 800, "turret={}", parts.turret.count_solid());
        assert!(parts.barrel.count_solid() > 400, "barrel={}", parts.barrel.count_solid());

        assert!(has_material(&parts.hull, Material::TankRubber));
        assert!(has_material(&parts.hull, Material::TankWheel));
        assert!(has_material(&parts.hull, Material::TankLens));
        assert!(has_material(&parts.hull, Material::TankExhaust));
        assert!(has_material(&parts.turret, Material::TankStar));

        let hm = mesh_grid_ao(&parts.hull);
        let tm = mesh_grid_ao(&parts.turret);
        let bm = mesh_grid_ao(&parts.barrel);
        assert!(!hm.is_empty() && !tm.is_empty() && !bm.is_empty());
        let total_quads = hm.quad_count() + tm.quad_count() + bm.quad_count();
        assert!(
            total_quads < 35_000,
            "слишком тяжёлый AO-меш танка: {total_quads} квадов"
        );

        let sz = parts.hull.size_m();
        assert!((sz.x - spec.profile.hull_width).abs() < 0.4, "sz={sz:?}");
        assert!((sz.z - spec.profile.hull_length).abs() < 0.6, "sz={sz:?}");
    }

    #[test]
    fn all_fifteen_tanks_build_distinct_detailed_models_and_enemy_camo() {
        let tanks = available_tanks();
        assert_eq!(tanks.len(), 18);
        let mut counts = Vec::new();
        for t in &tanks {
            let parts = build_tank_parts_for_team(&t.profile, Team::Player);
            let h = parts.hull.count_solid();
            let tr = parts.turret.count_solid();
            let b = parts.barrel.count_solid();
            assert!(h > 1_000 && tr > 220 && b > 140, "{}: {h}/{tr}/{b}", t.id);

            let hm = mesh_grid_ao(&parts.hull);
            let tm = mesh_grid_ao(&parts.turret);
            let bm = mesh_grid_ao(&parts.barrel);
            let total_quads = hm.quad_count() + tm.quad_count() + bm.quad_count();
            assert!(total_quads < 45_000, "{}: quads={total_quads}", t.id);

            let tip = Vec3::new(0.0, 0.0, -t.profile.barrel_length + 0.04);
            let tip_v = parts.barrel.world_to_voxel(tip);
            assert!(!parts.barrel.is_solid(tip_v), "{} должен иметь полый канал у среза", t.id);

            let enemy = build_tank_parts_for_team(&t.profile, Team::Enemy);
            assert!(has_material(&enemy.hull, Material::EnemyArmor), "{}", t.id);
            assert!(has_material(&enemy.hull, Material::EnemyCamo), "{}", t.id);
            assert!(has_material(&enemy.turret, Material::EnemyMark), "{}", t.id);

            counts.push((t.id.clone(), h, tr, b));
        }

        let ms1 = counts.iter().find(|c| c.0 == "ms_1").unwrap();
        let m24 = counts.iter().find(|c| c.0 == "m24_chaffee").unwrap();
        let tiger = counts.iter().find(|c| c.0 == "tiger_1").unwrap();
        assert!(tiger.1 > m24.1 && tiger.3 > m24.3);
        assert!(tiger.1 > ms1.1 && tiger.2 > ms1.2 && tiger.3 > ms1.3);

        let tiger_parts = build_tank_parts(&load_tank_spec("tiger_1").unwrap().profile);
        let t34_parts = build_tank_parts(&load_tank_spec("t34_85").unwrap().profile);
        let brake_x = |grid: &VoxelGrid, bl: f32| {
            let mut max_x = 0.0f32;
            for y in 0..grid.dims.y as i32 {
                for z in 0..grid.dims.z as i32 {
                    for x in 0..grid.dims.x as i32 {
                        let v = IVec3::new(x, y, z);
                        if grid.is_solid(v) {
                            let p = grid.voxel_center(v);
                            if p.z <= -bl + 0.30 {
                                max_x = max_x.max(p.x.abs());
                            }
                        }
                    }
                }
            }
            max_x
        };
        assert!(
            brake_x(&tiger_parts.barrel, 4.8) > brake_x(&t34_parts.barrel, 4.0) + 0.06,
            "у Tiger I должен быть широкий дульный тормоз"
        );
    }

    #[test]
    fn sherman_pershing_panther_t34_and_asymmetric_turrets_are_distinct() {
        let sherman = load_tank_spec("m4a3e8").unwrap();
        let pershing = load_tank_spec("m26_pershing").unwrap();
        let panther = load_tank_spec("panther").unwrap();
        let t34_76 = load_tank_spec("t34_76").unwrap();
        let t34_85 = load_tank_spec("t34_85").unwrap();
        let ms1 = load_tank_spec("ms_1").unwrap();
        let pz2 = load_tank_spec("pz_2_c").unwrap();
        let pz4 = load_tank_spec("pz_4").unwrap();
        let kv1 = load_tank_spec("kv_1").unwrap();

        // Разнообразное историческое расположение башен:
        assert!(ms1.profile.turret_pivot[2] > 0.10, "башня МС-1 смещена назад");
        assert!(pz2.profile.turret_pivot[0] < -0.05, "башня Pz. II C смещена влево");
        assert!(t34_76.profile.turret_pivot[2] < -0.70, "башня Т-34-76 вынесена далеко вперёд");
        assert!(panther.profile.turret_pivot[2] > t34_76.profile.turret_pivot[2] + 0.30);

        let sh_p = build_tank_parts(&sherman.profile);
        let pe_p = build_tank_parts(&pershing.profile);
        let pa_p = build_tank_parts(&panther.profile);
        let t76_p = build_tank_parts(&t34_76.profile);
        let t85_p = build_tank_parts(&t34_85.profile);
        let ms1_p = build_tank_parts(&ms1.profile);
        let pz4_p = build_tank_parts(&pz4.profile);
        let kv1_p = build_tank_parts(&kv1.profile);

        // Высокий Шерман vs широкий низкий Першинг vs длинная Пантера.
        let sh_ext = grid_solid_extent_m(&sh_p.hull);
        let pe_ext = grid_solid_extent_m(&pe_p.hull);
        let pa_ext = grid_solid_extent_m(&pa_p.hull);
        assert!(sh_ext.y > pe_ext.y + 0.20, "M4A3E8 выше M26: {} vs {}", sh_ext.y, pe_ext.y);
        assert!(pe_ext.x > sh_ext.x + 0.30, "M26 шире M4A3E8: {} vs {}", pe_ext.x, sh_ext.x);
        assert!(pa_ext.z > t85_p.hull.size_m().z + 0.45, "Пантера длиннее Т-34");

        // Башня Т-34-85 существенно массивнее башни Т-34-76; кормовая ниша Першинга длиннее Шермана.
        assert!(t85_p.turret.count_solid() > t76_p.turret.count_solid() * 2);
        let pe_tz = grid_z_bounds_m(&pe_p.turret);
        let sh_tz = grid_z_bounds_m(&sh_p.turret);
        assert!(pe_tz.1 > sh_tz.1 + 0.20, "кормовая ниша M26 длиннее M4A3E8");

        // Хвост МС-1 выступает далеко за корпус, экраны башни Pz. IV шире самой базы, цельностальные катки КВ-1.
        let ms1_hz = grid_z_bounds_m(&ms1_p.hull);
        assert!(ms1_hz.1 > ms1.profile.hull_length * 0.5 + 0.30, "хвост МС-1 должен выступать назад");
        assert!(grid_solid_extent_m(&pz4_p.turret).x > pz4.profile.turret_radius_x * 2.0 + 0.20);
        assert!(!has_material(&kv1_p.hull, Material::TankRubber), "КВ-1 имеет цельностальные катки");
    }

    fn connected_components(grid: &VoxelGrid) -> Vec<Vec<IVec3>> {
        let d = grid.dims.as_ivec3();
        let total = (d.x * d.y * d.z) as usize;
        let idx = |v: IVec3| ((v.y * d.z + v.z) * d.x + v.x) as usize;
        let mut seen = vec![false; total];
        let mut comps = Vec::new();
        let dirs = [
            IVec3::X,
            IVec3::NEG_X,
            IVec3::Y,
            IVec3::NEG_Y,
            IVec3::Z,
            IVec3::NEG_Z,
        ];
        for y in 0..d.y {
            for z in 0..d.z {
                for x in 0..d.x {
                    let start = IVec3::new(x, y, z);
                    if !grid.is_solid(start) || seen[idx(start)] {
                        continue;
                    }
                    let mut stack = vec![start];
                    seen[idx(start)] = true;
                    let mut comp = Vec::new();
                    while let Some(v) = stack.pop() {
                        comp.push(v);
                        for dir in dirs {
                            let n = v + dir;
                            if n.x >= 0
                                && n.y >= 0
                                && n.z >= 0
                                && n.x < d.x
                                && n.y < d.y
                                && n.z < d.z
                                && grid.is_solid(n)
                                && !seen[idx(n)]
                            {
                                seen[idx(n)] = true;
                                stack.push(n);
                            }
                        }
                    }
                    comps.push(comp);
                }
            }
        }
        comps
    }

    #[test]
    fn all_fifteen_tanks_have_no_floating_disconnected_voxels() {
        let mut issues = Vec::new();
        for t in available_tanks() {
            let parts = build_tank_parts_for_team(&t.profile, Team::Player);
            for (part_name, grid) in [
                ("hull", &parts.hull),
                ("turret", &parts.turret),
                ("barrel", &parts.barrel),
            ] {
                let mut comps = connected_components(grid);
                if comps.len() != 1 {
                    comps.sort_by_key(|c| std::cmp::Reverse(c.len()));
                    let sizes: Vec<usize> = comps.iter().map(|c| c.len()).collect();
                    let sample_pts: Vec<Vec3> = comps
                        .iter()
                        .skip(1)
                        .take(4)
                        .filter_map(|c| c.first().map(|&v| grid.voxel_center(v)))
                        .collect();
                    issues.push(format!(
                        "{} {part_name}: ожидается 1 связная компонента, найдено {} (размеры: {:?}, примеры центров: {:?})",
                        t.id,
                        comps.len(),
                        sizes,
                        sample_pts
                    ));
                }
            }
        }
        assert!(issues.is_empty(), "Обнаружены висячие воксели:\n{}", issues.join("\n"));
    }
}
