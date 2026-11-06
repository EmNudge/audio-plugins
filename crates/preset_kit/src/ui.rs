//! The `<  name  >` picker that sits in each editor's header.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{Align2, Color32, FontId, Id, Pos2, Rect, Sense, Ui, Vec2};

use crate::Bank;

pub struct Theme {
    pub fill: Color32,
    pub hover: Color32,
    pub ink: Color32,
    pub dim: Color32,
}

const ARROW_W: f32 = 24.0;
const NAME_W: f32 = 150.0;
const GAP: f32 = 4.0;
pub const WIDTH: f32 = ARROW_W * 2.0 + NAME_W + GAP * 2.0;

/// Draw the picker with its top-left corner at `origin`. Reads "custom" once any parameter moves
/// away from a preset; stepping from there continues from the last preset shown.
pub fn picker(ui: &mut Ui, origin: Pos2, height: f32, bank: &Bank, params: &(impl Params + ?Sized), setter: &ParamSetter, theme: &Theme) {
    let n = bank.presets.len();
    let memory = Id::new("preset_kit_last");
    let current = bank.current(params);
    if let Some(i) = current {
        ui.data_mut(|d| d.insert_temp(memory, i));
    }
    let last: usize = ui.data(|d| d.get_temp(memory)).unwrap_or(0);
    let from = current.unwrap_or(last);

    let cell = |x: f32, w: f32| Rect::from_min_size(Pos2::new(origin.x + x, origin.y), Vec2::new(w, height));
    let cells = [cell(0.0, ARROW_W), cell(ARROW_W + GAP, NAME_W), cell(ARROW_W + NAME_W + GAP * 2.0, ARROW_W)];
    let painter = ui.painter().clone();
    let mut pick = None;
    for (i, rect) in cells.into_iter().enumerate() {
        let response = ui.interact(rect, ui.id().with(("preset_kit", i)), Sense::click());
        painter.rect_filled(rect, 4.0, if response.hovered() { theme.hover } else { theme.fill });
        let (label, ink) = match (i, current) {
            (0, _) => ("<".to_owned(), theme.dim),
            (2, _) => (">".to_owned(), theme.dim),
            (_, Some(p)) => (bank.presets[p].name.to_uppercase(), theme.ink),
            (_, None) => ("CUSTOM".to_owned(), theme.dim),
        };
        painter.text(rect.center(), Align2::CENTER_CENTER, label, FontId::monospace(10.0), ink);
        if response.clicked() {
            pick = Some(if i == 0 { (from + n - 1) % n } else { (from + 1) % n });
        }
    }
    if let Some(i) = pick {
        ui.data_mut(|d| d.insert_temp(memory, i));
        bank.apply(i, setter, params);
    }
}

/// The picker on the first row of `ui`, flush with the right edge.
pub fn picker_top_right(ui: &mut Ui, height: f32, bank: &Bank, params: &(impl Params + ?Sized), setter: &ParamSetter, theme: &Theme) {
    let area = ui.max_rect();
    let origin = Pos2::new(area.right() - WIDTH, ui.cursor().top());
    picker(ui, origin, height, bank, params, setter, theme);
}
