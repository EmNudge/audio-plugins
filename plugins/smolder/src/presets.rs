//! Factory and user presets.
//!
//! A preset is a list of `(parameter id, plain value)` pairs. Applying one sets every parameter:
//! the ones the preset names to its values, everything else to its default, so presets never
//! inherit settings from whatever was loaded before. User presets are plain text files, one
//! `id=value` line per parameter.

use nih_plug::prelude::*;
use std::fs;
use std::path::PathBuf;

use crate::dsp::{Bands, LfoShape, Model, ModSource, ModTarget, Oversampling};
use crate::SmolderParams;

#[derive(Clone)]
pub struct Preset {
    pub name: String,
    pub values: Vec<(String, f32)>,
}

fn preset(name: &str, values: &[(&str, f32)]) -> Preset {
    Preset { name: name.into(), values: values.iter().map(|(k, v)| (k.to_string(), *v)).collect() }
}

fn m(model: Model) -> f32 {
    model.to_index() as f32
}

fn db(db: f32) -> f32 {
    util::db_to_gain(db)
}

pub fn factory() -> Vec<Preset> {
    let multi = ("bands", Bands::Multi.to_index() as f32);
    let lfo = ("modsrc", ModSource::Lfo.to_index() as f32);
    let env = ("modsrc", ModSource::Envelope.to_index() as f32);
    let target = |t: ModTarget| ("modtgt", t.to_index() as f32);
    let x4 = ("oversamp", Oversampling::X4.to_index() as f32);
    vec![
        preset("Init", &[]),
        preset("Warm Tube", &[("model", m(Model::Tube)), ("drive", 8.0), ("bias", 0.25), ("tone", -0.2)]),
        preset(
            "Tape Glue",
            &[("model", m(Model::Tape)), ("drive", 6.0), ("tone", -0.3), ("lowcut", 30.0), ("mix", 0.7)],
        ),
        preset("Diode Bite", &[("model", m(Model::Diode)), ("drive", 18.0), ("lowcut", 120.0), ("tone", 0.2)]),
        preset(
            "Fuzz Box",
            &[("model", m(Model::Crunch)), ("drive", 30.0), ("bias", 0.3), ("lowcut", 80.0), ("tone", -0.3), x4],
        ),
        preset("Wavefolder Lead", &[("model", m(Model::Fold)), ("drive", 22.0), ("bias", 0.15), x4]),
        preset(
            "Bit Rot",
            &[("model", m(Model::Crush)), ("drive", 6.0), ("tone", 0.3), ("oversamp", Oversampling::Off.to_index() as f32)],
        ),
        preset("Even Rectifier", &[("model", m(Model::Rectify)), ("drive", 16.0), ("bias", 0.3), x4]),
        preset("Drum Crush", &[("model", m(Model::Clip)), ("drive", 14.0), ("lowcut", 90.0), ("mix", 0.5)]),
        preset(
            "Bass Warmth",
            &[
                multi,
                ("drive", 8.0),
                ("xover1", 180.0),
                ("xover2", 2500.0),
                ("model_1", m(Model::Tape)),
                ("drive_1", 6.0),
                ("model_2", m(Model::Tube)),
                ("model_3", m(Model::Tube)),
                ("drive_3", -12.0),
                ("level_3", db(-3.0)),
            ],
        ),
        preset(
            "Vocal Air",
            &[
                multi,
                ("drive", 10.0),
                ("xover1", 250.0),
                ("xover2", 3500.0),
                ("model_1", m(Model::Tube)),
                ("drive_1", -18.0),
                ("level_1", db(-3.0)),
                ("model_2", m(Model::Tube)),
                ("model_3", m(Model::Tape)),
                ("drive_3", 6.0),
                ("mix", 0.6),
            ],
        ),
        preset(
            "Master Heat",
            &[
                multi,
                ("drive", 4.0),
                ("xover1", 150.0),
                ("xover2", 5000.0),
                ("model_1", m(Model::Tape)),
                ("model_2", m(Model::Tape)),
                ("model_3", m(Model::Tape)),
                ("drive_3", -6.0),
                ("mix", 0.5),
            ],
        ),
        preset(
            "Wobble Drive",
            &[
                ("model", m(Model::Tube)),
                ("drive", 14.0),
                lfo,
                target(ModTarget::Drive),
                ("modamt", 0.5),
                ("modrate", 2.0),
            ],
        ),
        preset(
            "Envelope Grit",
            &[("model", m(Model::Crunch)), ("drive", 10.0), env, target(ModTarget::Drive), ("modamt", 0.7)],
        ),
        preset(
            "Slow Fold",
            &[
                ("model", m(Model::Fold)),
                ("drive", 18.0),
                lfo,
                target(ModTarget::Bias),
                ("modamt", 0.6),
                ("modrate", 0.4),
                ("modshape", LfoShape::Triangle.to_index() as f32),
                x4,
            ],
        ),
    ]
}

/// Sets every parameter to the preset's value for it, or to its default if it has none.
pub fn apply(params: &SmolderParams, setter: &ParamSetter, preset: &Preset) {
    for (id, ptr, _) in params.param_map() {
        let normalized = unsafe {
            match preset.values.iter().find(|(k, _)| *k == id) {
                Some((_, plain)) => ptr.preview_normalized(*plain),
                None => ptr.default_normalized_value(),
            }
        };
        unsafe {
            setter.raw_context.raw_begin_set_parameter(ptr);
            setter.raw_context.raw_set_parameter_normalized(ptr, normalized);
            setter.raw_context.raw_end_set_parameter(ptr);
        }
    }
}

pub fn capture(params: &SmolderParams, name: &str) -> Preset {
    let values = params
        .param_map()
        .into_iter()
        .map(|(id, ptr, _)| (id, unsafe { ptr.unmodulated_plain_value() }))
        .collect();
    Preset { name: name.into(), values }
}

fn user_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/Smolder/Presets"))
}

fn serialize(values: &[(String, f32)]) -> String {
    values.iter().map(|(k, v)| format!("{k}={v}\n")).collect()
}

/// Lines that aren't `id=number` are skipped.
fn parse(text: &str) -> Vec<(String, f32)> {
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((k.to_string(), v.parse().ok()?))
        })
        .collect()
}

/// Characters that are fine in a file name on every platform.
fn file_stem(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || " -_().".contains(c) { c } else { '_' }).collect()
}

pub fn save_user(preset: &Preset) -> std::io::Result<()> {
    let dir = user_dir().ok_or_else(|| std::io::Error::other("no home directory"))?;
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(format!("{}.smolder", file_stem(&preset.name))), serialize(&preset.values))
}

/// User presets, sorted by name. Unreadable files are skipped.
pub fn load_user() -> Vec<Preset> {
    let Some(dir) = user_dir() else { return vec![] };
    let Ok(entries) = fs::read_dir(dir) else { return vec![] };
    let mut presets: Vec<Preset> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "smolder"))
        .filter_map(|e| {
            let name = e.path().file_stem()?.to_string_lossy().into_owned();
            Some(Preset { name, values: parse(&fs::read_to_string(e.path()).ok()?) })
        })
        .collect();
    presets.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    presets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_presets_only_name_real_parameters() {
        let params = SmolderParams::default();
        let ids: Vec<String> = params.param_map().into_iter().map(|(id, _, _)| id).collect();
        for p in factory() {
            for (k, _) in &p.values {
                assert!(ids.contains(k), "preset {:?} names unknown parameter {k:?} (known: {ids:?})", p.name);
            }
        }
    }

    #[test]
    fn factory_preset_values_are_in_range() {
        let params = SmolderParams::default();
        let map = params.param_map();
        for p in factory() {
            for (k, v) in &p.values {
                let (_, ptr, _) = map.iter().find(|(id, _, _)| id == k).unwrap();
                let back = unsafe { ptr.preview_plain(ptr.preview_normalized(*v)) };
                let tolerance = 1e-3 * v.abs().max(1.0);
                assert!((back - v).abs() < tolerance, "preset {:?}: {k} = {v} is out of range (clamps to {back})", p.name);
            }
        }
    }

    #[test]
    fn user_presets_round_trip_through_text() {
        let values = vec![("drive".to_string(), 12.5), ("model".to_string(), 3.0)];
        assert_eq!(parse(&serialize(&values)), values);
        assert_eq!(parse("drive=1\nbad line\nmix=x\n"), vec![("drive".to_string(), 1.0)]);
        assert_eq!(file_stem("a/b: c"), "a_b_ c");
    }
}
