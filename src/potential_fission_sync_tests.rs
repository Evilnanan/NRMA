use super::*;
use crate::win::*;
use sha2::{Digest, Sha256};
use std::collections::VecDeque;

const HOST_ID: usize = 0x7654;
const SOURCE: usize = 0x340a050b11200006;
const REPRESENTATIVE: u32 = 0xb2;
const OTHER_OPTION: u32 = 0x92;
const FLAGS: [u16; 9] = [
    0x5a82, 0x5a82, 0x5a82, 0x5d02, 0x5d02, 0x5d03, 0x5f82, 0x5f82, 0x5f82,
];

thread_local! {
    static INBOX: RefCell<VecDeque<(usize, Vec<u8>)>> = const { RefCell::new(VecDeque::new()) };
    static OUTBOX: RefCell<Vec<(u32, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
    static VISIBLE: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

unsafe fn write<T>(object: usize, offset: usize, value: T) {
    ptr::write_unaligned((object + offset) as *mut T, value);
}

unsafe fn jump(base: usize, rva: usize, destination: usize) {
    let mut instructions = [0; 12];
    instructions[..2].copy_from_slice(&[0x48, 0xb8]);
    instructions[2..10].copy_from_slice(&destination.to_le_bytes());
    instructions[10..].copy_from_slice(&[0xff, 0xe0]);
    ptr::copy_nonoverlapping(
        instructions.as_ptr(),
        (base + rva) as *mut u8,
        instructions.len(),
    );
}

unsafe extern "system" fn next_packet(
    _: usize,
    _: u32,
    into: *mut u8,
    size: usize,
    sender: *mut usize,
) -> u32 {
    INBOX.with(|inbox| {
        let Some((who, packet)) = inbox.borrow_mut().pop_front() else {
            return 0;
        };
        if packet.len() > size {
            return 0;
        }
        ptr::copy_nonoverlapping(packet.as_ptr(), into, packet.len());
        *sender = who;
        packet.len() as u32
    })
}

unsafe extern "system" fn broadcast(_: usize, channel: u32, data: *const u8, size: u32) {
    OUTBOX.with(|packets| {
        packets.borrow_mut().push((
            channel,
            std::slice::from_raw_parts(data, size as usize).to_vec(),
        ))
    });
}

extern "system" fn show(node: usize) {
    VISIBLE.with(|nodes| nodes.borrow_mut().push(node));
}

extern "system" fn hide(_: usize) {}

unsafe extern "system" fn source_handle(handle: *const usize, out: *mut usize) -> *mut usize {
    *out = *handle;
    out
}

struct Rewards {
    manager: [u64; 0x680 / 8],
    nodes: [[u64; 32]; 10],
    buffer: [u8; 128],
    peer_queue: [u64; 8],
}

impl Rewards {
    fn new() -> Self {
        Self {
            manager: [0; 0x680 / 8],
            nodes: [[0; 32]; 10],
            buffer: [0; 128],
            peer_queue: [0; 8],
        }
    }

    unsafe fn init(&mut self, base: usize) -> usize {
        let items = self.manager.as_mut_ptr() as usize;
        write(base, ITEMS_RVA, items);
        write(items, 0x3e8, self.buffer.as_mut_ptr());
        write(items, 0x3f0, self.buffer.len() as u32);
        write(items, 0x43a, u16::MAX);
        let queue = self.peer_queue.as_mut_ptr() as usize;
        write(items, 0x510, queue);
        write(items, 0x518, queue);
        write(
            items,
            0x520,
            queue + std::mem::size_of_val(&self.peer_queue),
        );
        for (i, flags) in FLAGS.into_iter().enumerate() {
            let node = self.node(i);
            write(node, 0x78, REPRESENTATIVE + 0x20 * i as u32);
            write(node, 0x7c, 0x48000166 - i as u32);
            write(node, 0x80, SOURCE);
            write(node, 0xca, flags);
        }
        let node = self.node(9);
        write(node, 0x78, 0xc02_u32);
        write(node, 0x80, SOURCE);
        write(node, 0xca, 0x5ac2_u16);
        write(node, 0xd8, crate::fission_catalog::ROOTS[0].1);
        self.link(&[0, 1, 2, 3, 4, 5, 6, 7, 8]);
        items
    }

    fn node(&self, i: usize) -> usize {
        self.nodes[i].as_ptr() as usize
    }

    unsafe fn link(&mut self, order: &[usize]) {
        for (i, &index) in order.iter().enumerate() {
            self.nodes[index][0] = order.get(i + 1).map_or(0, |&next| self.node(next)) as u64;
        }
        self.manager[0x58 / 8] = order.first().map_or(0, |&head| self.node(head)) as u64;
    }

    fn enabled(&self) -> usize {
        self.nodes[..9]
            .iter()
            .filter(|node| node[0xca / 8] & (0x40 << 16) != 0)
            .count()
    }
}

fn notice(sender: usize, handle: u32, value: u8) {
    let mut packet = vec![10, 0, 0];
    packet.extend_from_slice(&handle.to_le_bytes());
    packet.push(value);
    INBOX.with(|inbox| inbox.borrow_mut().push_back((sender, packet)));
}

#[test]
#[ignore = "requires NRHEX_NATIVE_EXE; runs mapped PE and game-hook fixtures in isolation"]
fn compiled_fission_activation_repairs_both_network_paths() {
    unsafe {
        let raw = std::fs::read(std::env::var_os("NRHEX_NATIVE_EXE").unwrap()).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&raw)), crate::EXE_HASH);
        let image = crate::pe::Headers::parse(&raw, 0).unwrap();
        let memory = VirtualAlloc(
            ptr::null_mut(),
            image.image_size as usize,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_EXECUTE_READWRITE,
        );
        assert!(!memory.is_null());
        let base = memory as usize;
        for section in &image.sections {
            ptr::copy_nonoverlapping(
                raw.as_ptr().add(section.raw_offset as usize),
                (base + section.virtual_address as usize) as *mut u8,
                section.raw_size as usize,
            );
        }
        for (rva, dest) in [
            (0xdf1de0, next_packet as *const () as usize),
            (0xdf24d0, broadcast as *const () as usize),
            (0x572220, show as *const () as usize),
            (0x5749e0, hide as *const () as usize),
            (0xaec6d0, source_handle as *const () as usize),
        ] {
            jump(base, rva, dest);
        }

        let old_base = BASE.swap(base, Ordering::AcqRel);
        let old_active = ACTIVE.swap(true, Ordering::AcqRel);
        let mut session = [0u64; 0x100 / 8];
        let session = session.as_mut_ptr() as usize;
        write(base, SESSION_RVA, session);
        write(session, 0xa0, HOST_ID);
        write(session, 0x10, 7i32);
        let mut player = [0i32; 8];
        let mut game = [0usize, player.as_mut_ptr() as usize];
        write(base, crate::GAME_DATA_RVA, game.as_mut_ptr());
        let source_actor = [0usize, SOURCE];
        let activate: unsafe extern "system" fn(usize, *const usize, u8) -> u8 =
            std::mem::transmute(base + 0x577820);
        let pump: unsafe extern "system" fn(usize, f32) = std::mem::transmute(base + PUMP_RVA);
        let all = [0, 1, 2, 3, 4, 5, 6, 7, 8];
        let without_head = &all[1..];
        let private_first = [9, 0, 1, 2, 3, 4, 5, 6, 7, 8];

        for patched in [false, true] {
            if patched {
                for (rva, hook) in prepare(base).unwrap() {
                    crate::hook::write_code((base + rva) as *mut u8, &hook.patch).unwrap();
                }
            }

            // The first native match is a local-only fission child of this source.
            let mut host = Rewards::new();
            let items = host.init(base);
            host.link(&private_first);
            write(session, 0xc, HOST);
            player[2] = 1;
            let actor = source_actor.as_ptr();
            assert_eq!(activate(items, actor, 1), 1);
            assert_eq!(host.enabled(), 9);
            OUTBOX.with(|packets| {
                let mut packets = packets.borrow_mut();
                assert_eq!(packets.len(), usize::from(patched));
                if patched {
                    assert_eq!(packets[0].0, 0x2c);
                    assert_eq!(packets[0].1, [10, 0, 0, 0xb2, 0, 0, 0, 1]);
                }
                packets.clear();
            });
            assert_eq!(activate(items, actor, 1), 1);
            assert!(OUTBOX.with(|packets| packets.borrow().is_empty()));

            let mut guest = Rewards::new();
            let items = guest.init(base);
            guest.link(without_head);
            write(session, 0xc, GUEST);
            player[2] = 2;
            VISIBLE.with(|nodes| nodes.borrow_mut().clear());
            notice(HOST_ID, REPRESENTATIVE, 1);
            pump(items, 0.016);
            assert_eq!(guest.enabled(), 0);
            guest.link(&all);
            pump(items, 0.016);
            assert_eq!(guest.enabled(), if patched { 9 } else { 0 });
            VISIBLE.with(|nodes| {
                let expected = if patched {
                    vec![guest.node(3), guest.node(4), guest.node(5)]
                } else {
                    Vec::new()
                };
                assert_eq!(*nodes.borrow(), expected);
            });
        }

        // The native receive-by-source, resend and handover paths use the same
        // representative rule, with their own saved registers and stack slots.
        let resend: unsafe extern "system" fn(usize) = std::mem::transmute(base + 0x582260);
        let handover: unsafe extern "system" fn(usize) = std::mem::transmute(base + 0x573d80);
        for path in 0..3 {
            let mut host = Rewards::new();
            let items = host.init(base);
            host.link(&private_first);
            write(session, 0xc, HOST);
            player[2] = 1;
            match path {
                0 => {
                    let mut packet = vec![9, 0, 0];
                    packet.extend_from_slice(&SOURCE.to_le_bytes());
                    packet.push(1);
                    INBOX.with(|inbox| inbox.borrow_mut().push_back((HOST_ID + 1, packet)));
                    pump(items, 0.016);
                }
                1 => {
                    let mut request = [0, SOURCE as u64, 1u64 << 32];
                    let start = request.as_mut_ptr() as usize;
                    write(items, 0x488, start);
                    write(items, 0x490, start + 0x18);
                    resend(items);
                }
                _ => {
                    let mut queued = [0, 0, SOURCE as u64, 1u64 << 32];
                    let start = queued.as_mut_ptr() as usize;
                    write(items, 0x590, start);
                    write(items, 0x598, start + 0x20);
                    handover(items);
                }
            }
            assert_eq!(host.enabled(), 9, "host source path {path}");
            OUTBOX.with(|packets| {
                let mut packets = packets.borrow_mut();
                assert_eq!(&*packets, &[(0x2c, vec![10, 0, 0, 0xb2, 0, 0, 0, 1])]);
                packets.clear();
            });
        }

        // A newer host decision can use another representative from the same source.
        let mut guest = Rewards::new();
        let items = guest.init(base);
        write(session, 0xc, GUEST);
        player[2] = 2;
        write(guest.node(1), 0x78, OTHER_OPTION);
        guest.link(without_head);
        notice(HOST_ID, REPRESENTATIVE, 1);
        notice(HOST_ID, OTHER_OPTION, 0);
        pump(items, 0.016);
        guest.link(&all);
        pump(items, 0.016);
        assert_eq!(guest.enabled(), 0);

        // Native sender checks must run before the mod ever remembers a notice.
        guest.link(without_head);
        notice(HOST_ID + 1, REPRESENTATIVE, 1);
        pump(items, 0.016);
        guest.link(&all);
        pump(items, 0.016);
        assert_eq!(guest.enabled(), 0);
        assert_eq!(
            read_ptr(items + 0x518),
            read_ptr(items + 0x510) + 0x20,
            "stock peer queue remains intact"
        );

        guest.link(without_head);
        notice(HOST_ID, REPRESENTATIVE, 1);
        pump(items, 0.016);
        guest.link(&all);
        replay(items, base, Instant::now() + LIFETIME);
        assert_eq!(guest.enabled(), 0);

        guest.link(without_head);
        notice(HOST_ID, REPRESENTATIVE, 1);
        pump(items, 0.016);
        write(session, 0xa0, HOST_ID + 1);
        guest.link(&all);
        pump(items, 0.016);
        assert_eq!(guest.enabled(), 0);

        reset();
        ACTIVE.store(old_active, Ordering::Release);
        BASE.store(old_base, Ordering::Release);
        VirtualFree(memory, 0, MEM_RELEASE);
    }
}
