//! Carry the native precision-aim state from arrow release to accepted damage.
use crate::{has_effect, read_i32, read_ptr, ACTIVE, WORLD_RVA};
use std::{
    cell::Cell,
    collections::BTreeMap,
    sync::{atomic::Ordering, Mutex},
};

// The two queries used by the native HKS IsPrecisionShoot (234) environment command.
pub(crate) const SCOPED_RVA: usize = 0x3d4850;
pub(crate) const SCOPED_PATTERN: &str = "0F B6 81 F3 01 00 00 D0 E8 24 01 C3";
pub(crate) const AIM_EFFECT_RVA: usize = 0x3d4c50;
pub(crate) const AIM_EFFECT_PATTERN: &str =
    "40 53 48 83 EC 20 48 8B 89 90 01 00 00 66 BA 0F 00 32 DB E8";
const BOW_ATTACK_CATEGORY: usize = 105;

#[derive(Clone, Copy)]
struct Shot(u64);

struct Flights {
    groups: BTreeMap<usize, (i32, Shot)>,
    instances: BTreeMap<usize, (usize, Shot)>,
}

static FLIGHTS: Mutex<Flights> = Mutex::new(Flights {
    groups: BTreeMap::new(),
    instances: BTreeMap::new(),
});
thread_local! {
    static SHOT: Cell<Option<Shot>> = const { Cell::new(None) };
    static CHILD: Cell<Option<Option<Shot>>> = const { Cell::new(None) };
}

pub(crate) struct ShotGuard(Option<Shot>);
impl ShotGuard {
    fn replace(shot: Option<Shot>) -> Self {
        Self(SHOT.with(|slot| slot.replace(shot)))
    }
}
impl Drop for ShotGuard {
    fn drop(&mut self) {
        SHOT.with(|slot| slot.set(self.0));
    }
}

pub(crate) unsafe fn factory(payload: usize, base: usize) -> ShotGuard {
    let shot = if !ACTIVE.load(Ordering::Acquire) || payload == 0 {
        None
    } else {
        let owner = read_ptr(payload + 0x10) as u64;
        match CHILD.with(Cell::get) {
            Some(parent) => parent.filter(|shot| shot.0 == owner),
            None => at_release(owner, base),
        }
    };
    ShotGuard::replace(shot)
}

unsafe fn at_release(owner: u64, base: usize) -> Option<Shot> {
    let world = read_ptr(base + WORLD_RVA);
    if world == 0 {
        return None;
    }
    let local = read_ptr(world + 0x174e8);
    if local == 0
        || read_ptr(local + 8) as u64 != owner
        || !has_effect(local, crate::sniper::OWNER, base)
    {
        return None;
    }
    let scoped: unsafe extern "system" fn(usize) -> u8 = std::mem::transmute(base + SCOPED_RVA);
    let aim_effect: unsafe extern "system" fn(usize) -> u8 =
        std::mem::transmute(base + AIM_EFFECT_RVA);
    (scoped(local) != 0 || (read_ptr(local + 0x190) != 0 && aim_effect(local) != 0))
        .then_some(Shot(owner))
}

pub(crate) unsafe fn group_created(group: usize) {
    if group == 0 {
        return;
    }
    if let Ok(mut flights) = FLIGHTS.lock() {
        flights.groups.remove(&group);
        if let Some(shot) = SHOT.with(Cell::get) {
            flights.groups.insert(group, (read_i32(group), shot));
        }
    }
}

pub(crate) unsafe fn initialize(group: usize) -> ShotGuard {
    let shot = FLIGHTS.lock().ok().and_then(|flights| {
        let &(id, shot) = flights.groups.get(&group)?;
        (id == read_i32(group)).then_some(shot)
    });
    ShotGuard::replace(shot.or_else(|| SHOT.with(Cell::get)))
}

pub(crate) unsafe fn bound(instance: usize) {
    if instance == 0 {
        return;
    }
    if let Ok(mut flights) = FLIGHTS.lock() {
        flights.instances.remove(&instance);
        if let Some(shot) = SHOT
            .with(Cell::get)
            .filter(|shot| shot.0 == read_ptr(instance + 0xb0) as u64)
        {
            flights
                .instances
                .insert(instance, (read_ptr(instance + 8), shot));
        }
    }
}

unsafe fn recorded(instance: usize) -> Option<Shot> {
    let flights = FLIGHTS.lock().ok()?;
    let &(generation, shot) = flights.instances.get(&instance)?;
    (generation == read_ptr(instance + 8) && shot.0 == read_ptr(instance + 0xb0) as u64)
        .then_some(shot)
}

pub(crate) struct ChildGuard(Option<Option<Shot>>);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        CHILD.with(|slot| slot.set(self.0));
    }
}
pub(crate) unsafe fn child(instance: usize) -> ChildGuard {
    ChildGuard(CHILD.with(|slot| slot.replace(Some(recorded(instance)))))
}

pub(crate) fn finished(group: usize) {
    if let Ok(mut flights) = FLIGHTS.lock() {
        flights.groups.remove(&group);
    }
}

pub(crate) fn retired(instance: usize) {
    if let Ok(mut flights) = FLIGHTS.lock() {
        flights.instances.remove(&instance);
    }
}

pub(crate) unsafe fn accepted_hit(hit: usize, owner: u64) -> bool {
    // The native AtkParam subcategory appears in the constructed hit mask.
    // Only ranged weapon projectile damage can consume the release snapshot.
    let flags = read_i32(hit + 0x124 + 4 * (BOW_ATTACK_CATEGORY / 32)) as u32;
    flags & (1 << (BOW_ATTACK_CATEGORY % 32)) != 0
        && recorded(read_ptr(hit + 0x228)).is_some_and(|shot| shot.0 == owner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrow_keeps_release_aim_across_delayed_creation_but_hip_fire_cannot_gain_it() {
        unsafe {
            let mut group = [0i32; 1];
            let mut arrow = [0usize; 24];
            arrow[1] = 32;
            arrow[0xb0 / 8] = 91;
            let group_addr = group.as_mut_ptr() as usize;
            let arrow_addr = arrow.as_mut_ptr() as usize;
            let mut hit = [0u8; 0x230];
            hit[0x228..0x230].copy_from_slice(&arrow_addr.to_le_bytes());
            let offset = 0x124 + 4 * (BOW_ATTACK_CATEGORY / 32);
            let hit_addr = hit.as_mut_ptr() as usize;
            {
                let _release = ShotGuard::replace(Some(Shot(91)));
                group_created(group_addr);
            }
            {
                let _initialize = initialize(group_addr);
                bound(arrow_addr);
            }
            finished(group_addr);
            assert!(!accepted_hit(hit_addr, 91));
            hit[offset + 1] = 2;
            assert!(accepted_hit(hit_addr, 91));
            assert!(!accepted_hit(hit_addr, 90));
            {
                let _child = child(arrow_addr);
                assert_eq!(CHILD.with(Cell::get).flatten().unwrap().0, 91);
            }
            arrow[1] += 1;
            assert!(!accepted_hit(hit_addr, 91));
            retired(arrow_addr);
            group[0] += 1;
            group_created(group_addr);
            let _initialize = initialize(group_addr);
            bound(arrow_addr);
            assert!(!accepted_hit(hit_addr, 91));
        }
    }
}
