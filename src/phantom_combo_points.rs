// All slot-2 listeners compare the native total. A credit-only contribution
// crossing still spends the slot, but cannot start another ghost.
use crate::phantom_combo::{self, RESET_POINTS, TRIGGER};
use crate::{read_i32, read_ptr};
use std::cell::Cell;
use std::ptr::{null_mut, read_unaligned, write_unaligned};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicUsize, Ordering};

pub(crate) const SOURCE_RVA: usize = 0x505b90;
pub(crate) const SOURCE_PATTERN: &str =
    "33 C0 48 BA 00 00 00 00 00 00 00 F0 48 85 51 28 74 40 48 8B 49 08";
pub(crate) static SOURCE: AtomicPtr<u8> = AtomicPtr::new(null_mut());
thread_local! { static DISPATCH_MANAGER:Cell<usize> = const {Cell::new(0)}; }
thread_local! { static PHANTOM_DISPATCH: Cell<bool> = const { Cell::new(false) }; }
static CREDIT_ONLY: AtomicBool = AtomicBool::new(false);
static PENDING: AtomicI32 = AtomicI32::new(0);
pub(crate) const DISPATCH_RVA: usize = 0x50e680;
pub(crate) const DISPATCH_PATTERN:&str="48 89 5C 24 10 48 89 6C 24 18 48 89 74 24 20 57 41 54 41 55 41 56 41 57 48 83 EC 20 48 8B F1 48 8B 09 E8 E9 74 FF FF";
pub(crate) const CHECK_RVA: usize = 0x50dbb0;
pub(crate) const CHECK_PATTERN:&str="40 53 48 83 EC 20 8B 51 20 48 8B D9 85 D2 0F 88 A5 00 00 00 48 8B 09 48 85 C9 0F 84 99 00 00 00 48 89 7C 24 30 E8";
pub(crate) static DISPATCH: AtomicPtr<u8> = AtomicPtr::new(null_mut());
pub(crate) static CHECK: AtomicPtr<u8> = AtomicPtr::new(null_mut());
static LOCAL: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn bind(acc: usize) {
    if LOCAL.swap(acc, Ordering::AcqRel) != acc {
        CREDIT_ONLY.store(false, Ordering::Release);
        PENDING.store(0, Ordering::Release);
    }
}
pub(crate) fn queue(amount: i32) {
    if amount > 0 {
        PENDING.fetch_add(amount, Ordering::AcqRel);
    }
}

// Called by the native distributor before it applies the source total to
// accepting slots. Combine the native hit with pending visible-playback credit.
pub(crate) unsafe extern "system" fn source(manager: usize) -> i32 {
    let orig: unsafe extern "system" fn(usize) -> i32 =
        std::mem::transmute(SOURCE.load(Ordering::Acquire));
    let original = orig(manager);
    if manager == 0 || DISPATCH_MANAGER.get() != manager {
        return original;
    }
    let bonus = if PHANTOM_DISPATCH.get() { PENDING.swap(0, Ordering::AcqRel) } else { 0 };
    let actor = read_ptr(manager + 0x10);
    if actor == 0 {
        return original;
    }
    let base = crate::BASE.load(Ordering::Acquire);
    let phantom = if bonus > 0 && phantom_combo::can_credit(actor, base) { bonus } else { 0 };
    let flowing = crate::flowing_swordsman::distribute(manager, actor, original, phantom);
    let real_hit = original.saturating_add(flowing);
    if !PHANTOM_DISPATCH.get() || (real_hit <= 0 && phantom == 0) {
        return real_hit;
    }
    let active: unsafe extern "system" fn(usize, u16) -> u8 = std::mem::transmute(base + 0x508e20);
    let accepts: unsafe extern "system" fn(usize, u16) -> u8 = std::mem::transmute(base + 0x509890);
    if active(manager, 304) != 0 && accepts(manager, 304) != 0 {
        CREDIT_ONLY.store(real_hit <= 0 && phantom > 0, Ordering::Release);
    }
    real_hit.saturating_add(phantom)
}

pub(crate) unsafe extern "system" fn dispatch(acc: usize) -> usize {
    let orig: unsafe extern "system" fn(usize) -> usize =
        std::mem::transmute(DISPATCH.load(Ordering::Acquire));
    let local = acc != 0 && acc == LOCAL.load(Ordering::Acquire);
    let previous_phantom = PHANTOM_DISPATCH.replace(local);
    let previous = DISPATCH_MANAGER.replace(if local || crate::flowing_swordsman::listens(acc) { read_ptr(acc) } else { 0 });
    let result = orig(acc);
    DISPATCH_MANAGER.set(previous);
    PHANTOM_DISPATCH.set(previous_phantom);
    result
}
pub(crate) unsafe extern "system" fn check(listener: usize) -> usize {
    let orig: unsafe extern "system" fn(usize) -> usize =
        std::mem::transmute(CHECK.load(Ordering::Acquire));
    if listener == 0
        || read_i32(listener + 12) != TRIGGER
        || read_i32(listener + 0x20) != 1
        || read_ptr(listener) == 0
        || read_ptr(listener) != LOCAL.load(Ordering::Acquire)
    {
        return orig(listener);
    }
    compare(listener, orig, CREDIT_ONLY.load(Ordering::Acquire))
}

unsafe fn compare(
    listener: usize,
    orig: unsafe extern "system" fn(usize) -> usize,
    credit_only: bool,
) -> usize {
    if !credit_only {
        return orig(listener);
    }
    // The native threshold check copies its output ID into the deferred
    // effect queue. A ghost-only crossing spends the same shared slot but
    // only enqueues the reset; other listeners still see the native total.
    let output = (listener + 12) as *mut i32;
    let saved = read_unaligned(output);
    write_unaligned(output, RESET_POINTS);
    let result = orig(listener);
    write_unaligned(output, saved);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    // Return the effect ID that the native check would copy into its queue.
    unsafe extern "system" fn queued_output(listener: usize) -> usize {
        let acc = read_ptr(listener);
        if read_i32(acc + 12) >= read_i32(listener + 8) {
            read_i32(listener + 12) as usize
        } else {
            0
        }
    }

    #[test]
    fn credit_only_threshold_spends_the_slot_without_another_ghost() {
        let mut acc = [0u64; 8];
        let a = acc.as_mut_ptr() as usize;
        let mut node = [0u64; 8];
        let n = node.as_mut_ptr() as usize;
        unsafe {
            write_unaligned(n as *mut usize, a);
            write_unaligned((n + 8) as *mut i32, 32);
            write_unaligned((n + 12) as *mut i32, TRIGGER);
            for (total, solo_credit, expected) in [
                (31, true, 0),
                (32, true, RESET_POINTS),
                (36, true, RESET_POINTS),
                (8, false, 0),
                (32, false, TRIGGER),
                (36, false, TRIGGER),
            ] {
                write_unaligned((a + 12) as *mut i32, total);
                assert_eq!(compare(n, queued_output, solo_credit), expected as usize);
                assert_eq!(read_i32(n + 8), 32);
                assert_eq!(read_i32(n + 12), TRIGGER);
            }
        }
    }
}
