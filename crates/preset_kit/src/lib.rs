//! Factory presets shared by every plugin.
//!
//! A preset lists only the parameters it changes; everything else takes the plugin's default.
//! The same table drives three things: the `< preset >` picker in the editor, the "custom"
//! detection when a parameter moves, and `.vstpreset` files that hosts such as REAPER list in
//! their own preset menu.

use nih_plug::prelude::*;
use nih_plug::wrapper::state::{ParamValue, PluginState};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

pub mod ui;

/// A parameter value in plain units.
pub enum V {
    /// A float or integer parameter's plain value.
    F(f32),
    /// A gain parameter set in dB (these are stored as linear gain).
    Db(f32),
    B(bool),
    /// An enum parameter, by the variant's display name.
    Name(&'static str),
}

pub struct Preset {
    pub name: &'static str,
    /// `(parameter id, value)`.
    pub set: &'static [(&'static str, V)],
}

pub struct Bank {
    pub class_id: [u8; 16],
    pub version: &'static str,
    /// Parameters presets never touch, because they belong to the song and not the sound.
    pub keep: &'static [&'static str],
    pub presets: &'static [Preset],
}

/// Normalized values for the parameters of one preset, `(id, ptr, target)`.
type Targets = Vec<(String, ParamPtr, f32)>;

impl V {
    fn normalized(&self, ptr: ParamPtr) -> Option<f32> {
        // SAFETY: the pointers come from a `Params` object that outlives this call.
        unsafe {
            Some(match self {
                V::F(x) => ptr.preview_normalized(*x),
                V::Db(db) => ptr.preview_normalized(util::db_to_gain(*db)),
                V::B(b) => ptr.preview_normalized(if *b { 1.0 } else { 0.0 }),
                V::Name(name) => {
                    let steps = ptr.step_count()?;
                    (0..=steps)
                        .map(|i| i as f32 / steps.max(1) as f32)
                        .find(|n| ptr.normalized_value_to_string(*n, false) == *name)?
                }
            })
        }
    }
}

impl Bank {
    fn targets(&self, index: usize, params: &(impl Params + ?Sized)) -> Targets {
        let preset = &self.presets[index];
        params
            .param_map()
            .into_iter()
            .filter(|(id, ..)| !self.keep.contains(&id.as_str()))
            .map(|(id, ptr, _)| {
                // SAFETY: `ptr` belongs to `params`, which the caller holds.
                let target = match preset.set.iter().find(|(k, _)| *k == id) {
                    Some((_, v)) => v.normalized(ptr).unwrap_or_else(|| panic!("{}: bad value for {id}", preset.name)),
                    None => unsafe { ptr.default_normalized_value() },
                };
                (id, ptr, target)
            })
            .collect()
    }

    /// Set every parameter the preset covers, as one host gesture each. Unchanged ones are skipped.
    pub fn apply(&self, index: usize, setter: &ParamSetter, params: &(impl Params + ?Sized)) {
        for (_, ptr, target) in self.targets(index, params) {
            // SAFETY: as above.
            unsafe {
                if (ptr.unmodulated_normalized_value() - target).abs() < 1e-6 {
                    continue;
                }
                setter.raw_context.raw_begin_set_parameter(ptr);
                setter.raw_context.raw_set_parameter_normalized(ptr, target);
                setter.raw_context.raw_end_set_parameter(ptr);
            }
        }
    }

    /// The preset the parameters currently match, if any.
    pub fn current(&self, params: &(impl Params + ?Sized)) -> Option<usize> {
        (0..self.presets.len()).find(|&i| {
            // SAFETY: as above.
            self.targets(i, params).iter().all(|(_, ptr, target)| unsafe {
                (ptr.unmodulated_normalized_value() - target).abs() < 1e-3
            })
        })
    }

    /// The state nih-plug would save for this preset. Editor size is not part of it, so loading a
    /// preset does not resize the window.
    pub fn state(&self, index: usize, params: &(impl Params + ?Sized)) -> PluginState {
        let values: BTreeMap<String, ParamValue> = self
            .targets(index, params)
            .into_iter()
            .map(|(id, ptr, target)| {
                // SAFETY: as above.
                let plain = unsafe { ptr.preview_plain(target) };
                let value = match ptr {
                    ParamPtr::FloatParam(_) => ParamValue::F32(plain),
                    ParamPtr::BoolParam(_) => ParamValue::Bool(plain > 0.5),
                    ParamPtr::IntParam(_) | ParamPtr::EnumParam(_) => ParamValue::I32(plain.round() as i32),
                };
                (id, value)
            })
            .collect();
        PluginState { version: self.version.to_owned(), params: values, fields: BTreeMap::new() }
    }

    /// One `.vstpreset` per preset in `dir`.
    pub fn write_vstpresets(&self, dir: &Path, params: &(impl Params + ?Sized)) -> io::Result<()> {
        std::fs::create_dir_all(dir)?;
        for (i, preset) in self.presets.iter().enumerate() {
            let state = serde_json::to_vec(&self.state(i, params)).map_err(io::Error::other)?;
            std::fs::write(dir.join(format!("{}.vstpreset", preset.name)), vstpreset(self.class_id, &state))?;
        }
        Ok(())
    }

    /// For each plugin's tests: every id exists, every value is valid and in range, names are unique.
    pub fn validate(&self, params: &(impl Params + ?Sized)) -> Result<(), String> {
        let map = params.param_map();
        let mut names: Vec<&str> = self.presets.iter().map(|p| p.name).collect();
        names.sort();
        names.dedup();
        if names.len() != self.presets.len() {
            return Err("duplicate preset names".into());
        }
        for id in self.keep {
            if !map.iter().any(|(k, ..)| k == id) {
                return Err(format!("kept parameter {id} does not exist"));
            }
        }
        for preset in self.presets {
            for (id, value) in preset.set {
                let (_, ptr, _) = map
                    .iter()
                    .find(|(k, ..)| k == id)
                    .ok_or_else(|| format!("{}: no parameter {id}", preset.name))?;
                let n = value.normalized(*ptr).ok_or_else(|| format!("{}: {id} has an unusable value", preset.name))?;
                // SAFETY: as above.
                let (plain, back) = unsafe { (ptr.preview_plain(n), ptr.preview_normalized(ptr.preview_plain(n))) };
                let wanted = match value {
                    V::F(x) => Some(*x),
                    V::Db(db) => Some(util::db_to_gain(*db)),
                    V::B(_) | V::Name(_) => None,
                };
                if let Some(x) = wanted {
                    if (plain - x).abs() > 1e-2 * x.abs().max(1.0) {
                        return Err(format!("{}: {id} = {x} is outside its range (clamped to {plain})", preset.name));
                    }
                }
                if (back - n).abs() > 1e-3 {
                    return Err(format!("{}: {id} does not round-trip", preset.name));
                }
            }
        }
        Ok(())
    }
}

/// A VST3 preset file: header, the component state as one `Comp` chunk, then the chunk list.
/// All integers are little-endian.
pub fn vstpreset(class_id: [u8; 16], state: &[u8]) -> Vec<u8> {
    const HEADER: usize = 4 + 4 + 32 + 8;
    let mut out = Vec::with_capacity(HEADER + state.len() + 40);
    out.extend(b"VST3");
    out.extend(1i32.to_le_bytes());
    for byte in class_id {
        out.extend(format!("{byte:02X}").bytes());
    }
    out.extend(((HEADER + state.len()) as i64).to_le_bytes());
    out.extend(state);
    out.extend(b"List");
    out.extend(1i32.to_le_bytes());
    out.extend(b"Comp");
    out.extend((HEADER as i64).to_le_bytes());
    out.extend((state.len() as i64).to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vstpreset_layout() {
        let state = br#"{"x":1}"#;
        let file = vstpreset(*b"CalvinKilnSatur1", state);
        assert_eq!(&file[..4], b"VST3");
        assert_eq!(&file[8..40], b"43616C76696E4B696C6E536174757231");
        let list = i64::from_le_bytes(file[40..48].try_into().unwrap()) as usize;
        assert_eq!(&file[list..list + 4], b"List");
        assert_eq!(&file[list + 8..list + 12], b"Comp");
        let offset = i64::from_le_bytes(file[list + 12..list + 20].try_into().unwrap()) as usize;
        let size = i64::from_le_bytes(file[list + 20..list + 28].try_into().unwrap()) as usize;
        assert_eq!(&file[offset..offset + size], state);
        assert_eq!(file.len(), list + 28);
    }
}
