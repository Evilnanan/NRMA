//! Keep native reward activation separate from locally generated fission choices.
use super::{generated, lookup, GUEST, HOST, SESSION_RVA};
use crate::{ptr, read_i32, read_ptr, ACTIVE, BASE, ITEMS_RVA};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::time::{Duration, Instant};

// The immediately following native code uses the chosen handle to decide whether
// to broadcast. All four source lookups inspect the same type-2 node list.
pub const LOCAL_RVA: usize = 0x57788c;
pub const LOCAL_PATTERN: &str = "8B 42 78 89 44 24 38 48 8D 4C 24 38 E8 13 A3 12 00";
pub const REQUEST_RVA: usize = 0x57f27c;
pub const REQUEST_PATTERN: &str = "8B 42 78 89 44 24 68 48 8D 4C 24 68 E8 13 29 12 00";
pub const HANDOVER_RVA: usize = 0x57419c;
pub const HANDOVER_PATTERN: &str = "8B 42 78 89 44 24 50 48 8D 4C 24 50 E8 F3 D9 12 00";
pub const RESEND_RVA: usize = 0x58235c;
pub const RESEND_PATTERN: &str = "8B 42 78 89 44 24 40 48 8D 4C 24 40 E8 33 F8 11 00";

pub const RECEIVE_RVA: usize = 0x57ec20;
pub const RECEIVE_PATTERN: &str = "E8 5B 3A 00 00 48 8B D8 48 85 C0 0F 84 AA 00 00 00 44 0F B6 C6";
pub const PUMP_RVA: usize = 0x57d640;
pub const PUMP_PATTERN: &str = "48 8B C4 48 89 58 08 55 56 57 41 54 41 55 41 56 41 57 48 8D 68 A1 48 81 EC E0 00 00 00 0F 29 70 B8 0F 29 78 A8 0F 28 F9 48 8B F9 48 83 3D 55 48 6A 03 00";
pub static PUMP_ORIGINAL: AtomicPtr<u8> = AtomicPtr::new(ptr::null_mut());
static RECEIVE_RETURN: AtomicPtr<u8> = AtomicPtr::new(ptr::null_mut());

const NETWORK_CHOICE: u32 = 0x12;
const ENABLE_RVA: usize = 0x57cdd0;
const CONFIRM_RVA: usize = 0x581350;
const MAX_PENDING: usize = 128;
const MAX_SOURCES: usize = 256;
// Match the game's pending item-message lifetime.
const LIFETIME: Duration = Duration::from_secs(30);

unsafe extern "system" fn broadcast_handle(manager: usize, node: usize) -> u32 {
    let handle = read_i32(node + 0x78) as u32;
    if handle & 0x1f != 2 || !ACTIVE.load(Ordering::Acquire) || !generated(read_i32(node + 0xd8)) {
        return handle;
    }
    let base = BASE.load(Ordering::Acquire);
    let session = read_ptr(base + SESSION_RVA);
    if session == 0 || read_i32(session + 0xc) != HOST || manager != read_ptr(base + ITEMS_RVA) {
        return handle;
    }
    let source = read_ptr(node + 0x80);
    let mut candidate = read_ptr(manager + 0x58);
    for _ in 0..65536 {
        if candidate == 0 {
            break;
        }
        let candidate_handle = read_i32(candidate + 0x78) as u32;
        if candidate_handle & 0x1f == NETWORK_CHOICE && read_ptr(candidate + 0x80) == source {
            return candidate_handle;
        }
        candidate = read_ptr(candidate);
    }
    handle
}

macro_rules! select_handle {
    ($name:ident, $resume:ident, $manager:literal, $slot:literal) => {
        static $resume: AtomicPtr<u8> = AtomicPtr::new(ptr::null_mut());
        #[unsafe(naked)]
        pub unsafe extern "system" fn $name() {
            // Already in an aligned native frame, immediately before another call.
            core::arch::naked_asm!(
                concat!("mov rcx,", $manager),
                "call {select}",
                concat!("mov [rsp+", $slot, "],eax"),
                "jmp qword ptr [rip+{resume}]",
                select = sym broadcast_handle, resume = sym $resume,
            );
        }
    };
}
select_handle!(local_anchor, LOCAL_RETURN, "rdi", "0x38");
select_handle!(request_anchor, REQUEST_RETURN, "rdi", "0x68");
select_handle!(handover_anchor, HANDOVER_RETURN, "rdi", "0x50");
select_handle!(resend_anchor, RESEND_RETURN, "rsi", "0x40");

pub unsafe fn prepare(base: usize) -> Result<Vec<(usize, crate::hook::PreparedHook)>, String> {
    let mut hooks = Vec::new();
    for (rva, detour, location, width) in [
        (LOCAL_RVA, local_anchor as *const u8, &LOCAL_RETURN, 7),
        (REQUEST_RVA, request_anchor as *const u8, &REQUEST_RETURN, 7),
        (
            HANDOVER_RVA,
            handover_anchor as *const u8,
            &HANDOVER_RETURN,
            7,
        ),
        (RESEND_RVA, resend_anchor as *const u8, &RESEND_RETURN, 7),
        (RECEIVE_RVA, receive as *const u8, &RECEIVE_RETURN, 5),
        (PUMP_RVA, pump as *const u8, &PUMP_ORIGINAL, 0),
    ] {
        let patch = crate::hook::prepare((base + rva) as *mut u8, detour, 64)?;
        if width != 0 && patch.stolen != width {
            return Err(format!(
                "Unexpected fission activation hook width at {rva:x}"
            ));
        }
        let target = if width == 0 {
            patch.trampoline
        } else {
            (base + rva + width) as *mut u8
        };
        location.store(target, Ordering::Release);
        hooks.push((rva, patch));
    }
    Ok(hooks)
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Session {
    items: usize,
    instance: usize,
    host: usize,
}

unsafe fn guest_session(base: usize, items: usize) -> Option<Session> {
    let instance = read_ptr(base + SESSION_RVA);
    if instance == 0
        || read_i32(instance + 0xc) != GUEST
        || read_i32(instance + 0x10) != 7
        || items != read_ptr(base + ITEMS_RVA)
    {
        return None;
    }
    Some(Session {
        items,
        instance,
        host: read_ptr(instance + 0xa0),
    })
}

#[derive(Clone, Copy)]
struct Notice {
    handle: u32,
    value: u8,
    order: u64,
    received: Instant,
}

#[derive(Default)]
struct Deferred {
    session: Option<Session>,
    next_order: u64,
    pending: Vec<Notice>,
    applied: BTreeMap<usize, u64>,
}

impl Deferred {
    fn begin(&mut self, session: Option<Session>) {
        if self.session != session {
            *self = Self {
                session,
                ..Self::default()
            };
        }
    }

    fn receive(&mut self, handle: u32, value: u8, source: Option<usize>, now: Instant) {
        self.pending.retain(|notice| notice.handle != handle);
        if self.pending.is_empty() {
            self.applied.clear();
        }
        self.next_order += 1;
        if let Some(source) = source {
            // An older missing representative may name another option of this source.
            if !self.pending.is_empty() && source as u32 != u32::MAX {
                if self.applied.len() == MAX_SOURCES && !self.applied.contains_key(&source) {
                    self.pending.clear();
                    self.applied.clear();
                } else {
                    self.applied.insert(source, self.next_order);
                }
            }
        } else if self.pending.len() < MAX_PENDING {
            self.pending.push(Notice {
                handle,
                value,
                order: self.next_order,
                received: now,
            });
        }
    }
}

thread_local! { static DEFERRED: RefCell<Deferred> = RefCell::new(Deferred::default()); }

pub(super) fn reset() {
    DEFERRED.with(|state| *state.borrow_mut() = Deferred::default());
}

#[unsafe(naked)]
pub unsafe extern "system" fn receive() {
    // This branch is reached only after the native length, kind, and host checks.
    // Let the original receiver use our lookup return exactly as before.
    core::arch::naked_asm!(
        "movzx r8d,sil", "call {find}", "jmp qword ptr [rip+{resume}]",
        find = sym receive_lookup, resume = sym RECEIVE_RETURN,
    );
}

unsafe extern "system" fn receive_lookup(items: usize, handle: u32, value: u8) -> usize {
    let base = BASE.load(Ordering::Acquire);
    let node = lookup(items, handle, base);
    if value <= 1 && ACTIVE.load(Ordering::Acquire) {
        if let Some(session) = guest_session(base, items) {
            let source = (node != 0).then(|| read_ptr(node + 0x80));
            DEFERRED.with(|state| {
                let mut state = state.borrow_mut();
                state.begin(Some(session));
                state.receive(handle, value, source, Instant::now());
            });
        }
    }
    node
}

pub unsafe extern "system" fn pump(items: usize, dt: f32) {
    let original: unsafe extern "system" fn(usize, f32) =
        std::mem::transmute(PUMP_ORIGINAL.load(Ordering::Acquire));
    if !ACTIVE.load(Ordering::Acquire) {
        reset();
        original(items, dt);
        return;
    }
    let base = BASE.load(Ordering::Acquire);
    let session = guest_session(base, items);
    DEFERRED.with(|state| state.borrow_mut().begin(session));
    original(items, dt);
    // A single native update can dispatch multiple option creates and activations.
    // Wait until it has drained that batch, on the same thread as the receiver.
    let current = guest_session(base, items);
    DEFERRED.with(|state| state.borrow_mut().begin(current));
    if current.is_some() {
        replay(items, base, Instant::now());
    }
}

unsafe fn replay(items: usize, base: usize, now: Instant) {
    let ready = DEFERRED.with(|state| {
        let mut state = state.borrow_mut();
        let mut ready = Vec::new();
        let Deferred {
            pending, applied, ..
        } = &mut *state;
        pending.retain(|notice| {
            if now.duration_since(notice.received) >= LIFETIME {
                return false;
            }
            let node = lookup(items, notice.handle, base);
            if node == 0 {
                return true;
            }
            let source = read_ptr(node + 0x80);
            if applied
                .get(&source)
                .is_none_or(|&order| order < notice.order)
            {
                if source as u32 != u32::MAX {
                    applied.insert(source, notice.order);
                }
                ready.push((*notice, source));
            }
            false
        });
        if pending.is_empty() || applied.len() > MAX_SOURCES {
            pending.clear();
            applied.clear();
        }
        ready
    });
    for (notice, source) in ready {
        let node = lookup(items, notice.handle, base);
        if node == 0 || read_ptr(node + 0x80) != source {
            continue;
        }
        // The same state transition and pending-request cleanup as 57EB00.
        let enable: unsafe extern "system" fn(usize, usize, u8) -> u8 =
            std::mem::transmute(base + ENABLE_RVA);
        let confirm: unsafe extern "system" fn(usize, &u32, usize) =
            std::mem::transmute(base + CONFIRM_RVA);
        enable(items, node, notice.value);
        confirm(items, &notice.handle, source);
    }
}

#[cfg(test)]
#[path = "potential_fission_sync_tests.rs"]
mod tests;
