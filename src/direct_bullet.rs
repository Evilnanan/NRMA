//! Native direct bullet request with neutral coefficients and an independent world pose.
#[repr(C, align(16))]
pub(crate) struct Descriptor(pub(crate) [u8; 0x110]);
pub(crate) fn descriptor(owner: u64, id: i32, p: [f32; 4]) -> Descriptor {
    let mut d = Descriptor([0; 0x110]);
    for off in [8, 0xc, 0x14, 0x18, 0x1c, 0x24] {
        d.0[off..off + 4].copy_from_slice(&(-1i32).to_le_bytes());
    }
    d.0[..8].copy_from_slice(&owner.to_le_bytes());
    d.0[0x14..0x18].copy_from_slice(&id.to_le_bytes());
    for off in [0x28, 0x2c] {
        d.0[off..off + 4].copy_from_slice(&16i32.to_le_bytes());
    }
    for off in [0x30, 0x34, 0x38, 0x3c] {
        d.0[off..off + 4].copy_from_slice(&1f32.to_le_bytes());
    }
    // independent creation + force direct reference + native network queue.
    d.0[0x44..0x48].copy_from_slice(&13u32.to_le_bytes());
    let matrix = [
        1f32, 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., p[0], p[1], p[2], 1.,
    ];
    for (i, v) in matrix.iter().enumerate() {
        d.0[0x50 + i * 4..0x54 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    d.0[0xa8] = 0xff;
    d.0[0x105] = 0xff;
    // Empty native target descriptor (348260): no refcounted target object.
    d.0[0xb8..0xc0].fill(0xff);
    d.0[0xe0..0xe4].fill(0xff);
    d.0[0xf0..0xf4].fill(0xff);
    d
}
