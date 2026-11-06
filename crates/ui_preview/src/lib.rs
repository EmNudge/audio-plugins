//! Renders a plugin editor to a PNG with no host and no window, for README screenshots.
//!

//! renderer. Between frames `feed` pushes synthetic audio through the plugin's DSP, so the meters the
//! editor reads (and the fall-slow, rise-fast smoothing on top of them) behave as they do live.

use nih_plug::prelude::*;
use nih_plug_egui::egui;
use std::path::Path;

/// A host stand-in: parameter edits go nowhere, which is what a still image wants.
pub struct NoHost;

impl GuiContext for NoHost {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Standalone
    }
    fn request_resize(&self) -> bool {
        false
    }
    unsafe fn raw_begin_set_parameter(&self, _param: ParamPtr) {}
    unsafe fn raw_set_parameter_normalized(&self, _param: ParamPtr, _normalized: f32) {}
    unsafe fn raw_end_set_parameter(&self, _param: ParamPtr) {}
    fn get_state(&self) -> nih_plug::wrapper::state::PluginState {
        nih_plug::wrapper::state::PluginState { version: String::new(), params: Default::default(), fields: Default::default() }
    }
    fn set_state(&self, _state: nih_plug::wrapper::state::PluginState) {}
}

pub struct Shot<'a> {
    /// Editor size in points, as in `EguiState::from_size`.
    pub size: (u32, u32),
    /// Frames to run before the capture. `feed` is called with the frame index before each one.
    pub frames: usize,
    /// Output pixel density relative to the editor's points.
    pub scale: f32,
    /// Inner margin of the panel, as in the plugin's `CentralPanel` frame.
    pub margin: f32,
    pub background: egui::Color32,
    pub feed: Box<dyn FnMut(usize) + 'a>,
    /// Draws the editor body; the same closure body the plugin passes to `create_egui_editor`.
    pub draw: Box<dyn FnMut(&mut egui::Ui, &ParamSetter) + 'a>,
}

impl Shot<'_> {
    pub fn save(mut self, path: impl AsRef<Path>) {
        let host = NoHost;
        let setter = ParamSetter::new(&host);
        let (w, h) = self.size;
        let (background, margin) = (self.background, self.margin);
        let mut frame = 0usize;
        let (feed, draw) = (&mut self.feed, &mut self.draw);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(w as f32, h as f32))
            .with_pixels_per_point(self.scale)
            .wgpu()
            .build(|ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(background).inner_margin(margin))
                    .show(ctx, |ui| draw(ui, &setter));
            });
        for _ in 0..self.frames {
            feed(frame);
            frame += 1;
            harness.step();
        }
        let image = harness.render().expect("render failed");
        if let Some(dir) = path.as_ref().parent() {
            std::fs::create_dir_all(dir).expect("could not create output directory");
        }
        image.save(path.as_ref()).expect("could not write png");
    }
}

/// `args[1]` as the output path, defaulting to `docs/screenshots/<name>.png`.
pub fn output_path(name: &str) -> std::path::PathBuf {
    std::env::args()
        .nth(1)
        .map(Into::into)
        .unwrap_or_else(|| format!("docs/screenshots/{name}.png").into())
}
