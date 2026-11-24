//! Clip clocks are separate from the native action/variable snapshots. Keep
//! them by value so no behavior graph pointers survive an animation update.
use super::{History, WINDOW};
use crate::{read_i32, read_ptr};
use std::{ffi::CStr, ptr, sync::Arc};

const CLIP_GENERATOR_VT: usize = 0x2e26d50;
type Clock = (Box<[u8]>, f32);
pub(super) type Clips = Arc<[Clock]>;

unsafe fn visit_clips(actor: usize, base: usize, mut visit: impl FnMut(&[u8], usize, usize, f32)) {
    let Some(graph) = crate::ram_stagger::graph(actor, base) else {
        return;
    };
    let list = read_ptr(graph + 0xe0);
    if list == 0 {
        return;
    }
    let slots = read_ptr(list);
    let count = read_i32(list + 8);
    if slots == 0 || !(1..=2048).contains(&count) {
        return;
    }
    for index in 0..count as usize {
        let info = read_ptr(slots + index * 8);
        if info == 0 {
            continue;
        }
        let node = read_ptr(info + 8);
        if node == 0 || read_ptr(node) != base + CLIP_GENERATOR_VT {
            continue;
        }
        let name = read_ptr(node + 0x48) & !1;
        let control = read_ptr(node + 0xd8);
        if name == 0 || control == 0 {
            continue;
        }
        let time = ptr::read_unaligned((node + 0x140) as *const f32);
        if time.is_finite() && time >= 0. {
            visit(
                CStr::from_ptr(name as *const i8).to_bytes(),
                node,
                control,
                time,
            );
        }
    }
}

pub(super) unsafe fn capture(actor: usize, base: usize) -> Clips {
    let mut samples = Vec::new();
    visit_clips(actor, base, |name, _, _, time| {
        samples.push((Box::from(name), time))
    });
    samples.into()
}

#[derive(Debug)]
pub(super) struct Track(Vec<(f32, Clips)>);

impl Track {
    pub(super) fn from_history(history: &History, end: f64) -> Self {
        let start = end - f64::from(WINDOW);
        let first = history
            .frames
            .partition_point(|f| f.time <= start)
            .saturating_sub(1);
        Self(
            history
                .frames
                .iter()
                .skip(first)
                .take_while(|f| f.time <= end + 1e-7)
                .map(|f| ((f.time - start) as f32, f.clips.clone()))
                .collect(),
        )
    }

    pub(super) unsafe fn align(&self, actor: usize, base: usize, elapsed: f32) -> bool {
        let next = self.0.partition_point(|(t, _)| *t <= elapsed);
        let Some((t, clips)) = self.0.get(next.saturating_sub(1)) else {
            return true;
        };
        let after = self.0.get(next);
        // While the final pose is held, even sub-frame drift is visible.
        let tolerance = if elapsed >= WINDOW { 0. } else { 0.045 };
        let mut matched = 0;
        let mut corrected = false;
        visit_clips(actor, base, |name, node, control, current| {
            let Some(expected) = phase_at(name, *t, clips, after, elapsed) else {
                return;
            };
            matched += 1;
            if (current - expected).abs() > tolerance {
                // These are the two playback clocks committed together by
                // hkbClipGenerator::update, including the sampled animation control.
                corrected = true;
                ptr::write_unaligned((node + 0x140) as *mut f32, expected);
                ptr::write_unaligned((control + 0x18) as *mut f32, expected);
            }
        });
        matched > 0 && !corrected
    }
}

fn phase_at(
    name: &[u8],
    t: f32,
    clips: &[Clock],
    next: Option<&(f32, Clips)>,
    elapsed: f32,
) -> Option<f32> {
    fn lookup(name: &[u8], clips: &[Clock]) -> Option<f32> {
        let mut matches = clips.iter().filter(|(key, _)| key.as_ref() == name);
        let (_, value) = matches.next()?;
        matches.next().is_none().then_some(*value)
    }
    let from = lookup(name, clips)?;
    if let Some((end, clocks)) = next {
        if let Some(to) = lookup(name, clocks) {
            let dt = end - t;
            // A backward jump is a loop/restart, not reverse movement across the clip.
            if dt > 0. && to >= from {
                return Some(from + (to - from) * ((elapsed - t) / dt).clamp(0., 1.));
            }
        }
    }
    Some(from)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn clips(values: &[(&[u8], f32)]) -> Clips {
        values
            .iter()
            .map(|(name, t)| (Box::from(*name), *t))
            .collect::<Vec<_>>()
            .into()
    }
    #[test]
    fn retains_mid_swing_offset_and_interpolates_recorded_speed() {
        let previous = clips(&[(b"swing", 1.65)]);
        let next = (0.025, clips(&[(b"swing", 1.75)]));
        let at = phase_at(b"swing", -0.025, &previous, Some(&next), 0.).unwrap();
        assert!((at - 1.70).abs() < 1e-5);
    }
    #[test]
    fn loops_and_unrelated_layers_do_not_inherit_another_phase() {
        let previous = clips(&[(b"swing", 1.65), (b"foot", 0.1), (b"foot", 0.6)]);
        let next = (0.03, clips(&[(b"swing", 0.01), (b"finish", 0.02)]));
        assert_eq!(
            phase_at(b"swing", 0., &previous, Some(&next), 0.02),
            Some(1.65)
        );
        assert_eq!(phase_at(b"foot", 0., &previous, Some(&next), 0.02), None);
        assert_eq!(phase_at(b"finish", 0., &previous, Some(&next), 0.02), None);
    }
}
