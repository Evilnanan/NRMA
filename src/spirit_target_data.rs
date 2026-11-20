// Standalone uses its original NPC catalogue. MMV supplies target-generated data.
pub const EXTRA_NPCS: &[(i32,i32)] = &[];
pub const MODELS: &[(i32,i32)] = &[];
pub const BOSSES: &[i32] = &[];
// Standalone catalogue; MMV generates the same constant from its own map and event layers.
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/reaver_bosses.rs"));
