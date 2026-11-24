//! Only the local player earns combo points. Native slot 2 owns the threshold,
//! decay and purge; its private notification is the sole playback trigger.
use crate::phantom_combo_points as points;
use crate::{has_effect, is_local, native_actor, read_ptr};
use std::sync::Mutex;
#[path = "phantom_combo_visual.rs"]
pub(crate) mod visual;
const OWNER: i32 = 95005800;
pub(crate) const TRIGGER: i32 = OWNER + 1;
pub(super) const RESET_POINTS: i32 = OWNER + 2;

struct Ghost {
    handle: std::sync::Arc<visual::Replay>,
    elapsed: f32,
}

#[derive(Default)]
struct Playback {
    identity: (usize, usize, usize),
    ghosts: Vec<Ghost>,
}
static PLAYBACK: Mutex<Playback> = Mutex::new(Playback {
    identity: (0, 0, 0),
    ghosts: Vec::new(),
});

unsafe fn identity(actor: usize, base: usize) -> Option<(usize, usize, usize)> {
    if !is_local(actor, base)
        || !has_effect(actor, OWNER, base)
        || has_effect(actor, crate::reward_cycle_runtime::RESET, base)
    {
        return None;
    }
    let stats = native_actor::stats(actor);
    if stats == 0 || native_actor::incapacitated(actor, stats) {
        return None;
    }
    Some((actor, read_ptr(actor + 0x78), stats))
}

pub(super) unsafe fn can_credit(actor: usize, base: usize) -> bool {
    identity(actor, base).is_some()
}

pub(super) unsafe fn trigger(actor: usize, effect: i32, base: usize) -> bool {
    if effect != TRIGGER {
        return false;
    }
    let Some(id) = identity(actor, base) else {
        return true;
    };
    if !PLAYBACK.lock().is_ok_and(|state| state.identity == id) {
        return true;
    }
    apply(actor, RESET_POINTS, base);
    let handle = visual::play(actor, base);
    crate::nlog_info!("Phantom Combo: native threshold reached, visual handle={handle:?}");
    if let Some(handle) = handle {
        if let Ok(mut state) = PLAYBACK.lock() {
            if state.identity == id {
                state.ghosts.push(Ghost {
                    handle,
                    elapsed: 0.,
                });
            } else {
                drop(state);
                handle.cancel(base);
            }
        }
    }
    true
}

unsafe fn apply(actor: usize, effect: i32, base: usize) -> u8 {
    let apply: unsafe extern "system" fn(usize, i32, u8) -> u8 =
        std::mem::transmute(base + crate::APPLY_SELF_RVA);
    apply(actor, effect, 0)
}

pub(super) unsafe fn update(actor: usize, base: usize, dt: f32) {
    let Some(id) = identity(actor, base) else {
        visual::clear_history();
        points::bind(0);
        if let Ok(mut state) = PLAYBACK.lock() {
            for ghost in state.ghosts.drain(..) {
                ghost.handle.cancel(base);
            }
            *state = Playback::default();
        }
        return;
    };
    let manager = read_ptr(actor + 0x190);
    points::bind(if manager == 0 {
        0
    } else {
        read_ptr(manager + 0x18)
    });
    visual::sample(actor, base, dt);
    let ghosts = {
        let Ok(mut state) = PLAYBACK.lock() else {
            return;
        };
        if state.identity != id {
            for ghost in state.ghosts.drain(..) {
                ghost.handle.cancel(base);
            }
            *state = Playback {
                identity: id,
                ..Default::default()
            };
        }
        std::mem::take(&mut state.ghosts)
    };
    let mut playing = Vec::new();
    let mut due = 0;
    for mut ghost in ghosts {
        if dt.is_finite() && dt > 0. {
            ghost.elapsed += dt;
        }
        ghost.handle.start_if_due(actor, base);
        let finished = ghost.handle.finished();
        due += ghost.handle.take_due();
        if !finished {
            if ghost.elapsed < 5. {
                playing.push(ghost);
            } else {
                ghost.handle.cancel(base);
            }
        }
    }
    if let Ok(mut state) = PLAYBACK.lock() {
        if state.identity == id {
            state.ghosts.extend(playing);
            points::queue(due);
        } else {
            drop(state);
            for ghost in playing {
                ghost.handle.cancel(base);
            }
        }
    }
}
