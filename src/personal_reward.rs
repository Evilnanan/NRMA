//! Scope is restricted to one synchronous custom native spawn on this thread.
use std::sync::atomic::{AtomicU32, Ordering};

pub const SOURCE_KIND: u32 = 0xe000_0000;
static NEXT_SOURCE: AtomicU32 = AtomicU32::new(1);

// Native entity lookup ignores kind 14 and the reward codec preserves it.
// Source comparisons use all 64 bits. Reserve slot zero for local groups and
// distinguish networked grants from different players by the generating slot.
pub fn next_source(sender_slot: u8) -> u64 {
    let serial = NEXT_SOURCE.fetch_add(1, Ordering::Relaxed);
    (u64::from(serial) << 32) | u64::from(SOURCE_KIND | u32::from(sender_slot))
}

#[derive(Clone, Copy, Default)]
pub struct Scope { pub manager: usize, pub lot: i32, pub slot: u8, pub source:Option<u64>, pub created: u32, pub choices:Option<[i32;3]>, pub attempted:u8, pub created_mask:u8, pub nodes:[usize;3] }

#[derive(Debug, PartialEq)]
pub enum Route { Original, Skip, Local(u32) }

pub fn route(scope: Scope, manager: usize, lot: i32, kind: u32, owner: u16) -> Route {
    if scope.manager == 0 || manager != scope.manager || lot != scope.lot || kind & 15 != 2 {
        return Route::Original;
    }
    if ((owner >> 7) & 3) as u8 != scope.slot { return Route::Skip; }
    Route::Local(kind & !0x10)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn filters_all_party_slots_and_only_clears_network_bit() {
        for slot in 1..=3 {
            let scope = Scope { manager: 42, lot: 95200000, slot, created: 0, ..Default::default() };
            for owner in 1..=3 {
                let result = route(scope, 42, scope.lot, 0xfedcba12, 0x4043 | (owner << 7));
                assert_eq!(result, if owner == slot as u16 { Route::Local(0xfedcba02) } else { Route::Skip });
            }
        }
    }
    #[test] fn native_rewards_other_lots_other_managers_and_other_threads_are_untouched() {
        let scope = Scope { manager: 42, lot: 95200000, slot: 1, created: 0, ..Default::default() };
        assert_eq!(route(Scope::default(), 42, scope.lot, 0x12, 0xc0), Route::Original);
        assert_eq!(route(scope, 43, scope.lot, 0x12, 0xc0), Route::Original);
        assert_eq!(route(scope, 42, 6100002, 0x12, 0xc0), Route::Original);
        assert_eq!(route(scope, 42, scope.lot, 0x11, 0xc0), Route::Original);
    }
}
