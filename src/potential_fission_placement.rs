//! Convert a selected private fission weapon to a native shared ground item.
use super::{generated, local, lookup, GUEST, HOST, PLACE_ORIGINAL, REMOVE_GROUP_RVA, SESSION_RVA, STATE};
use crate::{ptr, read_i32, read_ptr, ACTIVE, BASE, ITEMS_RVA, REWARD_CREATE_RVA};
use std::collections::BTreeMap;
use std::sync::{atomic::Ordering, Mutex};

const CHOICE: u32 = 2;
const NETWORK_DROP: u32 = 0x11;
const ENABLED: u16 = 0x40;
const OWNER_MASK: u16 = 0x180;

#[derive(Clone, Copy)]
struct Placement {
    manager: usize,
    group: u32,
    selected: u32,
    selected_uid: u32,
    weapon: u32,
    confirmed: bool,
}

static PENDING: Mutex<BTreeMap<u32, Placement>> = Mutex::new(BTreeMap::new());

pub(super) fn reset() {
    if let Ok(mut pending) = PENDING.lock() {
        pending.clear();
    }
}

unsafe fn get_u32(address: usize) -> u32 {
    ptr::read_unaligned(address as *const u32)
}

unsafe fn head(mut node: usize) -> Option<usize> {
    for _ in 0..3 {
        let previous = read_ptr(node + 0x18);
        if previous == 0 {
            return Some(node);
        }
        node = previous;
    }
    None
}

pub(super) unsafe fn reserved(manager: usize, handle: u32, base: usize) -> bool {
    let Ok(pending) = PENDING.lock() else {
        return false;
    };
    if pending.is_empty() {
        return false;
    }
    let node = lookup(manager, handle, base);
    if node == 0 {
        return false;
    }
    head(node).is_some_and(|group| {
        let root = get_u32(group + 0x78);
        pending
            .values()
            .any(|p| p.manager == manager && p.group == root)
    })
}

// Match the native type-1 inventory placement descriptor. Keep all 0x50 bytes
// of ItemData from the chosen option and its original map-local landing point.
fn public_drop(option: &[u8; 0xc0], uid: u32) -> [u8; 0xc0] {
    let mut result = [0; 0xc0];
    result[0] = 1;
    result[0x08..0x58].copy_from_slice(&option[0x08..0x58]);
    result[0x58..0x5c].copy_from_slice(&NETWORK_DROP.to_le_bytes());
    result[0x5c..0x60].copy_from_slice(&uid.to_le_bytes());
    result[0x60..0x70].fill(0xff);
    result[0x74..0x84].copy_from_slice(&option[0x74..0x84]);
    result[0x84..0x90].copy_from_slice(&option[0x78..0x84]);
    result[0x90..0x94].copy_from_slice(&option[0x90..0x94]);
    result[0x94] = 0x5a;
    result[0xa0..0xa4].fill(0xff);
    result[0xa9] = 0xff;
    result[0xaa..0xac].copy_from_slice(&ENABLED.to_le_bytes());
    result[0xac..0xb0].fill(0xff);
    result[0xb8..0xbc].fill(0xff);
    result
}

pub(super) unsafe fn created(manager: usize, descriptor: usize) {
    let uid = get_u32(descriptor + 0x5c);
    if uid >> 29 != 1 || read_i32(descriptor + 0x70) != 0 {
        return;
    }
    if let Ok(mut pending) = PENDING.lock() {
        if let Some(receipt) = pending.get_mut(&uid) {
            if receipt.manager == manager && receipt.weapon == get_u32(descriptor + 8) {
                receipt.confirmed = true;
            }
        }
    }
}

unsafe fn queued(manager: usize, uid: u32) -> bool {
    // Native guest create requests remain here until the host echoes the drop.
    let start = read_ptr(manager + 0x448);
    let end = read_ptr(manager + 0x450);
    let size = end.saturating_sub(start);
    start != 0
        && size % 0xc8 == 0
        && size / 0xc8 <= 65536
        && (start..end)
            .step_by(0xc8)
            .any(|entry| get_u32(entry + 0x5c) == uid)
}

pub(super) unsafe fn finish(manager: usize, base: usize) {
    if manager == 0 {
        return;
    }
    let ready = PENDING
        .lock()
        .map(|pending| {
            pending
                .iter()
                .filter_map(|(&uid, &p)| (p.confirmed && p.manager == manager).then_some((uid, p)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let remove: unsafe extern "system" fn(usize, u32, u8) -> u8 =
        std::mem::transmute(base + REMOVE_GROUP_RVA);
    for (uid, receipt) in ready {
        let node = lookup(manager, receipt.selected, base);
        if node != 0
            && get_u32(node + 0x7c) == receipt.selected_uid
            && head(node).is_some_and(|root| get_u32(root + 0x78) == receipt.group)
            && remove(manager, receipt.selected, 0) == 0
        {
            continue;
        }
        if let Ok(mut pending) = PENDING.lock() {
            pending.remove(&uid);
        }
    }
}

pub(crate) unsafe extern "system" fn place(manager: usize, handle: u32) {
    let original: unsafe extern "system" fn(usize, u32) =
        std::mem::transmute(PLACE_ORIGINAL.load(Ordering::Acquire));
    let base = BASE.load(Ordering::Acquire);
    if !ACTIVE.load(Ordering::Acquire) || manager == 0 || manager != read_ptr(base + ITEMS_RVA) {
        original(manager, handle);
        return;
    }
    if STATE.lock().is_ok_and(|state| state.locked.contains(&handle)) {
        return;
    }
    let node = lookup(manager, handle, base);
    if node == 0 || handle & 0x1f != CHOICE || !generated(read_i32(node + 0xd8)) {
        original(manager, handle);
        return;
    }
    let weapon = get_u32(node + 0x28);
    if weapon == 0 || weapon >= 0x0fff_ffff {
        original(manager, handle);
        return;
    }
    let session = read_ptr(base + SESSION_RVA);
    let mode = if session == 0 {
        0
    } else {
        read_i32(session + 0xc)
    };
    if !matches!(mode, HOST | GUEST) {
        original(manager, handle);
        return;
    }
    let Some((_, slot)) = local(base) else {
        return;
    };
    let flags = ptr::read_unaligned((node + 0xca) as *const u16);
    let Some(group) = head(node) else {
        return;
    };
    let root = get_u32(group + 0x78);
    let selected_uid = get_u32(node + 0x7c);
    if flags & OWNER_MASK != u16::from(slot) << 7
        || flags & ENABLED == 0
        || ptr::read((node + 0x20) as *const u8) != 1
        || ptr::read((node + 0xf9) as *const u8) == 1
        || (mode == GUEST && ptr::read((manager + 0x4c0) as *const u8) != 0)
        || STATE
            .lock()
            .is_ok_and(|state| state.locked.contains(&handle))
    {
        return;
    }
    let uid = {
        let Ok(mut pending) = PENDING.lock() else {
            return;
        };
        if pending
            .values()
            .any(|p| p.manager == manager && p.group == root)
        {
            return;
        }
        // Same sequence and creator namespace as the native ordinary drop.
        let counter = get_u32(manager + 0x1c).wrapping_add(1);
        let uid = 0x20000000 | (u32::from(slot) << 27) | (counter & 0x07ffffff);
        if pending.contains_key(&uid) {
            return;
        }
        ptr::write_unaligned((manager + 0x1c) as *mut u32, counter % 0xffffff);
        pending.insert(
            uid,
            Placement {
                manager,
                group: root,
                selected: handle,
                selected_uid,
                weapon,
                confirmed: false,
            },
        );
        uid
    };

    let option = ptr::read_unaligned((node + 0x20) as *const [u8; 0xc0]);
    let drop = public_drop(&option, uid);
    let dispatch: unsafe extern "system" fn(usize, usize) -> usize =
        std::mem::transmute(base + REWARD_CREATE_RVA);
    dispatch(manager, drop.as_ptr() as usize);

    if let Ok(mut pending) = PENDING.lock() {
        let accepted = pending.get(&uid).is_some_and(|p| p.confirmed)
            || (mode == GUEST && queued(manager, uid));
        if !accepted {
            pending.remove(&uid);
        }
    }
    finish(manager, base);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_descriptor_keeps_exact_weapon_and_position_without_choice_metadata() {
        let mut option = [0xa5; 0xc0];
        option[0x74..0x84].copy_from_slice(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
        let uid = 0x30001234u32;
        let drop = public_drop(&option, uid);
        assert_eq!(drop[0], 1);
        assert_eq!(drop[8..0x58], option[8..0x58]);
        assert_eq!(drop[0x58..0x5c], NETWORK_DROP.to_le_bytes());
        assert_eq!(drop[0x5c..0x60], uid.to_le_bytes());
        assert_eq!(drop[0x60..0x70], [0xff; 16]);
        assert_eq!(drop[0x74..0x84], option[0x74..0x84]);
        assert_eq!(drop[0x84..0x90], option[0x78..0x84]);
        assert_eq!(drop[0xa9], 0xff);
        assert_eq!(drop[0xaa..0xac], ENABLED.to_le_bytes());
        assert_eq!(drop[0xb8..0xbc], (-1i32).to_le_bytes());
    }
}
