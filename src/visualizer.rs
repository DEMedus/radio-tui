//! Turns mpv loudness numbers into the EQ bar heights.

use serde_json::Value;

pub const BAR_COUNT: usize = 16;

#[derive(Debug, Clone, Copy)]
pub struct AudioLevels {
    pub rms: f32,
    pub peak: f32,
    pub crest: f32,
    pub brightness: f32,
}

#[derive(Debug)]
pub struct Visualizer {
    bars: [f32; BAR_COUNT],
    prev_peak: f32,
    agc_peak: f32,
    tick: u32,
}

impl Default for Visualizer {
    fn default() -> Self {
        Self {
            bars: [0.0; BAR_COUNT],
            prev_peak: 0.0,
            agc_peak: 0.2,
            tick: 0,
        }
    }
}

impl Visualizer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bars(&self) -> &[f32; BAR_COUNT] {
        &self.bars
    }

    pub fn energy(&self) -> f32 {
        self.bars.iter().sum::<f32>() / BAR_COUNT as f32
    }

    pub fn tick(&self) -> u32 {
        self.tick
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn decay(&mut self) {
        for bar in &mut self.bars {
            *bar *= 0.72;
            if *bar < 0.01 {
                *bar = 0.0;
            }
        }
        self.prev_peak *= 0.85;
        self.agc_peak = (self.agc_peak * 0.92).max(0.08);
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn push_levels(&mut self, levels: AudioLevels) {
        let onset = (levels.peak - self.prev_peak).max(0.0);
        self.prev_peak = levels.peak;
        self.tick = self.tick.wrapping_add(1);

        let instant = levels.peak.max(levels.rms);
        if instant > self.agc_peak {
            self.agc_peak += (instant - self.agc_peak) * 0.22;
        } else {
            self.agc_peak += (instant - self.agc_peak) * 0.01;
        }
        self.agc_peak = self.agc_peak.max(0.08);

        // Compressed radio sits in a narrow loudness band. Treat current
        // level relative to recent peaks and expand that into the full meter.
        let norm = (levels.rms / self.agc_peak).clamp(0.0, 1.2);
        let drive = ((norm - 0.42) / 0.58).clamp(0.0, 1.0);
        let gate = smoothstep(levels.rms, 0.03, 0.14);
        let energy = (0.12 + 0.62 * drive) * gate;

        let mut targets = [0.0; BAR_COUNT];
        for (i, target) in targets.iter_mut().enumerate() {
            let x = i as f32 / (BAR_COUNT - 1) as f32;
            let shape = 0.68 - 0.50 * x.powf(0.8);
            let punch = onset * gaussian(x, 0.08, 0.22);
            let sizzle = levels.crest * levels.brightness * x.powf(1.45) * 0.42;
            let phase = self.tick as f32 * (0.85 + x * 2.2) + i as f32 * 1.7;
            let wobble = phase.sin() * 0.16 * (0.35 + 0.65 * energy) * (0.4 + 0.6 * x);
            *target = (shape * energy + punch + sizzle * gate + wobble).clamp(0.0, 1.0);
        }

        let mut smoothed = targets;
        for i in 0..BAR_COUNT {
            let left = if i == 0 { targets[i] } else { targets[i - 1] };
            let right = if i + 1 == BAR_COUNT {
                targets[i]
            } else {
                targets[i + 1]
            };
            smoothed[i] = 0.12 * left + 0.76 * targets[i] + 0.12 * right;
        }

        for (i, (bar, target)) in self.bars.iter_mut().zip(smoothed).enumerate() {
            let x = i as f32 / (BAR_COUNT - 1) as f32;
            let attack = 0.58 - x * 0.16;
            let release = 0.20 + x * 0.28;
            let rate = if target > *bar { attack } else { release };
            *bar += (target - *bar) * rate;
        }
    }
}

pub fn parse_af_metadata(data: &Value) -> Option<AudioLevels> {
    let map = data.as_object()?;
    let rms = db_to_level(number_field(map, "lavfi.astats.Overall.RMS_level")?);
    let peak = db_to_level(number_field(map, "lavfi.astats.Overall.Peak_level")?);
    let crest_raw = mean_present(&[
        number_field(map, "lavfi.astats.1.Crest_factor"),
        number_field(map, "lavfi.astats.2.Crest_factor"),
    ]);
    let zcr = mean_present(&[
        number_field(map, "lavfi.astats.1.Zero_crossings_rate"),
        number_field(map, "lavfi.astats.2.Zero_crossings_rate"),
    ]);
    Some(AudioLevels {
        rms,
        peak,
        crest: ((crest_raw - 1.3) / 3.0).clamp(0.0, 1.0),
        brightness: (zcr / 0.12).clamp(0.0, 1.0),
    })
}

pub fn db_to_level(db: f32) -> f32 {
    if !db.is_finite() {
        return 0.0;
    }
    const MIN_DB: f32 = -42.0;
    const MAX_DB: f32 = -3.0;
    let t = ((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0);
    t * t
}

fn smoothstep(value: f32, edge0: f32, edge1: f32) -> f32 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn gaussian(x: f32, center: f32, width: f32) -> f32 {
    let n = (x - center) / width;
    (-n * n).exp()
}

fn number_field(map: &serde_json::Map<String, Value>, key: &str) -> Option<f32> {
    match map.get(key)? {
        Value::String(text) => text.parse().ok(),
        Value::Number(num) => num.as_f64().map(|v| v as f32),
        _ => None,
    }
}

fn mean_present(values: &[Option<f32>]) -> f32 {
    let mut sum = 0.0;
    let mut count = 0.0;
    for value in values.iter().flatten() {
        if value.is_finite() {
            sum += *value;
            count += 1.0;
        }
    }
    if count == 0.0 {
        0.0
    } else {
        sum / count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn silence_maps_to_zero() {
        assert_eq!(db_to_level(f32::NEG_INFINITY), 0.0);
        assert!(db_to_level(-80.0) < 0.02);
    }

    #[test]
    fn loud_radio_is_midrange() {
        let level = db_to_level(-14.3);
        assert!(level > 0.35, "got {level}");
        assert!(level < 0.7, "got {level}");
    }

    #[test]
    fn parses_mpv_astats_payload() {
        let data = json!({
            "lavfi.astats.1.Peak_level": "-6.937818",
            "lavfi.astats.1.RMS_level": "-15.702371",
            "lavfi.astats.1.Crest_factor": "2.743012",
            "lavfi.astats.1.Zero_crossings_rate": "0.046875",
            "lavfi.astats.2.Peak_level": "-6.879997",
            "lavfi.astats.2.RMS_level": "-16.005601",
            "lavfi.astats.2.Crest_factor": "2.859435",
            "lavfi.astats.2.Zero_crossings_rate": "0.048611",
            "lavfi.astats.Overall.Peak_level": "-6.879997",
            "lavfi.astats.Overall.RMS_level": "-15.851340"
        });
        let levels = parse_af_metadata(&data).expect("should parse");
        assert!(levels.rms > 0.3 && levels.rms < 0.7, "rms={}", levels.rms);
        assert!(levels.peak > levels.rms);
        assert!(levels.crest > 0.3);
        assert!(levels.brightness > 0.2);
        assert!(levels.brightness < 0.6);
    }

    #[test]
    fn steady_loud_signal_uses_the_lower_half() {
        let mut vis = Visualizer::new();
        let levels = AudioLevels {
            rms: 0.5,
            peak: 0.62,
            crest: 0.35,
            brightness: 0.3,
        };
        for _ in 0..50 {
            vis.push_levels(levels);
        }
        let min = vis.bars().iter().copied().fold(f32::MAX, f32::min);
        let max = vis.bars().iter().copied().fold(0.0, f32::max);
        assert!(min < 0.45, "shortest bar should dip, min={min}");
        assert!(max < 0.85, "steady signal should leave headroom, max={max}");
        assert!(max - min > 0.12, "bars should not be a flat wall");
    }

    #[test]
    fn onset_lifts_bass_more_than_air() {
        let mut vis = Visualizer::new();
        vis.push_levels(AudioLevels {
            rms: 0.4,
            peak: 0.4,
            crest: 0.2,
            brightness: 0.2,
        });
        vis.push_levels(AudioLevels {
            rms: 0.55,
            peak: 0.95,
            crest: 0.7,
            brightness: 0.25,
        });
        let bass = vis.bars()[0] + vis.bars()[1];
        let air = vis.bars()[14] + vis.bars()[15];
        assert!(bass > air, "bass={bass} air={air}");
    }

    #[test]
    fn decay_drives_bars_to_zero() {
        let mut vis = Visualizer::new();
        vis.push_levels(AudioLevels {
            rms: 0.8,
            peak: 0.9,
            crest: 0.4,
            brightness: 0.4,
        });
        assert!(vis.bars().iter().any(|b| *b > 0.1));
        for _ in 0..40 {
            vis.decay();
        }
        assert!(vis.bars().iter().all(|b| *b < 0.02));
    }
}
