//! Пользовательский интерфейс: ангар, боевой HUD и WT X-Ray Hit-Cam.

pub mod hangar;
pub mod hud;
pub mod xray_cam;

use bevy::prelude::*;

use crate::core::{BattleSetupSet, GameState};

const BUNDLED_FONT: &[u8] = include_bytes!("../../assets/fonts/DejaVuSansMono.ttf");

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        if let Some(mut fonts) = app.world_mut().get_resource_mut::<Assets<Font>>() {
            let _ = fonts.insert(AssetId::default(), Font::from_bytes(BUNDLED_FONT.to_vec()));
        }

        app.init_resource::<hangar::HangarViewState>()
            .init_resource::<hud::HudHitMarker>()
            .add_systems(OnEnter(GameState::Hangar), hangar::setup_hangar)
            .add_systems(Update, hangar::update_hangar.run_if(in_state(GameState::Hangar)))
            .add_systems(
                OnEnter(GameState::InBattle),
                (
                    hud::setup_hud.in_set(BattleSetupSet::SpawnHud),
                    xray_cam::setup_xray_hit_cam,
                ),
            )
            .add_systems(
                Update,
                (hud::update_hud, xray_cam::update_xray_hit_cam)
                    .run_if(in_state(GameState::InBattle)),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::text::{
        ComputedTextBlock, FontAtlasSet, FontCx, FontHinting, LayoutCx, ScaleCx, TextLayoutInfo,
        TextPipeline, TextPlugin,
    };

    #[test]
    fn bundled_default_font_shapes_cyrillic_and_xray_symbols() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            ImagePlugin::default(),
            TextPlugin,
        ))
        .add_plugins(UiPlugin);

        let txt = "Т-34-85 • ЭКИПАЖ ┌─│═·└┐┘*X ▼◆◈■□ ←→×";
        let entity = app
            .world_mut()
            .spawn((
                Text::new(txt),
                TextFont::from_font_size(14.0),
                TextLayout::default(),
            ))
            .id();
        app.finish();
        app.cleanup();
        app.update();

        // Убеждаемся, что AssetId::default() действительно подменён на наш DejaVuSansMono.ttf.
        {
            let fonts = app.world().resource::<Assets<Font>>();
            let default_font = fonts.get(AssetId::default()).expect("default font asset");
            assert_eq!(default_font.data.as_ref(), BUNDLED_FONT);
        }

        let world = app.world_mut();
        bevy::ecs::system::RunSystemOnce::run_system_once(
            world,
            move |mut pipeline: ResMut<TextPipeline>,
                  fonts: Res<Assets<Font>>,
                  mut textures: ResMut<Assets<Image>>,
                  mut font_cx: ResMut<FontCx>,
                  mut layout_cx: ResMut<LayoutCx>,
                  mut scale_cx: ResMut<ScaleCx>,
                  mut atlas_set: ResMut<FontAtlasSet>| {
                let mut computed = ComputedTextBlock::default();
                let mut layout_info = TextLayoutInfo::default();
                let tf = TextFont::from_font_size(14.0);

                pipeline
                    .update_buffer(
                        &fonts,
                        std::iter::once((
                            entity,
                            0usize,
                            bevy::text::TextElement::Text {
                                text: txt,
                                font: &tf,
                                color: Color::WHITE,
                                line_height: bevy::text::LineHeight::default(),
                                letter_spacing: bevy::text::LetterSpacing::default(),
                            },
                        )),
                        bevy::text::LineBreak::WordBoundary,
                        bevy::text::Justify::Left,
                        bevy::text::TextBounds::UNBOUNDED,
                        1.0,
                        &mut computed,
                        &mut font_cx,
                        &mut layout_cx,
                        Vec2::new(1280.0, 720.0),
                        bevy::text::RemSize(16.0),
                    )
                    .expect("update_buffer");

                pipeline
                    .update_text_layout_info(
                        &mut layout_info,
                        &mut atlas_set,
                        &mut textures,
                        &mut computed,
                        &mut scale_cx,
                        bevy::text::TextBounds::UNBOUNDED,
                        bevy::text::Justify::Left,
                        FontHinting::Enabled,
                    )
                    .expect("update_text_layout_info");

                assert!(layout_info.size.x > 100.0 && layout_info.size.y > 10.0);
                assert!(
                    layout_info.glyphs.len() >= 25,
                    "ожидались растеризованные глифы кириллицы/символов, получено {}",
                    layout_info.glyphs.len()
                );
            },
        )
        .expect("run_system_once");
    }
}

