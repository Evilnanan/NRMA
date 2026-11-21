//! Private PE execution: real reward codec, dispatch, choice linking and pickup cleanup.
//! Allocation, native lottery, character/effect state and rendering are fixtures.
use super::*;
use crate::win::*;
use sha2::{Digest,Sha256};
use std::cell::RefCell;
use std::sync::atomic::AtomicUsize;
static IMAGE:AtomicUsize=AtomicUsize::new(0);
thread_local!{
 static NODES:RefCell<Vec<Box<[u64;32]>>>=const{RefCell::new(Vec::new())};
 static EFFECTS:RefCell<Vec<i32>>=const{RefCell::new(Vec::new())};
 static FAILURE:Cell<i32>=const{Cell::new(-1)};
 static ROLLS:RefCell<VecDeque<usize>>=const{RefCell::new(VecDeque::new())};
 static ROLL:Cell<Option<usize>>=const{Cell::new(None)};
 static UID:Cell<u32>=const{Cell::new(1)};
}
pub(super) fn next_roll()->Option<usize>{ROLLS.with(|r|r.borrow_mut().pop_front()).or_else(||ROLL.with(Cell::get))}
unsafe fn native_flight(base:usize,dt:f32){
 let tick:unsafe extern "system" fn(usize,f32)=std::mem::transmute(base+0x583240);
 let nodes=NODES.with(|n|n.borrow().iter().map(|p|p.as_ptr()as usize).collect::<Vec<_>>());
 for node in nodes{tick(node,dt);}
}
fn head_count()->usize{NODES.with(|n|n.borrow().iter().filter(|p|p[3]==0).count())}
unsafe fn jump(base:usize,rva:usize,dest:usize){let mut bytes=vec![0x48,0xb8];bytes.extend_from_slice(&dest.to_le_bytes());bytes.extend_from_slice(&[0xff,0xe0]);ptr::copy_nonoverlapping(bytes.as_ptr(),(base+rva)as*mut u8,bytes.len());}
unsafe extern "system" fn has(_:usize,id:i32)->u8{EFFECTS.with(|c|c.borrow().contains(&id))as u8}
unsafe extern "system" fn apply(_:usize,id:i32,_:u8)->u8{EFFECTS.with(|c|{if !c.borrow().contains(&id){c.borrow_mut().push(id);}});1}
unsafe extern "system" fn remove(_:usize,id:i32)->u8{EFFECTS.with(|c|c.borrow_mut().retain(|&n|n!=id));1}
unsafe extern "system" fn allocate(manager:usize,descriptor:usize)->usize{
 let index=ptr::read((descriptor+0xa9)as*const u8)as i32;
 if generated(read_i32(descriptor+0xb8))&&FAILURE.with(Cell::get)==index{return 0;}
 let base=IMAGE.load(Ordering::Acquire);let mut node=Box::new([0u64;32]);let p=node.as_mut_ptr()as usize;
 let copy:unsafe extern "system" fn(usize,usize)->usize=std::mem::transmute(base+0x570410);copy(p+0x20,descriptor);
 if ptr::read_unaligned((p+0xca)as*const u16)&0x800!=0{ptr::write((p+0xf9)as*mut u8,1);ptr::write_unaligned((p+0xfc)as*mut f32,0.75);}
 if read_i32(p+0x78)&15==2{
  let counter=(manager+0x10)as*mut u32;*counter+=1;
  ptr::write_unaligned((p+0x78)as*mut u32,(*counter<<5)|2);
 }
 let head=NODES.with(|c|c.borrow().first().map_or(0,|n|n.as_ptr()as usize));
 // Native group algorithm walks the manager's list via node+0 and builds
 // the choice chain via node+10/+18, used by native claiming/removal.
 let group:unsafe extern "system" fn(usize,usize,usize)=std::mem::transmute(base+0x57a840);group(0,head,p);
 NODES.with(|c|{let mut nodes=c.borrow_mut();if let Some(last)=nodes.last_mut(){last[0]=p as u64;}nodes.push(node);});ptr::write_unaligned((manager+0x58)as*mut usize,if head==0{p}else{head});p
}
// Engine allocator plumbing for the real 57D400 pickup cleanup. The native
// code resolves the chosen handle and traverses both native option links.
static CLAIM_ALLOCATOR:AtomicUsize=AtomicUsize::new(0);
unsafe extern "system" fn claim_allocator()->usize{CLAIM_ALLOCATOR.load(Ordering::Acquire)}
unsafe extern "system" fn allocator_flags(_:usize,out:*mut u32,_:usize)->*mut u32{*out=0x20;out}
unsafe extern "system" fn grow_handles(vec:*mut usize,_end:*mut u32,handle:*const u32){
 let buffer=Box::into_raw(Box::new([*handle,0,0]))as usize;
 *vec.add(1)=buffer;*vec.add(2)=buffer+4;*vec.add(3)=buffer+12;
}
unsafe extern "system" fn free_handles(_:usize,buffer:*mut [u32;3]){drop(Box::from_raw(buffer));}
unsafe extern "system" fn retire(manager:usize,handle:u32){
 NODES.with(|n|{
  let mut nodes=n.borrow_mut();let old=nodes.len();
  nodes.retain(|node|read_i32(node.as_ptr()as usize+0x78)as u32!=handle);
  assert_eq!(nodes.len(),old-1);
  let mut head=0;
  for node in nodes.iter_mut().rev(){node[0]=head;head=node.as_ptr()as u64;}
  ptr::write_unaligned((manager+0x58)as*mut u64,head);
 });
}
fn descriptor(lot:i32,slot:u8,index:u8,source:u64)->[u64;24]{unsafe{
 let mut data=[0u64;24];let p=data.as_mut_ptr()as usize;
 ptr::write(p as*mut u8,1);ptr::write_unaligned((p+8)as*mut u32,0x40000000|8000000+index as u32);
 ptr::write_unaligned((p+0x58)as*mut u32,2);
 let uid=UID.with(|c|{let n=c.get();c.set(n+1);n});ptr::write_unaligned((p+0x5c)as*mut u32,uid);
 ptr::write_unaligned((p+0x60)as*mut u64,source);ptr::write_unaligned((p+0x74)as*mut u32,0x3c000000);
 ptr::write_unaligned((p+0x78)as*mut [f32;3],[12.,34.,56.]);ptr::write((p+0xa9)as*mut u8,index);
 ptr::write_unaligned((p+0xaa)as*mut u16,0x4841|((slot as u16)<<7));ptr::write_unaligned((p+0xb8)as*mut i32,lot);data
}}
unsafe extern "system" fn lottery(manager:usize,_:usize,lot:i32,kind:u8,_:u8,_:u8,_:u8,_:*const f32)->u8{
 let base=IMAGE.load(Ordering::Acquire);
 let dispatch:unsafe extern "system" fn(usize,usize)->usize=std::mem::transmute(base+REWARD_CREATE_RVA);
 if crate::wraith::ground_good(lot).is_some(){assert_eq!(kind,1);}else{assert_eq!(kind,0);}
 let count=if crate::wraith::ground_good(lot).is_some(){1}else if crate::snack::bonus_lot(lot){3}else{if lot>=FLOOR_LOT{ROOTS[((lot-FLOOR_LOT)/10)as usize].3}else{ROOTS.iter().find(|r|r.1==lot||r.2==lot).unwrap().3}};
 for slot in 1..=3{for index in (0..count).rev(){let mut d=descriptor(lot,slot,index,999);if crate::wraith::ground_good(lot).is_some(){ptr::write_unaligned((d.as_mut_ptr()as usize+8)as*mut u32,(0x40000000|crate::wraith::ground_good(lot).unwrap())as u32);}
 if (98_000_000..FLOOR_LOT).contains(&lot){let flags=(d.as_mut_ptr()as usize+0xaa)as*mut u16;*flags=(*flags&!15)|2;}
 let before=d;dispatch(manager,d.as_mut_ptr()as usize);assert_eq!(d,before);}}1
}
// Stub only the ItemData body and world-handle conversion. The actual reward
// writer/reader still select the packet layout, advance the cursor and fill the descriptor.
unsafe extern "system" fn encode_good(item:usize,stream:usize)->usize{
 ptr::write_unaligned(read_ptr(stream)as*mut u32,ptr::read_unaligned(item as*const u32));4
}
unsafe extern "system" fn decode_good(item:usize,stream:usize)->u8{
 let cursor=read_ptr(stream);ptr::write_unaligned(item as*mut u32,ptr::read_unaligned(cursor as*const u32));
 ptr::write_unaligned(stream as*mut usize,cursor+4);*((stream+8)as*mut i32)-=4;1
}
unsafe extern "system" fn handle_out(out:usize,handle:u64)->usize{ptr::write_unaligned(out as*mut u64,handle);out}
unsafe extern "system" fn handle_in(handle:usize,out:usize)->usize{ptr::write_unaligned(out as*mut u64,ptr::read_unaligned(handle as*const u64));out}
unsafe extern "system" fn no_item_flag(_:usize)->u8{0}
unsafe fn online_descriptor(lot:i32,slot:u8,index:u8,source:u64,sequence:u32,tag:bool)->[u64;24]{
 let mut input=descriptor(lot,slot,index,source);
 let p=input.as_mut_ptr()as usize;
 ptr::write_unaligned((p+0x58)as*mut u32,0x12);
 ptr::write_unaligned((p+0x5c)as*mut u32,0x48000000|sequence);
 ptr::write_unaligned((p+0xac)as*mut i32,-1);
 let mut buffer=[0u64;128];let mut stream=[buffer.as_mut_ptr()as usize,1024];
 let before=input;
 let writer:unsafe extern "system" fn(usize,usize)->usize=std::mem::transmute(ENCODE_ORIGINAL.load(Ordering::Acquire));
 let size=if tag{encode(p,stream.as_mut_ptr()as usize)}else{writer(p,stream.as_mut_ptr()as usize)};
 assert_eq!(input,before);
 let mut decoded=[0u64;24];let out=decoded.as_mut_ptr()as usize;
 ptr::write_unaligned((out+0xb8)as*mut i32,-1);
 stream=[buffer.as_mut_ptr()as usize,size];
 assert_eq!(decode(out,stream.as_mut_ptr()as usize),1);
 assert_eq!(stream[1],0);
 assert_eq!(read_i32(out+0xb8),if tag{lot}else{-1});
 assert_eq!(read_ptr(out+0xb0),0);assert_eq!(read_ptr(out+0x60),source as usize);
 decoded
}
unsafe fn emit(base:usize,manager:usize,lot:i32,slot:u8,source:u64,count:u8){
 let dispatch:unsafe extern "system" fn(usize,usize)->usize=std::mem::transmute(base+REWARD_CREATE_RVA);
 for index in 0..count{let mut d=descriptor(lot,slot,index,source);let before=d;dispatch(manager,d.as_mut_ptr()as usize);assert_eq!(d,before);}
}
unsafe fn verify_envelope_and_hex(base:usize,items:usize,player:usize){
 let claim:unsafe extern "system" fn(usize,u32,u8)->u8=std::mem::transmute(base+REMOVE_GROUP_RVA);
 let hex=hex_pool::TEMPLATE_LOT;
 for envelope in red_envelope::LOTS{
  for (first,second) in [(envelope,hex),(hex,envelope)]{
   for claim_first in [envelope,hex]{
    reset();NODES.with(|n|n.borrow_mut().clear());ptr::write_unaligned((items+0x58)as*mut usize,0);
    ROLL.with(|r|r.set(Some(3)));
    for lot in [first,second]{
     for i in 0..3{
      let mut d=descriptor(lot,1,i,777);let p=d.as_mut_ptr()as usize;
      if lot==hex{ptr::write_unaligned((p+8)as*mut u32,0x40000000|hex_pool::OWNER_IDS[i as usize]as u32);}
      capture(items,p,base+0x575a53);
     }
     assert_eq!(unclaimed_hex_ball(base),lot==hex||first==hex);
    }
    assert_eq!(head_count(),2);
    for lot in [claim_first,if claim_first==hex{envelope}else{hex}]{
     let handle=NODES.with(|nodes|nodes.borrow().iter().find(|n|n[3]==0&&read_i32(n.as_ptr()as usize+0xd8)==lot)
      .map(|n|read_i32(n.as_ptr()as usize+0x78)as u32).unwrap());
     assert_eq!(claim(items,handle,0),1);
     assert_eq!(unclaimed_hex_ball(base),lot==envelope&&claim_first==envelope);
    }
    assert_eq!(head_count(),0);
   }
  }
  reset();ROLL.with(|r|r.set(Some(0)));emit(base,items,envelope,1,777,3);
  assert_eq!(head_count(),1);native_flight(base,0.8);update(player,base,0.3);
  assert_eq!(head_count(),2);assert!(!unclaimed_hex_ball(base));
 }
}
#[test]
#[ignore="requires NRHEX_NATIVE_EXE; maps private PE and must run alone"]
fn compiled_fission_routes_online_draws_and_native_independent_groups(){unsafe{
 let raw=std::fs::read(std::env::var_os("NRHEX_NATIVE_EXE").expect("NRHEX_NATIVE_EXE")).unwrap();assert_eq!(format!("{:x}",Sha256::digest(&raw)),EXE_HASH);
 let h=pe::Headers::parse(&raw,0).unwrap();let allocation=VirtualAlloc(ptr::null_mut(),h.image_size as usize,MEM_COMMIT|MEM_RESERVE,PAGE_EXECUTE_READWRITE);assert!(!allocation.is_null());let base=allocation as usize;
 for s in &h.sections{ptr::copy_nonoverlapping(raw[s.raw_offset as usize..].as_ptr(),(base+s.virtual_address as usize)as*mut u8,s.raw_size as usize);}
 IMAGE.store(base,Ordering::Release);
 for(rva,dest)in [(HAS_EFFECT_RVA,has as*const()as usize),(APPLY_SELF_RVA,apply as*const()as usize),(REMOVE_EFFECT_RVA,remove as*const()as usize),(SPAWN_RVA,lottery as*const()as usize)]{jump(base,rva,dest);}
 for(rva,dest)in [(0x6c1540,encode_good as*const()as usize),(0x6c0de0,decode_good as*const()as usize),
   (0x6c1530,no_item_flag as*const()as usize),(0x6c12c0,no_item_flag as*const()as usize),
   (0xaec590,handle_out as*const()as usize),(0xaec6d0,handle_in as*const()as usize)]{jump(base,rva,dest);}
 let mut vtable=[0usize;14];vtable[3]=allocator_flags as*const()as usize;vtable[13]=free_handles as*const()as usize;
 let heap=vtable.as_ptr()as usize;CLAIM_ALLOCATOR.store(&heap as*const usize as usize,Ordering::Release);
 for(rva,dest)in [(0x214f3e0,claim_allocator as*const()as usize),(0x56d840,grow_handles as*const()as usize),(0x580f70,retire as*const()as usize)]{jump(base,rva,dest);}
 ptr::write((base+0x571dd0)as*mut u8,0xc3);
 ptr::write((base+0x580b20)as*mut u8,0xc3);
 jump(base,0x572c10,no_item_flag as*const()as usize);
 ENCODE_ORIGINAL.store((base+ENCODE_RVA)as*mut u8,Ordering::Release);
 DECODE_ORIGINAL.store((base+DECODE_RVA)as*mut u8,Ordering::Release);
 let entry=hook::prepare((base+CREATE_RVA)as*mut u8,create as*const u8,64).unwrap();CREATE_ORIGINAL.store(allocate as*mut u8,Ordering::Release);hook::write_code((base+CREATE_RVA)as*mut u8,&entry.patch).unwrap();
 let link=hook::prepare((base+GROUP_RVA)as*mut u8,group as*const u8,64).unwrap();assert_eq!(link.stolen,7);GROUP_ORIGINAL.store(link.trampoline,Ordering::Release);GROUP_SKIP.store((base+0x57a8e1)as*mut u8,Ordering::Release);hook::write_code((base+GROUP_RVA)as*mut u8,&link.patch).unwrap();
 let dispatch=hook::prepare((base+REWARD_CREATE_RVA)as*mut u8,reward_create_detour as*const u8,64).unwrap();let old_dispatch=REWARD_CREATE_ORIG.swap(dispatch.trampoline,Ordering::AcqRel);hook::write_code((base+REWARD_CREATE_RVA)as*mut u8,&dispatch.patch).unwrap();
 let old_base=BASE.swap(base,Ordering::AcqRel);let old_active=ACTIVE.swap(true,Ordering::AcqRel);
 let mut world=vec![0u64;0x17500/8];let mut player=vec![0u64;0x718/8+1];let player=player.as_mut_ptr()as usize;let mut modules=[0usize;1];let mut stats=vec![0i32;0x148/4];stats[0x140/4]=100;modules[0]=stats.as_mut_ptr()as usize;ptr::write_unaligned((player+0x1b8)as*mut usize,modules.as_mut_ptr()as usize);
 let mut effects=[0u64;8];ptr::write_unaligned((player+0x190)as*mut usize,effects.as_mut_ptr()as usize);
 world[0x174e8/8]=player as u64;ptr::write_unaligned((base+WORLD_RVA)as*mut usize,world.as_mut_ptr()as usize);
 let mut data=[0i32;64];let mut game=[0usize;2];game[1]=data.as_mut_ptr()as usize;ptr::write_unaligned((base+GAME_DATA_RVA)as*mut usize,game.as_mut_ptr()as usize);
 let mut item_manager=vec![0u64;0x600/8];let items=item_manager.as_mut_ptr()as usize;ptr::write_unaligned((base+ITEMS_RVA)as*mut usize,items);
 ROLL.with(|r|r.set(Some(0)));
 let old_state={let mut s=STATE.lock().unwrap();std::mem::replace(&mut*s,State::new())};
 let (source,_,_,count)=ROOTS[0];
 for(raw_slot,local_slot)in [(0,1),(1,1),(2,2),(3,3)]{
  data[2]=raw_slot;*STATE.lock().unwrap()=State::new();ptr::write_unaligned((items+0x58)as*mut usize,0);NODES.with(|n|n.borrow_mut().clear());EFFECTS.with(|c|c.borrow_mut().clear());
  // Existing groups and even late option packets remain original on acquiring.
  emit(base,items,source,local_slot as u8,100,count);*STATE.lock().unwrap()=State::new();apply(player,OWNER,0);emit(base,items,source,local_slot as u8,100,count);
  assert!(STATE.lock().unwrap().queue.is_empty());ptr::write_unaligned((items+0x58)as*mut usize,0);NODES.with(|n|n.borrow_mut().clear());
  // The real parent exists until its native flight ends; teammates keep theirs.
  for slot in 1..=3{emit(base,items,source,slot,200,count);}
  assert_eq!(head_count(),3);assert_eq!(STATE.lock().unwrap().queue.len(),1);
  let handle=*STATE.lock().unwrap().locked.iter().next().unwrap();assert_eq!(pickup(items,handle,0),0);
  native_flight(base,0.5);update(player,base,0.3);assert_eq!(head_count(),3);
  native_flight(base,0.3);
  FAILURE.with(|c|c.set(1));update(player,base,0.3);assert_eq!(head_count(),4);
  FAILURE.with(|c|c.set(-1));update(player,base,0.3);assert!(STATE.lock().unwrap().queue.is_empty());
  assert!(STATE.lock().unwrap().locked.is_empty());assert_eq!(head_count(),4);
  for n in [300,400,500]{emit(base,items,source,local_slot as u8,n,count);native_flight(base,0.8);update(player,base,0.3);}
  assert_eq!(head_count(),10);
  ROLL.with(|r|r.set(Some(3)));emit(base,items,source,local_slot as u8,600,count);
  ROLL.with(|r|r.set(Some(0)));emit(base,items,source,local_slot as u8,600,count);
  assert!(STATE.lock().unwrap().queue.is_empty());
  apply(player,reward_cycle_runtime::RESET,0);update(player,base,0.3);assert!(STATE.lock().unwrap().seen.is_empty());remove(0,reward_cycle_runtime::RESET);

 }
 for slot in 1..=3{
  data[2]=slot;
  for (receive,fresh) in [(0x57ee94,false),(0x57f046,true),(0x57f42c,true)]{
   *STATE.lock().unwrap()=State::new();NODES.with(|n|n.borrow_mut().clear());ptr::write_unaligned((items+0x58)as*mut usize,0);
   let legacy=online_descriptor(source,slot as u8,0,777,1,false);
   assert_eq!(read_i32(legacy.as_ptr()as usize+0xb8),-1);
   // Four drops from the same actor/lot, with duplicate and out-of-order packets.
   for draw in 0..4{
    for index in (0..count).rev().chain(0..count){
     let mut d=online_descriptor(source,slot as u8,index,777,1+draw*3+u32::from(index),true);
     capture(items,d.as_mut_ptr()as usize,base+receive);
    }
    native_flight(base,0.8);update(player,base,0.3);
   }
   assert_eq!(STATE.lock().unwrap().seen.len(),4);
   let heads=NODES.with(|nodes|nodes.borrow().iter().filter(|n|generated(read_i32(n.as_ptr()as usize+0xd8))&&n[3]==0).count());
   assert_eq!(heads,if fresh{8}else{0});
   if fresh{
    let claim:unsafe extern "system" fn(usize,u32,u8)->u8=std::mem::transmute(base+0x57d400);
    let handles=NODES.with(|nodes|nodes.borrow().iter().filter(|n|n[3]==0)
     .map(|n|read_i32(n.as_ptr()as usize+0x78)as u32).collect::<Vec<_>>());
    for (i,handle) in handles.into_iter().enumerate(){
     assert_eq!(claim(items,handle,0),1);
     assert_eq!(claim(items,handle,0),0);
     assert_eq!(NODES.with(|n|n.borrow().len()),(7-i)*count as usize);
    }
   }

  }
 }
 // Real native flight and grouping for every generation. Rolls are independent
 // even when the two siblings share a lot and both land on the same update.
 for (tier,rolls,expected) in [(1,vec![0],2),(2,vec![1,1,2],3),(2,vec![0,2,1],3),(2,vec![0,0,0],4),(3,vec![0;7],8),(2,vec![2],1),(2,vec![3],1)]{
  data[2]=1;*STATE.lock().unwrap()=State::new();NODES.with(|n|n.borrow_mut().clear());ptr::write_unaligned((items+0x58)as*mut usize,0);
  ROLLS.with(|r|*r.borrow_mut()=rolls.into());
  for index in 0..count{
   let mut d=descriptor(source,1,index,2345);let p=d.as_mut_ptr()as usize;
   let flags=(p+0xaa)as*mut u16;if index==count-1{*flags=(*flags&!15)|tier;}
   capture(items,p,base+0x575a53);
  }
  assert_eq!(head_count(),1);update(player,base,0.1);assert_eq!(head_count(),1);
  for _ in 0..4{native_flight(base,0.8);update(player,base,0.1);}
  assert_eq!(head_count(),expected);assert!(ROLLS.with(|r|r.borrow().is_empty()));
  assert!(STATE.lock().unwrap().locked.is_empty());assert!(STATE.lock().unwrap().queue.is_empty());
  let count_before=head_count();for _ in 0..4{update(player,base,0.3);}assert_eq!(head_count(),count_before);
 }
 data[2]=1;verify_envelope_and_hex(base,items,player);
 for slot in 1..=3{
  data[2]=slot;ptr::write_unaligned((items+0x58)as*mut usize,0);NODES.with(|nodes|nodes.borrow_mut().clear());
  emit(base,items,crate::snack::LOT,slot as u8,999,3);emit(base,items,crate::snack::LOT,slot as u8,999,3);
  assert_eq!(NODES.with(|nodes|nodes.borrow().iter().filter(|node|read_ptr(node.as_ptr()as usize+0x18)==0).count()),1);
  ptr::write_unaligned((items+0x58)as*mut usize,0);NODES.with(|nodes|nodes.borrow_mut().clear());
  let mut balls=Vec::new();
  for lot in [crate::snack::LOT,crate::snack::PREMIUM_LOT,crate::snack::LOT]{
   assert_eq!(crate::spawn_personal_lot(player,base,lot,None),3);
   let nodes=NODES.with(|nodes|nodes.borrow().iter().rev().take(3).map(|node|node.as_ptr()as usize).collect::<Vec<_>>());balls.push(nodes);
  }
  let nodes=NODES.with(|nodes|nodes.borrow().iter().map(|node|node.as_ptr()as usize).collect::<Vec<_>>());
  assert_eq!(nodes.len(),9);
  assert_eq!(nodes.iter().filter(|&&node|read_ptr(node+0x18)==0).count(),3,"Snack rewards must not merge into an existing ball");
  for ball in &balls{
   let mut node=*ball.iter().find(|&&node|read_ptr(node+0x18)==0).unwrap();let mut seen=Vec::new();
   while node!=0{assert!(ball.contains(&node));assert!(!seen.contains(&node));seen.push(node);node=read_ptr(node+0x10);}
   assert_eq!(seen.len(),3);
  }
  emit(base,items,crate::snack::LOT+100,slot as u8,999,3);
  assert_eq!(NODES.with(|nodes|nodes.borrow().iter().filter(|node|read_ptr(node.as_ptr()as usize+0x18)==0).count()),4);
  assert_eq!(crate::spawn_personal_lot(player,base,crate::snack::LOT,None),3);
  assert_eq!(NODES.with(|nodes|nodes.borrow().iter().filter(|node|read_ptr(node.as_ptr()as usize+0x18)==0).count()),5);
  assert_eq!(PERSONAL_REWARD.with(|scope|scope.get().manager),0);
 }
 // Single-bell map results become ordinary placed items for each local slot.
 // Native grouping ignores their source-less, non-choice descriptors.
 for slot in 1..=3{
  data[2]=slot;ptr::write_unaligned((items+0x58)as*mut usize,0);NODES.with(|nodes|nodes.borrow_mut().clear());
  assert_eq!(crate::spawn_personal_lot(player,base,crate::snack::LOT,None),3);
  for kind in 0..3{for _ in 0..2{assert_eq!(crate::spawn_personal_lot_kind(player,base,crate::wraith::GROUND_LOT+kind*10,None,1),1);}}
  let bells=NODES.with(|nodes|nodes.borrow().iter().map(|node|node.as_ptr()as usize).filter(|&p|crate::wraith::ORIGINALS.iter().any(|&good|read_i32(p+0x28)==(0x40000000|good))&&read_i32(p+0x78)&15==1).collect::<Vec<_>>());
  assert_eq!(bells.len(),6);for good in crate::wraith::ORIGINALS{assert_eq!(bells.iter().filter(|&&p|read_i32(p+0x28)==(0x40000000|good)).count(),2);}for &p in &bells{assert_eq!(read_ptr(p+0x10),0);assert_eq!(read_ptr(p+0x18),0);assert_eq!(ptr::read_unaligned((p+0xca)as*const u16),0x40);assert_eq!(read_i32(p+0x78),1);assert_eq!(read_i32(p+0xd8),-1);assert_eq!(read_i32(p+0xcc),-1);assert_eq!(read_i32(p+0xdc),0);assert_eq!(read_i32(p+0x90),0);assert_eq!(read_ptr(p+0x80),usize::MAX);assert_eq!(read_ptr(p+0x88),usize::MAX);assert_eq!(ptr::read((p+0xc9)as*const u8),0xff);assert_eq!((read_i32(p+0x7c)as u32)&0x60000000,0x20000000);}
  assert_eq!(crate::spawn_personal_lot_kind(player,base,crate::metal::GROUND_LOT,None,1),1);
  assert_eq!(NODES.with(|nodes|nodes.borrow().iter().filter(|node|read_i32(node.as_ptr()as usize+0x28)==(0x40000000|crate::metal::GOOD)&&read_i32(node.as_ptr()as usize+0x78)==1&&read_ptr(node.as_ptr()as usize+0x10)==0&&read_ptr(node.as_ptr()as usize+0x18)==0&&read_i32(node.as_ptr()as usize+0xd8)==-1).count()),1);
  assert_eq!(read_i32(items+0x1c),slot*7);assert_ne!(read_i32(bells[0]+0x7c),read_i32(bells[1]+0x7c));
  assert_eq!(NODES.with(|nodes|nodes.borrow().len()),10);assert_eq!(PERSONAL_REWARD.with(|scope|scope.get().manager),0);
 }
 ROLL.with(|r|r.set(None));
 *STATE.lock().unwrap()=old_state;BASE.store(old_base,Ordering::Release);ACTIVE.store(old_active,Ordering::Release);REWARD_CREATE_ORIG.store(old_dispatch,Ordering::Release);NODES.with(|n|n.borrow_mut().clear());VirtualFree(allocation,0,MEM_RELEASE);
}}
