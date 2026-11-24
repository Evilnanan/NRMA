//! Sample the locked target once in the one-handed running-heavy startup.
use crate::{ptr, read_i32, read_ptr};
use std::sync::Mutex;

pub const QUERY: i32 = 990157;
// Native 52347D loads the manager for the target-selection update at 757D20.
const TARGET_MANAGER: usize = 0x3c13c48;
const STARTUP_TRAVEL: f32 = 2.3905556;
const NORMAL: f32 = 1.25;
const FULL: f32 = 2.5;
// At the first active hit (20/30s) the root has moved 3.719m * 1.25.
// Allow roughly 1.6m for the blade's forward reach.
const NORMAL_REACH: f32 = 6.25;
const UNIT: f32 = 1024.;

#[derive(Default)]
struct Startup(Option<(usize, u64, i32)>);

impl Startup {
    fn sample(&mut self, actor: usize, handle: u64, distance: impl FnOnce() -> Option<f32>) -> i32 {
        if let Some((a, h, scale)) = self.0 {
            if (a, h) == (actor, handle) {
                return scale;
            }
        }
        let scale = distance().filter(|d| d.is_finite()).map_or(FULL, |d| {
            (NORMAL + (d - NORMAL_REACH).max(0.) / STARTUP_TRAVEL).min(FULL)
        });
        let encoded = (scale * UNIT).round() as i32;
        self.0 = Some((actor, handle, encoded));
        encoded
    }
}
static STARTUP: Mutex<Startup> = Mutex::new(Startup(None));

fn horizontal(origin: [f32; 4], point: [f32; 3]) -> f32 {
    (origin[0] - point[0]).hypot(origin[2] - point[2])
}

// 759105 traverses this active list; 7592A7 marks its selected part.
unsafe fn selected_lock_distance(manager: usize, origin: [f32; 4]) -> Option<f32> {
    if manager == 0 || ptr::read((manager + 0x2830) as *const u8) == 0 {
        return None;
    }
    let mut node = read_ptr(manager + 0x10);
    for _ in 0..64 {
        // native lock-point pool capacity
        if node == 0 {
            break;
        }
        if ptr::read((node + 0x98) as *const u8) & 0x20 != 0 {
            let point = ptr::read_unaligned(node as *const [f32; 3]);
            let distance = horizontal(origin, point);
            return distance.is_finite().then_some(distance);
        }
        node = read_ptr(node + 0x88);
    }
    None
}

unsafe fn distance(actor: usize, base: usize) -> Option<f32> {
    if !crate::spatial_ready(actor) {
        return None;
    }
    selected_lock_distance(
        read_ptr(base + TARGET_MANAGER),
        crate::position(actor, base),
    )
}

pub unsafe fn scale(actor: usize, base: usize) -> i32 {
    if !super::dash_ready(actor, base) {
        return 0;
    }
    let mut startup = STARTUP.lock().unwrap_or_else(|e| e.into_inner());
    startup.sample(actor, read_ptr(actor + 8) as u64, || distance(actor, base))
}

pub unsafe fn update(actor: usize) {
    let mut startup = STARTUP.lock().unwrap_or_else(|e| e.into_inner());
    let Some((a, h, _)) = startup.0 else {
        return;
    };
    // Same action mask as native HKS env(301, 28) at 3F8604. Observe it even
    // when a cancel goes to a state which never requests travel magnification.
    let modules = read_ptr(actor + 0x60);
    let flags = if modules == 0 {
        0
    } else {
        read_ptr(modules + 0xc8)
    };
    if actor != a
        || read_ptr(actor + 8) as u64 != h
        || flags == 0
        || read_i32(flags + 0x28) & (1 << 28) == 0
    {
        startup.0 = None;
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_mid_far_unlocked_and_repeated_samples() {
        for (gap, expected) in [
            (Some(2.), 1280),
            (Some(NORMAL_REACH), 1280),
            (Some(NORMAL_REACH + STARTUP_TRAVEL * 0.5), 1792),
            (Some(24.), 2560),
            (None, 2560),
        ] {
            let mut s = Startup::default();
            assert_eq!(s.sample(3, 17, || gap), expected);
            assert_eq!(s.sample(3, 17, || panic!("resampled mid-stride")), expected);
            assert_eq!(s.sample(3, 18, || Some(0.)), 1280);
        }
    }

    #[test]
    fn switching_lock_from_far_part_to_near_part_changes_distance() {
        unsafe {
            let mut manager = vec![0usize; 0x2840 / 8];
            let mut near = [0usize; 0xa0 / 8];
            let mut far = [0usize; 0xa0 / 8];
            near[0] = 4f32.to_bits() as usize;
            far[0] = 10f32.to_bits() as usize | ((200f32.to_bits() as usize) << 32);
            near[0x88 / 8] = far.as_mut_ptr() as usize;
            manager[0x10 / 8] = near.as_mut_ptr() as usize;
            let addr = manager.as_mut_ptr() as usize;
            far[0x98 / 8] = 0x20;
            assert_eq!(selected_lock_distance(addr, [0.; 4]), None);
            manager[0x2830 / 8] = 1;
            assert_eq!(selected_lock_distance(addr, [0.; 4]), Some(10.));
            far[0x98 / 8] = 0;
            assert_eq!(selected_lock_distance(addr, [0.; 4]), None);
            near[0x98 / 8] = 0x20;
            assert_eq!(selected_lock_distance(addr, [0.; 4]), Some(4.));
        }
    }
}
