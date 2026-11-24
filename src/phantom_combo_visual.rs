//! Visual part of Duchess Restage. The ghost owns a native copy of the pose and
//! transform keys. The damage-record player and its network dispatch are never called.
use crate::{read_ptr, WORLD_RVA};
#[path = "phantom_combo_motion.rs"]
mod motion;
use std::{
    collections::VecDeque,
    mem, ptr,
    sync::{
        atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
};

const MANAGER: usize = 0x3c137a0;
const REPLAY_PLAYER_VT: usize = 0x2c17578;
const MANIPULATOR_VT: usize = 0x2c09858;
const LEAD: f32 = 0.5;
const TAIL: f32 = 0.5;
const WINDOW: f32 = LEAD + TAIL;
const FADE_OUT: f32 = 0.25;
// Both rows use the same cool-white rim and neutral front RGB. The second
// has a stronger visible body (material alpha 0.75 instead of 0.20).
const RESTAGE_MATERIAL: i32 = 370;
const COMBO_MATERIAL: i32 = 340;
// Keep the same white material and scale its native 0.75 alpha to 0.50.
const VISIBLE_ACTOR_ALPHA: f32 = 0.50 / 0.75;

macro_rules! native {
    ($base:expr, $rva:expr, ($($arg:ty),*) -> $ret:ty) => {
        mem::transmute::<usize, unsafe extern "system" fn($($arg),*) -> $ret>($base + $rva)
    };
}

pub(super) unsafe fn record(actor: usize, base: usize) -> usize {
    let manager = read_ptr(base + MANAGER);
    if manager == 0 {
        return 0;
    }
    // The native registry holds the lock while finding or inserting the recorder,
    // and enables transform recording on the registered character.
    native!(base, 0x732800, (usize,usize)->usize)(manager, actor + 0x78)
}

#[repr(align(16))]
struct Clip([usize; 0x298 / 8]);

// Native recorder keys are ages in seconds, oldest first. Retain the state
// already in effect at the cut and keep the original intervals of later keys.
fn clip_track(keys: &[u64], words: usize, end_key: bool) -> Option<Vec<u64>> {
    if words < 2 || keys.len() % words != 0 {
        return None;
    }
    let age = |key: &[u64]| f32::from_bits(key[1] as u32);
    let mut start = 0;
    let mut previous = f32::INFINITY;
    for (i, frame) in keys.chunks_exact(words).enumerate() {
        let t = age(frame);
        if !t.is_finite() || t < 0. || t > previous {
            return None;
        }
        if t >= WINDOW {
            start = i;
        }
        previous = t;
    }
    let mut out = keys[start * words..].to_vec();
    if out.is_empty() {
        return Some(out);
    }
    let set_age = |row: &mut [u64], v: f32| {
        row[1] = (row[1] & !0xffff_ffff) | u64::from(v.to_bits());
    };
    set_age(&mut out[..words], WINDOW);
    if end_key && age(&out[out.len() - words..]) > 0. {
        out.extend_from_within(out.len() - words..);
        let at = out.len() - words;
        set_age(&mut out[at..], 0.);
    }
    Some(out)
}

// Ordinary characters can lose an empty Restage recorder at the next world
// update. Keep the visual keys by value, independently of that registry.
#[derive(Clone)]
struct Frame {
    time: f64,
    movement: [u64; 6],
    pose: [u64; 53],
    clips: motion::Clips,
}
const REPLAY_POINTS: i32 = 36;

#[derive(Debug, Default)]
struct Awards {
    paid: i32,
}

impl Awards {
    fn take_through(&mut self, elapsed: f32) -> i32 {
        if !elapsed.is_finite() {
            return 0;
        }
        let earned = (elapsed.clamp(0., WINDOW) / WINDOW * REPLAY_POINTS as f32).floor() as i32;
        let due = (earned - self.paid).max(0);
        self.paid += due;
        due
    }
}

#[derive(Default)]
struct History {
    owner: (usize, usize),
    clock: f64,
    frames: VecDeque<Frame>,
}
static HISTORY: Mutex<History> = Mutex::new(History {
    owner: (0, 0),
    clock: 0.,
    frames: VecDeque::new(),
});

impl History {
    fn push(&mut self, frame: Frame) {
        while self.frames.len() >= 2 && self.frames[1].time <= frame.time - f64::from(WINDOW + TAIL)
        {
            self.frames.pop_front();
        }
        self.frames.push_back(frame);
    }
    fn tracks(&self, end: f64) -> Option<(Vec<u64>, Vec<u64>)> {
        if !end.is_finite() || self.clock + 1e-7 < end || self.frames.get(1)?.time > end + 1e-7 {
            return None;
        }
        let mut movement = Vec::with_capacity(self.frames.len() * 6);
        let mut poses = Vec::with_capacity(self.frames.len() * 53);
        for frame in self.frames.iter().take_while(|f| f.time <= end + 1e-7) {
            let age = (end - frame.time).max(0.) as f32;
            let mut m = frame.movement;
            m[1] = u64::from(age.to_bits());
            movement.extend(m);
            let mut p = frame.pose;
            p[1] = u64::from(age.to_bits());
            poses.extend(p);
        }
        Some((
            clip_track(&movement, 6, true)?,
            clip_track(&poses, 53, false)?,
        ))
    }
}

pub(super) fn clear_history() {
    if let Ok(mut history) = HISTORY.lock() {
        if history.owner != (0, 0) {
            *history = History::default();
        }
    }
}

#[repr(align(16))]
struct Vector([f32; 4]);

unsafe fn capture(actor: usize, base: usize, time: f64) -> Option<Frame> {
    let modules = read_ptr(actor + 0x1b8);
    let model = read_ptr(actor + 0x60);
    if modules == 0 || model == 0 || read_ptr(modules + 0x28) == 0 {
        return None;
    }
    let action = read_ptr(modules + 0x30);
    if action == 0 || read_ptr(action + 0x10) == 0 {
        return None;
    }
    let mut world = Vector([0.; 4]);
    let mut rotation = Vector([0.; 4]);
    let mut local = Vector([0.; 4]);
    let mut map = u32::MAX;
    native!(base,crate::CHR_POSITION_RVA,(usize,*mut f32)->usize)(actor, world.0.as_mut_ptr());
    native!(base,0x3d0040,(usize,*mut u32)->usize)(actor, &mut map);
    native!(base,crate::CHR_ROTATION_RVA,(usize,*mut f32)->usize)(actor, rotation.0.as_mut_ptr());
    if native!(base,crate::MAP_POSITION_RVA,(*mut f32,*const f32,*mut u32)->u8)(
        local.0.as_mut_ptr(),
        world.0.as_ptr(),
        &mut map,
    ) == 0
    {
        return None;
    }
    let mut frame = Frame {
        time,
        movement: [0; 6],
        pose: [0; 53],
        clips: motion::capture(actor, base),
    };
    // Native transform and pose key layouts, written by the two replay collectors.
    frame.movement[0] = (base + 0x2bda590) as u64;
    let m = frame.movement.as_mut_ptr().cast::<u8>();
    ptr::write_unaligned(m.add(0x10).cast::<u32>(), map);
    ptr::copy_nonoverlapping(local.0.as_ptr().cast::<u8>(), m.add(0x14), 12);
    ptr::copy_nonoverlapping(rotation.0.as_ptr().cast::<u8>(), m.add(0x20), 12);
    frame.pose[0] = (base + 0x2bda590) as u64;
    let pose = frame.pose.as_mut_ptr().cast::<u8>().add(0x10);
    native!(base,0x72dc90,(*mut u8)->usize)(pose);
    ptr::write_unaligned(
        pose.add(0x16c).cast::<i32>(),
        crate::read_i32(model + 0x180),
    );
    native!(base,0x40a130,(usize,*mut u8)->())(action, pose);
    Some(frame)
}

pub(super) unsafe fn sample(actor: usize, base: usize, dt: f32) {
    let owner = (actor, read_ptr(actor + 0x78));
    let time = {
        let Ok(mut h) = HISTORY.lock() else {
            return;
        };
        if h.owner != owner || dt > WINDOW {
            *h = History {
                owner,
                ..Default::default()
            };
        }
        if !dt.is_finite() || dt <= 0. || dt > WINDOW {
            return;
        }
        h.clock += f64::from(dt);
        if h.frames
            .back()
            .is_some_and(|last| h.clock - last.time < 1. / 30. - 0.00001)
        {
            return;
        }
        h.clock
    };
    if let Some(frame) = capture(actor, base, time) {
        if let Ok(mut h) = HISTORY.lock() {
            if h.owner == owner && h.clock == time {
                h.push(frame);
            }
        }
    }
}

unsafe fn visual_data(
    actor: usize,
    rec: usize,
    base: usize,
    end: f64,
    existing: Option<usize>,
) -> Option<usize> {
    let (movement, poses) = {
        let h = HISTORY.lock().ok()?;
        if h.owner != (actor, read_ptr(actor + 0x78)) {
            return None;
        }
        h.tracks(end)?
    };
    let mut view = Clip([0; 0x298 / 8]);
    ptr::copy_nonoverlapping(rec as *const usize, view.0.as_mut_ptr(), view.0.len());
    // Seed the native initial pose from the oldest retained key.
    ptr::copy_nonoverlapping(
        poses.as_ptr().add(2),
        view.0.as_mut_ptr().add(0xc8 / 8).cast::<u64>(),
        0x198 / 8,
    );
    let set_range = |view: &mut Clip, off: usize, from: usize, size: usize| {
        view.0[off / 8] = from;
        view.0[off / 8 + 1] = from + size;
        view.0[off / 8 + 2] = from + size;
    };
    for (off, data) in [(0x70, &movement), (0x90, &poses)] {
        set_range(&mut view, off, data.as_ptr() as usize, data.len() * 8);
    }
    set_range(&mut view, 0xb0, 0, 0);
    // The time-origin helper reads the first event age; the visual loader
    // uses it only to turn ages into forward playback times. No combat track
    // or combat-replay object is passed to a game recipient.
    let mut origin = [0u64; 15];
    origin[1] = u64::from(WINDOW.to_bits());
    set_range(&mut view, 0x10, 0, 0);
    set_range(&mut view, 0x30, origin.as_ptr() as usize, 0x78);
    set_range(&mut view, 0x50, 0, 0);
    let obj = if let Some(existing) = existing {
        existing
    } else {
        let allocator = read_ptr(base + 0x3c2d9f8);
        if allocator == 0 {
            return None;
        }
        let obj = native!(base,0x214e8d0,(usize,usize,usize)->usize)(0x238, 8, allocator);
        if obj == 0 {
            return None;
        }
        native!(base,0x72db20,(usize)->usize)(obj);
        obj
    };
    native!(base,0x72e420,(usize,usize)->())(obj, view.0.as_ptr() as usize);
    Some(obj)
}

unsafe fn drop_data(data: usize, base: usize) {
    let heap = native!(base,0xf61390,(usize)->usize)(data);
    native!(base,0x72e1a0,(usize)->())(data);
    let release = mem::transmute::<usize, unsafe extern "system" fn(usize, usize)>(read_ptr(
        read_ptr(heap) + 0x68,
    ));
    release(heap, data);
}

#[repr(align(16))]
struct Appearance([u8; 0x350]);

// The renderer owns the actor and replay data. The weak entry exists only while
// the local combo is waiting for this particular playback to finish.
#[derive(Debug)]
pub(super) struct Replay {
    actor: usize,
    data: usize,
    end: f64,
    armed: Mutex<Option<Awards>>,
    motion: OnceLock<motion::Track>,
    warmup_frames: AtomicU32,
    cursor: AtomicU32,
    finished: AtomicBool,
}
static OWNED: Mutex<Vec<Weak<Replay>>> = Mutex::new(Vec::new());
pub(crate) const TICK_RVA: usize = 0x3a8ce0;
pub(crate) const TICK_PATTERN: &str = "48 8B C4 48 89 58 10 48 89 70 18 48 89 78 20 55 41 54 41 55 41 56 41 57 48 8D A8 18 FF FF FF 48 81 EC C0 01 00 00 0F 29 70 C8 0F 29 78 B8 44 0F 29 40 A8 44 0F 29 48 98 44 0F 29 50 88 44 0F 29 98 78 FF FF FF 44 0F 29 A0 68 FF FF FF 44 0F 29 A8 58 FF FF FF 44 0F 29 B0 48 FF FF FF 44 0F 29 B8 38 FF FF FF 48 8B 05 1C D2 73 03";
pub(crate) static TICK: AtomicPtr<u8> = AtomicPtr::new(ptr::null_mut());
pub(crate) const MATERIAL_RVA: usize = 0x4d2c20;
pub(crate) const MATERIAL_PATTERN: &str =
    "48 83 EC 28 48 8D 4C 24 38 E8 22 EA 9A 00 48 8B 08 48 85 C9 74 0B 8B 81 E0 00 00 00 48 83 C4 28 C3";
pub(crate) static MATERIAL: AtomicPtr<u8> = AtomicPtr::new(ptr::null_mut());

pub(crate) unsafe extern "system" fn replay_material(actor: usize) -> i32 {
    let original: unsafe extern "system" fn(usize) -> i32 =
        mem::transmute(MATERIAL.load(Ordering::Acquire));
    let id = original(actor);
    if id != RESTAGE_MATERIAL {
        return id;
    }
    // Native reuses replay actors. Match their current controller payload as
    // well as the actor so an ordinary Restage can reuse the same pool slot.
    let owned = OWNED.lock().is_ok_and(|entries| {
        entries.iter().filter_map(Weak::upgrade).any(|replay| {
            if replay.actor != actor {
                return false;
            }
            let base = crate::BASE.load(Ordering::Acquire);
            let controller = native!(base,0x3d0100,(usize)->usize)(actor);
            controller != 0
                && read_ptr(controller) == base + MANIPULATOR_VT
                && read_ptr(controller + 0xc0) == replay.data
        })
    });
    if owned {
        COMBO_MATERIAL
    } else {
        id
    }
}

type Tick = unsafe extern "system" fn(usize, f32);

unsafe fn set_opacity(actor: usize, alpha: f32) {
    // Native fade-in/out use this pair as current opacity and change per second.
    ptr::write_unaligned((actor + 0x27c) as *mut f32, alpha);
    ptr::write_unaligned((actor + 0x280) as *mut f32, 0.);
}

unsafe fn present(replay: &Replay, m: usize, dt: f32, tick: Tick) {
    if ptr::read((m + 0x138) as *const u8) == 0 {
        return; // Original tick also waits until the renderer marks the actor ready.
    }
    let phase = || crate::read_i32(m + 0xdc);
    if phase() == 0 {
        // Position, rotation and the first recorded animation are installed here.
        tick(m, 0.);
    }
    if phase() == 1 {
        *((replay.actor + 0x1f3) as *mut u8) |= 8;
        set_opacity(replay.actor, 0.);
        // Skip the native fade, but keep the model hidden while the graph
        // activates the clips selected by the initial pose.
        ptr::write_unaligned((m + 0xdc) as *mut i32, 2);
    }
    if phase() == 2 {
        tick(m, 0.);
    }
    if phase() == 3 {
        let warmup = replay.warmup_frames.load(Ordering::Relaxed);
        if warmup < 5 {
            let ready = replay.motion.get().is_none_or(|track| {
                track.align(replay.actor, crate::BASE.load(Ordering::Acquire), 0.)
            });
            if warmup < 2 || !ready {
                tick(m, 0.);
                set_opacity(replay.actor, 0.);
                if dt > 0. {
                    replay.warmup_frames.store(warmup + 1, Ordering::Relaxed);
                }
                return;
            }
        }
        if warmup != u32::MAX {
            replay.warmup_frames.store(u32::MAX, Ordering::Relaxed);
            set_opacity(replay.actor, VISIBLE_ACTOR_ALPHA);
        }
        tick(m, dt);
        let elapsed = ptr::read_unaligned((m + 0xc8) as *const f32);
        if elapsed.is_finite() {
            if let Some(track) = replay.motion.get() {
                let base = crate::BASE.load(Ordering::Acquire);
                track.align(replay.actor, base, elapsed);
            }
            // The native track duration can differ from WINDOW by a fraction of a frame.
            let duration = ptr::read_unaligned((m + 0xcc) as *const f32);
            let progress = if (duration - WINDOW).abs() <= 0.015 {
                (elapsed / duration).clamp(0., 1.) * WINDOW
            } else {
                elapsed.clamp(0., WINDOW)
            };
            replay.cursor.store(progress.to_bits(), Ordering::Release);
        }
        // The tick that finishes the clip has already spent this frame's dt.
        if phase() == 4 {
            return;
        }
    }
    if phase() == 4 {
        let elapsed = ptr::read_unaligned((m + 0xc8) as *const f32);
        let duration = ptr::read_unaligned((m + 0xcc) as *const f32);
        let complete =
            elapsed.is_finite() && elapsed >= duration && (duration - WINDOW).abs() <= 0.015;
        if complete {
            if let Some(track) = replay.motion.get() {
                track.align(replay.actor, crate::BASE.load(Ordering::Acquire), WINDOW);
            }
            // The replay controller stays at its last transform and never advances
            // the award cursor during the fade. Keep the renderer's fade rate zero.
            let current = ptr::read_unaligned((replay.actor + 0x27c) as *const f32);
            let alpha = (current - VISIBLE_ACTOR_ALPHA * (dt.max(0.) / FADE_OUT))
                .clamp(0., VISIBLE_ACTOR_ALPHA);
            set_opacity(replay.actor, alpha);
            if alpha > 0. {
                return;
            }
        } else {
            set_opacity(replay.actor, 0.);
        }
        ptr::write_unaligned((m + 0xdc) as *mut i32, 5);
        replay.finished.store(true, Ordering::Release);
        return;
    }
    if phase() >= 5 {
        tick(m, dt);
    }
}

pub(crate) unsafe extern "system" fn on_tick(m: usize, dt: f32) {
    let tick: Tick = mem::transmute(TICK.load(Ordering::Acquire));
    let replay = OWNED.lock().ok().and_then(|list| {
        list.iter()
            .filter_map(Weak::upgrade)
            .find(|r| r.actor == read_ptr(m + 0xa8) && r.data == read_ptr(m + 0xc0))
    });
    if let Some(replay) = replay {
        if let Ok(armed) = replay.armed.lock() {
            if armed.is_some() {
                present(&replay, m, dt, tick);
            } else {
                set_opacity(replay.actor, 0.);
            }
        }
    } else {
        tick(m, dt);
    }
}

pub(super) unsafe fn play(actor: usize, base: usize) -> Option<Arc<Replay>> {
    let world = read_ptr(base + WORLD_RVA);
    if world == 0 {
        return None;
    }
    let rec = record(actor, base);
    if rec == 0 {
        return None;
    }
    let vt = read_ptr(actor);
    let game_data = native!(0,read_ptr(vt+0x190),(usize)->usize)(actor);
    let body = native!(0,read_ptr(vt+0x248),(usize)->usize)(actor);
    if game_data == 0 || body == 0 {
        return None;
    }
    let now = HISTORY.lock().ok()?.clock;
    // Start model loading at trigger time. This provisional clip stays hidden
    // until the post-trigger part of the clip has been recorded.
    let data = visual_data(actor, rec, base, now, None)?;
    let mut look = Appearance([0; 0x350]);
    let p = look.0.as_mut_ptr() as usize;
    native!(base,0x737990,(usize)->usize)(p);
    native!(base,0x4cd740,(usize)->usize)(p + 0x1f8);
    native!(base,0x1f7be0,(usize)->usize)(p + 0x214);
    native!(base,0x20bde0,(usize,usize)->())(game_data, p);
    ptr::copy_nonoverlapping(body as *const u8, (p + 0x1f8) as *mut u8, 0x1c);
    let visuals = native!(base,0x20bc50,(usize)->usize)(game_data);
    native!(base,0x4d2470,(usize,usize)->())(p + 0x214, visuals);
    let chosen = crate::read_i32(game_data + 0x868);
    ptr::write_unaligned(
        (p + 0x344) as *mut i32,
        if chosen < 0 {
            crate::read_i32(game_data + 0xe8)
        } else {
            chosen
        },
    );
    for i in 0..4 {
        let item = native!(base,0x1f2730,(usize,i32)->i32)(game_data + 0x1d8, 6 + i);
        ptr::write_unaligned((p + 0x334 + i as usize * 4) as *mut i32, item);
    }
    let ghost = native!(base,0x518220,(usize,usize,usize,usize)->usize)(world, data, p, actor);
    native!(base,0x2690a5c,(usize,usize,usize,usize)->())(p, 0x48, 6, base + 0x1dc910);
    if ghost == 0 {
        drop_data(data, base);
        return None;
    }
    set_opacity(ghost, 0.);
    let replay = Arc::new(Replay {
        end: now + f64::from(TAIL),
        motion: OnceLock::new(),
        armed: Mutex::new(None),
        warmup_frames: AtomicU32::new(0),
        cursor: AtomicU32::new((-1f32).to_bits()),
        actor: ghost,
        data,
        finished: AtomicBool::new(false),
    });
    let mut owned = OWNED.lock().ok()?;
    owned.retain(|w| w.strong_count() != 0);
    owned.push(Arc::downgrade(&replay));
    drop(owned);
    Some(replay)
}

// Replay players live in WorldChrMan's dedicated visual pool. They can
// legitimately have an invalid combat handle for their entire lifetime.
unsafe fn in_pool(pool: usize, actor: usize) -> bool {
    let count = crate::read_i32(pool + 0x10);
    let slots = read_ptr(pool + 0x18);
    slots != 0
        && (1..=1024).contains(&count)
        && (0..count as usize).any(|slot| read_ptr(slots + 16 * slot) == actor)
}

impl Replay {
    unsafe fn controller(&self, base: usize) -> Option<usize> {
        let world = read_ptr(base + WORLD_RVA);
        if world == 0
            || !in_pool(world + 0x9a98, self.actor)
            || read_ptr(self.actor) != base + REPLAY_PLAYER_VT
        {
            return None;
        }
        let m = native!(base,0x3d0100,(usize)->usize)(self.actor);
        (m != 0 && read_ptr(m) == base + MANIPULATOR_VT && read_ptr(m + 0xc0) == self.data)
            .then_some(m)
    }
    pub(super) unsafe fn start_if_due(&self, actor: usize, base: usize) {
        let Ok(mut armed) = self.armed.lock() else {
            return;
        };
        if armed.is_some() || !HISTORY.lock().is_ok_and(|h| h.clock + 1e-7 >= self.end) {
            return;
        }
        let Some(m) = self.controller(base) else {
            return;
        };
        let rec = record(actor, base);
        if rec == 0 || visual_data(actor, rec, base, self.end, Some(self.data)).is_none() {
            return;
        }
        // Reload cursor bounds after replacing the provisional tracks. The
        // same native object retains ownership of its vectors throughout.
        native!(base,0x3a8b90,(usize,usize)->())(m, self.data);
        let Ok(h) = HISTORY.lock() else {
            return;
        };
        let _ = self.motion.set(motion::Track::from_history(&h, self.end));
        *armed = Some(Awards::default());
        drop(h);
        let tick: Tick = mem::transmute(TICK.load(Ordering::Acquire));
        present(self, m, 0., tick);
    }
    pub(super) unsafe fn cancel(&self, base: usize) {
        let Ok(mut armed) = self.armed.lock() else {
            return;
        };
        if let Some(m) = self.controller(base) {
            set_opacity(self.actor, 0.);
            if crate::read_i32(m + 0xdc) < 5 {
                ptr::write_unaligned((m + 0xdc) as *mut i32, 5);
            }
        }
        *armed = Some(Awards { paid: REPLAY_POINTS });
    }
    pub(super) fn take_due(&self) -> i32 {
        let Ok(mut armed) = self.armed.lock() else {
            return 0;
        };
        armed.as_mut().map_or(0, |schedule| {
            schedule.take_through(f32::from_bits(self.cursor.load(Ordering::Acquire)))
        })
    }
    pub(super) fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    unsafe extern "system" fn native_phase_stub(m: usize, dt: f32) {
        let phase = (m + 0xdc) as *mut i32;
        let elapsed = (m + 0xc8) as *mut f32;
        match *phase {
            0 => {
                *((m + 0x100) as *mut i32) = 1;
                *phase = 1;
            }
            2 => {
                assert_eq!(dt, 0.);
                *phase = 3;
            }
            3 => {
                *elapsed += dt;
                if *elapsed >= WINDOW {
                    *phase = 4;
                }
            }
            5 => {
                *phase = 6;
            }
            _ => {
                *((m + 0x104) as *mut i32) = 1;
            }
        }
    }
    #[test]
    fn immediate_visual_preserves_first_pose_clock_and_terminal_reward() {
        let mut actor = [0u64; 86];
        let mut controller = [0u64; 54];
        let m = controller.as_mut_ptr() as usize;
        let a = actor.as_mut_ptr() as usize;
        let replay = Replay {
            end: 0.,
            motion: OnceLock::new(),
            armed: Mutex::new(Some(Awards::default())),
            warmup_frames: AtomicU32::new(0),
            cursor: AtomicU32::new((-1f32).to_bits()),
            actor: a,
            data: 1,
            finished: AtomicBool::new(false),
        };
        unsafe {
            ptr::write_unaligned((m + 0xcc) as *mut f32, WINDOW);
            present(&replay, m, 0.2, native_phase_stub);
            assert_eq!(crate::read_i32(m + 0xdc), 0);
            *((m + 0x138) as *mut u8) = 1;
            present(&replay, m, 0., native_phase_stub);
            assert_eq!(crate::read_i32(m + 0x100), 1);
            assert_eq!(crate::read_i32(m + 0xdc), 3);
            assert_eq!(*((m + 0xc8) as *const f32), 0.);
            // The graph activates the first clip after the controller tick.
            for _ in 0..2 {
                present(&replay, m, 1. / 60., native_phase_stub);
                assert_eq!(*((a + 0x27c) as *const f32), 0.);
                assert_eq!(*((m + 0xc8) as *const f32), 0.);
                assert_eq!(f32::from_bits(replay.cursor.load(Ordering::Relaxed)), -1.);
            }
            present(&replay, m, 0., native_phase_stub);
            assert_eq!(*((a + 0x27c) as *const f32), VISIBLE_ACTOR_ALPHA);
            assert!(!replay.finished());
            assert_eq!(replay.take_due(), 0);
            present(&replay, m, WINDOW / 4., native_phase_stub);
            assert_eq!(replay.take_due(), 9);
            present(&replay, m, WINDOW * 0.75, native_phase_stub);
            assert_eq!(replay.take_due(), 27);
            assert_eq!(crate::read_i32(m + 0xdc), 4);
            assert_eq!(*((a + 0x27c) as *const f32), VISIBLE_ACTOR_ALPHA);
            present(&replay, m, FADE_OUT / 2., native_phase_stub);
            assert_eq!(*((a + 0x27c) as *const f32), VISIBLE_ACTOR_ALPHA / 2.);
            assert!(!replay.finished());
            present(&replay, m, 0., native_phase_stub);
            assert_eq!(*((a + 0x27c) as *const f32), VISIBLE_ACTOR_ALPHA / 2.);
            present(&replay, m, FADE_OUT / 2., native_phase_stub);
            assert_eq!(*((m + 0xc8) as *const f32), WINDOW);
            assert_eq!(
                f32::from_bits(replay.cursor.load(Ordering::Relaxed)),
                WINDOW
            );
            assert_eq!(*((a + 0x27c) as *const f32), 0.);
            assert_eq!(crate::read_i32(m + 0xdc), 5);
            assert!(replay.finished());
            assert_eq!(replay.take_due(), 0);
            present(&replay, m, 0.01, native_phase_stub);
            assert_eq!(crate::read_i32(m + 0xdc), 6);
            assert_eq!(crate::read_i32(m + 0x104), 0);
        }
    }

    #[test]
    fn replay_lookup_tracks_only_occupied_native_visual_slots() {
        let mut pool = [0usize; 4];
        let mut slots = [0x1234usize, 0, 0x5678, 0];
        pool[2] = 2;
        pool[3] = slots.as_ptr() as usize;
        unsafe {
            assert!(in_pool(pool.as_ptr() as usize, 0x5678));
            slots[2] = 0;
            std::hint::black_box(&slots);
            assert!(!in_pool(pool.as_ptr() as usize, 0x5678));
        }
    }
    #[test]
    fn private_history_keeps_moving_and_attack_frames_without_engine_registration() {
        let mut history = History::default();
        for index in 0..=90 {
            let time = f64::from(index) / 30.;
            let mut frame = Frame {
                time,
                movement: [0; 6],
                pose: [0; 53],
                clips: Default::default(),
            };
            frame.movement[2] = index as u64;
            frame.pose[2] = (index / 10) as u64;
            history.clock = time;
            history.push(frame);
        }
        let (movement, poses) = history.tracks(history.clock).unwrap();
        assert!(history.frames.len() <= 47);
        assert_eq!(movement[2], 60);
        assert_eq!(movement.chunks_exact(6).last().unwrap()[2], 90);
        assert_eq!(poses[2], 6);
        assert_eq!(poses.chunks_exact(53).last().unwrap()[2], 9);
        assert_eq!(f32::from_bits(poses[1] as u32), WINDOW);
    }

    #[test]
    fn awards_follow_visible_time_once_per_point_at_any_frame_rate() {
        for frames in [1, 7, 30, 60, 144] {
            let mut awards = Awards::default();
            let mut total = 0;
            assert_eq!(awards.take_through(-1.), 0);
            for frame in 0..=frames {
                let elapsed = WINDOW * frame as f32 / frames as f32;
                total += awards.take_through(elapsed);
                assert_eq!(total, frame * REPLAY_POINTS / frames);
                assert_eq!(awards.take_through(elapsed), 0);
            }
            assert_eq!(total, REPLAY_POINTS);
            assert_eq!(awards.take_through(WINDOW + FADE_OUT), 0);
        }
        let mut awards = Awards::default();
        assert_eq!(awards.take_through(WINDOW / 2.), 18);
        assert_eq!(awards.take_through(f32::NAN), 0);
        assert_eq!(awards.take_through(WINDOW / 4.), 0);
        assert_eq!(awards.take_through(WINDOW), 18);
    }

    #[test]
    fn each_trigger_includes_its_own_followthrough_and_excludes_later_attacks() {
        let mut h = History::default();
        for n in 0..=48 {
            h.clock = f64::from(n) / 30.;
            let mut frame = Frame {
                time: h.clock,
                movement: [0; 6],
                pose: [0; 53],
                clips: Default::default(),
            };
            frame.pose[2] = n as u64;
            h.push(frame);
        }
        // Triggers at 1.0 and 1.1 share history, but have different cutoffs.
        for (trigger, first, last) in [(1.0, 15, 45), (1.1, 18, 48)] {
            let end = trigger + f64::from(TAIL);
            let (_, keys) = h.tracks(end).unwrap();
            assert_eq!(keys[2], first);
            assert_eq!(keys.chunks_exact(53).last().unwrap()[2], last);
            assert_eq!(f32::from_bits(keys[1] as u32), WINDOW);
        }
        assert!(h.tracks(1.7).is_none());
    }

    #[test]
    fn cut_keeps_preceding_pose_and_closes_movement_at_the_trigger() {
        let rows: Vec<_> = [1.8_f32, 0.8, 0.52, 0.08]
            .iter()
            .enumerate()
            .flat_map(|(i, t)| [i as u64, u64::from(t.to_bits())])
            .collect();
        let out = clip_track(&rows, 2, true).unwrap();
        assert_eq!(
            out.chunks_exact(2)
                .map(|k| (k[0], f32::from_bits(k[1] as u32)))
                .collect::<Vec<_>>(),
            [(0, 1.0), (1, 0.8), (2, 0.52), (3, 0.08), (3, 0.)]
        );
        assert!(clip_track(&[0, 0, 1, u64::from(1f32.to_bits())], 2, false).is_none());
    }
}
