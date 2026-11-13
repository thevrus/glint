//! Persistent app config (one JSON file in the app config dir): last zone colours,
//! user presets, adaptive settings.
use crate::adaptive::Adaptive;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default)]
    pub icon: String,
    pub front_on: bool,
    pub brightness: u16,
    pub temperature: u16,
    pub back_on: bool,
    pub back_brightness: u8,
    pub zones: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Config {
    pub zones: Option<Vec<String>>,
    pub presets: Vec<Preset>,
    pub adaptive: Adaptive,
}

pub struct Store {
    path: PathBuf,
    pub cfg: Config,
}

impl Store {
    pub fn load(dir: PathBuf) -> Store {
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");
        let cfg = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Store { path, cfg }
    }

    pub fn save(&self) {
        let tmp = self.path.with_extension("json.tmp");
        if let Ok(s) = serde_json::to_string_pretty(&self.cfg) {
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }
}

pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

pub fn to_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

pub fn default_zones() -> Vec<String> {
    vec!["#ffffff".to_string(); 7]
}

fn ramp(from: [u8; 3], to: [u8; 3]) -> Vec<String> {
    (0..7)
        .map(|i| {
            let t = i as f32 / 6.0;
            let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
            to_hex([m(from[0], to[0]), m(from[1], to[1]), m(from[2], to[2])])
        })
        .collect()
}

pub fn builtins() -> Vec<Preset> {
    let p = |id: &str, name: &str, icon: &str, fo, br, t, bo, bb, zones: Vec<String>| Preset {
        id: id.into(),
        name: name.into(),
        builtin: true,
        icon: icon.into(),
        front_on: fo,
        brightness: br,
        temperature: t,
        back_on: bo,
        back_brightness: bb,
        zones,
    };
    vec![
        p("daylight", "Daylight", "sun", true, 240, 5600, false, 30, vec!["#fff4e8".into(); 7]),
        p("warm", "Warm", "bulb", true, 160, 3200, false, 30, vec!["#ffb46b".into(); 7]),
        p("evening", "Soft Evening", "moon", true, 70, 2700, true, 25, ramp([0xff, 0x8a, 0x3d], [0xff, 0xc2, 0x7a])),
        p("call", "Video Call", "video", true, 280, 4800, true, 35, vec!["#ffe6cc".into(); 7]),
        p("focus", "Cool Focus", "focus", true, 330, 6200, false, 30, vec!["#e8f1ff".into(); 7]),
        p("sunset", "Sunset", "sunset", true, 90, 2900, true, 70, ramp([0xd6, 0x22, 0x2e], [0xff, 0xa8, 0x3a])),
    ]
}
