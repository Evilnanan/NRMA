//! Precision-aimed shots fire the uncharged Loretta volley and refund its cooldown.
use crate::ram_motion::V;
use crate::{has_effect, native_actor, read_i32, read_ptr};
use std::sync::{atomic::Ordering, Mutex};

pub const OWNER: i32 = 95004500;
const LAUNCH_HEIGHT: f32 = 1.5;
pub(crate) const TARGET_TRANSFORM_RVA: usize = 0x3d14f0;
pub(crate) const TARGET_TRANSFORM_PATTERN: &str =
    "48 8B 41 60 0F 10 80 40 02 00 00 0F 10 88 50 02 00 00 0F 29 02 0F 10 80 60 02 00 00 0F 29 4A 10 0F 10 88 70 02 00 00 48 8B C2 0F 29 42 20 0F 29 4A 30 C3 48 89 43 38";
pub(crate) const ACTOR_TARGET_RVA: usize = 0x349790;
pub(crate) const ACTOR_TARGET_PATTERN: &str =
    "48 8B C4 48 89 58 18 48 89 50 10 55 56 57 41 56 41 57 48 81 EC D0";
const COOLDOWN: f32 = 9.0;
// Timed recipient 8600105 of native HP-on-hit lasts 0.3 s.
// Use one non-refreshing window per shooter, shared by every target and attack.
const REFUND_INTERVAL: f32 = 0.3;
// Each level selects its own root, flight and damage rows, in that order.
const FIRST_ROOT: i32 = 95004510;
const LEVELS: i32 = 15;
const CHAIN_ROWS: i32 = 3;
const MAGIC: i32 = 4381;
type Point = [f32; 4];
type Identity = (u64, usize);

#[derive(Clone, Copy, Debug, PartialEq)]
struct Trigger {
    victim: u64,
    offset: Option<Point>,
}

#[derive(Default)]
struct State {
    owner: Option<Identity>,
    cooldown: f32,
    refund_gate: f32,
    pending: Option<Trigger>,
}

impl State {
    fn bind(&mut self, owner: Option<Identity>) {
        if self.owner != owner {
            *self = Self {
                owner,
                ..Self::default()
            };
        }
    }
    fn hit(&mut self, trigger: Option<Trigger>) {
        if self.pending.is_some() {
            return;
        }
        if self.cooldown <= 0.0 {
            let Some(trigger) = trigger else {
                return;
            };
            self.pending = Some(trigger);
            self.cooldown = COOLDOWN;
            self.refund_gate = REFUND_INTERVAL;
        } else if self.refund_gate <= 0.0 {
            self.cooldown = (self.cooldown - 1.0).max(0.0);
            self.refund_gate = REFUND_INTERVAL;
        }
    }
    fn advance(&mut self, dt: f32) -> Option<Trigger> {
        self.cooldown = (self.cooldown - dt).max(0.0);
        self.refund_gate = (self.refund_gate - dt).max(0.0);
        std::mem::take(&mut self.pending)
    }
}
static STATE: Mutex<State> = Mutex::new(State {
    owner: None,
    cooldown: 0.,
    refund_gate: 0.,
    pending: None,
});

unsafe fn identity(actor: usize, base: usize) -> Option<Identity> {
    if !has_effect(actor, OWNER, base)
        || has_effect(actor, crate::reward_cycle_runtime::RESET, base)
    {
        return None;
    }
    let stats = native_actor::stats(actor);
    (stats != 0 && !native_actor::incapacitated(actor, stats))
        .then(|| (read_ptr(actor + 8) as u64, stats))
}

pub unsafe fn outgoing(module: usize, source: usize, hit: usize) {
    if module == 0
        || source == 0
        || hit == 0
        || read_i32(hit + 0x280) <= 0
        || std::ptr::read((hit + 0x2c3) as *const u8) & 2 != 0
    {
        return;
    }
    let base = crate::BASE.load(Ordering::Acquire);
    if !crate::is_local(source, base) {
        return;
    }
    let Some(owner) = identity(source, base) else {
        return;
    };
    let target = read_ptr(module + 8);
    if target == 0
        || target == source
        || crate::ambulance::party_member(target, base)
        || !crate::spatial_ready(source)
    {
        return;
    }
    let trigger = if read_i32(hit + 0x44) == 1 && is_volley_attack(read_i32(hit + 0x40)) {
        // A volley still refunds its own active cooldown, but cannot fire a new volley.
        None
    } else {
        if !crate::sniper_aim::accepted_hit(hit, owner.0) {
            return;
        }
        Some(Trigger {
            victim: read_ptr(target + 8) as u64,
            offset: hit_offset(target, crate::sword_hit_point(hit), base),
        })
    };
    if crate::static_charge::first_sniper_contact(module, hit) {
        if let Ok(mut state) = STATE.lock() {
            state.bind(Some(owner));
            state.hit(trigger);
        }
    }
}

fn root_for_level(level: i32) -> i32 {
    FIRST_ROOT + (level.clamp(1, LEVELS) - 1) * CHAIN_ROWS
}

fn is_volley_root(id: i32) -> bool {
    (FIRST_ROOT..FIRST_ROOT + LEVELS * CHAIN_ROWS).contains(&id)
        && (id - FIRST_ROOT) % CHAIN_ROWS == 0
}

fn is_volley_attack(id: i32) -> bool {
    id.checked_sub(CHAIN_ROWS - 1).is_some_and(is_volley_root)
}

fn shot_descriptor(
    owner: u64,
    level: i32,
    mut feet: Point,
    facing: Point,
) -> Option<crate::direct_bullet::Descriptor> {
    feet[1] += LAUNCH_HEIGHT;
    feet[3] = 1.;
    let forward = V(facing[0], 0., facing[2]).unit();
    if !V::from(feet).finite() || !forward.finite() || forward.len() < 0.5 {
        return None;
    }
    let up = V(0., 1., 0.);
    let right = up.cross(forward).unit();
    let mut d = crate::direct_bullet::descriptor(owner, root_for_level(level), feet);
    d.0[0x18..0x1c].copy_from_slice(&MAGIC.to_le_bytes());
    d.0[0x44..0x48].copy_from_slice(&15u32.to_le_bytes());
    for (slot, v) in d.0[0x50..0x80].chunks_exact_mut(4).zip(
        [right.array(), forward.cross(right).array(), forward.array()]
            .into_iter()
            .flatten(),
    ) {
        slot.copy_from_slice(&v.to_le_bytes());
    }
    Some(d)
}

thread_local! {
    static FIRING: std::cell::Cell<*const u8> = const { std::cell::Cell::new(std::ptr::null()) };
}

// The native targeting pass may replace the supplied pose/target. Reapply both
// at the same final factory boundary as Echo Cast, for our root only.
pub unsafe fn factory(payload: usize, base: usize) {
    if payload == 0 || !is_volley_root(read_i32(payload + 4)) {
        return;
    }
    FIRING.with(|slot| {
        let d = slot.get();
        if d.is_null()
            || read_i32(payload + 4) != read_i32(d as usize + 0x14)
            || read_ptr(payload + 0x10) != read_ptr(d as usize)
        {
            return;
        }
        let target = read_ptr(payload + 0x148);
        if target != 0 && read_ptr(payload + 0x150).wrapping_sub(target) == 0x50 {
            let copy: unsafe extern "system" fn(usize, usize) -> usize =
                std::mem::transmute(base + crate::echo_cast::TARGET_COPY_RVA);
            copy(target, d.add(0xb0) as usize);
        }
        std::ptr::copy_nonoverlapping(d.add(0x50), (payload + 0x190) as *mut u8, 0x40);
    });
}

#[repr(align(16))]
struct AlignedPoint(Point);

#[repr(align(16))]
struct TargetTransform([Point; 4]);

fn world_to_target(contact: Point, transform: [Point; 4]) -> Option<Point> {
    let [x, y, z, origin] = transform.map(V::from);
    let delta = V::from(contact).sub(origin);
    let normal = y.cross(z);
    let determinant = x.dot(normal);
    if !determinant.is_finite() || determinant.abs() < 1e-6 {
        return None;
    }
    let offset = V(
        delta.dot(normal) / determinant,
        delta.dot(z.cross(x)) / determinant,
        delta.dot(x.cross(y)) / determinant,
    );
    offset
        .finite()
        .then_some([offset.0, offset.1, offset.2, 1.])
}

unsafe fn hit_offset(target: usize, contact: Point, base: usize) -> Option<Point> {
    if read_ptr(target + 0x60) == 0 || !V::from(contact).finite() {
        return None;
    }
    let get_transform: unsafe extern "system" fn(usize, *mut TargetTransform) -> usize =
        std::mem::transmute(base + TARGET_TRANSFORM_RVA);
    let mut transform = TargetTransform([[0.; 4]; 4]);
    get_transform(target, &mut transform);
    world_to_target(contact, transform.0)
}

unsafe fn set_hit_target(d: &mut crate::direct_bullet::Descriptor, hit: Trigger, base: usize) {
    if hit.victim as u32 == u32::MAX {
        return;
    }
    let actor_target: unsafe extern "system" fn(*mut u8, u64, u8) -> usize =
        std::mem::transmute(base + ACTOR_TARGET_RVA);
    // Kind 2 without a lock-node reference. Native targeting multiplies the
    // local offset at +20 by the victim's current root transform every time.
    actor_target(d.0.as_mut_ptr().add(0xb0), hit.victim, 0);
    if let Some(offset) = hit.offset.filter(|_| d.0[0xb0] == 2) {
        for (field, value) in d.0[0xd0..0xe0].chunks_exact_mut(4).zip(offset) {
            field.copy_from_slice(&value.to_le_bytes());
        }
    }
}

unsafe fn emit(actor: usize, owner: u64, hit: Trigger, base: usize) -> bool {
    let manager = read_ptr(base + 0x3c0ae30);
    let entry = crate::echo_cast::EMIT_ORIGINAL.load(Ordering::Acquire);
    if manager == 0 || entry.is_null() || !crate::spatial_ready(actor) {
        return false;
    }
    let mut facing = AlignedPoint([0.; 4]);
    let get_forward: unsafe extern "system" fn(usize, *mut f32) -> usize =
        std::mem::transmute(base + crate::FORWARD_RVA);
    get_forward(actor, facing.0.as_mut_ptr());
    let level = crate::static_charge::level(base);
    let Some(mut d) = shot_descriptor(owner, level, crate::position(actor, base), facing.0) else {
        return false;
    };
    set_hit_target(&mut d, hit, base);
    let fire: unsafe extern "system" fn(usize, *mut i32, *const u8, *mut i32) -> usize =
        std::mem::transmute(entry);
    let mut handle = -1;
    let mut error = -1;
    FIRING.with(|slot| {
        let old = slot.replace(d.0.as_ptr());
        fire(manager, &mut handle, d.0.as_ptr(), &mut error);
        slot.set(old);
    });
    let release: unsafe extern "system" fn(*mut u8) =
        std::mem::transmute(base + crate::echo_cast::RELEASE_RVA);
    release(d.0.as_mut_ptr());
    handle >= 0
}

pub unsafe fn update(actor: usize, base: usize, dt: f32) {
    let owner = identity(actor, base);
    let pending = {
        let Ok(mut state) = STATE.lock() else {
            return;
        };
        state.bind(owner);
        state.advance(dt)
    };
    if let (Some(owner), Some(hit)) = (owner, pending) {
        if !emit(actor, owner.0, hit, base) {
            if let Ok(mut state) = STATE.lock() {
                if state.owner == Some(owner) {
                    state.cooldown = 0.;
                    state.refund_gate = 0.;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refunds_are_shared_by_arrows_and_targets() {
        let mut s = State::default();
        s.bind(Some((1, 2)));
        let first = Trigger {
            victim: 12,
            offset: Some([1., 0., 1., 1.]),
        };
        let other = Trigger {
            victim: 20,
            offset: Some([0., 3., 0., 1.]),
        };
        s.hit(None);
        assert!(s.pending.is_none());
        s.hit(Some(first));
        s.hit(Some(other));
        assert_eq!(s.advance(0.), Some(first));
        assert_eq!(s.cooldown, 9.);
        s.advance(0.3);
        for _ in 1..=20 {
            s.hit(None);
            s.hit(Some(other));
        }
        assert!((s.cooldown - 7.7).abs() < 1e-4);
        s.advance(7.0);
        s.hit(Some(other));
        assert_eq!(s.cooldown, 0.);
        assert!(s.pending.is_none());
        s.hit(None);
        assert!(s.pending.is_none());
        s.hit(Some(other));
        assert_eq!(s.pending, Some(other));
        s.bind(None);
        assert!(s.pending.is_none());
        assert_eq!(s.cooldown, 0.);
    }
    #[test]
    fn hit_point_is_preserved_in_the_victims_rotated_and_scaled_coordinates() {
        let pose = [
            [0., 0., -2., 0.],
            [0., 3., 0., 0.],
            [4., 0., 0., 0.],
            [10., 20., 30., 1.],
        ];
        assert_eq!(
            world_to_target([22., 26., 28., 0.], pose),
            Some([1., 2., 3., 1.])
        );
        assert_eq!(world_to_target([f32::NAN, 26., 28., 1.], pose), None);
        assert_eq!(world_to_target([22., 26., 28., 1.], [[0.; 4]; 4]), None);
    }

    #[test]
    fn volley_uses_current_facing_and_level_row() {
        for (level, facing, expected) in [
            (1, [1., 0., 0., 0.], FIRST_ROOT),
            (15, [0., 0.3, -1., 0.], FIRST_ROOT + 42),
        ] {
            let d = shot_descriptor(11, level, [2., 7., 3., 1.], facing).unwrap();
            let root = i32::from_le_bytes(d.0[0x14..0x18].try_into().unwrap());
            assert_eq!(root, expected);
            assert!(is_volley_root(root));
            assert!(!is_volley_root(root + 1));
            assert!(!is_volley_root(root + 2));
            assert!(!is_volley_attack(root + 1));
            assert!(is_volley_attack(root + 2));
            for (i, v) in [facing[0], 0., facing[2], 0., 2., 8.5, 3., 1.]
                .into_iter()
                .enumerate()
            {
                let offset = 0x70 + i * 4;
                assert_eq!(
                    f32::from_le_bytes(d.0[offset..offset + 4].try_into().unwrap()),
                    v
                );
            }
        }
    }
    #[test]
    fn sword_conflict_uses_actual_ownership_in_all_draw_paths() {
        use crate::reward_choices::*;
        let index = |id| {
            crate::hex_pool::OWNER_IDS
                .iter()
                .position(|&v| v == id)
                .unwrap()
        };
        let s = index(OWNER);
        let sword = index(crate::sword::OWNER);
        for owned in [mask([]), mask([s]), mask([sword]), mask([s, sword])] {
            let blocked = ownership_exclusions(owned.clone());
            assert_eq!(blocked.contains(&s), !owned.is_empty());
            assert_eq!(blocked.contains(&sword), !owned.is_empty());
            for seen in [mask([]), all(crate::hex_pool::HEX_COUNT)] {
                let round = crate::reward_cycle::Round::new(&blocked, &seen);
                let single = crate::reward_cycle::Single::new(&blocked, &seen, &mask([0, 1]));
                for id in [s, sword] {
                    assert_eq!(round.candidates.contains(&id), owned.is_empty());
                    assert_eq!(single.candidates.contains(&id), owned.is_empty());
                }
                let focused = round.preferred([s, sword, 0]).unwrap();
                if !owned.is_empty() {
                    assert!(!focused.contains(&s) && !focused.contains(&sword));
                }
            }
        }
        // A caster restriction alone must not exclude Sniper as if Sword were owned.
        let blocked = weapon_exclusions(ownership_exclusions(mask([])), Some(51));
        assert!(!blocked.contains(&s));
        assert!(blocked.contains(&sword));
        for hero in [None, Some(1), Some(7)] {
            for affinity in [None, Some(57), Some(51)] {
                let caster = affinity == Some(57);
                let physical = affinity == Some(51);
                for owned in [mask([]), mask([s]), mask([sword])] {
                    let blocked = combat_exclusions(owned.clone(), hero, affinity);
                    assert_eq!(blocked.contains(&s), !owned.is_empty());
                    assert_eq!(blocked.contains(&sword), !owned.is_empty() || !caster);
                    for id in [95001100, 95002600] {
                        assert_eq!(
                            blocked.contains(&index(id)),
                            !physical && !owned.contains(&sword)
                        );
                    }
                }
            }
        }
        for hero in 1..=10 {
            assert!(!role_exclusions(mask([]), Some(hero)).contains(&s));
        }
        assert_eq!(crate::hex_pool::HISTORY_BITS[s], 42);
    }
}
