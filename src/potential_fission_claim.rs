//! Consume a networked parent through the native pickup handshake.
use super::{
    lookup, Ball, Key, GUEST, HOST, PICKUP_ORIGINAL, REMOVE_GROUP_RVA, SESSION_RVA, STATE,
};
use crate::{ptr, read_i32, read_ptr, ACTIVE, ARTS_PICKUP_RVA, BASE, ITEMS_RVA};
use std::sync::atomic::{AtomicPtr, Ordering};

pub const ACK_RVA: usize = 0x57eac8;
pub const ACK_PATTERN: &str = "E8 93 DF FF FF 48 8B 6C 24 50 48 8B 5C 24 58 48";
pub const REMOVAL_RVA: usize = 0x57d8aa;
pub const REMOVAL_PATTERN: &str = "E8 51 FB FF FF 48 8D 55 77 48 8B CF E8 E5 3B 00 00";
pub const BROADCAST_RVA: usize = 0x57fb50;
pub const BROADCAST_PATTERN: &str =
    "48 89 5C 24 10 56 48 83 EC 20 48 8B 05 67 23 6A 03 8B DA 48 8B F1";
const DROP_REQUEST_RVA: usize = 0x5814a0;

static ACK_RESUME: AtomicPtr<u8> = AtomicPtr::new(ptr::null_mut());
static REMOVAL_RESUME: AtomicPtr<u8> = AtomicPtr::new(ptr::null_mut());

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Session {
    instance: usize,
    host: usize,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Pending {
    session: Session,
    handle: u32,
    uid: u32,
    confirmed: bool,
    removed_by_host: bool,
}

pub(super) enum Progress {
    Wait,
    Ready,
    Cancel { remove_local: bool },
}

unsafe fn guest_session(base: usize, items: usize) -> Option<Session> {
    let instance = read_ptr(base + SESSION_RVA);
    if instance == 0
        || items != read_ptr(base + ITEMS_RVA)
        || read_i32(instance + 0xc) != GUEST
        || read_i32(instance + 0x10) != 7
    {
        return None;
    }
    let host = read_ptr(instance + 0xa0);
    (host != 0).then_some(Session { instance, host })
}

pub(super) unsafe fn mode(base: usize) -> i32 {
    let session = read_ptr(base + SESSION_RVA);
    if session == 0 {
        0
    } else {
        read_i32(session + 0xc)
    }
}

unsafe fn queued(items: usize, handle: u32) -> bool {
    if handle == 0 {
        return false;
    }
    let start = read_ptr(items + 0x468);
    let end = read_ptr(items + 0x470);
    let size = end.saturating_sub(start);
    start != 0
        && size % 12 == 0
        && size / 12 <= 65536
        && (start..end)
            .step_by(12)
            .any(|entry| read_i32(entry) as u32 == handle)
}

pub(super) unsafe fn advance(
    items: usize,
    base: usize,
    key: Key,
    ball: Ball,
    (node, handle): (usize, u32),
) -> Progress {
    let session = guest_session(base, items);
    if let Some(pending) = ball.claim {
        let selected = lookup(items, pending.handle, base);
        if session != Some(pending.session)
            || selected == 0
            || read_i32(selected + 0x7c) as u32 != pending.uid
        {
            return Progress::Cancel {
                remove_local: false,
            };
        }
        if pending.confirmed {
            return Progress::Ready;
        }
        // The native queue owns timeout and resubmission. A removal message may
        // be held locally without consuming the request that its ACK needs.
        if queued(items, pending.handle) {
            return Progress::Wait;
        }
        return Progress::Cancel {
            remove_local: pending.removed_by_host,
        };
    }
    let Some(session) = session else {
        return Progress::Wait;
    };
    if ball.handles.into_iter().any(|option| queued(items, option)) {
        return Progress::Cancel {
            remove_local: false,
        };
    }
    if ptr::read((items + 0x4c0) as *const u8) != 0 {
        return Progress::Wait;
    }
    let uid = read_i32(node + 0x7c) as u32;
    let pending = Pending {
        session,
        handle,
        uid,
        confirmed: false,
        removed_by_host: false,
    };
    {
        let Ok(mut state) = STATE.lock() else {
            return Progress::Wait;
        };
        let Some(original) = state.originals.get_mut(&key) else {
            return Progress::Wait;
        };
        original.claim = Some(pending);
    }
    // This trampoline deliberately bypasses our manual pickup lock. The game
    // sends type 6 and records its own pending entry, even during native flight.
    let pickup: unsafe extern "system" fn(usize, u32, i32) -> usize =
        std::mem::transmute(PICKUP_ORIGINAL.load(Ordering::Acquire));
    pickup(items, handle, -1);
    if !queued(items, handle) {
        if let Ok(mut state) = STATE.lock() {
            if let Some(original) = state.originals.get_mut(&key) {
                if original.claim.is_some_and(|claim| !claim.confirmed) {
                    original.claim = None;
                }
            }
        }
    }
    Progress::Wait
}

unsafe fn pending_for(
    items: usize,
    handle: u32,
    change: impl FnOnce(&mut Pending) -> bool,
) -> bool {
    if !ACTIVE.load(Ordering::Acquire) {
        return false;
    }
    let Some(session) = guest_session(BASE.load(Ordering::Acquire), items) else {
        return false;
    };
    let Ok(mut state) = STATE.lock() else {
        return false;
    };
    let Some(pending) = state
        .originals
        .values_mut()
        .filter_map(|ball| ball.claim.as_mut())
        .find(|pending| pending.handle == handle && pending.session == session)
    else {
        return false;
    };
    change(pending)
}

unsafe extern "system" fn acknowledge(
    items: usize,
    descriptor: usize,
    source: usize,
    out: usize,
    position: usize,
    flag: u8,
    choice: i32,
) -> usize {
    let handle = read_i32(descriptor + 0x58) as u32;
    let uid = read_i32(descriptor + 0x5c) as u32;
    if pending_for(items, handle, |pending| {
        if pending.uid != uid {
            return false;
        }
        pending.confirmed = true;
        true
    }) {
        return 0;
    }
    let native: crate::ArtsPickupFn =
        std::mem::transmute(BASE.load(Ordering::Acquire) + ARTS_PICKUP_RVA);
    native(items, descriptor, source, out, position, flag, choice)
}

unsafe extern "system" fn remove_broadcast(items: usize, handle: u32, flag: u8) {
    if pending_for(items, handle, |pending| {
        pending.removed_by_host = true;
        true
    }) {
        return;
    }
    let base = BASE.load(Ordering::Acquire);
    let remove: unsafe extern "system" fn(usize, u32, u8) -> u8 =
        std::mem::transmute(base + REMOVE_GROUP_RVA);
    let drop_request: unsafe extern "system" fn(usize, *const u32) =
        std::mem::transmute(base + DROP_REQUEST_RVA);
    remove(items, handle, flag);
    drop_request(items, &handle);
}

// These replace a native CALL. Push its continuation to retain the original
// Windows x64 ABI, including all three stack arguments to the award routine.
#[unsafe(naked)]
unsafe extern "system" fn ack_hook() {
    core::arch::naked_asm!("push qword ptr [rip+{resume}]", "jmp {function}",
        resume=sym ACK_RESUME, function=sym acknowledge);
}

#[unsafe(naked)]
unsafe extern "system" fn removal_hook() {
    core::arch::naked_asm!("push qword ptr [rip+{resume}]", "jmp {function}",
        resume=sym REMOVAL_RESUME, function=sym remove_broadcast);
}

pub unsafe fn prepare(base: usize) -> Result<Vec<(usize, crate::hook::PreparedHook)>, String> {
    let mut hooks = Vec::new();
    for (rva, target, resume, length) in [
        (ACK_RVA, ack_hook as *const u8, &ACK_RESUME, 5),
        (REMOVAL_RVA, removal_hook as *const u8, &REMOVAL_RESUME, 17),
    ] {
        let hook = crate::hook::prepare((base + rva) as *mut u8, target, length)?;
        if hook.stolen != 5 {
            return Err(format!("Unexpected fission pickup call width at {rva:x}"));
        }
        resume.store((base + rva + length) as *mut u8, Ordering::Release);
        hooks.push((rva, hook));
    }
    Ok(hooks)
}

pub(super) unsafe fn remove_parent(items: usize, base: usize, handle: u32, kind: i32) -> bool {
    let remove: unsafe extern "system" fn(usize, u32, u8) -> u8 =
        std::mem::transmute(base + REMOVE_GROUP_RVA);
    if remove(items, handle, u8::from(kind == HOST || kind == GUEST)) == 0 {
        return false;
    }
    if kind == HOST {
        let broadcast: unsafe extern "system" fn(usize, u32) =
            std::mem::transmute(base + BROADCAST_RVA);
        broadcast(items, handle);
    }
    true
}
