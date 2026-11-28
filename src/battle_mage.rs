use crate::{archmage, archmage::Catalyst, archmage_runtime, native_actor, read_i32, read_ptr};
use std::sync::{atomic::{AtomicPtr, Ordering}, Mutex};

#[path = "battle_mage_pose_blend.rs"]
pub(crate) mod pose_blend;
#[path = "battle_mage_poses.rs"]
mod poses;

pub const OWNER: i32 = 95007100;
const CHAIN_SPEED: i32 = OWNER + 1;
pub const CROSS_CHAIN: i32 = 990065;
pub const CHAIN_ACCEPTED: i32 = 990066;
pub const CHAIN_FULL_BODY: i32 = 990067;
pub const MOTION_OFFSET_RVA: usize = 0x402590;
pub const MOTION_OFFSET_PATTERN: &str =
    "48 89 5C 24 08 55 56 57 48 83 EC 20 41 8B F0 48 8B D9 33 ED 8B FD 83 C2 F5 83 FA 07";
pub static MOTION_OFFSET_ORIGINAL: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cast {
    player: usize,
    expedition: usize,
    hand: u32,
    slot: u32,
    catalyst: Catalyst,
    spell: i32,
    next: Option<i32>,
    released: bool,
    previous: Option<(i32, u8, u8)>,
    phase: u8,
    motion_phase: u8,
    combo: bool,
}

static CAST: Mutex<Option<Cast>> = Mutex::new(None);

pub fn clear() {
    if let Ok(mut cast) = CAST.lock() {
        *cast = None;
    }
    pose_blend::clear();
}

fn animation_family(spell: i32) -> Option<(u8, u8)> {
    let spells = crate::archmage_data::COMBO_SPELLS;
    let i = spells.binary_search_by_key(&spell, |row| row.0).ok()?;
    let (_, motion, last_stage) = spells[i];
    Some((motion, last_stage))
}

pub fn chain_spell(spell: i32) -> bool {
    animation_family(spell).is_some()
}

fn extended_family(family: u8) -> bool {
    crate::archmage_data::COMBO_EXTENDED_FAMILIES.contains(&family)
}

fn full_body_chain(spell: i32) -> bool {
    animation_family(spell).is_some_and(|(family, _)| matches!(family, 14 | 65 | 74 | 120 | 144 | 147))
}

pub fn continuation(previous: i32, phase: u8, motion: u8, next: i32) -> Option<(u8, u8)> {
    let (from, _) = animation_family(previous)?;
    let (to, last_stage) = animation_family(next)?;

    let target = poses::sweeps(to);
    if from == to && matches!(target, poses::Sweeps::Neutral) {
        return Some((0, 0));
    }
    // Keep the state transition separate from the selected swing clip.
    let stage = match phase {
        1 if last_stage == 3 && poses::prefers_third(from, to) => 3,
        1 | 3 => 2,
        2 if last_stage == 3 => 3,
        _ => return None,
    };
    if let Some(direction) = poses::sweeps(from).direction(motion) {
        if target.direction(stage) == Some(direction) {
            let other = 5 - stage;
            let next_phase = if phase == 1 { other } else { stage };
            return (other <= last_stage && target.direction(other) != Some(direction))
                .then_some((next_phase, other));
        }
    }
    Some((stage, stage))
}

pub unsafe fn enabled(player: usize, base: usize) -> bool {
    if player == 0
        || !crate::is_local(player, base)
        || !crate::has_effect(player, OWNER, base)
    {
        return false;
    }
    let stats = native_actor::stats(player);
    stats != 0 && !native_actor::incapacitated(player, stats)
}

impl Cast {
    fn identity(self, player: usize, expedition: usize, spell: i32) -> bool {
        self.player == player && self.expedition == expedition && self.spell == spell
    }

    fn accept_follow_up(&mut self, native_stage: u8) -> u8 {
        if !matches!(native_stage, 2 | 3) {
            return 0;
        }
        let route = self.previous.and_then(|(spell, phase, motion)|
            continuation(spell, phase, motion, self.spell));
        self.combo = route.is_some();
        (self.phase, self.motion_phase) = route.filter(|&(phase, _)| phase != 0)
            .unwrap_or((native_stage, native_stage));
        self.phase
    }

    fn follows(self, previous: Self) -> bool {
        self.player == previous.player
            && self.expedition == previous.expedition
            && self.hand == previous.hand
            && self.slot == 1
            && previous.slot == 1
            && self.catalyst == previous.catalyst
            && previous.released
            && previous.next == Some(self.spell)
            && continuation(previous.spell, previous.phase, previous.motion_phase, self.spell).is_some()
    }

    fn previous_for(self, current: Self) -> Option<(i32, u8, u8)> {
        if self.follows(current) {
            return Some((current.spell, current.phase, current.motion_phase));
        }
        // The native selector can run again before the chain window accepts
        // the same pending request. Preserve its source until it is accepted.
        (self.identity(current.player, current.expedition, current.spell)
            && self.hand == current.hand
            && self.slot == current.slot
            && self.catalyst == current.catalyst
            && !current.released
            && !current.combo)
            .then_some(current.previous)
            .flatten()
    }
}

pub unsafe fn selected(player: usize, hand: u32, slot: u32, skill: u8, spell: i32, base: usize) {
    if skill != 0
        || hand > 1
        || slot > 1
        || archmage::kind(spell).is_none()
        || !enabled(player, base)
    {
        clear();
        return;
    }
    let Some(catalyst) = archmage_runtime::hand(player, hand as i32, base) else {
        clear();
        return;
    };
    if let Ok(mut state) = CAST.lock() {
        let mut cast = Cast {
            player,
            expedition: read_ptr(base + 0x3c131c8),
            hand,
            slot,
            catalyst,
            spell,
            next: None,
            released: false,
            previous: None,
            phase: 1,
            motion_phase: 1,
            combo: false,
        };
        cast.previous = state.and_then(|current| cast.previous_for(current));
        *state = Some(cast);
    }
}

pub unsafe fn combo_query(player: usize, code: i32, native_stage: i32, base: usize) -> i32 {
    // For families with added clips, no ordinary cast may enter those clips.
    // CROSS_CHAIN receives the same animation family the HKS selector uses.
    let fallback = i32::from(code == CROSS_CHAIN
        && u8::try_from(native_stage).ok().is_some_and(extended_family)) * 2;
    if !enabled(player, base) {
        return fallback;
    }
    let expedition = read_ptr(base + 0x3c131c8);
    let spell = crate::active_magic(player);
    let Ok(mut state) = CAST.lock() else { return fallback };
    let Some(cast) = state
        .as_mut()
        .filter(|c| c.identity(player, expedition, spell))
    else {
        return fallback;
    };
    if code == CHAIN_FULL_BODY {
        return i32::from(cast.combo && full_body_chain(spell));
    }
    if code == CROSS_CHAIN {
        return if cast.previous.is_some() { 1 } else { fallback };
    }
    let phase = cast.accept_follow_up(native_stage as u8);
    let boost = cast.combo;
    let blend = boost
        && cast.previous.is_some_and(|(previous, _, _)| {
            let from = animation_family(previous).map(|(family, _)| family);
            let to = animation_family(spell).map(|(family, _)| family);
            // Added stages replay the fast release, even within one family.
            from != to || to.is_some_and(extended_family)
        });
    drop(state);
    if blend {
        if let Some(graph) = crate::ram_stagger::graph(player, base) {
            pose_blend::arm(graph, phase);
        }
    }
    if boost && !crate::has_effect(player, CHAIN_SPEED, base) {
        // Keep the bridge through the transition into the first follow-up callback.
        crate::archmage_runtime::notify(None);
        crate::wraith::set(player, CHAIN_SPEED, true, base);
    }
    i32::from(phase)
}

fn remap_motion_offset(cast: Cast, action: i32, native: i32) -> i32 {
    if !cast.combo || cast.phase == cast.motion_phase ||
        !matches!(cast.phase, 2 | 3) || !matches!(cast.motion_phase, 2 | 3) {
        return native;
    }
    let Some((family, _)) = animation_family(cast.spell) else { return native; };
    let stage_action = 45000 + i32::from(cast.phase) * 10;
    if native == (400 + i32::from(family)) * 1_000_000 &&
        (action == stage_action || action == stage_action + 100) {
        native + (i32::from(cast.motion_phase) - i32::from(cast.phase)) * 10
    } else {
        native
    }
}

pub unsafe extern "system" fn motion_offset(module: usize, kind: i32, action: i32) -> i32 {
    let original: unsafe extern "system" fn(usize, i32, i32) -> i32 =
        std::mem::transmute(MOTION_OFFSET_ORIGINAL.load(Ordering::Acquire));
    let native = original(module, kind, action);
    if kind != 17 || module == 0 || !matches!(action, 45020 | 45030 | 45120 | 45130) ||
        !crate::ACTIVE.load(Ordering::Acquire) {
        return native;
    }
    let Some(cast) = CAST.lock().ok().and_then(|state| *state) else { return native; };
    if !cast.combo || cast.phase == cast.motion_phase || read_ptr(module + 8) != cast.player {
        return native;
    }
    let base = crate::BASE.load(Ordering::Acquire);
    if cast.expedition != read_ptr(base + 0x3c131c8) || crate::active_magic(cast.player) != cast.spell {
        return native;
    }
    remap_motion_offset(cast, action, native)
}

pub unsafe fn stop_speed(player: usize, base: usize) {
    if crate::has_effect(player, CHAIN_SPEED, base) {
        crate::wraith::set(player, CHAIN_SPEED, false, base);
    }
}

pub fn chain_cost(cost: i32, surcharge: i32) -> (i32, i32) {
    let reduced = cost / 2 + cost % 2;
    let total = (i64::from(cost) + i64::from(surcharge) + 1) / 2;
    (reduced, total as i32 - reduced)
}

// Each accepted action in the selected chain gets the discount. Only its
// first projectile, sweep or effect advances the catalyst's random spell.
pub unsafe fn released(player: usize, context: usize, base: usize) -> bool {
    let spell = read_i32(context + 0xc);
    if spell < 0 || !enabled(player, base) || crate::active_magic(player) != spell {
        return false;
    }
    let expedition = read_ptr(base + 0x3c131c8);
    let cast = {
        let Ok(mut state) = CAST.lock() else {
            return false;
        };
        let Some(cast) = state
            .as_mut()
            .filter(|c| c.identity(player, expedition, spell))
        else {
            return false;
        };
        if cast.slot != 1 {
            return false;
        }
        if cast.released {
            return cast.combo;
        }
        cast.released = true;
        *cast
    };

    if let Some(next) = archmage_runtime::reroll_continuation(
        cast.catalyst, cast.spell, cast.phase, cast.motion_phase)
    {
        if let Ok(mut state) = CAST.lock() {
            if state.as_ref() == Some(&cast) {
                state.as_mut().unwrap().next = Some(next);
            }
        }
    }
    cast.combo
}

// The active action's behavior may look up its catalyst again for later events.
// Only behavior resolution gets the latched spell. The selector, UI and
// equipment broadcast can see the new roll immediately.
pub unsafe fn behavior_spell(player: usize, catalyst: Catalyst, base: usize) -> Option<i32> {
    let expedition = read_ptr(base + 0x3c131c8);
    let cast = (*CAST.lock().ok()?)?;
    (cast.slot == 1
        && cast.catalyst == catalyst
        && cast.next.is_some()
        && cast.identity(player, expedition, crate::active_magic(player)))
    .then_some(cast.spell)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selected_phase(previous: i32, phase: u8, next: i32) -> Option<u8> {
        continuation(previous, phase, phase, next).map(|(phase, _)| phase)
    }

    #[test]
    fn chain_halves_the_full_stamina_payment_with_one_rounding() {
        assert_eq!(chain_cost(0, 0), (0, 0));
        assert_eq!(chain_cost(35, 0), (18, 0));
        assert_eq!(chain_cost(46, 0), (23, 0));
        assert_eq!(chain_cost(35, 25), (18, 12));
        assert_eq!(chain_cost(17, 11), (9, 5));
        assert_eq!(chain_cost(i32::MAX, 0), (1073741824, 0));
    }

    #[test]
    fn added_spells_chain_both_ways_without_admitting_other_family_members() {
        for spell in [4040, 4050, 4210, 4220, 4410, 4720, 4721, 4800, 4820, 5000, 5001] {
            assert!(chain_spell(spell));
            assert_eq!(selected_phase(4000, 2, spell), Some(3));
            assert_eq!(selected_phase(spell, 3, 4000), Some(2));
        }
        assert_eq!(selected_phase(4050, 2, 4220), Some(0));
        for spell in [4370, 4420, 6500] {
            assert!(!chain_spell(spell));
            assert_eq!(selected_phase(4050, 1, spell), None);
        }
        for spell in [6001, 6010, 6020, 6100, 6110, 6210, 6230, 6240, 6270, 6400, 6410,
            6820, 6900, 7200, 7210, 7320, 7500] {
            assert!(chain_spell(spell));
            assert_eq!(selected_phase(6000, 2, spell), Some(3));
            assert_eq!(selected_phase(spell, 3, 6000), Some(2));
            assert_eq!(archmage::kind(spell), archmage::kind(6000));
        }
        for spell in [6520, 6530, 6906, 7220, 7510] {
            assert!(!chain_spell(spell));
        }
        #[cfg(feature = "mmv-compat")]
        for spell in [6908, 2006710, 2006900, 2007210, 2007700, 2007710, 2007730, 2007800] {
            assert_eq!(selected_phase(6000, 2, spell), Some(3));
            assert_eq!(selected_phase(spell, 3, 6000), Some(2));
        }
        #[cfg(feature = "mmv-compat")]
        {
            assert!(full_body_chain(2007730));
            assert_eq!(continuation(6900, 2, 2, 2006710), None);
            assert_eq!(continuation(2006710, 2, 2, 6800), Some((3, 3)));
        }
    }

    #[test]
    fn only_the_accepted_successor_can_choose_its_continuation() {
        let catalyst = Catalyst {
            handle: 1,
            base: 1,
            original: [4000, 4001],
        };
        let first = Cast {
            player: 1,
            expedition: 2,
            hand: 1,
            slot: 1,
            catalyst,
            spell: 4000,
            next: Some(4021),
            released: true,
            previous: None,
            phase: 1,
            motion_phase: 1,
            combo: false,
        };
        let next = Cast {
            spell: 4021,
            next: None,
            released: false,
            ..first
        };
        assert!(next.follows(first));
        assert_eq!(next.previous_for(first), Some((4000, 1, 1)));
        let pending = Cast {
            previous: Some((4000, 1, 1)),
            ..next
        };
        assert_eq!(next.previous_for(pending), pending.previous);
        assert_eq!(
            next.previous_for(Cast {
                combo: true,
                ..pending
            }),
            None
        );
        assert_eq!(Cast { hand: 0, ..next }.previous_for(pending), None);
        assert!(!next.follows(Cast {
            released: false,
            ..first
        }));
        assert!(!next.follows(Cast {
            next: None,
            ..first
        }));
        assert!(!Cast { hand: 0, ..next }.follows(first));
        assert!(!Cast { slot: 0, ..next }.follows(first));
        assert!(!Cast {
            expedition: 3,
            ..next
        }
        .follows(first));
        let mut ordinary = Cast { slot: 0, ..next };
        assert_eq!(ordinary.accept_follow_up(2), 2);
        assert!(!ordinary.combo);
        let mut cross_family = pending;
        assert_eq!(cross_family.accept_follow_up(3), 2);
        assert!(cross_family.combo);
        let mut same_family = Cast {
            spell: 4001,
            ..pending
        };
        assert_eq!(same_family.accept_follow_up(3), 2);
        assert!(same_family.combo);
        assert!(chain_spell(6000) && chain_spell(6920) && chain_spell(6930));
        assert!([4100, 4120, 5111, 7010, 7011, 7060, 7530]
            .iter()
            .all(|&id| !chain_spell(id)));
        assert!(!chain_spell(4900) && !chain_spell(4910) && !chain_spell(2004910));
    }

    #[test]
    fn carian_and_crystal_chains_select_opposite_sweeps() {
        assert_eq!(continuation(4440, 1, 1, 4430), Some((3, 3)));
        assert_eq!(continuation(4440, 2, 2, 4430), Some((3, 2)));
        assert_eq!(continuation(4430, 3, 2, 4440), Some((2, 2)));
        for crystal in [4090, 4500] {
            assert!(chain_spell(crystal));
            assert_eq!(continuation(4440, 1, 1, crystal), Some((2, 2)));
            assert_eq!(continuation(4440, 2, 2, crystal), None);
            assert_eq!(continuation(crystal, 2, 2, 4070), Some((3, 3)));
            assert_eq!(continuation(crystal, 2, 2, 4000), Some((3, 2)));
            assert_eq!(continuation(crystal, 3, 3, 4000), Some((2, 2)));
            assert_eq!(continuation(crystal, 3, 3, 4090), None);
        }
        let cast = Cast {
            player: 1,
            expedition: 2,
            hand: 1,
            slot: 1,
            catalyst: Catalyst { handle: 3, base: 4, original: [4000, -1] },
            spell: 4430, next: None, previous: Some((4440, 2, 2)), released: false,
            phase: 1, motion_phase: 1, combo: false,
        };
        let mut chain = cast;
        assert_eq!(chain.accept_follow_up(3), 3);
        assert_eq!(remap_motion_offset(chain, 45030, 409_000_000) + 45030, 409_045_020);
        assert_eq!(remap_motion_offset(chain, 45130, 409_000_000) + 45130, 409_045_120);
        assert_eq!(remap_motion_offset(chain, 45020, 409_000_000), 409_000_000);
        assert_eq!(remap_motion_offset(cast, 45030, 409_000_000), 409_000_000);
    }

    #[test]
    fn neutral_spells_can_chain_before_and_after_fixed_sweeps() {
        for fixed in [4070, 4090, 4500] {
            assert_eq!(continuation(4050, 1, 1, fixed), Some((2, 2)));
            assert_eq!(continuation(fixed, 2, 2, 4050), Some((3, 3)));
        }
        assert_eq!(continuation(4440, 1, 1, 4070), None);
        assert_eq!(continuation(4070, 3, 3, 4000), Some((2, 3)));
        assert_eq!(continuation(4070, 2, 2, 4390), None);
    }

    #[test]
    fn slicer_alternates_with_other_sweeps_in_both_directions() {
        for spell in [4000, 4400, 4021, 4620, 4430, 4431, 1604000] {
            if spell == 1604000 && !chain_spell(spell) { continue; }
            for (previous, next) in [(4440, spell), (spell, 4440)] {
                assert_eq!(continuation(previous, 1, 1, next), Some((3, 3)));
                assert_eq!(continuation(previous, 2, 2, next), Some((3, 2)));
                assert_eq!(continuation(previous, 3, 3, next), Some((2, 3)));
            }
        }
    }

    #[test]
    fn swift_shard_has_a_neutral_second_clip_in_either_chain_phase() {
        use poses::Direction::Backhand;
        for shard in [4010, 4610] {
            let profile = poses::sweeps(animation_family(shard).unwrap().0);
            assert_eq!([profile.direction(1), profile.direction(2), profile.direction(3)],
                       [Some(Backhand), None, Some(Backhand)]);
            assert_eq!(continuation(4000, 2, 3, shard), Some((3, 2)));
            assert_eq!(continuation(shard, 3, 2, 4090), Some((2, 2)));
            assert_eq!(continuation(shard, 3, 3, 4090), None);
        }
    }

    #[test]
    fn registered_incantations_alternate_and_neutral_casts_can_bridge_them() {
        for spell in [6010, 6020, 6100, 6210, 6240, 6820, 6900] {
            assert_eq!(continuation(spell, 1, 1, 6900), None);
            assert_eq!(continuation(spell, 2, 2, 6810), Some((3, 2)));
            assert_eq!(continuation(6800, 1, 1, spell), Some((2, 2)));
            assert_eq!(continuation(6800, 2, 2, spell), None);
        }
        for spell in [6000, 6110, 6230, 6270, 6400, 6410, 7320, 7500] {
            assert!(chain_spell(spell));
            assert!(continuation(spell, 1, 1, 6900).is_some());
            assert_eq!(continuation(6900, 2, 2, spell), Some((3, 3)));
        }
        for spell in [6700, 6701, 6810] {
            assert_eq!(continuation(6300, 2, 2, spell), Some((3, 2)));
        }
        for spell in [4800, 4820] {
            assert_eq!(continuation(4070, 2, 2, spell), Some((3, 3)));
            assert_eq!(continuation(4090, 2, 2, spell), None);
        }
        #[cfg(feature = "mmv-compat")]
        {
            assert_eq!(continuation(6908, 2, 2, 6900), None);
            assert_eq!(continuation(6800, 2, 3, 6908), Some((3, 3)));
            assert_eq!(continuation(6240, 1, 1, 2007800), None);
            assert_eq!(continuation(2007800, 2, 2, 2006710), None);
            for arc in [2007700, 2007710] {
                assert_eq!(continuation(6800, 1, 1, arc), None);
                assert_eq!(continuation(2007800, 2, 2, arc), Some((3, 3)));
                assert_eq!(continuation(arc, 2, 2, 6240), Some((3, 3)));
            }
            assert_eq!(continuation(2007700, 1, 1, 2007710), None);
            assert_eq!(continuation(6900, 2, 2, 2007730), Some((3, 3)));
        }
    }

    #[test]
    fn every_chain_stage_can_roll_a_distinct_compatible_spell() {
        use crate::archmage_data::COMBO_SPELLS;
        for &(spell, motion, last_stage) in COMBO_SPELLS {
            let kind = archmage::kind(spell).unwrap();
            let actual_motion = animation_family(spell).unwrap().0;
            for phase in 1..=last_stage {
                let choices: Vec<_> = archmage::pool(spell)
                    .into_iter()
                    .filter(|&id| id != spell)
                    .filter_map(|id| continuation(spell, phase, phase, id).map(|route| (id, route)))
                    .collect();
                assert!(!choices.is_empty(), "{spell} ({motion}): stage {phase}");
                for (id, (route, motion)) in choices {
                    assert_eq!(archmage::kind(id), Some(kind));
                    let (target, max) = animation_family(id).unwrap();
                    if let Some((from, to)) = poses::sweeps(actual_motion).direction(phase)
                        .zip(poses::sweeps(target).direction(motion)) {
                        assert_ne!(from, to);
                    }
                    if route != 0 {
                        assert!(route != phase && (2..=max).contains(&route));
                        assert!((2..=max).contains(&motion));
                    } else {
                        assert_eq!(actual_motion, target);
                    }
                }
            }
        }
    }
}
