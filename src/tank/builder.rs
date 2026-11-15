//! Процедурный конструктор детализированных воксельных сеток танка из [`ProceduralTankProfile`].

use std::f32::consts::TAU;

use bevy::math::{IVec3, UVec3, Vec2, Vec3};

use super::spec::{HullShape, ProceduralTankProfile, TurretShape};
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

/// Строит воксельные части танка по профилю (палитра игрока по умолчанию).
pub fn build_tank_parts(p: &ProceduralTankProfile) -> TankVoxelParts {
    build_tank_parts_for_team(p, Team::Player)
}

/// Строит детализированные воксельные части танка с учётом команды, ходовой и силуэта.
pub fn build_tank_parts_for_team(p: &ProceduralTankProfile, team: Team) -> TankVoxelParts {
    let ctx = TankBuildCtx::new(p, team);

    let half_w = ctx.hw + p.voxel_size * 1.2;
    let min_z = -ctx.hl - 0.16;
    let max_z = ctx.hl + 0.30;
    let max_y = p.deck_height + 0.36;
    let hull = build_symmetric_grid(
        half_w,
        (0.0, max_y),
        (min_z, max_z),
        p.voxel_size,
        false,
        |pt| ctx.sample_hull(pt),
    );

    let t_half_x = p.turret_radius_x + 0.28;
    let turret = build_symmetric_grid(
        t_half_x,
        (-p.voxel_size * 0.5, p.turret_height + 0.48),
        (-p.turret_radius_z - 0.24, p.turret_radius_z + 0.48),
        p.voxel_size,
        false,
        |pt| ctx.sample_turret(pt),
    );

    let b_half_x = if p.turret_shape == TurretShape::BoxHeavy {
        0.78
    } else {
        0.46
    };
    let barrel = build_symmetric_grid(
        b_half_x,
        (-0.30, 0.30),
        (-p.barrel_length - 0.04, 0.36),
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
}

#[derive(Clone, Copy)]
struct ReturnRoller {
    z: f32,
    y: f32,
    r: f32,
}

struct GearSpec {
    road_wheels: Vec<RoadWheel>,
    rollers: Vec<ReturnRoller>,
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
}

struct TankBuildCtx<'a> {
    p: &'a ProceduralTankProfile,
    team: Team,
    pal: TankPalette,
    hl: f32,
    hw: f32,
    inner_w: f32,
    fender_y0: f32,
    fender_y1: f32,
    gear: GearSpec,
}

impl<'a> TankBuildCtx<'a> {
    fn new(p: &'a ProceduralTankProfile, team: Team) -> Self {
        let vs = p.voxel_size;
        let hl = p.hull_length * 0.5;
        let hw = p.hull_width * 0.5;
        let inner_w = hw - p.track_width;

        // Верхняя ветвь гусеницы занимает верхний слой в пределах `track_height`,
        // а надгусеничная полка ложится строго на следующий слой вокселей.
        let top_row = ((p.track_height / vs).round() as i32 - 1).max(4);
        let y_track_top = (top_row as f32 + 0.5) * vs;
        let y_track_bot = 0.5 * vs;
        let fender_y0 = (top_row as f32 + 1.0) * vs - 0.005;
        let fender_y1 = (top_row as f32 + 2.0) * vs - 0.005;

        let belt_h = y_track_top - y_track_bot;
        let z_front = -(hl - 0.16);
        let z_rear = hl - 0.18;

        let gear = match p.hull_shape {
            HullShape::SlopedT34 | HullShape::Casemate => {
                // 5 крупных катков Кристи без поддерживающих роликов; ведущее сзади.
                let r_road = (belt_h - vs * 1.1) * 0.49;
                let y_w = y_track_bot + vs * 0.5 + r_road;
                let zs = [-2.02, -1.03, -0.04, 0.96, 1.98];
                let road_wheels = zs
                    .into_iter()
                    .map(|z| RoadWheel {
                        z,
                        y: y_w,
                        r: r_road,
                        x_min: inner_w + 0.04,
                        x_max: hw - 0.02,
                        spoked: true,
                    })
                    .collect();
                let r_front = 0.24;
                let r_rear = 0.26;
                GearSpec {
                    road_wheels,
                    rollers: Vec::new(),
                    z_front,
                    y_front: y_track_top - r_front - vs * 0.3,
                    r_front,
                    front_drive: false,
                    z_rear,
                    y_rear: y_track_top - r_rear - vs * 0.3,
                    r_rear,
                    y_track_bot,
                    y_track_top,
                    z_bot_start: -2.08,
                    z_bot_end: 2.04,
                }
            }
            HullShape::LightBox => {
                // M24 Chaffee: 5 компактных катков + 3 поддерживающих ролика; ведущее спереди.
                let r_road = 0.22;
                let y_w = y_track_bot + vs * 0.5 + r_road;
                let zs = [-1.62, -0.82, -0.02, 0.78, 1.58];
                let road_wheels = zs
                    .into_iter()
                    .map(|z| RoadWheel {
                        z,
                        y: y_w,
                        r: r_road,
                        x_min: inner_w + 0.04,
                        x_max: hw - 0.02,
                        spoked: false,
                    })
                    .collect();
                let r_ret = 0.10;
                let y_ret = y_track_top - vs * 0.45 - r_ret;
                let rollers = [-1.05, 0.0, 1.05]
                    .into_iter()
                    .map(|z| ReturnRoller { z, y: y_ret, r: r_ret })
                    .collect();
                let r_front = 0.23;
                let r_rear = 0.21;
                GearSpec {
                    road_wheels,
                    rollers,
                    z_front,
                    y_front: y_track_top - r_front - vs * 0.3,
                    r_front,
                    front_drive: true,
                    z_rear,
                    y_rear: y_track_top - r_rear - vs * 0.3,
                    r_rear,
                    y_track_bot,
                    y_track_top,
                    z_bot_start: -1.68,
                    z_bot_end: 1.64,
                }
            }
            HullShape::HeavyBox => {
                // Tiger I: 8 шахматных катков (4 внешних + 4 внутренних); ведущее спереди.
                let r_road = 0.33;
                let y_w = y_track_bot + vs * 0.5 + r_road;
                let mut road_wheels = Vec::with_capacity(8);
                for z in [-1.72, -0.58, 0.56, 1.70] {
                    road_wheels.push(RoadWheel {
                        z,
                        y: y_w,
                        r: r_road,
                        x_min: hw - 0.24,
                        x_max: hw - 0.02,
                        spoked: false,
                    });
                }
                for z in [-1.15, -0.01, 1.13, 2.15] {
                    road_wheels.push(RoadWheel {
                        z,
                        y: y_w,
                        r: r_road,
                        x_min: inner_w + 0.04,
                        x_max: hw - 0.28,
                        spoked: false,
                    });
                }
                let r_front = 0.27;
                let r_rear = 0.24;
                GearSpec {
                    road_wheels,
                    rollers: Vec::new(),
                    z_front: -(hl - 0.20),
                    y_front: y_track_top - r_front - vs * 0.3,
                    r_front,
                    front_drive: true,
                    z_rear: hl - 0.22,
                    y_rear: y_track_top - r_rear - vs * 0.3,
                    r_rear,
                    y_track_bot,
                    y_track_top,
                    z_bot_start: -1.80,
                    z_bot_end: 2.18,
                }
            }
            HullShape::PikeNose => {
                // ИС-3: 6 тяжёлых катков + 3 поддерживающих ролика; ведущее сзади.
                let r_road = 0.24;
                let y_w = y_track_bot + vs * 0.5 + r_road;
                let road_wheels = (0..6)
                    .map(|i| {
                        let z = -2.10 + i as f32 * 0.84;
                        RoadWheel {
                            z,
                            y: y_w,
                            r: r_road,
                            x_min: inner_w + 0.04,
                            x_max: hw - 0.02,
                            spoked: false,
                        }
                    })
                    .collect();
                let r_ret = 0.105;
                let y_ret = y_track_top - vs * 0.45 - r_ret;
                let rollers = [-1.35, 0.0, 1.35]
                    .into_iter()
                    .map(|z| ReturnRoller { z, y: y_ret, r: r_ret })
                    .collect();
                let r_front = 0.24;
                let r_rear = 0.26;
                GearSpec {
                    road_wheels,
                    rollers,
                    z_front: -(hl - 0.18),
                    y_front: y_track_top - r_front - vs * 0.3,
                    r_front,
                    front_drive: false,
                    z_rear: hl - 0.20,
                    y_rear: y_track_top - r_rear - vs * 0.3,
                    r_rear,
                    y_track_bot,
                    y_track_top,
                    z_bot_start: -2.16,
                    z_bot_end: 2.16,
                }
            }
        };

        Self {
            p,
            team,
            pal: TankPalette::for_team(team),
            hl,
            hw,
            inner_w,
            fender_y0,
            fender_y1,
            gear,
        }
    }

    fn sample_hull(&self, pt: Vec3) -> Option<Material> {
        let ax = pt.x.abs();

        // 1. Внешнее навесное оборудование и детали корпуса (поверх/снаружи основного блока).
        if let Some(m) = self.hull_addons(pt, ax) {
            return Some(m);
        }

        // 2. Ходовая часть: гусеницы, катки, ведущие звёздочки, ленивцы и балансиры.
        if let Some(m) = self.sample_gear(pt, ax) {
            return Some(m);
        }

        // 3. Надгусеничные полки и брызговики.
        if let Some(m) = self.sample_fenders(pt, ax) {
            return Some(m);
        }

        // 4. Основной бронекорпус (ванна + надгусеничные скосы + ВЛД/НЛД + крыша МТО).
        self.sample_hull_armor(pt, ax)
    }

    fn sample_gear(&self, pt: Vec3, ax: f32) -> Option<Material> {
        let g = &self.gear;
        let vs = self.p.voxel_size;
        let track_in = self.inner_w + 0.02;
        let track_out = self.hw;

        // Оси балансиров: соединяют корпус с центрами катков (устраняет эффект «парящих» колёс).
        if ax >= self.inner_w - 0.06 && ax <= track_out - 0.04 {
            for w in &g.road_wheels {
                if ax <= w.x_min + 0.05 && (pt.z - w.z).hypot(pt.y - w.y) <= 0.11 {
                    return Some(self.pal.dark);
                }
            }
            for r in &g.rollers {
                if ax <= track_out - 0.08 && (pt.z - r.z).hypot(pt.y - r.y) <= 0.075 {
                    return Some(self.pal.dark);
                }
            }
        }

        if ax < track_in || ax > track_out {
            return None;
        }

        let zy = Vec2::new(pt.z, pt.y);

        // Ведущее колесо (с зубцами) и ленивец.
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

        // Опорные катки (резиновый бандаж + утопленный диск + стальная ступица).
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
                if d > w.r * 0.72 {
                    return Some(Material::TankRubber);
                }
                // Внутренняя чаша катка утоплена на 1 воксель для рельефа и AO.
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

        // Поддерживающие ролики (M24, ИС-3).
        for r in &g.rollers {
            if ax <= track_out - 0.05 && (pt.z - r.z).hypot(pt.y - r.y) <= r.r {
                return Some(Material::TankRubber);
            }
        }

        // Замкнутая гусеничная лента: низ, кормовой подъём, дуга кормы, верх, носовая дуга, носовой спуск.
        let rf = g.r_front + vs * 0.35;
        let rr = g.r_rear + vs * 0.35;
        let p_bot_f = Vec2::new(g.z_bot_start, g.y_track_bot);
        let p_bot_r = Vec2::new(g.z_bot_end, g.y_track_bot);
        let p_rear_lo = Vec2::new(g.z_rear + rr * 0.74, g.y_rear - rr * 0.67);
        let p_front_lo = Vec2::new(g.z_front - rf * 0.74, g.y_front - rf * 0.67);

        let d_bot = dist_seg2(zy, p_bot_f, p_bot_r);
        let d_top = dist_seg2(
            zy,
            Vec2::new(g.z_front, g.y_track_top),
            Vec2::new(g.z_rear, g.y_track_top),
        );
        let d_ramp_f = dist_seg2(zy, p_bot_f, p_front_lo);
        let d_ramp_r = dist_seg2(zy, p_bot_r, p_rear_lo);

        let d_arc_f = {
            let v = zy - Vec2::new(g.z_front, g.y_front);
            if v.x <= 0.0 && v.y >= -rf * 0.72 {
                (v.length() - rf).abs()
            } else {
                f32::MAX
            }
        };
        let d_arc_r = {
            let v = zy - Vec2::new(g.z_rear, g.y_rear);
            if v.x >= 0.0 && v.y >= -rr * 0.72 {
                (v.length() - rr).abs()
            } else {
                f32::MAX
            }
        };

        let d_belt = d_bot.min(d_top).min(d_ramp_f).min(d_ramp_r).min(d_arc_f).min(d_arc_r);
        let half_t = vs * 0.68;
        if d_belt <= half_t {
            // Траки чередуются по шагу ~0.16 м для читаемого рельефа гусеницы.
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

    fn wheel_end(&self, ax: f32, zy: Vec2, center: Vec2, r: f32, is_sprocket: bool) -> Option<Material> {
        let v = zy - center;
        let d = v.length();
        let outer_r = if is_sprocket { r * 1.04 } else { r };
        if d > outer_r || ax > self.hw - 0.02 {
            return None;
        }
        if d <= r * 0.30 {
            return Some(Material::TankSteel);
        }
        if is_sprocket && d >= r * 0.78 {
            let sector = (v.y.atan2(v.x).rem_euclid(TAU / 6.0) / (TAU / 6.0)) * 2.0 - 1.0;
            if sector.abs() < 0.46 && ax >= self.hw - 0.22 {
                return Some(Material::TankSteel);
            }
            if d > r * 0.88 {
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
        if ax < self.inner_w - 0.02 || ax > self.hw + 0.04 {
            return None;
        }
        // На Tiger I корпус нависает над полкой — даём классические передний/задний щитки и бортовой срез.
        if pt.z >= -self.hl + 0.14 && pt.z <= self.hl - 0.12 {
            if pt.y >= self.fender_y0 && pt.y <= self.fender_y1 {
                return Some(self.pal.hull_paint(pt, false));
            }
            // Лёгкий бортовой экран-отбортовка на Tiger I.
            if self.p.hull_shape == HullShape::HeavyBox
                && ax >= self.hw - 0.04
                && pt.y >= self.fender_y0 - self.p.voxel_size * 0.8
                && pt.y <= self.fender_y0
            {
                return Some(self.pal.dark);
            }
        }
        // Передний изогнутый брызговик.
        if pt.z < -self.hl + 0.14 && pt.z >= -self.hl - 0.12 {
            let drop = (-self.hl + 0.14 - pt.z) * 0.48;
            if pt.y >= self.fender_y0 - drop - 0.04 && pt.y <= self.fender_y1 - drop {
                return Some(self.pal.dark);
            }
        }
        // Задний брызговик.
        if pt.z > self.hl - 0.12 && pt.z <= self.hl + 0.12 {
            let drop = (pt.z - (self.hl - 0.12)) * 0.45;
            if pt.y >= self.fender_y0 - drop - 0.04 && pt.y <= self.fender_y1 - drop {
                return Some(self.pal.dark);
            }
        }
        None
    }

    fn hull_front_z(&self, ax: f32, y: f32) -> f32 {
        match self.p.hull_shape {
            HullShape::SlopedT34 => {
                if y >= 0.65 {
                    -self.hl + (y - 0.65) * 1.15
                } else {
                    -self.hl + (0.65 - y) * 0.70
                }
            }
            HullShape::PikeNose => {
                let v = ax * 0.56;
                if y >= 0.72 {
                    -self.hl + v + (y - 0.72) * 1.05
                } else {
                    -self.hl + v + (0.72 - y) * 0.70
                }
            }
            HullShape::Casemate => {
                if y >= 0.65 {
                    -self.hl + (y - 0.65) * 0.95
                } else {
                    -self.hl + (0.65 - y) * 0.65
                }
            }
            HullShape::LightBox => {
                if y >= 0.65 {
                    -self.hl + (y - 0.65) * 0.72
                } else {
                    -self.hl + (0.65 - y) * 0.42
                }
            }
            HullShape::HeavyBox => {
                if y >= 0.96 {
                    // Вертикальная верхняя лобовая плита мехвода.
                    -self.hl + 0.44 + (y - 0.96) * 0.14
                } else if y >= 0.80 {
                    // Пологий передний трансмиссионный уступ.
                    -self.hl + 0.12 + ((y - 0.80) / 0.16) * 0.32
                } else {
                    -self.hl + 0.12 + (0.80 - y) * 0.72
                }
            }
        }
    }

    fn hull_rear_z(&self, y: f32) -> f32 {
        let base_drop = (y - 0.34).max(0.0);
        match self.p.hull_shape {
            HullShape::HeavyBox => self.hl - base_drop * 0.08,
            _ => self.hl - base_drop * 0.25,
        }
    }

    fn hull_half_width(&self, y: f32) -> f32 {
        let t = if self.p.deck_height > self.fender_y1 {
            ((y - self.fender_y1) / (self.p.deck_height - self.fender_y1)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if y <= self.fender_y1 {
            return self.inner_w;
        }
        match self.p.hull_shape {
            HullShape::HeavyBox => self.hw - 0.04,
            HullShape::SlopedT34 | HullShape::Casemate => {
                (self.hw - 0.08) * (1.0 - t) + (self.inner_w + 0.16) * t
            }
            HullShape::PikeNose => (self.hw - 0.06) * (1.0 - t) + (self.inner_w + 0.20) * t,
            HullShape::LightBox => (self.hw - 0.12) * (1.0 - t) + (self.inner_w + 0.14) * t,
        }
    }

    fn sample_hull_armor(&self, pt: Vec3, ax: f32) -> Option<Material> {
        let vs = self.p.voxel_size;
        let clearance = 0.34;
        if pt.y < clearance || pt.y > self.p.deck_height + 0.01 {
            return None;
        }

        let max_x = self.hull_half_width(pt.y);
        if ax > max_x {
            return None;
        }

        let front_z = self.hull_front_z(ax, pt.y);
        let rear_z = self.hull_rear_z(pt.y);
        if pt.z < front_z || pt.z > rear_z {
            return None;
        }

        // Рельефная решётка МТО на кормовой крыше (углублена на 1 воксель через чередование ребер).
        if pt.y > self.p.deck_height - vs * 1.15 && pt.z > 0.65 && pt.z < rear_z - 0.18 && ax < self.inner_w * 0.72 {
            // Боковые круглые вентиляторы / крышки и поперечные жалюзи.
            let slat = (pt.z / (vs * 2.0)).floor() as i32;
            if pt.y > self.p.deck_height - vs * 0.75 && slat % 2 == 0 && ax > 0.12 {
                return None;
            }
            return Some(self.pal.dark);
        }

        // Литая/сварная планка на шве носа (особенно «щучий нос» ИС-3 и стык Т-34).
        if matches!(self.p.hull_shape, HullShape::PikeNose)
            && ax < vs * 0.7
            && pt.z < front_z + vs * 1.4
            && pt.y >= 0.70
        {
            return Some(self.pal.armor_light);
        }

        // Смотровая щель мехвода и шаровая установка курсового пулемёта на ВЛД.
        if pt.y >= 0.78 && pt.z < front_z + vs * 1.5 {
            match self.p.hull_shape {
                HullShape::SlopedT34 => {
                    // Люк мехвода слева с перископами.
                    if (-0.62..=-0.14).contains(&pt.x) && (0.84..=1.06).contains(&pt.y) {
                        if (0.96..=1.04).contains(&pt.y) && (pt.x + 0.38).abs() <= 0.16 {
                            return Some(Material::TankLens);
                        }
                        return Some(self.pal.armor_light);
                    }
                }
                HullShape::HeavyBox
                    if (1.12..=1.24).contains(&pt.y) && (-0.74..=-0.34).contains(&pt.x) =>
                {
                    return Some(Material::TankLens);
                }
                _ => {}
            }
        }

        // Детализация верхней плиты корпуса (лючки, стык передней плиты).
        let is_hatch = match self.p.hull_shape {
            HullShape::LightBox | HullShape::HeavyBox => {
                pt.y > self.p.deck_height - vs * 1.1
                    && (-2.15..=-0.95).contains(&pt.z)
                    && (0.25..=0.75).contains(&ax)
            }
            _ => false,
        };

        // Задний круглый люк трансмиссии (Т-34 / СУ-100).
        if matches!(self.p.hull_shape, HullShape::SlopedT34 | HullShape::Casemate)
            && pt.z > rear_z - vs * 1.3
            && (pt.x).hypot(pt.y - 0.76) <= 0.30
        {
            return Some(if (pt.x).hypot(pt.y - 0.76) > 0.22 {
                self.pal.dark
            } else {
                self.pal.armor_light
            });
        }

        Some(self.pal.hull_paint(pt, is_hatch))
    }

    fn hull_addons(&self, pt: Vec3, ax: f32) -> Option<Material> {
        let vs = self.p.voxel_size;
        let front_at_hinge = self.hull_front_z(ax, 0.72);

        // 1. Погон башни на крыше (для поворотных башен) — гарантирует плотную посадку без зазора.
        if self.p.turret_shape != TurretShape::FixedCasemate {
            let tp = Vec3::from_array(self.p.turret_pivot);
            if pt.y >= self.p.deck_height - 0.04 && pt.y <= self.p.deck_height + 0.03 {
                let d = (pt.x - tp[0]).hypot(pt.z - tp[2]);
                if d <= self.p.turret_radius_x * 0.94 && d >= self.p.turret_radius_x * 0.72 {
                    return Some(self.pal.dark);
                }
            }
        }

        // 2. Буксирные крюки на носу (передняя пара скоб).
        if (ax - self.inner_w * 0.62).abs() <= vs * 0.6
            && pt.y >= 0.56
            && pt.y <= 0.72
            && pt.z >= front_at_hinge - 0.14
            && pt.z <= front_at_hinge + 0.04
        {
            return Some(Material::TankSteel);
        }

        // 3. Запасные траки на нижней/средней части ВЛД (Т-34, СУ-100, Tiger I).
        if matches!(
            self.p.hull_shape,
            HullShape::SlopedT34 | HullShape::Casemate | HullShape::HeavyBox
        ) && ax <= 0.52
        {
            let rack_y = match self.p.hull_shape {
                HullShape::HeavyBox => (0.80, 0.94),
                _ => (0.64, 0.82),
            };
            if pt.y >= rack_y.0 && pt.y <= rack_y.1 {
                let fz = self.hull_front_z(ax, pt.y);
                if pt.z >= fz - 0.075 && pt.z <= fz + 0.02 {
                    let link = (pt.x / vs).floor() as i32;
                    return Some(if link % 2 == 0 {
                        Material::TankTrack
                    } else {
                        Material::TankTrackLight
                    });
                }
            }
        }

        // 4. Курсовой пулемёт справа на носу (Т-34, Tiger I, M24).
        if matches!(
            self.p.hull_shape,
            HullShape::SlopedT34 | HullShape::HeavyBox | HullShape::LightBox
        ) {
            let mg_pos = match self.p.hull_shape {
                HullShape::HeavyBox => Vec3::new(0.52, 1.18, -self.hl + 0.40),
                HullShape::LightBox => Vec3::new(0.45, 0.88, -self.hl + 0.22),
                _ => Vec3::new(0.44, 0.90, -self.hl + 0.26),
            };
            let radial = Vec2::new(pt.x - mg_pos.x, pt.y - mg_pos.y).length();
            if radial <= 0.095 && pt.z >= mg_pos.z - 0.08 && pt.z <= mg_pos.z + 0.14 {
                return Some(self.pal.dark);
            }
            if radial <= 0.045 && pt.z >= mg_pos.z - 0.22 && pt.z <= mg_pos.z {
                return Some(Material::TankSteel);
            }
        }

        // 5. Перископический колпак мехвода у ИС-3 в вершине «щучьего носа».
        if self.p.hull_shape == HullShape::PikeNose
            && ax <= 0.22
            && pt.y >= self.p.deck_height
            && pt.y <= self.p.deck_height + 0.12
            && pt.z >= -self.hl + 0.72
            && pt.z <= -self.hl + 1.06
        {
            if pt.z <= -self.hl + 0.82 && ax <= 0.16 {
                return Some(Material::TankLens);
            }
            return Some(self.pal.dark);
        }

        // 6. Фара слева спереди.
        let lamp_pos = match self.p.hull_shape {
            HullShape::HeavyBox => Vec3::new(-(self.hw - 0.25), self.p.deck_height + 0.08, -self.hl + 0.56),
            HullShape::PikeNose => Vec3::new(-0.56, self.p.deck_height + 0.04, -self.hl + 0.75),
            _ => Vec3::new(-(self.inner_w + 0.16), self.fender_y1 + 0.07, -self.hl + 0.26),
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

        // 7. Наружные топливные баки на бортах / корме (Т-34-85, СУ-100, ИС-3).
        if matches!(
            self.p.hull_shape,
            HullShape::SlopedT34 | HullShape::Casemate | HullShape::PikeNose
        ) {
            let (drum_x, drum_y, z0, z1) = match self.p.hull_shape {
                HullShape::PikeNose => (self.hw - 0.18, self.p.deck_height + 0.06, 0.85, 2.75),
                _ => (self.hw - 0.14, self.fender_y1 + 0.17, 1.05, 2.55),
            };
            if pt.z >= z0 && pt.z <= z1 {
                let d = (ax - drum_x).hypot(pt.y - drum_y);
                let in_gap = (pt.z - (z0 + z1) * 0.5).abs() < 0.07;
                if !in_gap && d <= 0.18 {
                    let strap = ((pt.z - z0) % 0.42).abs() < 0.06;
                    return Some(if strap {
                        self.pal.dark
                    } else {
                        self.pal.hull_paint(pt, false)
                    });
                }
            }
        }

        // 8. Бревно самовытаскивания на левом борту СУ-100 или ящик ЗИП на полках.
        if self.p.hull_shape == HullShape::Casemate
            && pt.x < -(self.inner_w + 0.04)
            && pt.x > -(self.hw - 0.04)
            && pt.z >= -0.65
            && pt.z <= 0.85
            && (pt.y - (self.fender_y1 + 0.12)).abs() <= 0.10
        {
            return Some(Material::TankWood);
        }
        if matches!(self.p.hull_shape, HullShape::LightBox | HullShape::HeavyBox)
            && ax >= self.inner_w + 0.02
            && ax <= self.hw - 0.06
            && pt.y >= self.fender_y1
            && pt.y <= self.fender_y1 + 0.16
            && pt.z >= -0.45
            && pt.z <= 0.65
        {
            let strap = (pt.z.abs() < 0.05) || (pt.z + 0.40).abs() < 0.04;
            return Some(if strap { Material::TankWood } else { self.pal.dark });
        }

        // 9. Выхлопные трубы на корме (у Tiger I — вертикальные глушители Фейфеля с кожухами).
        if self.p.hull_shape == HullShape::HeavyBox {
            if (ax - 0.62).abs() <= 0.14
                && pt.z >= self.hl - 0.10
                && pt.z <= self.hl + 0.24
                && pt.y >= 0.72
                && pt.y <= self.p.deck_height + 0.24
            {
                let inner = (ax - 0.62).abs() <= 0.06 && pt.y >= self.p.deck_height + 0.04;
                return Some(if inner {
                    Material::TankExhaust
                } else {
                    self.pal.dark
                });
            }
        } else {
            let rz = self.hull_rear_z(0.84);
            if (ax - 0.54).hypot(pt.y - 0.84) <= 0.085 && pt.z >= rz - 0.04 && pt.z <= rz + 0.20 {
                return Some(Material::TankExhaust);
            }
        }

        None
    }

    fn sample_turret(&self, pt: Vec3) -> Option<Material> {
        let vs = self.p.voxel_size;
        let rx = self.p.turret_radius_x;
        let rz = self.p.turret_radius_z;
        let th = self.p.turret_height;

        // 1. Утопленный погон башни (стыкует башню с крышей без просвета).
        if pt.y < 0.06 && pt.y >= -vs * 0.6 && self.p.turret_shape != TurretShape::FixedCasemate {
            let nx = pt.x / (rx * 0.90);
            let nz = (pt.z - 0.03) / (rz * 0.90);
            if nx * nx + nz * nz <= 1.0 {
                return Some(self.pal.dark);
            }
        }

        // 2. Навесные и крышевые детали башни (башенки, пулемёты, дымовые мортирки, Rommelkiste).
        if let Some(m) = self.turret_addons(pt, rx, rz, th, vs) {
            return Some(m);
        }

        if pt.y < 0.0 || pt.y > th {
            return None;
        }

        // 3. Основной броневой силуэт башни/рубки.
        let t = (pt.y / th).clamp(0.0, 1.0);
        let inside = match self.p.turret_shape {
            TurretShape::BoxHeavy => {
                let k = 1.0 - t * 0.06;
                if pt.z < 0.0 {
                    let nx = pt.x / (rx * k);
                    let nz = pt.z / (rz * k);
                    nx * nx + nz * nz <= 1.0
                } else {
                    pt.x.abs() <= rx * k && pt.z <= rz * k
                }
            }
            TurretShape::FixedCasemate => {
                let k = 1.0 - t * 0.22;
                let front_slope = -rz + t * 0.58;
                let right_blister =
                    pt.x >= 0.25 && pt.x <= rx * 0.96 && pt.z >= -0.45 && pt.z <= 0.35 && t >= 0.45;
                right_blister || (pt.x.abs() <= rx * k && pt.z >= front_slope && pt.z <= rz)
            }
            TurretShape::RoundedDome => {
                // ИС-3 «перевёрнутая сковорода» со слегка поджатым нижним отливом.
                let base_pinch = if pt.y < vs * 1.2 { 0.95 } else { 1.0 };
                let k = (1.0 - t.powf(1.9) * 0.42).max(0.46) * base_pinch;
                let nx = pt.x / (rx * k);
                let nz = (pt.z - 0.05) / (rz * k);
                nx * nx + nz * nz <= 1.0
            }
            TurretShape::CompactLight => {
                let k = 1.0 - t * 0.15;
                let bustle = pt.z > rz * 0.30
                    && pt.z <= rz + 0.22
                    && pt.x.abs() <= rx * 0.72
                    && pt.y >= th * 0.22;
                let nx = pt.x / (rx * k);
                let nz = (pt.z - 0.02) / (rz * k);
                bustle || (nx * nx + nz * nz <= 1.0)
            }
            TurretShape::HexCast => {
                let k = 1.0 - t * 0.18;
                let bustle = pt.z > rz * 0.25
                    && pt.z <= rz + 0.14
                    && pt.x.abs() <= rx * 0.70 * k
                    && pt.y >= th * 0.16;
                let nx = pt.x / (rx * k);
                let nz = (pt.z - 0.05) / (rz * k);
                bustle || (nx * nx + nz * nz <= 1.0)
            }
        };
        if !inside {
            return None;
        }

        // Опознавательный знак на бортах башни/рубки (только внешний слой).
        let mark_z = if self.p.turret_shape == TurretShape::FixedCasemate {
            0.20
        } else {
            0.05
        };
        let min_mark_x = rx * 0.68;
        if pt.x.abs() > min_mark_x
            && in_tactical_mark(self.team, pt.z - mark_z, pt.y - th * 0.52)
        {
            return Some(self.pal.mark);
        }

        // Задняя дверь рубки у СУ-100.
        if self.p.turret_shape == TurretShape::FixedCasemate
            && pt.z > rz - vs * 1.2
            && pt.x.abs() <= 0.32
            && pt.y >= 0.08
            && pt.y <= th * 0.78
        {
            let border = pt.x.abs() > 0.24 || pt.y > th * 0.70 || pt.y < 0.16;
            return Some(if border {
                self.pal.dark
            } else {
                self.pal.armor_light
            });
        }

        // Тонкая контрастная окантовка крыши.
        let roof_rim = pt.y > th - vs * 0.8 && pt.x.abs() > rx * 0.55;
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
        // Командирская башенка со смотровыми приборами.
        let (cup_pos, cup_r, cup_h) = match self.p.turret_shape {
            TurretShape::FixedCasemate => (Vec2::new(0.54, -0.06), 0.26, 0.18),
            TurretShape::BoxHeavy => (Vec2::new(-rx * 0.36, rz * 0.22), 0.34, 0.28),
            TurretShape::RoundedDome => (Vec2::new(-rx * 0.34, rz * 0.14), 0.24, 0.09),
            TurretShape::CompactLight => (Vec2::new(-rx * 0.32, rz * 0.18), 0.25, 0.15),
            TurretShape::HexCast => (Vec2::new(-rx * 0.35, rz * 0.22), 0.28, 0.21),
        };
        let d_cup = Vec2::new(pt.x - cup_pos.x, pt.z - cup_pos.y).length();
        if pt.y >= th - 0.02 && pt.y <= th + cup_h && d_cup <= cup_r {
            // Смотровая щель / перископы в верхней трети башенки.
            if pt.y >= th + cup_h * 0.48
                && pt.y <= th + cup_h * 0.78
                && d_cup >= cup_r * 0.65
                && cup_h >= 0.14
            {
                return Some(Material::TankLens);
            }
            if pt.y > th + cup_h - vs * 0.9 {
                return Some(self.pal.armor_light);
            }
            return Some(self.pal.dark);
        }

        // Люк заряжающего справа.
        let load_x = if self.p.turret_shape == TurretShape::FixedCasemate {
            -0.38
        } else {
            rx * 0.34
        };
        let d_load = Vec2::new(pt.x - load_x, pt.z - rz * 0.14).length();
        if pt.y >= th - 0.02 && pt.y <= th + 0.08 && d_load <= 0.23 {
            return Some(if d_load > 0.16 {
                self.pal.dark
            } else {
                self.pal.armor_light
            });
        }

        // Грибки вентиляции на корме башни Т-34-85.
        if self.p.turret_shape == TurretShape::HexCast && pt.y >= th - 0.01 && pt.y <= th + 0.09 {
            for sx in [-0.26f32, 0.26] {
                if (pt.x - sx).hypot(pt.z - rz * 0.56) <= 0.10 {
                    return Some(self.pal.dark);
                }
            }
        }

        // Кормовой ящик Rommelkiste, дымовые мортирки и запасные траки на башне Tiger I.
        if self.p.turret_shape == TurretShape::BoxHeavy {
            if pt.y >= 0.12
                && pt.y <= th * 0.76
                && pt.x.abs() <= rx * 0.68
                && pt.z >= rz * 0.90
                && pt.z <= rz + 0.34
            {
                let strap = (pt.x.abs() - rx * 0.42).abs() < 0.05 || pt.y > th * 0.68;
                return Some(if strap {
                    self.pal.dark
                } else {
                    self.pal.hull_paint(pt, false)
                });
            }
            // Бортовые мортирки 3+3 у передних скул башни.
            if pt.y >= th * 0.45
                && pt.y <= th * 0.68
                && pt.x.abs() >= rx * 0.78
                && pt.x.abs() <= rx + 0.12
                && pt.z >= -rz * 0.54
                && pt.z <= -rz * 0.30
            {
                return Some(Material::TankSteel);
            }
            // Запасные траки на бортах башни.
            if pt.x.abs() >= rx * 0.88
                && pt.x.abs() <= rx + 0.08
                && pt.y >= 0.14
                && pt.y <= th * 0.82
                && pt.z >= rz * 0.25
                && pt.z <= rz * 0.84
            {
                let link = (pt.y / vs).floor() as i32;
                return Some(if link % 2 == 0 {
                    Material::TankTrack
                } else {
                    Material::TankTrackLight
                });
            }
        }

        // Зенитный крупнокалиберный пулемёт на крыше M24 (.50 Browning) и ИС-3 (ДШК).
        if matches!(
            self.p.turret_shape,
            TurretShape::CompactLight | TurretShape::RoundedDome
        ) {
            let mount = Vec3::new(0.26, th, rz * 0.44);
            let post = (pt.x - mount.x).hypot(pt.z - mount.z) <= 0.055
                && pt.y >= th - 0.02
                && pt.y <= th + 0.24;
            let body = (pt.x - mount.x).abs() <= 0.075
                && (pt.y - (th + 0.24)).abs() <= 0.065
                && pt.z >= mount.z - 0.16
                && pt.z <= mount.z + 0.18;
            let barrel = (pt.x - mount.x).hypot(pt.y - (th + 0.25)) <= 0.042
                && pt.z >= mount.z - 0.62
                && pt.z <= mount.z - 0.12;
            if post || body || barrel {
                return Some(Material::TankSteel);
            }
        }

        // Поручни на бортах башни (Т-34 / ИС-3).
        if matches!(self.p.turret_shape, TurretShape::HexCast | TurretShape::RoundedDome)
            && pt.x.abs() >= rx * 0.70
            && pt.x.abs() <= rx * 0.94
            && (pt.y - th * 0.52).abs() <= 0.045
            && pt.z >= rz * 0.18
            && pt.z <= rz * 0.56
        {
            return Some(Material::TankSteel);
        }

        // Штыревая база антенны на корме крыши башни.
        let ant = match self.p.turret_shape {
            TurretShape::RoundedDome => Vec2::new(-rx * 0.22, rz * 0.44),
            TurretShape::HexCast => Vec2::new(-rx * 0.36, rz * 0.62),
            _ => Vec2::new(-rx * 0.48, rz * 0.66),
        };
        if (pt.x - ant.x).hypot(pt.z - ant.y) <= 0.05 && pt.y >= th - 0.03 && pt.y <= th + 0.34 {
            return Some(self.pal.dark);
        }

        None
    }

    fn sample_barrel(&self, pt: Vec3) -> Option<Material> {
        let br = self.p.barrel_radius;
        let bl = self.p.barrel_length;
        let gs = self.p.gun_voxel_size;
        let r_xy = pt.x.hypot(pt.y);

        // 1. Полый канал ствола в дульной части (только 1x1 центральный воксель по X/Y).
        if pt.z <= -bl + 0.22 && pt.z >= -bl - 0.02 && pt.x.abs() < gs * 0.5 && pt.y.abs() < gs * 0.5 {
            return None;
        }

        // 2. Дульный тормоз (двухкамерный с реальными прорезями у Tiger I и ИС-3).
        let has_muzzle_brake = matches!(
            self.p.turret_shape,
            TurretShape::BoxHeavy | TurretShape::RoundedDome
        );
        if has_muzzle_brake && pt.z >= -bl && pt.z <= -bl + 0.40 {
            let rx_b = br * 1.90;
            let ry_b = br * 1.42;
            // Вырезы дульных камер по бокам ствола.
            let in_slot_1 = pt.z >= -bl + 0.08 && pt.z <= -bl + 0.17;
            let in_slot_2 = pt.z >= -bl + 0.22 && pt.z <= -bl + 0.31;
            if (in_slot_1 || in_slot_2) && pt.x.abs() >= br * 0.85 && pt.y.abs() <= br * 0.72 {
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
        } else if !has_muzzle_brake && pt.z >= -bl && pt.z <= -bl + 0.16 {
            // Утолщение дульного среза (Т-34-85, СУ-100, M24).
            if r_xy <= (br * 1.16).max(gs * 2.2) {
                return Some(Material::TankSteel);
            }
        }

        // 3. Откатной кожух / утолщённая казённая муфта ствола.
        let sleeve_len = match self.p.turret_shape {
            TurretShape::CompactLight => 0.72,
            TurretShape::BoxHeavy => 0.56,
            _ => 0.68,
        };
        if pt.z >= -sleeve_len && pt.z <= 0.0 && r_xy <= br * 1.45 {
            return Some(self.pal.dark);
        }

        // 4. Маска орудия по типу башни/рубки (с прицелом и спаренным пулемётом).
        let mantlet = match self.p.turret_shape {
            TurretShape::BoxHeavy => {
                // Широкая ступенчатая маска KwK 36 (Tiger I).
                let in_plate = pt.z >= -0.26
                    && pt.z <= 0.24
                    && pt.x.abs() <= 0.72
                    && pt.y.abs() <= 0.22;
                if in_plate {
                    // Два отверстия бинокулярного прицела слева и спаренный MG справа.
                    if pt.z < -0.14 && ((pt.x + 0.32).abs() <= 0.07 || (pt.x + 0.44).abs() <= 0.05) && pt.y.abs() <= 0.05 {
                        return Some(Material::TankLens);
                    }
                    if pt.z < -0.10 && (pt.x - 0.30).hypot(pt.y) <= 0.05 {
                        return Some(Material::TankSteel);
                    }
                    return Some(self.pal.hull_paint(pt, pt.y > 0.14));
                }
                false
            }
            TurretShape::FixedCasemate => {
                // Литая шаровая бронемаска СУ-100.
                let ell = (pt.x / 0.36).powi(2) + (pt.y / 0.26).powi(2) + ((pt.z + 0.02) / 0.36).powi(2);
                ell <= 1.0
            }
            TurretShape::HexCast => {
                // Округлая литая «рыльцевидная» маска Д-5Т / С-53.
                let ell = (pt.x / 0.34).powi(2) + (pt.y / 0.23).powi(2) + ((pt.z + 0.04) / 0.32).powi(2);
                ell <= 1.0
            }
            TurretShape::RoundedDome => {
                // Узкая клиновидная маска ИС-3.
                pt.z >= -0.32
                    && pt.z <= 0.28
                    && pt.x.abs() <= 0.31
                    && pt.y.abs() <= 0.21 - (pt.z + 0.32).max(0.0) * 0.05
            }
            TurretShape::CompactLight => {
                // Компактный щиток 75-мм пушки Chaffee.
                pt.z >= -0.26 && pt.z <= 0.24 && pt.x.abs() <= 0.28 && pt.y.abs() <= 0.19
            }
        };
        if mantlet {
            return Some(self.pal.hull_paint(pt, false));
        }

        // 5. Основной конически сужающийся ствол орудия.
        if pt.z <= 0.08 && pt.z >= -bl {
            let frac = (-pt.z / bl.max(0.5)).clamp(0.0, 1.0);
            let r_taper = (br * (1.14 - 0.20 * frac)).max(gs * 2.15);
            if r_xy <= r_taper {
                // Стяжное кольцо в средней части длинных стволов.
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
    fn all_five_tanks_build_distinct_detailed_models_and_enemy_camo() {
        let tanks = available_tanks();
        assert_eq!(tanks.len(), 5);
        let mut counts = Vec::new();
        for t in &tanks {
            let parts = build_tank_parts_for_team(&t.profile, Team::Player);
            let h = parts.hull.count_solid();
            let tr = parts.turret.count_solid();
            let b = parts.barrel.count_solid();
            assert!(h > 3_500 && tr > 700 && b > 300, "{}: {h}/{tr}/{b}", t.id);

            let hm = mesh_grid_ao(&parts.hull);
            let tm = mesh_grid_ao(&parts.turret);
            let bm = mesh_grid_ao(&parts.barrel);
            let total_quads = hm.quad_count() + tm.quad_count() + bm.quad_count();
            assert!(total_quads < 40_000, "{}: quads={total_quads}", t.id);

            // На дульном срезе канал ствола полый в самом центре (0, 0).
            let tip = Vec3::new(0.0, 0.0, -t.profile.barrel_length + 0.04);
            let tip_v = parts.barrel.world_to_voxel(tip);
            assert!(!parts.barrel.is_solid(tip_v), "{} должен иметь полый канал у среза", t.id);

            let enemy = build_tank_parts_for_team(&t.profile, Team::Enemy);
            assert!(has_material(&enemy.hull, Material::EnemyArmor), "{}", t.id);
            assert!(has_material(&enemy.hull, Material::EnemyCamo), "{}", t.id);
            assert!(has_material(&enemy.turret, Material::EnemyMark), "{}", t.id);

            counts.push((t.id.clone(), h, tr, b));
        }

        // Проверяем различие габаритов и дульных тормозов.
        let m24 = counts.iter().find(|c| c.0 == "m24_chaffee").unwrap();
        let tiger = counts.iter().find(|c| c.0 == "tiger_1").unwrap();
        assert!(tiger.1 > m24.1 && tiger.3 > m24.3);

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
    fn all_five_tanks_have_no_floating_disconnected_voxels() {
        for t in available_tanks() {
            let parts = build_tank_parts_for_team(&t.profile, Team::Player);
            for (part_name, grid) in [
                ("hull", &parts.hull),
                ("turret", &parts.turret),
                ("barrel", &parts.barrel),
            ] {
                let comps = connected_components(grid);
                assert_eq!(
                    comps.len(),
                    1,
                    "{} {part_name}: ожидается 1 связная компонента, найдено {} (размеры: {:?})",
                    t.id,
                    comps.len(),
                    comps.iter().map(|c| c.len()).collect::<Vec<_>>()
                );
            }
        }
    }
}
