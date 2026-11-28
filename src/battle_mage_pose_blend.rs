//! Inertialize the local cast pose after Havok has composed both body layers.
use std::{
    sync::{
        atomic::{AtomicPtr, AtomicUsize, Ordering},
        Mutex,
    },
    time::Instant,
};

// hkbBehaviorGraph::generate, with its six-argument native call sites.
pub const RVA: usize = 0x15a1e00;
pub const PATTERN: &str = "E9 A2 AC 05 01 8B 45 00 C1 E0 04 48 8B 55 38 03 42 08 48 89 7C 24 F8";
pub static ORIGINAL: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());
static GRAPH: AtomicUsize = AtomicUsize::new(0);
static STATE: Mutex<Option<Transition>> = Mutex::new(None);

const DURATION: f32 = 0.16;
const MAX_WAIT: f32 = 0.10;
// c0000 pelvis, legs, spine, neck and casting arms. Leave Master, RootPos
// and all six foot IK targets to the native controllers.
const JOINTS: [usize; 20] = [
    8, 10, 11, 15, 30, 31, 35, 46, 47, 48, 50, 53, 56, 58, 61, 85, 94, 97, 99, 102,
];
const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

#[repr(C, align(16))]
#[derive(Clone, Copy, Default)]
struct Transform {
    position: [f32; 4],
    rotation: [f32; 4],
    scale: [f32; 4],
}

#[derive(Clone, Copy)]
struct Offset {
    position: [f32; 3],
    rotation: [f32; 4],
}

type Pose = [Transform; JOINTS.len()];
type Offsets = [Offset; JOINTS.len()];

struct Transition {
    graph: usize,
    phase: u8,
    requested: Instant,
    offsets: Option<(Instant, Offsets, bool)>,
}

pub fn arm(graph: usize, phase: u8) {
    if let Ok(mut state) = STATE.lock() {
        *state = Some(Transition {
            graph,
            phase,
            requested: Instant::now(),
            offsets: None,
        });
        GRAPH.store(graph, Ordering::Release);
    }
}

pub fn clear() {
    if let Ok(mut state) = STATE.lock() {
        *state = None;
        GRAPH.store(0, Ordering::Release);
    }
}

unsafe fn pose_track(output: usize) -> Option<*mut Transform> {
    if output == 0 {
        return None;
    }
    let master = crate::read_ptr(output);
    if master == 0 {
        return None;
    }
    let word = |offset| ((master + offset) as *const u16).read_unaligned();
    let dword = |offset| ((master + offset) as *const u32).read_unaligned() as usize;
    let size = dword(0);
    let tracks = dword(4);
    // Standard track 2 is the local-space pose (32-byte track descriptors).
    // The stock dense-pose consumer also checks track 2's flags at +0x60.
    if !(3..=64).contains(&tracks) || !(16 + tracks * 32..=0x100000).contains(&size) {
        return None;
    }
    let count = word(0x52) as usize;
    let offset = dword(0x54);
    let weight = f32::from_bits(dword(0x5c) as u32);
    if !(JOINTS[JOINTS.len() - 1] + 1..=256).contains(&count)
        || count > word(0x50) as usize
        || word(0x58) as usize != std::mem::size_of::<Transform>()
        || word(0x60) & 1 != 0
        || !(weight > 0.0 && weight.is_finite())
        || offset < 16 + tracks * 32
        || offset + count * std::mem::size_of::<Transform>() > size
        || (master + offset) & 15 != 0
    {
        return None;
    }
    Some((master + offset) as *mut Transform)
}

unsafe fn sample(track: *const Transform) -> Pose {
    JOINTS.map(|joint| track.add(joint).read())
}

fn unit(q: [f32; 4]) -> Option<[f32; 4]> {
    let length = q.iter().map(|x| x * x).sum::<f32>();
    (length.is_finite() && length > 0.25).then(|| q.map(|x| x / length.sqrt()))
}

fn multiply(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [x, y, z, w] = a;
    let [i, j, k, r] = b;
    [
        w * i + x * r + y * k - z * j,
        w * j - x * k + y * r + z * i,
        w * k + x * j - y * i + z * r,
        w * r - x * i - y * j - z * k,
    ]
}

fn offset(before: Transform, after: Transform) -> Offset {
    let position = std::array::from_fn(|i| before.position[i] - after.position[i]);
    let position = if position.iter().all(|v| v.is_finite() && v.abs() < 0.75) {
        position
    } else {
        [0.0; 3]
    };
    let rotation = unit(before.rotation)
        .zip(unit(after.rotation))
        .map(|(from, [x, y, z, w])| multiply(from, [-x, -y, -z, w]))
        .unwrap_or(IDENTITY);
    let rotation = if rotation[3] < 0.0 {
        rotation.map(|v| -v)
    } else {
        rotation
    };
    Offset { position, rotation }
}

fn apply(mut native: Transform, delta: Offset, weight: f32) -> Transform {
    for (value, correction) in native.position.iter_mut().zip(delta.position) {
        *value += correction * weight;
    }
    let rotation =
        std::array::from_fn(|i| IDENTITY[i] + (delta.rotation[i] - IDENTITY[i]) * weight);
    if let Some(q) = unit(multiply(rotation, native.rotation)) {
        native.rotation = q;
    }
    native
}

unsafe fn active_layers(graph: usize, phase: u8, base: usize) -> Option<bool> {
    let query: unsafe extern "system" fn(usize, *const u8) -> u8 =
        std::mem::transmute(base + crate::ram_stagger::NODE_RVA);
    let states: [(&[u8], &[u8]); 2] = if phase == 2 {
        [(b"MagicFireRight2_Upper Script\0", b"MagicFireRight2_CMSG\0"),
         (b"MagicFireLeft2_Upper Script\0", b"MagicFireLeft2_CMSG\0")]
    } else {
        [(b"MagicFireRight3_Upper Script\0", b"MagicFireRight3_CMSG\0"),
         (b"MagicFireLeft3_Upper Script\0", b"MagicFireLeft3_CMSG\0")]
    };
    states.into_iter()
        .filter(|(upper, _)| query(graph, upper.as_ptr()) != 0)
        .map(|(_, lower)| query(graph, lower.as_ptr()) != 0)
        .max()
}

type Generate = unsafe extern "system" fn(usize, usize, usize, u8, f32, u8);

pub unsafe extern "system" fn generate(
    graph: usize,
    context: usize,
    output: usize,
    mode: u8,
    dt: f32,
    flags: u8,
) {
    let native: Generate = std::mem::transmute(ORIGINAL.load(Ordering::Acquire));
    let base = crate::BASE.load(Ordering::Acquire);
    if graph == 0
        || graph != GRAPH.load(Ordering::Acquire)
        || context == 0
        || !crate::ACTIVE.load(Ordering::Acquire)
    {
        native(graph, context, output, mode, dt, flags);
        return;
    }
    let character = crate::read_ptr(context);
    if character == 0
        || crate::read_ptr(character) != base + 0x2e1a2b8
        || crate::read_ptr(character + 0x98) != graph
    {
        native(graph, context, output, mode, dt, flags);
        return;
    }
    let previous = pose_track(crate::read_ptr(character + 0x38)).map(|track| sample(track));
    native(graph, context, output, mode, dt, flags);
    let Ok(mut state) = STATE.lock() else { return };
    let Some(transition) = state.as_mut().filter(|t| t.graph == graph) else {
        return;
    };
    let now = Instant::now();
    let layers = active_layers(graph, transition.phase, base);
    if transition.offsets.is_none() {
        let waiting = now.duration_since(transition.requested).as_secs_f32() <= MAX_WAIT;
        if waiting && layers.is_none() {
            return;
        }
        if waiting {
            if let Some(((previous, current), full_body)) = previous.zip(pose_track(output)).zip(layers) {
                transition.offsets = Some((
                    now,
                    std::array::from_fn(|i| offset(previous[i], current.add(JOINTS[i]).read())),
                    full_body,
                ));
            }
        }
    }
    if let Some((((started, corrections, full_body), track), still_full_body)) =
        transition.offsets.as_ref().zip(pose_track(output)).zip(layers)
    {
        let elapsed = now.duration_since(*started).as_secs_f32();
        if elapsed < DURATION {
            let t = elapsed / DURATION;
            let weight = 1.0 - t * t * (3.0 - 2.0 * t);
            for (&index, &correction) in JOINTS.iter().zip(corrections) {
                // Upper-body casts can begin while moving and finish once
                // stationary. Leave the independent lower track to Havok.
                if index <= 46 && !(*full_body && still_full_body) {
                    continue;
                }
                let joint = track.add(index);
                joint.write(apply(joint.read(), correction, weight));
            }
            return;
        }
    }
    *state = None;
    GRAPH.store(0, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decay_preserves_new_motion_without_editing_native_scale() {
        let source = Transform {
            position: [0., 1., 0., 0.],
            rotation: IDENTITY,
            scale: [1.; 4],
        };
        let target = Transform {
            position: [0.25, 0.75, 0., 0.],
            rotation: [0., 0.6, 0., 0.8],
            scale: [2.; 4],
        };
        let correction = offset(source, target);
        let first = apply(target, correction, 1.);
        for (actual, expected) in first.rotation.into_iter().zip(source.rotation) {
            assert!((actual - expected).abs() < 0.0001);
        }
        assert_eq!(first.position, source.position);
        assert_eq!(first.scale, target.scale);
        let moving = Transform {
            position: [0.5, 0.75, 0., 0.],
            ..target
        };
        assert_eq!(
            apply(moving, correction, 0.5).position,
            [0.375, 0.875, 0., 0.]
        );
        assert_eq!(apply(moving, correction, 0.).position, moving.position);
    }

    #[test]
    fn accepts_only_a_dense_and_bounded_c0000_pose_track() {
        #[repr(align(16))]
        struct Master([u8; 0x2000]);
        let mut master = Master([0; 0x2000]);
        let put_word = |m: &mut Master, at: usize, value: u16| {
            m.0[at..at + 2].copy_from_slice(&value.to_le_bytes());
        };
        let put_dword = |m: &mut Master, at: usize, value: u32| {
            m.0[at..at + 4].copy_from_slice(&value.to_le_bytes());
        };
        put_dword(&mut master, 0, 0x2000);
        put_dword(&mut master, 4, 32);
        put_word(&mut master, 0x50, 128);
        put_word(&mut master, 0x52, 128);
        put_dword(&mut master, 0x54, 0x410);
        put_word(&mut master, 0x58, 48);
        put_dword(&mut master, 0x5c, 1.0f32.to_bits());
        let pointer = master.0.as_mut_ptr() as usize;
        let output = &pointer as *const usize as usize;
        unsafe {
            assert_eq!(pose_track(output).unwrap() as usize, pointer + 0x410);
            put_word(&mut master, 0x60, 1);
            assert!(pose_track(output).is_none());
            put_word(&mut master, 0x60, 0);
            put_dword(&mut master, 0x54, 0x810);
            assert!(pose_track(output).is_none());
        }
    }
}
