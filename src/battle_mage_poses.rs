//! Opening poses with a closer landing in the third native continuation.
//! Measured over the TAE input window on the c0000 casting arm, torso and legs.

pub fn prefers_third(from: u8, to: u8) -> bool {
    THIRD
        .binary_search_by_key(&from, |row| row.0)
        .ok()
        .is_some_and(|index| THIRD[index].1.contains(&to))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forehand,
    Backhand,
}

pub enum Sweeps {
    ForehandFirst,
    BackhandFirst,
    ForehandOnly,
    BackhandOnly,
    BackhandThenNeutral,
    Neutral,
}

impl Sweeps {
    // Direction is relative to the casting hand. Stage 3 repeats stage 1's sweep.
    pub fn direction(&self, motion: u8) -> Option<Direction> {
        use Direction::{Backhand, Forehand};
        match self {
            Self::ForehandFirst => Some(if motion == 2 { Backhand } else { Forehand }),
            Self::BackhandFirst => Some(if motion == 2 { Forehand } else { Backhand }),
            Self::ForehandOnly => Some(Forehand),
            Self::BackhandOnly => Some(Backhand),
            Self::BackhandThenNeutral => (motion != 2).then_some(Backhand),
            Self::Neutral => None,
        }
    }
}

pub fn sweeps(family: u8) -> Sweeps {
    match family {
        // MMV's a576 imports the a402 sweep clips.
        1 | 2 | 9 | 38 | 94 | 176 => Sweeps::BackhandFirst,
        27 | 37 | 80 => Sweeps::ForehandFirst,
        16 | 17 | 36 | 40 | 65 | 99 | 149 | 153 => Sweeps::ForehandOnly,
        113 | 117 | 148 | 154 => Sweeps::BackhandOnly,
        79 => Sweeps::BackhandThenNeutral,
        // Scouring Black Flame, Rejection, Wrath of Gold, Noble Presence and Catch Flame.
        22 | 28 | 29 | 105 | 106 => Sweeps::Neutral,
        _ => Sweeps::Neutral,
    }
}

const THIRD: &[(u8, &[u8])] = &[
    (1, &[80]),
    (16, &[1, 2]),
    (27, &[94]),
    (37, &[38, 94]),
    (38, &[27, 37]),
    (79, &[1, 2]),
    (80, &[1, 2]),
    (94, &[27, 37]),
    (106, &[94]),
    (111, &[94]),
    (118, &[1, 2, 16]),
];
