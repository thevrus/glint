//! Animated back-light modes. Each is a pure function from time to seven zone colours, so the
//! worker that plays them only has to ask for the frame at "now" and write it.
use crate::store::ZONES;
use std::f32::consts::TAU;

pub type Rgb = [u8; 3];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effect {
    Rainbow,
    Aurora,
    Ocean,
    Ember,
    Breathe,
}

impl Effect {
    pub const ALL: [Effect; 5] = [Effect::Rainbow, Effect::Aurora, Effect::Ocean, Effect::Ember, Effect::Breathe];

    pub fn id(self) -> &'static str {
        match self {
            Effect::Rainbow => "rainbow",
            Effect::Aurora => "aurora",
            Effect::Ocean => "ocean",
            Effect::Ember => "ember",
            Effect::Breathe => "breathe",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Effect::Rainbow => "Rainbow",
            Effect::Aurora => "Aurora",
            Effect::Ocean => "Ocean",
            Effect::Ember => "Ember",
            Effect::Breathe => "Breathe",
        }
    }

    pub fn parse(id: &str) -> Option<Effect> {
        Effect::ALL.into_iter().find(|e| e.id() == id)
    }
}

/// Hue in degrees, saturation and value 0..1.
fn hsv(h: f32, s: f32, v: f32) -> Rgb {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u8 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r, g, b].map(|k| ((k + m) * 255.0).round().clamp(0.0, 255.0) as u8)
}

/// A slow wave between 0 and 1: `rate` in radians a second, `shift` per zone.
fn wave(t: f32, rate: f32, i: usize, shift: f32) -> f32 {
    0.5 + 0.5 * (t * rate + i as f32 * shift).sin()
}

/// The colours of `effect` at `t` seconds. `base` is the user's own zones, which Breathe dims
/// and brightens; the others bring their own palette.
pub fn frame(effect: Effect, t: f32, base: &[Rgb; ZONES]) -> [Rgb; ZONES] {
    std::array::from_fn(|i| match effect {
        // the whole spectrum laid across the strip, sliding one full turn every 24 s
        Effect::Rainbow => hsv(360.0 * (t / 24.0 + i as f32 / ZONES as f32), 1.0, 1.0),
        // green through teal to violet, with a second slower wave of brightness
        Effect::Aurora => hsv(150.0 + 115.0 * wave(t, 0.25, i, 0.7), 0.85, 0.55 + 0.45 * wave(t, 0.4, i, -1.1)),
        Effect::Ocean => hsv(185.0 + 45.0 * wave(t, 0.3, i, 0.9), 0.9, 0.5 + 0.5 * wave(t, 0.5, i, -0.6)),
        // two waves that never line up read as a flicker, kept slow enough to stay calm
        Effect::Ember => {
            let n = 0.5 * wave(t, 1.3, i, 2.1) + 0.5 * wave(t, 2.3, i, 5.3);
            hsv(8.0 + 26.0 * n, 1.0, 0.45 + 0.55 * n)
        }
        // one breath every 5 s, never fully dark
        Effect::Breathe => {
            let k = 0.25 + 0.75 * (0.5 - 0.5 * (TAU * t / 5.0).cos());
            base[i].map(|v| (v as f32 * k).round() as u8)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: [Rgb; ZONES] = [[255, 180, 107]; ZONES];

    #[test]
    fn ids_round_trip() {
        for e in Effect::ALL {
            assert_eq!(Effect::parse(e.id()), Some(e));
        }
        assert_eq!(Effect::parse("disco"), None);
    }

    #[test]
    fn hsv_hits_the_primaries() {
        assert_eq!(hsv(0.0, 1.0, 1.0), [255, 0, 0]);
        assert_eq!(hsv(120.0, 1.0, 1.0), [0, 255, 0]);
        assert_eq!(hsv(240.0, 1.0, 1.0), [0, 0, 255]);
        assert_eq!(hsv(360.0, 1.0, 1.0), [255, 0, 0]);
        assert_eq!(hsv(77.0, 0.0, 0.5), [128, 128, 128]);
    }

    #[test]
    fn rainbow_spans_the_strip_and_repeats() {
        let f = frame(Effect::Rainbow, 3.0, &BASE);
        for i in 0..ZONES {
            for j in i + 1..ZONES {
                assert_ne!(f[i], f[j]);
            }
        }
        assert_eq!(f, frame(Effect::Rainbow, 27.0, &BASE));
    }

    #[test]
    fn breathe_only_dims_the_users_colours() {
        for step in 0..100 {
            let f = frame(Effect::Breathe, step as f32 * 0.07, &BASE);
            assert!(f.iter().all(|c| (0..3).all(|k| c[k] <= BASE[0][k] && c[k] >= BASE[0][k] / 5)), "{f:?}");
        }
    }

    /// No effect may jump: between two fade ticks (80 ms) a channel moves by a small step.
    #[test]
    fn every_effect_moves_in_small_steps() {
        for e in Effect::ALL {
            let mut worst = 0u8;
            for step in 0..1500 {
                let t = step as f32 * 0.08;
                let (a, b) = (frame(e, t, &BASE), frame(e, t + 0.08, &BASE));
                for i in 0..ZONES {
                    for k in 0..3 {
                        worst = worst.max(a[i][k].abs_diff(b[i][k]));
                    }
                }
            }
            assert!(worst <= 16, "{} jumps by {worst}", e.name());
        }
    }
}
