//! Flowing Curved Sword mastery. Its attack buffs have a private combo meter.
use crate::{has_effect, native_actor, read_i32, read_ptr};
use std::sync::{
    atomic::{AtomicI32, AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    Mutex,
};

const OWNER: i32 = 95005900;
const RECEIPT: i32 = OWNER + 1;
const LOT: i32 = 95559000;
const THRESHOLDS: [i32; 4] = [17, 30, 45, 60];
const OUTPUTS: [i32; 4] = [OWNER + 2, OWNER + 3, OWNER + 4, OWNER + 6];
static ACC: AtomicUsize = AtomicUsize::new(0);
static PLAYER: AtomicUsize = AtomicUsize::new(0);

fn sword(id: i32) -> bool {
    id / 10000 == 706
}
fn heavy(id: i32) -> bool {
    // a157 native dancing R2 sequence, plus a28 running/jumping R2.
    (0..300000000).contains(&id)
        && matches!(id%1000,
        100|101|105..=107|110|111|115..=117|125|160|300|301|305..=307|310|311|315..=317|325|360)
}
pub unsafe fn cost(actor: usize, weapon: i32, behavior: i32, fee: i32, base: usize) -> i32 {
    if fee <= 0 || behavior < 0 || !sword(weapon) || !has_effect(actor, OWNER, base) {
        return fee;
    }
    if heavy(behavior) {
        fee - fee / 2
    } else {
        fee - fee / 4
    }
}
pub unsafe fn context_cost(ctx: usize, actor: usize, fee: i32, base: usize) -> i32 {
    if fee <= 0 || actor != PLAYER.load(Ordering::Acquire) || read_i32(ctx + 12) >= 0 {
        return fee;
    }
    let source = read_i32(ctx + 0x38);
    if !(-6..=15).contains(&source) {
        return fee;
    }
    let get = read_ptr(read_ptr(actor) + 0x250);
    if get == 0 {
        return fee;
    }
    let weapon: unsafe extern "system" fn(usize, i32) -> i32 = std::mem::transmute(get);
    cost(actor, weapon(actor, source), read_i32(ctx + 8), fee, base)
}
// Native hit construction copies the effective weapon row (including hand and
// overrides) to hit+190h. Read it at accepted contact, never edit the hit.
type AttackLookup = unsafe extern "system" fn(*mut usize, i32, i32);

unsafe fn damaging_sword_hit(source: usize, hit: usize, lookup: AttackLookup) -> bool {
    if !sword(read_i32(hit + 0x190)) || read_i32(hit + 0x280) <= 0
        || read_ptr(hit + 0x228) != source || read_i32(hit + 0x44) != 1
    {
        return false;
    }
    let mut attack = [0usize; 3];
    lookup(attack.as_mut_ptr(), 1, read_i32(hit + 0x40));
    // AtkParam.hitSourceType selects the model supplying the hit dummy points.
    // A roll/landing can carry the equipped weapon ID even though it uses the body.
    attack[2] != 0 && std::ptr::read((attack[2] + 0x7c) as *const u8) == 0
}

pub unsafe fn outgoing(module: usize, source: usize, hit: usize) {
    if source == 0 || source != PLAYER.load(Ordering::Acquire) || module == 0 || hit == 0 {
        return;
    }
    let base = crate::BASE.load(Ordering::Acquire);
    let lookup = std::mem::transmute::<usize, AttackLookup>(base + crate::devils_gaze_hit::LOOKUP_RVA);
    if !damaging_sword_hit(source, hit, lookup) {
        return;
    }
    let target = read_ptr(module + 8);
    if target == 0 || target == source || !crate::is_local(source, base)
        || crate::ambulance::party_member(target, base) || !has_effect(source, OWNER, base)
        || !crate::static_charge::first_flowing_contact(module, hit)
    {
        return;
    }
    if let Ok(mut state) = STATE.lock() {
        if let Some(meter) = state.as_mut().filter(|m| m.identity.0 == source) {
            meter.pending_bonus = meter.pending_bonus.saturating_add(2);
        }
    }
}
// The root graph distributes this timestep to the active generators, including
// the clip clock used by TAE. Never change the simulation timestep or store a
// speed in the actor: interrupts and character changes cannot inherit it.
pub const STEP_RVA: usize = 0x15a6fd0;
pub const STEP_PATTERN: &str =
    "48 8B C4 48 81 EC F8 00 00 00 48 89 58 08 48 8B DA 48 89 68 F8 48 8B E9";
pub static STEP_ORIG: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());
static CHARGE_GRAPH: AtomicUsize = AtomicUsize::new(0);

fn charge_step(graph: usize, charging_graph: usize, dt: f32) -> f32 {
    if charging_graph != 0 && graph == charging_graph && dt > 0.0 {
        dt * 1.5
    } else {
        dt
    }
}
pub unsafe extern "system" fn graph_step(graph: usize, context: usize, dt: f32) {
    let original: unsafe extern "system" fn(usize, usize, f32) =
        std::mem::transmute(STEP_ORIG.load(Ordering::Acquire));
    original(
        graph,
        context,
        charge_step(graph, CHARGE_GRAPH.load(Ordering::Acquire), dt),
    );
}
unsafe fn charge_graph(actor: usize, base: usize) -> usize {
    let action = read_ptr(actor + 0x60);
    let flags = if action == 0 {
        0
    } else {
        read_ptr(action + 0xc8)
    };
    if flags != 0 && read_i32(flags + 0x28) & (1 << 29) != 0 {
        crate::ram_stagger::graph(actor, base).unwrap_or(0)
    } else {
        0
    }
}

#[derive(Default)]
struct Meter {
    identity: (usize, usize, usize),
    weapon_given: bool,
    pending_bonus: i32,
    points: i32,
}

// Read-only telemetry ABI for NRTestKit. Readers accept a snapshot only
// when the sequence is even and unchanged around the 16-byte record.
#[repr(C)]
pub struct ComboTelemetry {
    sequence: AtomicU32,
    points: AtomicI32,
    player: AtomicU64,
}

#[no_mangle]
pub static NRMA_FLOWING_COMBO_V1: ComboTelemetry = ComboTelemetry {
    sequence: AtomicU32::new(0),
    points: AtomicI32::new(0),
    player: AtomicU64::new(0),
};

impl ComboTelemetry {
    // All writers hold STATE, so there is only one writer at a time.
    fn publish(&self, meter: Option<&Meter>) {
        let (player, points) = meter.map_or((0, 0), |m| (m.identity.0 as u64, m.points));
        if self.player.load(Ordering::Relaxed) == player
            && self.points.load(Ordering::Relaxed) == points
        {
            return;
        }
        self.sequence.fetch_add(1, Ordering::SeqCst);
        self.points.store(points, Ordering::SeqCst);
        self.player.store(player, Ordering::SeqCst);
        self.sequence.fetch_add(1, Ordering::SeqCst);
    }
}
// The owner, the three active buffs and the native 60-point overflow effect
// send this private debit on their own effect cycles. All real slots stay intact.
pub unsafe fn decay_pulse(actor: usize, effect: i32) -> bool {
    if effect != OWNER + 5 {
        return false;
    }
    if actor != 0 && actor == PLAYER.load(Ordering::Acquire) {
        if let Ok(mut state) = STATE.lock() {
            if let Some(m) = state.as_mut().filter(|m| m.identity.0 == actor) {
                m.points = (m.points - 1).max(0);
                NRMA_FLOWING_COMBO_V1.publish(Some(m));
            }
        }
    }
    true
}

fn feint_source(state: u16, style: i32) -> Option<i32> {
    match (state, style) {
        (25059, 0 | 1) | (25060, 3) => Some(-1),
        (25060, 2) => Some(-2),
        _ => None,
    }
}
pub unsafe fn feint_gate(manager: usize, state: u16) -> u8 {
    if manager == 0 || !crate::ACTIVE.load(Ordering::Acquire) {
        return 0;
    }
    let actor = read_ptr(manager + 0x10);
    let base = crate::BASE.load(Ordering::Acquire);
    if actor == 0
        || read_ptr(actor + 0x190) != manager
        || read_ptr(read_ptr(actor) + 0x118) != base + 0x4d2f00
        || !has_effect(actor, OWNER, base)
    {
        return 0;
    }
    let equip = read_ptr(actor + 0x670);
    if equip == 0 {
        return 0;
    }
    let Some(source) = feint_source(state, read_i32(equip)) else {
        return 0;
    };
    let weapon: unsafe extern "system" fn(usize, i32) -> i32 =
        std::mem::transmute(read_ptr(read_ptr(actor) + 0x250));
    sword(weapon(actor, source)) as u8
}
static STATE: Mutex<Option<Meter>> = Mutex::new(None);
pub fn listens(acc: usize) -> bool {
    acc != 0 && ACC.load(Ordering::Acquire) == acc
}
impl Meter {
    fn distribute(&mut self, original: i32, phantom: i32) -> i32 {
        let bonus = std::mem::take(&mut self.pending_bonus);
        self.points = self.points.saturating_add(original.max(0)).saturating_add(bonus).saturating_add(phantom);
        bonus
    }
}
pub unsafe fn distribute(manager: usize, actor: usize, original: i32, phantom: i32) -> i32 {
    if actor == 0 || actor != PLAYER.load(Ordering::Acquire) {
        return 0;
    }
    if read_ptr(actor + 0x190) != manager {
        return 0;
    }
    let identity = (actor, read_ptr(actor + 8), read_ptr(manager + 0x18));
    if let Ok(mut s) = STATE.lock() {
        if let Some(m) = s.as_mut().filter(|m| m.identity == identity) {
            let bonus = m.distribute(original, phantom);
            NRMA_FLOWING_COMBO_V1.publish(Some(m));
            return bonus;
        }
    }
    0
}
pub unsafe fn update(actor: usize, base: usize) {
    let stats = native_actor::stats(actor);
    if stats == 0
        || !has_effect(actor, OWNER, base)
        || native_actor::incapacitated(actor, stats)
        || has_effect(actor, crate::reward_cycle_runtime::RESET, base)
    {
        CHARGE_GRAPH.store(0, Ordering::Release);
        ACC.store(0, Ordering::Release);
        PLAYER.store(0, Ordering::Release);
        if let Ok(mut s) = STATE.lock() {
            *s = None;
            NRMA_FLOWING_COMBO_V1.publish(None);
        }
        return;
    }
    CHARGE_GRAPH.store(charge_graph(actor, base), Ordering::Release);
    let manager = read_ptr(actor + 0x190);
    if manager == 0 {
        if let Ok(_state) = STATE.lock() {
            NRMA_FLOWING_COMBO_V1.publish(None);
        }
        return;
    }
    let acc = read_ptr(manager + 0x18);
    let identity = (actor, read_ptr(actor + 8), acc);
    let (buffs, needs_weapon) = {
        let Ok(mut s) = STATE.lock() else {
            return;
        };
        let meter = s.get_or_insert_with(Meter::default);
        if meter.identity != identity {
            *meter = Meter {
                identity,
                ..Meter::default()
            };
        }
        ACC.store(acc, Ordering::Release);
        PLAYER.store(actor, Ordering::Release);
        NRMA_FLOWING_COMBO_V1.publish(Some(meter));
        (
            THRESHOLDS.map(|threshold| meter.points >= threshold),
            !meter.weapon_given,
        )
    };
    for (i, apply) in buffs.into_iter().enumerate() {
        if apply {
            crate::wraith::set(actor, OUTPUTS[i], true, base);
        }
    }
    if needs_weapon
        && !has_effect(actor, RECEIPT, base)
        && crate::spatial_ready(actor)
        && !crate::CALLBACK_LOCK.swap(true, Ordering::AcqRel)
    {
        let ok = crate::spawn_personal_lot_kind(actor, base, LOT, None, 1) > 0;
        if ok {
            crate::wraith::set(actor, RECEIPT, true, base);
            if let Ok(mut state) = STATE.lock() {
                if let Some(meter) = state.as_mut().filter(|m| m.identity == identity) {
                    meter.weapon_given = true;
                }
            }
        }
        crate::CALLBACK_LOCK.store(false, Ordering::Release);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_charging_players_graph_advances_faster() {
        let frame = 1.0 / 60.0;
        assert_eq!(charge_step(10, 10, frame), frame * 1.5);
        for (graph, active) in [(11, 10), (10, 0), (0, 0)] {
            assert_eq!(charge_step(graph, active, frame), frame);
        }
        assert_eq!(charge_step(10, 10, 0.0), 0.0);
    }
    #[test]
    fn independent_thresholds_and_feint_hand() {
        assert_eq!(THRESHOLDS.map(|v| 59 >= v), [true, true, true, false]);
        assert_eq!(OUTPUTS[3], OWNER + 6);
        for (state, style, expected) in [
            (25059, 1, Some(-1)),
            (25059, 2, None),
            (25060, 1, None),
            (25060, 2, Some(-2)),
            (25060, 3, Some(-1)),
        ] {
            assert_eq!(feint_source(state, style), expected);
        }
    }
    #[test]
    fn accepted_hit_uses_its_weapon_and_leaves_rescue_payload_intact() {
        static WEAPON: [u8; 0x7d] = [0; 0x7d];
        static BODY: [u8; 0x7d] = { let mut row = WEAPON; row[0x7c] = 1; row };
        unsafe extern "system" fn lookup(out: *mut usize, table: i32, id: i32) {
            assert_eq!(table, 1);
            let row = match id {
                301 | 304 | 307 | 550 | 551 | 560 => BODY.as_ptr() as usize,
                700000 | 700160 | 706105 => WEAPON.as_ptr() as usize,
                _ => 0,
            };
            out.add(2).write(row);
        }
        let mut hit = [0u8; 0x2c8];
        hit[0x70..0x74].copy_from_slice(&6903i32.to_le_bytes());
        for (weapon, damage, id, origin, table, expected) in [
            (7060900i32,100i32,700000i32,1usize,1i32,true), (7060000,100,700160,1,1,true),
            (7060000,100,706105,1,1,true), (7060000,0,700000,1,1,false),
            (7140000,100,700000,1,1,false), (7060000,100,304,1,1,false),
            (7060000,100,550,1,1,false), (7060000,100,301,1,1,false),
            (7060000,100,307,1,1,false), (7060000,100,551,1,1,false),
            (7060000,100,560,1,1,false), (7060000,100,700000,2,1,false),
            (7060000,100,700000,1,0,false), (7060000,100,-1,1,1,false),
        ] {
            hit[0x40..0x44].copy_from_slice(&id.to_le_bytes());
            hit[0x44..0x48].copy_from_slice(&table.to_le_bytes());
            hit[0x190..0x194].copy_from_slice(&weapon.to_le_bytes());
            hit[0x228..0x230].copy_from_slice(&origin.to_le_bytes());
            hit[0x280..0x284].copy_from_slice(&damage.to_le_bytes());
            let before = hit;
            assert_eq!(unsafe { damaging_sword_hit(1, hit.as_ptr() as usize, lookup) }, expected, "attack {id}");
            assert_eq!(before, hit);
        }
    }
    #[test]
    fn actual_weapon_and_each_real_hit() {
        for id in [7060000, 7061125] {
            assert!(sword(id))
        }
        for id in [-1, 7059999, 7070000] {
            assert!(!sword(id))
        }
        for judge in [101, 107, 111, 117, 307, 317, 125, 325, 360] {
            assert!(heavy(100706000 + judge));
        }
        assert!(!heavy(100706000));
        assert!(!heavy(300706105));
        let mut meter = Meter { pending_bonus: 4, ..Meter::default() };
        assert_eq!(meter.distribute(8, 3), 4);
        assert_eq!(meter.points, 15);
        assert_eq!(meter.distribute(0, 0), 0);
        meter.pending_bonus = 2;
        assert_eq!(meter.distribute(0, 0), 2);
        assert_eq!(meter.points, 17);
    }
}
