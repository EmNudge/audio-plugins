//! Factory kits. Each lists only what it changes from the defaults.
//! Per-drum ids carry the drum's number: `tune_1` is the kick, `_2` snare, `_3` low tom, `_4` high tom.
//! Plates (`mtune`, `mdecay`, `mtone`, `mlevel`) are `_1` hat, `_2` open hat, `_3` crash, `_4` ride.

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset};

use crate::Tympan;

pub static BANK: Bank = Bank {
    class_id: <Tympan as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &["level"],
    presets: &[Preset { name: "Init", set: &[] }],
};
