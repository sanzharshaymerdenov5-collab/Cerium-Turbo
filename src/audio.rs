//! Процедурное звуковое сопровождение без внешних аудио-файлов и без нативных OS-библиотек
//! (совместимо с `wasm32` WebGL2/GitHub Pages через Web Audio API и с headless CI `cargo test --lib`).
//!
//! Озвучивает:
//! - выстрелы разного калибра (75–122 мм) + далёкую канонаду;
//! - завершение перезарядки орудия (щелчок затвора);
//! - рикошет (свист), непробитие (глухой удар) и пробитие брони;
//! - попадания по грунту и разрушаемым постройкам;
//! - подрыв БК, уничтожение танка и падение сорванной башни на грунт;
//! - пожар (треск), огнетушитель (струя пены) и ремонт модулей;
//! - непрерывный гул двигателя игрока, зависящий от оборотов и скорости;
//! - клики UI и переключение звука клавишей `[N]`.

use std::f32::consts::{PI, TAU};

use bevy::prelude::*;

use crate::ballistics::{ArmorImpactMessage, TerrainImpactMessage, penetration::ImpactOutcome};
use crate::core::GameState;
use crate::damage::{TankActionKind, TankActionMessage, TankDestroyedMessage, TankModules};
use crate::tank::{PlayerTank, TankCamera, TankCombat, TankFiredMessage, TankRoot, TankState};
use crate::vfx::TurretLandedMessage;
use crate::voxel::Rng;

/// Частота дискретизации процедурного PCM-синтеза, Гц.
pub const SAMPLE_RATE: u32 = 22_050;
/// Максимальное число одномоментных звуковых сэмплов за один кадр (анти-клиппинг).
const MAX_SOUNDS_PER_FRAME: usize = 6;

/// Глобальные настройки громкости.
#[derive(Resource, Debug, Clone)]
pub struct AudioSettings {
    pub muted: bool,
    pub master_volume: f32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            muted: false,
            master_volume: 0.72,
        }
    }
}

/// Все процедурно синтезируемые звуки игры.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SoundPreset {
    CannonLight,
    CannonMedium,
    CannonHeavy,
    DistantCannon,
    ReloadReady,
    Ricochet(u8),
    NoPenetration,
    Penetration,
    AmmoDetonation,
    TankKilled,
    TerrainHit,
    StructureHit,
    TurretLanded,
    FireCrackle(u8),
    Extinguisher,
    RepairClink,
    UiClick,
    BattleStart,
}

/// Событие воспроизведения UI-звука.
#[derive(Message, Debug, Clone, Copy)]
pub struct UiSoundMessage(pub SoundPreset);

/// Телеметрия воспроизведённых звуков (используется и для юнит-тестов).
#[derive(Resource, Debug, Default)]
pub struct AudioStats {
    pub played_count: u64,
    pub last_preset: Option<SoundPreset>,
    pub last_gain: f32,
    pub last_pan: f32,
}

pub struct GameAudioPlugin;

impl Plugin for GameAudioPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(target_arch = "wasm32")]
        wasm_backend::install_gesture_unlockers();

        app.init_resource::<AudioSettings>()
            .init_resource::<AudioStats>()
            .add_message::<UiSoundMessage>()
            .add_systems(
                Update,
                (
                    toggle_mute_hotkey,
                    handle_ui_audio,
                    #[cfg(target_arch = "wasm32")]
                    wasm_unlock_on_bevy_input,
                ),
            )
            .add_systems(
                Update,
                (handle_battle_audio, update_engine_audio)
                    .chain()
                    .run_if(in_state(GameState::InBattle)),
            )
            .add_systems(OnExit(GameState::InBattle), silence_engine_on_exit);
    }
}

#[cfg(target_arch = "wasm32")]
fn wasm_unlock_on_bevy_input(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
) {
    let any_key = keys.is_some_and(|k| k.get_just_pressed().next().is_some());
    let any_mouse = mouse.is_some_and(|m| m.get_just_pressed().next().is_some());
    if any_key || any_mouse {
        wasm_backend::ensure_unlocked();
    }
}

fn toggle_mute_hotkey(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut settings: ResMut<AudioSettings>,
    mut stats: ResMut<AudioStats>,
) {
    if keys.is_some_and(|k| k.just_pressed(KeyCode::KeyN)) {
        settings.muted = !settings.muted;
        if settings.muted {
            #[cfg(target_arch = "wasm32")]
            wasm_backend::set_engine_params(0.0, 0.0, false);
        } else {
            emit_sound(&settings, &mut stats, SoundPreset::UiClick, 0.85, 0.0, 1.0);
        }
    }
}

fn handle_ui_audio(
    settings: Res<AudioSettings>,
    mut stats: ResMut<AudioStats>,
    mut ui_msgs: MessageReader<UiSoundMessage>,
) {
    for msg in ui_msgs.read() {
        let gain = match msg.0 {
            SoundPreset::BattleStart => 0.90,
            _ => 0.70,
        };
        emit_sound(&settings, &mut stats, msg.0, gain, 0.0, 1.0);
    }
}

fn silence_engine_on_exit() {
    #[cfg(target_arch = "wasm32")]
    wasm_backend::set_engine_params(0.0, 0.0, false);
}

fn update_engine_audio(
    settings: Res<AudioSettings>,
    player_q: Query<(&TankRoot, &TankState, &TankCombat), With<PlayerTank>>,
) {
    #[cfg(target_arch = "wasm32")]
    {
        if settings.muted {
            wasm_backend::set_engine_params(0.0, 0.0, false);
            return;
        }
        if let Ok((root, st, combat)) = player_q.single()
            && combat.alive
        {
            let speed_ratio =
                (st.speed_kmh.abs() / root.spec.max_speed_kmh.max(1.0)).clamp(0.0, 1.25);
            let load = (speed_ratio * 0.72 + st.throttle.abs() * 0.45 + st.steer.abs() * 0.22)
                .clamp(0.0, 1.2);
            let vol = (0.045 + load * 0.075) * settings.master_volume;
            let base_pitch = if root.spec.mass_kg > 42_000.0 {
                34.0
            } else if root.spec.mass_kg < 25_000.0 {
                48.0
            } else {
                41.0
            };
            wasm_backend::set_engine_params(base_pitch + load * 32.0, vol, true);
        } else {
            wasm_backend::set_engine_params(0.0, 0.0, false);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (&settings, &player_q);
    }
}

/// Рассчитывает пространственное затухание громкости `(0..1)` и стерео-панораму `(-0.92..0.92)`.
pub fn compute_spatial_audio(
    listener_pos: Vec3,
    listener_right: Vec3,
    sound_pos: Vec3,
    is_player_source: bool,
) -> (f32, f32) {
    if is_player_source {
        return (1.0, 0.0);
    }
    let delta = sound_pos - listener_pos;
    let dist = delta.length();
    if !dist.is_finite() || dist > 260.0 {
        return (0.0, 0.0);
    }
    let atten = if dist <= 6.0 {
        1.0
    } else {
        (1.0 / (1.0 + ((dist - 6.0) / 30.0).powf(1.32)))
            * (1.0 - ((dist - 190.0).max(0.0) / 70.0)).clamp(0.0, 1.0)
    };
    let right = listener_right.normalize_or_zero();
    let pan = if dist > 0.5 && right.length_squared() > 0.1 {
        (delta / dist).dot(right).clamp(-0.92, 0.92)
    } else {
        0.0
    };
    (atten.clamp(0.0, 1.0), pan)
}

#[allow(clippy::too_many_arguments)]
fn handle_battle_audio(
    time: Res<Time>,
    settings: Res<AudioSettings>,
    mut stats: ResMut<AudioStats>,
    cam_q: Query<&GlobalTransform, (With<TankCamera>, With<Camera3d>)>,
    player_q: Query<(Entity, &GlobalTransform, &TankState, &TankCombat), With<PlayerTank>>,
    burning_q: Query<(&GlobalTransform, &TankModules)>,
    mut fired_r: MessageReader<TankFiredMessage>,
    mut armor_r: MessageReader<ArmorImpactMessage>,
    mut terrain_r: MessageReader<TerrainImpactMessage>,
    mut destroyed_r: MessageReader<TankDestroyedMessage>,
    mut turret_r: MessageReader<TurretLandedMessage>,
    mut action_r: MessageReader<TankActionMessage>,
    mut prev_reload: Local<Option<(Entity, f32)>>,
    mut fire_loop_acc: Local<f32>,
    mut variant: Local<u64>,
) {
    let dt = time.delta_secs();
    let player_info = player_q.single().ok();
    let player_ent = player_info.map(|(e, _, _, _)| e);

    let (listener_pos, listener_right) = if let Some(cam_gt) = cam_q.iter().next() {
        (cam_gt.translation(), cam_gt.right().as_vec3())
    } else if let Some((_, p_gt, _, _)) = player_info {
        (p_gt.translation() + Vec3::Y * 2.0, p_gt.right().as_vec3())
    } else {
        (Vec3::ZERO, Vec3::X)
    };

    // Индикатор готовности орудия у игрока.
    if let Some((p_ent, _, st, combat)) = player_info {
        if let Some((prev_ent, old_r)) = *prev_reload
            && prev_ent == p_ent
            && old_r > 0.02
            && st.reload_timer <= 0.02
            && combat.alive
        {
            emit_sound(&settings, &mut stats, SoundPreset::ReloadReady, 0.72, 0.0, 1.0);
        }
        *prev_reload = Some((p_ent, st.reload_timer));
    }

    let mut played_this_frame = 0usize;

    // 1. Выстрелы (собственные и вражеские).
    for msg in fired_r.read() {
        if played_this_frame >= MAX_SOUNDS_PER_FRAME {
            continue;
        }
        *variant = variant.wrapping_add(1);
        let pitch = 0.96 + (*variant % 9) as f32 * 0.01;
        let dist = listener_pos.distance(msg.muzzle_pos);
        let (gain, pan) = compute_spatial_audio(listener_pos, listener_right, msg.muzzle_pos, msg.is_player);
        let preset = if !msg.is_player && dist > 62.0 {
            SoundPreset::DistantCannon
        } else if msg.caliber_mm >= 105.0 {
            SoundPreset::CannonHeavy
        } else if msg.caliber_mm >= 82.0 {
            SoundPreset::CannonMedium
        } else {
            SoundPreset::CannonLight
        };
        let final_gain = gain * if msg.is_player { 0.96 } else { 0.85 };
        if final_gain > 0.025 {
            emit_sound(&settings, &mut stats, preset, final_gain, pan, pitch);
            played_this_frame += 1;
        }
    }

    // 2. Взрывы БК и уничтожение техники (высокий приоритет).
    for msg in destroyed_r.read() {
        if played_this_frame >= MAX_SOUNDS_PER_FRAME {
            continue;
        }
        let is_player_hit = player_ent == Some(msg.tank);
        let (gain, pan) = compute_spatial_audio(listener_pos, listener_right, msg.pos, is_player_hit);
        let preset = if msg.ammo_detonated {
            SoundPreset::AmmoDetonation
        } else {
            SoundPreset::TankKilled
        };
        let g = (gain * if msg.ammo_detonated { 1.0 } else { 0.88 }).max(if msg.ammo_detonated {
            0.18
        } else {
            0.08
        });
        emit_sound(&settings, &mut stats, preset, g, pan, 1.0);
        played_this_frame += 1;
    }

    // 3. Падение сорванной башни о землю.
    for msg in turret_r.read() {
        if played_this_frame >= MAX_SOUNDS_PER_FRAME {
            continue;
        }
        let (gain, pan) = compute_spatial_audio(listener_pos, listener_right, msg.pos, false);
        let impact_scale = (msg.impact_speed / 8.5).clamp(0.40, 1.0);
        let g = gain * impact_scale * 0.90;
        if g > 0.04 {
            emit_sound(&settings, &mut stats, SoundPreset::TurretLanded, g, pan, 0.98);
            played_this_frame += 1;
        }
    }

    // 4. Броневые попадания: рикошет, непробитие, пробитие.
    for msg in armor_r.read() {
        if played_this_frame >= MAX_SOUNDS_PER_FRAME {
            continue;
        }
        *variant = variant.wrapping_add(1);
        let involves_player = player_ent.is_some()
            && (msg.shooter == player_ent || msg.owner_tank == player_ent);
        let (spatial_g, pan) = compute_spatial_audio(listener_pos, listener_right, msg.hit_point, false);
        let gain = if involves_player {
            spatial_g.max(0.55)
        } else {
            spatial_g * 0.85
        };
        if gain < 0.03 {
            continue;
        }
        let (preset, pitch) = match msg.outcome {
            ImpactOutcome::Ricochet => (
                SoundPreset::Ricochet((*variant % 3) as u8),
                0.94 + (*variant % 7) as f32 * 0.02,
            ),
            ImpactOutcome::NoPenetration { .. } => {
                (SoundPreset::NoPenetration, 0.95 + (*variant % 5) as f32 * 0.02)
            }
            ImpactOutcome::Penetration { .. } => {
                (SoundPreset::Penetration, 0.97 + (*variant % 5) as f32 * 0.015)
            }
        };
        emit_sound(&settings, &mut stats, preset, gain, pan, pitch);
        played_this_frame += 1;
    }

    // 5. Действия по ремонту и тушению пожара.
    for msg in action_r.read() {
        if played_this_frame >= MAX_SOUNDS_PER_FRAME {
            continue;
        }
        let (gain, pan) = compute_spatial_audio(listener_pos, listener_right, msg.pos, msg.is_player);
        let preset = match msg.kind {
            TankActionKind::FireStarted => SoundPreset::FireCrackle(0),
            TankActionKind::FireExtinguished => SoundPreset::Extinguisher,
            TankActionKind::RepairUsed | TankActionKind::AutoRepaired => SoundPreset::RepairClink,
        };
        let boost = if msg.is_player { 0.86 } else { gain * 0.65 };
        if boost > 0.04 {
            emit_sound(&settings, &mut stats, preset, boost, pan, 1.0);
            played_this_frame += 1;
        }
    }

    // 6. Попадания по грунту и разрушаемым укрытиям.
    for msg in terrain_r.read() {
        if played_this_frame >= MAX_SOUNDS_PER_FRAME {
            continue;
        }
        *variant = variant.wrapping_add(1);
        let (gain, pan) = compute_spatial_audio(listener_pos, listener_right, msg.point, false);
        let g = gain * if msg.destructible { 0.82 } else { 0.65 };
        if g > 0.04 {
            let preset = if msg.destructible {
                SoundPreset::StructureHit
            } else {
                SoundPreset::TerrainHit
            };
            emit_sound(
                &settings,
                &mut stats,
                preset,
                g,
                pan,
                0.93 + (*variant % 7) as f32 * 0.025,
            );
            played_this_frame += 1;
        }
    }

    // 7. Периодический треск горящих танков поблизости.
    *fire_loop_acc += dt;
    if *fire_loop_acc >= 0.24 {
        *fire_loop_acc = 0.0;
        let mut best: Option<(f32, Vec3)> = None;
        for (gt, mods) in &burning_q {
            if mods.fire_timer <= 0.0 {
                continue;
            }
            let p = gt.translation();
            let d = listener_pos.distance(p);
            if d < 75.0 && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, p));
            }
        }
        if let Some((_, pos)) = best {
            let (gain, pan) = compute_spatial_audio(listener_pos, listener_right, pos, false);
            let g = (gain * 0.42).min(0.45);
            if g > 0.04 {
                *variant = variant.wrapping_add(1);
                emit_sound(
                    &settings,
                    &mut stats,
                    SoundPreset::FireCrackle((*variant & 1) as u8),
                    g,
                    pan,
                    0.96 + (*variant % 5) as f32 * 0.02,
                );
            }
        }
    }
}

fn emit_sound(
    settings: &AudioSettings,
    stats: &mut AudioStats,
    preset: SoundPreset,
    gain: f32,
    pan: f32,
    pitch: f32,
) {
    if settings.muted || settings.master_volume <= 0.001 || gain <= 0.005 {
        return;
    }
    let effective_gain = (gain * settings.master_volume).clamp(0.0, 1.0);
    stats.played_count = stats.played_count.saturating_add(1);
    stats.last_preset = Some(preset);
    stats.last_gain = effective_gain;
    stats.last_pan = pan.clamp(-1.0, 1.0);

    #[cfg(target_arch = "wasm32")]
    wasm_backend::play(preset, effective_gain, stats.last_pan, pitch);

    #[cfg(not(target_arch = "wasm32"))]
    let _ = pitch;
}

/// Синтезирует моно-PCM буфер `f32` (`[-1.0, 1.0]`) на `SAMPLE_RATE` Гц.
pub fn synthesize_preset(preset: SoundPreset) -> Vec<f32> {
    let sr = SAMPLE_RATE as f32;
    let duration_s = match preset {
        SoundPreset::CannonLight => 0.72,
        SoundPreset::CannonMedium => 0.92,
        SoundPreset::CannonHeavy => 1.15,
        SoundPreset::DistantCannon => 0.85,
        SoundPreset::ReloadReady => 0.24,
        SoundPreset::Ricochet(_) => 0.50,
        SoundPreset::NoPenetration => 0.34,
        SoundPreset::Penetration => 0.44,
        SoundPreset::AmmoDetonation => 1.45,
        SoundPreset::TankKilled => 1.05,
        SoundPreset::TerrainHit => 0.40,
        SoundPreset::StructureHit => 0.56,
        SoundPreset::TurretLanded => 0.50,
        SoundPreset::FireCrackle(_) => 0.30,
        SoundPreset::Extinguisher => 0.58,
        SoundPreset::RepairClink => 0.42,
        SoundPreset::UiClick => 0.07,
        SoundPreset::BattleStart => 0.46,
    };
    let n = (duration_s * sr).round() as usize;
    let mut out = vec![0.0f32; n];
    let mut rng = Rng::new(preset_seed(preset));

    match preset {
        SoundPreset::CannonLight | SoundPreset::CannonMedium | SoundPreset::CannonHeavy => {
            let (f_start, f_end, crack_decay, boom_decay, tail_decay, drive) = match preset {
                SoundPreset::CannonLight => (210.0, 56.0, 38.0, 11.5, 4.4, 2.2),
                SoundPreset::CannonMedium => (175.0, 44.0, 30.0, 8.2, 3.2, 2.6),
                SoundPreset::CannonHeavy => (145.0, 34.0, 24.0, 5.8, 2.5, 3.1),
                _ => unreachable!(),
            };
            let mut phase = 0.0f32;
            let mut ring_phase = 0.0f32;
            let mut lp_noise = 0.0f32;
            let mut lp_tail = 0.0f32;

            for (i, sample) in out.iter_mut().enumerate() {
                let t = i as f32 / sr;
                let w = rng.signed();
                lp_noise += 0.38 * (w - lp_noise);
                lp_tail += 0.055 * (w - lp_tail);

                let freq = f_end + (f_start - f_end) * (-t * 17.0).exp();
                phase += TAU * freq / sr;
                ring_phase += TAU * (f_start * 2.15) / sr;

                let attack = (t / 0.004).min(1.0);
                let crack = lp_noise * (-t * crack_decay).exp() * 1.45;
                let punch = phase.sin() * (-t * boom_decay).exp() * 1.30;
                let ring = ring_phase.sin() * (-t * 28.0).exp() * 0.16;
                let tail = lp_tail * (-t * tail_decay).exp() * 0.72;

                *sample = (attack * (crack + punch + ring + tail) * drive * 0.48).tanh();
            }
        }
        SoundPreset::DistantCannon => {
            let mut phase = 0.0f32;
            let mut lp = 0.0f32;
            for (i, sample) in out.iter_mut().enumerate() {
                let t = i as f32 / sr;
                let w = rng.signed();
                lp += 0.032 * (w - lp);
                let attack = (t / 0.025).min(1.0);
                phase += TAU * (78.0 - 38.0 * (t / duration_s)) / sr;
                let body = phase.sin() * (-t * 5.0).exp() * 0.55 + lp * (-t * 2.7).exp() * 1.15;
                *sample = (attack * body * 1.4).tanh();
            }
        }
        SoundPreset::ReloadReady => {
            // Двойной стальной щелчок казённика: 0.00с и 0.095с.
            let mut lp = 0.0f32;
            for (i, sample) in out.iter_mut().enumerate() {
                let t = i as f32 / sr;
                let w = rng.signed();
                lp += 0.45 * (w - lp);
                let c1 = (-t * 75.0).exp() * ((TAU * 920.0 * t).sin() * 0.5 + (TAU * 1580.0 * t).sin() * 0.35 + lp * 0.3);
                let dt2 = t - 0.095;
                let c2 = if dt2 >= 0.0 {
                    (-dt2 * 52.0).exp() * ((TAU * 1240.0 * dt2).sin() * 0.6 + (TAU * 2280.0 * dt2).sin() * 0.45 + w * 0.25)
                } else {
                    0.0
                };
                *sample = ((c1 + c2) * 0.72).tanh();
            }
        }
        SoundPreset::Ricochet(v) => {
            let base = 2450.0 + (v as f32) * 320.0;
            let mut phase = 0.0f32;
            let mut fm_phase = 0.0f32;
            let mut over_phase = 0.0f32;
            for (i, sample) in out.iter_mut().enumerate() {
                let t = i as f32 / sr;
                let w = rng.signed();
                let ping = (-t * 95.0).exp() * w * 0.65;

                fm_phase += TAU * (44.0 + (v as f32) * 9.0) / sr;
                let glide = (-t * 3.8).exp();
                let f = base * (0.28 + 0.72 * glide) + fm_phase.sin() * 165.0 * (t * 4.5).min(1.0);
                phase += TAU * f / sr;
                over_phase += TAU * (f * 1.47) / sr;

                let env = (t / 0.006).min(1.0) * (-t * 5.2).exp();
                let whine = (phase.sin() * 0.75 + over_phase.sin() * 0.25) * env;
                *sample = ((ping + whine * 1.15) * 0.82).tanh();
            }
        }
        SoundPreset::NoPenetration => {
            let mut lp = 0.0f32;
            for (i, sample) in out.iter_mut().enumerate() {
                let t = i as f32 / sr;
                lp += 0.22 * (rng.signed() - lp);
                let attack = (t / 0.002).min(1.0);
                let thud = (TAU * (165.0 - 85.0 * (t * 6.0).min(1.0)) * t).sin() * (-t * 14.0).exp();
                let plate_mode = ((TAU * 410.0 * t).sin() + 0.6 * (TAU * 685.0 * t).sin()) * (-t * 22.0).exp() * 0.45;
                let scrape = lp * (-t * 25.0).exp() * 0.55;
                *sample = (attack * (thud + plate_mode + scrape) * 1.15).tanh();
            }
        }
        SoundPreset::Penetration => {
            let mut lp = 0.0f32;
            for (i, sample) in out.iter_mut().enumerate() {
                let t = i as f32 / sr;
                let w = rng.signed();
                lp += 0.42 * (w - lp);
                let attack = (t / 0.002).min(1.0);
                let punch = (TAU * (240.0 - 150.0 * (t * 7.0).min(1.0)) * t).sin() * (-t * 15.0).exp();
                let crunch = (w - lp * 0.5) * (-t * 18.0).exp() * 0.85;
                let dt_spall = t - 0.035;
                let spall = if dt_spall >= 0.0 {
                    w * (-dt_spall * 9.5).exp() * 0.50
                } else {
                    0.0
                };
                *sample = (attack * (punch + crunch + spall) * 1.25).tanh();
            }
        }
        SoundPreset::AmmoDetonation => {
            let mut phase = 0.0f32;
            let mut lp = 0.0f32;
            for (i, sample) in out.iter_mut().enumerate() {
                let t = i as f32 / sr;
                let w = rng.signed();
                lp += 0.08 * (w - lp);

                let f = 32.0 + 135.0 * (-t * 11.0).exp();
                phase += TAU * f / sr;

                let primary_blast = phase.sin() * (-t * 3.8).exp() * 0.95 + lp * (-t * 1.9).exp() * 1.35;
                // Вторичный хлопок детонации БК и разлёт осколков.
                let dt2 = t - 0.14;
                let cookoff = if dt2 >= 0.0 {
                    (TAU * 72.0 * dt2).sin() * (-dt2 * 6.5).exp() * 0.65
                        + w * (-dt2 * 14.0).exp() * 0.55
                } else {
                    0.0
                };
                let crackle_pop = if rng.f32() < 0.004 && t > 0.10 {
                    rng.signed() * 0.75 * (-t * 1.5).exp()
                } else {
                    0.0
                };
                let attack = (t / 0.005).min(1.0);
                *sample = (attack * (primary_blast + cookoff + crackle_pop) * 1.35).tanh();
            }
        }
        SoundPreset::TankKilled => {
            let mut phase = 0.0f32;
            let mut lp = 0.0f32;
            for (i, sample) in out.iter_mut().enumerate() {
                let t = i as f32 / sr;
                let w = rng.signed();
                lp += 0.12 * (w - lp);
                phase += TAU * (40.0 + 100.0 * (-t * 10.0).exp()) / sr;
                let attack = (t / 0.004).min(1.0);
                let boom = phase.sin() * (-t * 5.0).exp() + lp * (-t * 3.0).exp() * 1.1;
                let tear = w * (-t * 11.0).exp() * 0.45;
                *sample = (attack * (boom + tear) * 1.15).tanh();
            }
        }
        SoundPreset::TerrainHit => {
            let mut lp = 0.0f32;
            for (i, sample) in out.iter_mut().enumerate() {
                let t = i as f32 / sr;
                lp += 0.09 * (rng.signed() - lp);
                let thump = (TAU * (92.0 - 48.0 * (t * 5.0).min(1.0)) * t).sin() * (-t * 14.0).exp();
                let dirt = lp * (-t * 7.5).exp() * 1.2;
                *sample = ((thump + dirt) * 0.85).tanh();
            }
        }
        SoundPreset::StructureHit => {
            let mut lp = 0.0f32;
            for (i, sample) in out.iter_mut().enumerate() {
                let t = i as f32 / sr;
                let w = rng.signed();
                lp += 0.18 * (w - lp);
                let crack = w * (-t * 24.0).exp() * 0.75;
                let thump = (TAU * 115.0 * t).sin() * (-t * 12.0).exp() * 0.55;
                let rubble = if t > 0.05 {
                    lp * (-(t - 0.05) * 6.5).exp()
                        * (0.55 + 0.45 * (TAU * 22.0 * t).sin())
                        * 0.85
                } else {
                    0.0
                };
                *sample = ((crack + thump + rubble) * 0.95).tanh();
            }
        }
        SoundPreset::TurretLanded => {
            let mut lp = 0.0f32;
            for (i, sample) in out.iter_mut().enumerate() {
                let t = i as f32 / sr;
                lp += 0.14 * (rng.signed() - lp);
                let attack = (t / 0.004).min(1.0);
                let earth_thud = (TAU * (85.0 - 52.0 * (t * 4.5).min(1.0)) * t).sin() * (-t * 9.0).exp() * 0.95;
                let steel_clang = ((TAU * 212.0 * t).sin() * 0.48
                    + (TAU * 347.0 * t).sin() * 0.30
                    + (TAU * 518.0 * t).sin() * 0.18)
                    * (-t * 11.5).exp();
                let dust = lp * (-t * 7.0).exp() * 0.5;
                *sample = (attack * (earth_thud + steel_clang + dust) * 1.15).tanh();
            }
        }
        SoundPreset::FireCrackle(_) => {
            let mut lp = 0.0f32;
            let mut pop_env = 0.0f32;
            for (i, sample) in out.iter_mut().enumerate() {
                let t = i as f32 / sr;
                let w = rng.signed();
                lp += 0.07 * (w - lp);
                if rng.f32() < 0.008 {
                    pop_env = 0.55 + rng.f32() * 0.45;
                }
                pop_env *= 0.985;
                // Плавное окно на краях 0.30с, чтобы периодический повтор не щёлкал.
                let win = (PI * t / duration_s).sin();
                let hiss = w * 0.12 + lp * 0.72 + (w * pop_env) * 0.85;
                *sample = (win * hiss * 1.1).tanh();
            }
        }
        SoundPreset::Extinguisher => {
            let mut hp_prev = 0.0f32;
            let mut prev_w = 0.0f32;
            for (i, sample) in out.iter_mut().enumerate() {
                let t = i as f32 / sr;
                let w = rng.signed();
                hp_prev = 0.84 * (hp_prev + w - prev_w);
                prev_w = w;
                let env = (t / 0.025).min(1.0) * (-t * 3.8).exp();
                *sample = (env * (hp_prev * 0.85 + w * 0.25) * 1.1).tanh();
            }
        }
        SoundPreset::RepairClink => {
            // Серия 4 коротких звонов гаечного ключа.
            let mut lp = 0.0f32;
            let hits = [0.0f32, 0.095, 0.19, 0.285];
            for (i, sample) in out.iter_mut().enumerate() {
                let t = i as f32 / sr;
                let w = rng.signed();
                lp += 0.5 * (w - lp);
                let mut acc = 0.0f32;
                for (k, &h0) in hits.iter().enumerate() {
                    let dt = t - h0;
                    if dt >= 0.0 {
                        let f = 1480.0 + (k as f32) * 190.0;
                        acc += (-dt * 42.0).exp()
                            * ((TAU * f * dt).sin() * 0.6 + (TAU * f * 1.72 * dt).sin() * 0.35 + (w - lp) * 0.25);
                    }
                }
                *sample = (acc * 0.68).tanh();
            }
        }
        SoundPreset::UiClick => {
            for (i, sample) in out.iter_mut().enumerate() {
                let t = i as f32 / sr;
                let env = (t / 0.002).min(1.0) * (-t * 68.0).exp();
                let sig = (TAU * 980.0 * t).sin() * 0.6 + (TAU * 1840.0 * t).sin() * 0.3;
                *sample = (env * sig * 0.85).clamp(-1.0, 1.0);
            }
        }
        SoundPreset::BattleStart => {
            for (i, sample) in out.iter_mut().enumerate() {
                let t = i as f32 / sr;
                let thud = (TAU * (120.0 - 55.0 * (t * 4.0).min(1.0)) * t).sin() * (-t * 10.0).exp() * 0.65;
                let chime = ((TAU * 440.0 * t).sin() + 0.5 * (TAU * 660.0 * t).sin())
                    * (t / 0.015).min(1.0)
                    * (-t * 6.5).exp()
                    * 0.35;
                *sample = ((thud + chime) * 0.95).tanh();
            }
        }
    }

    out
}

fn preset_seed(preset: SoundPreset) -> u64 {
    match preset {
        SoundPreset::CannonLight => 0x101,
        SoundPreset::CannonMedium => 0x202,
        SoundPreset::CannonHeavy => 0x303,
        SoundPreset::DistantCannon => 0x404,
        SoundPreset::ReloadReady => 0x505,
        SoundPreset::Ricochet(v) => 0x600 + (v as u64) * 37,
        SoundPreset::NoPenetration => 0x707,
        SoundPreset::Penetration => 0x808,
        SoundPreset::AmmoDetonation => 0x909,
        SoundPreset::TankKilled => 0xA0A,
        SoundPreset::TerrainHit => 0xB0B,
        SoundPreset::StructureHit => 0xC0C,
        SoundPreset::TurretLanded => 0xD0D,
        SoundPreset::FireCrackle(v) => 0xE00 + (v as u64) * 53,
        SoundPreset::Extinguisher => 0xF0F,
        SoundPreset::RepairClink => 0x1111,
        SoundPreset::UiClick => 0x1212,
        SoundPreset::BattleStart => 0x1313,
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm_backend {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use wasm_bindgen::{JsCast, closure::Closure};
    use web_sys::{
        AudioBuffer, AudioContext, AudioContextState, BiquadFilterNode, BiquadFilterType,
        GainNode, OscillatorNode, OscillatorType,
    };

    use super::{SAMPLE_RATE, SoundPreset, synthesize_preset};

    struct EngineSynth {
        osc_low: OscillatorNode,
        osc_growl: OscillatorNode,
        filter: BiquadFilterNode,
        gain: GainNode,
    }

    struct WebAudioState {
        ctx: AudioContext,
        buffers: HashMap<SoundPreset, AudioBuffer>,
        engine: Option<EngineSynth>,
    }

    thread_local! {
        static STATE: RefCell<Option<WebAudioState>> = const { RefCell::new(None) };
        static UNLOCKERS_INSTALLED: RefCell<bool> = const { RefCell::new(false) };
    }

    pub fn install_gesture_unlockers() {
        UNLOCKERS_INSTALLED.with(|installed| {
            if *installed.borrow() {
                return;
            }
            let Some(win) = web_sys::window() else {
                return;
            };
            let cb = Closure::<dyn FnMut(web_sys::Event)>::new(|_: web_sys::Event| {
                ensure_unlocked();
            });
            let func = cb.as_ref().unchecked_ref();
            let _ = win.add_event_listener_with_callback("pointerdown", func);
            let _ = win.add_event_listener_with_callback("keydown", func);
            let _ = win.add_event_listener_with_callback("touchstart", func);
            cb.forget();
            *installed.borrow_mut() = true;
        });
    }

    fn with_ctx<F: FnOnce(&mut WebAudioState)>(f: F) {
        STATE.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_none()
                && let Ok(ctx) = AudioContext::new()
            {
                let engine = create_engine_synth(&ctx).ok();
                *slot = Some(WebAudioState {
                    ctx,
                    buffers: HashMap::new(),
                    engine,
                });
            }
            if let Some(ref mut st) = *slot {
                if st.ctx.state() == AudioContextState::Suspended {
                    let _ = st.ctx.resume();
                }
                f(st);
            }
        });
    }

    pub fn ensure_unlocked() {
        with_ctx(|_| {});
    }

    fn create_engine_synth(ctx: &AudioContext) -> Result<EngineSynth, wasm_bindgen::JsValue> {
        let osc_low = ctx.create_oscillator()?;
        osc_low.set_type(OscillatorType::Triangle);
        osc_low.frequency().set_value(40.0);

        let osc_growl = ctx.create_oscillator()?;
        osc_growl.set_type(OscillatorType::Sawtooth);
        osc_growl.frequency().set_value(80.0);

        let filter = ctx.create_biquad_filter()?;
        filter.set_type(BiquadFilterType::Lowpass);
        filter.frequency().set_value(170.0);
        filter.q().set_value(1.8);

        let gain = ctx.create_gain()?;
        gain.gain().set_value(0.0);

        osc_low.connect_with_audio_node(&filter)?;
        osc_growl.connect_with_audio_node(&filter)?;
        filter.connect_with_audio_node(&gain)?;
        gain.connect_with_audio_node(&ctx.destination())?;

        osc_low.start()?;
        osc_growl.start()?;

        Ok(EngineSynth {
            osc_low,
            osc_growl,
            filter,
            gain,
        })
    }

    pub fn set_engine_params(freq_hz: f32, target_gain: f32, active: bool) {
        STATE.with(|cell| {
            let slot = cell.borrow();
            let Some(ref st) = *slot else {
                return;
            };
            if st.ctx.state() != AudioContextState::Running {
                return;
            }
            let Some(ref eng) = st.engine else {
                return;
            };
            let now = st.ctx.current_time();
            if active && target_gain > 0.001 {
                let f = freq_hz.clamp(24.0, 140.0);
                let _ = eng.osc_low.frequency().set_target_at_time(f, now, 0.08);
                let _ = eng.osc_growl.frequency().set_target_at_time(f * 2.0, now, 0.08);
                let _ = eng
                    .filter
                    .frequency()
                    .set_target_at_time(120.0 + f * 3.2, now, 0.08);
                let _ = eng
                    .gain
                    .gain()
                    .set_target_at_time(target_gain.clamp(0.0, 0.16), now, 0.10);
            } else {
                let _ = eng.gain.gain().set_target_at_time(0.0, now, 0.08);
            }
        });
    }

    pub fn play(preset: SoundPreset, gain: f32, pan: f32, pitch: f32) {
        with_ctx(|st| {
            if st.ctx.state() != AudioContextState::Running {
                return;
            }
            let buf = st.buffers.entry(preset).or_insert_with(|| {
                let samples = synthesize_preset(preset);
                let b = st
                    .ctx
                    .create_buffer(1, samples.len() as u32, SAMPLE_RATE as f32)
                    .expect("AudioBuffer");
                let _ = b.copy_to_channel(&samples, 0);
                b
            });

            let Ok(src) = st.ctx.create_buffer_source() else {
                return;
            };
            let Ok(g_node) = st.ctx.create_gain() else {
                return;
            };
            let Ok(p_node) = st.ctx.create_stereo_panner() else {
                return;
            };

            src.set_buffer(Some(buf));
            src.playback_rate().set_value(pitch.clamp(0.65, 1.45));
            g_node.gain().set_value(gain.clamp(0.0, 1.0));
            p_node.pan().set_value(pan.clamp(-0.95, 0.95));

            if src.connect_with_audio_node(&p_node).is_err() {
                return;
            }
            if p_node.connect_with_audio_node(&g_node).is_err() {
                return;
            }
            if g_node.connect_with_audio_node(&st.ctx.destination()).is_err() {
                return;
            }
            let _ = src.start();
        });
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::core::Team;
    use bevy::state::app::StatesPlugin;
    use bevy::time::TimeUpdateStrategy;

    #[test]
    fn procedural_presets_generate_valid_audible_waveforms() {
        let presets = [
            SoundPreset::CannonLight,
            SoundPreset::CannonMedium,
            SoundPreset::CannonHeavy,
            SoundPreset::DistantCannon,
            SoundPreset::ReloadReady,
            SoundPreset::Ricochet(0),
            SoundPreset::Ricochet(1),
            SoundPreset::Ricochet(2),
            SoundPreset::NoPenetration,
            SoundPreset::Penetration,
            SoundPreset::AmmoDetonation,
            SoundPreset::TankKilled,
            SoundPreset::TerrainHit,
            SoundPreset::StructureHit,
            SoundPreset::TurretLanded,
            SoundPreset::FireCrackle(0),
            SoundPreset::Extinguisher,
            SoundPreset::RepairClink,
            SoundPreset::UiClick,
            SoundPreset::BattleStart,
        ];

        for preset in presets {
            let pcm = synthesize_preset(preset);
            assert!(!pcm.is_empty(), "пустой PCM для {preset:?}");
            let mut sum_sq = 0.0f64;
            let mut peak = 0.0f32;
            for &s in &pcm {
                assert!(s.is_finite(), "NaN/Inf в сэмпле {preset:?}");
                assert!((-1.05..=1.05).contains(&s), "перегрузка в {preset:?}: {s}");
                peak = peak.max(s.abs());
                sum_sq += (s as f64) * (s as f64);
            }
            let rms = (sum_sq / pcm.len() as f64).sqrt();
            assert!(rms > 0.01 && peak > 0.08, "слишком тихий сэмпл {preset:?}: rms={rms:.4}, peak={peak:.3}");
        }

        // Тяжёлое орудие звучит дольше и энергичнее клика UI.
        let heavy = synthesize_preset(SoundPreset::CannonHeavy);
        let click = synthesize_preset(SoundPreset::UiClick);
        assert!(heavy.len() > click.len() * 8);
    }

    #[test]
    fn spatial_audio_attenuates_with_distance_and_pans_left_right() {
        let listener = Vec3::ZERO;
        let right = Vec3::X;

        let (g_player, p_player) = compute_spatial_audio(listener, right, Vec3::new(30.0, 0.0, 0.0), true);
        assert_eq!(g_player, 1.0);
        assert_eq!(p_player, 0.0);

        let (g_near, _) = compute_spatial_audio(listener, right, Vec3::new(0.0, 0.0, -5.0), false);
        let (g_mid, _) = compute_spatial_audio(listener, right, Vec3::new(0.0, 0.0, -60.0), false);
        let (g_far, _) = compute_spatial_audio(listener, right, Vec3::new(0.0, 0.0, -280.0), false);
        assert!(g_near > g_mid && g_mid > g_far && g_far == 0.0);

        let (_, pan_right) = compute_spatial_audio(listener, right, Vec3::new(25.0, 0.0, 0.0), false);
        let (_, pan_left) = compute_spatial_audio(listener, right, Vec3::new(-25.0, 0.0, 0.0), false);
        assert!(pan_right > 0.8 && pan_left < -0.8);
    }

    #[test]
    fn audio_plugin_responds_to_battle_events_and_mute_hotkey() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .init_state::<GameState>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_message::<TankFiredMessage>()
            .add_message::<ArmorImpactMessage>()
            .add_message::<TerrainImpactMessage>()
            .add_message::<TankDestroyedMessage>()
            .add_message::<TurretLandedMessage>()
            .add_message::<TankActionMessage>()
            .add_plugins(GameAudioPlugin)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                0.03,
            )));

        app.world_mut()
            .resource_mut::<NextState<GameState>>()
            .set(GameState::InBattle);
        app.finish();
        app.cleanup();
        app.update();

        app.world_mut().write_message(TankFiredMessage {
            tank: Entity::PLACEHOLDER,
            team: Some(Team::Player),
            muzzle_pos: Vec3::ZERO,
            muzzle_dir: Vec3::NEG_Z,
            gun_pivot: Vec3::ZERO,
            caliber_mm: 122.0,
            is_player: true,
            has_muzzle_brake: true,
        });
        app.update();
        assert_eq!(app.world().resource::<AudioStats>().played_count, 1);
        assert_eq!(
            app.world().resource::<AudioStats>().last_preset,
            Some(SoundPreset::CannonHeavy)
        );

        let shell = crate::tank::load_tank_spec("t34_85").unwrap().shell;
        app.world_mut().write_message(ArmorImpactMessage {
            plate_entity: Entity::PLACEHOLDER,
            owner_tank: None,
            zone: None,
            target_name: "Броня".into(),
            shooter: None,
            shooter_team: Some(Team::Player),
            hit_point: Vec3::new(15.0, 1.0, -20.0),
            hit_normal: Vec3::Z,
            ray_dir: Vec3::NEG_Z,
            dist_m: 25.0,
            angle_deg: 76.0,
            armor_thickness_mm: 100.0,
            shell,
            outcome: ImpactOutcome::Ricochet,
        });
        app.update();
        assert_eq!(app.world().resource::<AudioStats>().played_count, 2);
        assert!(matches!(
            app.world().resource::<AudioStats>().last_preset,
            Some(SoundPreset::Ricochet(_))
        ));

        // Проверка Mute по кнопке N.
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyN);
        app.update();
        assert!(app.world().resource::<AudioSettings>().muted);
        let muted_count = app.world().resource::<AudioStats>().played_count;
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.release(KeyCode::KeyN);
            keys.clear_just_pressed(KeyCode::KeyN);
        }

        app.world_mut().write_message(UiSoundMessage(SoundPreset::UiClick));
        app.update();
        assert!(app.world().resource::<AudioSettings>().muted);
        assert_eq!(
            app.world().resource::<AudioStats>().played_count,
            muted_count,
            "в режиме Mute звуки не должны воспроизводиться"
        );
    }
}
