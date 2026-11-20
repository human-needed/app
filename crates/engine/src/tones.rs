// SPDX-License-Identifier: FSL-1.1-ALv2
//! The puck's sounds, rendered for the computer's speakers (D-059). A port of the firmware's
//! `audio.cpp` (2.0.x): the same notes, decay curve, "rich" voice and mix, so a sound is the
//! same tune on the puck and on the PC.

/// One note: frequency (Hz), start and length (ms).
type Note = (u16, u16, u16);

/// (rich voice, notes). Same order and names as the firmware's `SOUNDS`.
const SOUNDS: &[(&str, bool, &[Note])] = &[
    ("ding", false, &[(1568, 0, 500)]),
    ("chime", false, &[(1319, 0, 350), (1047, 120, 700)]),
    ("rising", false, &[(784, 0, 300), (1175, 180, 700)]),
    (
        "arpeggio",
        false,
        &[(1047, 0, 250), (1319, 110, 250), (1568, 220, 600)],
    ),
    ("bell", false, &[(880, 0, 1200)]),
    (
        "beeps",
        false,
        &[(1760, 0, 90), (1760, 160, 90), (1760, 320, 90)],
    ),
    ("bloop", false, &[(523, 0, 120), (1047, 70, 250)]),
    (
        "bright",
        true,
        &[
            (784, 0, 450),
            (1047, 180, 450),
            (1319, 360, 450),
            (1047, 620, 1100),
            (1319, 680, 1100),
            (1568, 740, 1100),
        ],
    ),
    (
        "clock",
        true,
        &[
            (1319, 0, 700),
            (1175, 400, 700),
            (1047, 800, 700),
            (784, 1200, 1500),
        ],
    ),
    ("doorbell", true, &[(1319, 0, 900), (1047, 450, 1400)]),
    (
        "fanfare",
        true,
        &[
            (784, 0, 250),
            (784, 150, 250),
            (784, 300, 250),
            (1047, 450, 1200),
            (1319, 500, 1200),
            (1568, 550, 1200),
        ],
    ),
    (
        "ripple",
        true,
        &[
            (2093, 0, 400),
            (1760, 100, 400),
            (1568, 200, 400),
            (1319, 300, 400),
            (1175, 400, 400),
            (1047, 500, 1000),
            (1568, 560, 1000),
        ],
    ),
    (
        "call",
        true,
        &[
            (1319, 0, 450),
            (1760, 200, 700),
            (1319, 800, 450),
            (1760, 1000, 1100),
        ],
    ),
    (
        "lullaby",
        true,
        &[
            (1319, 0, 450),
            (1175, 200, 450),
            (1047, 400, 450),
            (784, 600, 450),
            (880, 800, 450),
            (1047, 1000, 600),
            (784, 1400, 1400),
            (1047, 1460, 1400),
            (1319, 1520, 1400),
        ],
    ),
    (
        "harp",
        true,
        &[
            (1047, 0, 600),
            (1319, 70, 600),
            (1568, 140, 600),
            (2093, 210, 600),
            (2637, 280, 600),
            (1047, 500, 1300),
            (1319, 540, 1300),
            (1568, 580, 1300),
        ],
    ),
];

/// Silence between repeats (ms), as on the puck.
const REPEAT_GAP_MS: u32 = 350;

/// Shaped decay of a note `k` samples in (0..1), as the firmware's `noteEnv`.
fn env(k: u32, len: u32) -> f32 {
    let e = 1.0 - k as f32 / len as f32;
    e * e
}

/// Renders `name` as mono samples (-1..1) at `rate` Hz: `gain` 0..1, played `repeat` times.
/// `None` for an unknown sound or "none".
pub fn render(name: &str, gain: f32, repeat: u8, rate: u32) -> Option<Vec<f32>> {
    let (_, rich, notes) = SOUNDS.iter().find(|(n, _, _)| *n == name)?;
    let ms = |m: u32| (m as u64 * rate as u64 / 1000) as u32;
    let end = notes.iter().map(|(_, s, l)| *s as u32 + *l as u32).max()?;
    let one = ms(end + REPEAT_GAP_MS);
    // Mix scale: three full notes never clip; more than three at once are scaled to fit.
    let peak = notes
        .iter()
        .map(|(_, at, _)| {
            notes
                .iter()
                .map(|(_, s, l)| {
                    let k = *at as i32 - *s as i32;
                    if k < 0 || k >= *l as i32 {
                        0.0
                    } else {
                        env(k as u32, *l as u32)
                    }
                })
                .sum::<f32>()
        })
        .fold(0.0f32, f32::max);
    let norm = if peak > 3.0 { 1.0 / peak } else { 1.0 / 3.0 };
    let attack = rate / 250; // 4 ms
    let mut out = vec![0f32; one as usize];
    for &(hz, start, len) in notes.iter() {
        let (s0, n) = (ms(start as u32), ms(len as u32));
        let w = std::f32::consts::TAU * hz as f32 / rate as f32;
        for k in 0..n {
            let i = (s0 + k) as usize;
            if i >= out.len() {
                break;
            }
            let mut e = env(k, n);
            let ph = w * k as f32;
            let s = if *rich {
                if k < attack {
                    e *= k as f32 / attack as f32;
                }
                (ph.sin() * 176.0 + (2.0 * ph).sin() * 53.0 + (3.0 * ph).sin() * 27.0) / 256.0
            } else {
                ph.sin()
            };
            out[i] += s * e;
        }
    }
    let g = norm * gain.clamp(0.0, 1.0);
    for v in out.iter_mut() {
        *v = (*v * g).clamp(-1.0, 1.0);
    }
    let reps = repeat.clamp(1, 5) as usize;
    let mut all = Vec::with_capacity(out.len() * reps);
    for _ in 0..reps {
        all.extend_from_slice(&out);
    }
    Some(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_firmware_sound_renders() {
        for name in hn_sim::puck::SOUNDS.iter().filter(|s| **s != "none") {
            let s = render(name, 1.0, 1, 48_000).unwrap_or_else(|| panic!("{name}"));
            let peak = s.iter().fold(0f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.05 && peak <= 1.0, "{name} peak {peak}");
        }
        assert!(render("none", 1.0, 1, 48_000).is_none());
    }

    #[test]
    fn length_gain_and_repeat() {
        // ding: 500 ms + 350 ms gap.
        let one = render("ding", 1.0, 1, 48_000).unwrap();
        assert_eq!(one.len(), 48 * 850);
        assert_eq!(render("ding", 1.0, 3, 48_000).unwrap().len(), 3 * one.len());
        let quiet = render("ding", 0.5, 1, 48_000).unwrap();
        let p = |s: &[f32]| s.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!((p(&quiet) - p(&one) / 2.0).abs() < 1e-3);
        assert!(render("ding", 0.0, 1, 48_000)
            .unwrap()
            .iter()
            .all(|v| *v == 0.0));
    }
}
