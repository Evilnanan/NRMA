//! Roll once for each fresh local native Dormant Power.
//! Reuse native lottery and private routing, and keep separate native groups.
use crate::{*, fission_catalog::ROOTS};
use std::{collections::{BTreeMap,BTreeSet,VecDeque},sync::Mutex};
pub const OWNER:i32=95003200;
const SESSION_RVA:usize=0x3c21ec8;
const HOST:i32=3;
const GUEST:i32=6;
pub const CREATE_RVA:usize=0x57d0a0;
pub const CREATE_PATTERN:&str="40 56 57 41 56 48 81 EC 80 00 00 00 48 8B 05 ? ? ? ? 48 33 C4 48 89 44 24 50";
pub const ENCODE_RVA:usize=0x574320;
pub const ENCODE_PATTERN:&str="48 89 5C 24 20 48 89 54 24 10 55 56 57 41 54 41 55 41 56 41 57 48 8B EC 48 83 EC 70";
pub const DECODE_RVA:usize=0x571580;
pub const DECODE_PATTERN:&str="40 55 56 41 56 48 8D 6C 24 B9 48 81 EC D0 00 00 00 48 8B 05 ? ? ? ? 48 33 C4";
pub static ENCODE_ORIGINAL:AtomicPtr<u8>=AtomicPtr::new(ptr::null_mut());
pub static DECODE_ORIGINAL:AtomicPtr<u8>=AtomicPtr::new(ptr::null_mut());
pub const LOOKUP_RVA:usize=0x582680;
pub const LOOKUP_PATTERN:&str="48 89 5C 24 08 89 54 24 10 55 56 57 48 8B EC 48 83 EC 20 48 8B F1 48 8D 4D 28 E8 ? ? ? ? 83 E8 01 0F 84";
pub const REMOVE_GROUP_RVA:usize=0x57d400;
pub const REMOVE_GROUP_PATTERN:&str="48 89 5C 24 08 48 89 6C 24 10 48 89 74 24 18 57 41 56 41 57 48 83 EC 50 41 0F B6 F0 8B DA 48 8B E9 45 32 FF";
pub const PICKUP_RVA:usize=0x575490;
pub const PICKUP_PATTERN:&str="48 89 5C 24 10 48 89 74 24 18 48 89 7C 24 20 55 41 54 41 55 41 56 41 57 48 8D 6C 24 C9 48 81 EC B0 00 00 00 48 8B 05 ? ? ? ? 48 33 C4 48 89 45 27 41 8B F0 44 89 45 E3";
pub static PICKUP_ORIGINAL:AtomicPtr<u8>=AtomicPtr::new(ptr::null_mut());
pub const PLACE_RVA:usize=0x575810;
pub const PLACE_PATTERN:&str="48 89 5C 24 08 48 89 74 24 20 89 54 24 10 57 48 83 EC 20 8B DA 48 8B F9 E8 53 CE 00 00 48 8B F0 48 85 C0";
pub static PLACE_ORIGINAL:AtomicPtr<u8>=AtomicPtr::new(ptr::null_mut());
#[path="potential_fission_placement.rs"]
mod placement;
pub(crate) use placement::place;
#[path="potential_fission_sync.rs"]
pub(crate) mod sync;

pub const GROUP_RVA:usize=0x57a8c5;
pub const GROUP_PATTERN:&str="0F B7 8A CA 00 00 00 66 0F BA E1 0E 73 0E 66 C1 E9 07 66 83 E1 03";
pub static CREATE_ORIGINAL:AtomicPtr<u8>=AtomicPtr::new(ptr::null_mut());
pub static GROUP_ORIGINAL:AtomicPtr<u8>=AtomicPtr::new(ptr::null_mut());
pub static GROUP_SKIP:AtomicPtr<u8>=AtomicPtr::new(ptr::null_mut());
// Native potential option UIDs are consecutive within one recipient's draw.
// Keep the producing slot as well as the sequence (which wraps at 0xffffff).
#[derive(Clone,Copy,Debug,Eq,Ord,PartialEq,PartialOrd)]
struct Key{source:u64,lot:i32,slot:u8,batch:Option<(u32,u32)>}
impl Key{
 unsafe fn from_descriptor(p:usize,count:u8)->Self{
  let uid=ptr::read_unaligned((p+0x5c)as*const u32);
  let sequence=uid&0x07ff_ffff;
  let index=ptr::read((p+0xa9)as*const u8);
  let batch=(uid>>30==1 && (1..=0x00ff_ffff).contains(&sequence) && index<count)
   .then(||(uid>>27,(sequence+0x00ff_ffff-u32::from(index))%0x00ff_ffff));
  Self{source:ptr::read_unaligned((p+0x60)as*const u64),lot:read_i32(p+0xb8),
       slot:((ptr::read_unaligned((p+0xaa)as*const u16)>>7)&3)as u8,batch}
 }
}
#[derive(Clone,Copy,Debug,Default)]
struct Ball{handles:[u32;3],received:u8,tier:u8}
#[derive(Clone,Copy,Debug)]
enum Phase{Original,Landing(Ball),Children}
#[derive(Clone,Copy,Debug)]
struct Job{key:Key,lots:[i32;2],count:u8,map:u32,position:[f32;3],masks:[u8;2],wanted:[u8;2],handles:[[u32;3];2],phase:Phase,ceiling:u8,generation:u8,scheduled:u8,grace:f32}
impl Job{
 fn original(key:Key,lots:[i32;2],count:u8,map:u32,position:[f32;3])->Self{
  Self{key,lots,count,map,position,masks:[0;2],wanted:[0;2],handles:[[0;3];2],phase:Phase::Original,ceiling:3,generation:0,scheduled:0,grace:0.25}
 }
 fn child_position(&self,side:usize)->[f32;3]{
  let mut p=self.position;
  let (axis,spacing)=match self.generation{0=>(0,0.9),1=>(2,0.7),_=>(0,0.45)};
  p[axis]+=if side==0{-spacing}else{spacing};p
 }
}
struct State{seen:BTreeMap<Key,bool>,originals:BTreeMap<Key,Ball>,locked:BTreeSet<u32>,queue:VecDeque<Job>}
impl State{
 const fn new()->Self{Self{seen:BTreeMap::new(),originals:BTreeMap::new(),locked:BTreeSet::new(),queue:VecDeque::new()}}
 fn observe(&mut self,job:Job,eligible:bool,roll:impl FnOnce()->bool)->bool{
  if let Some(&split)=self.seen.get(&job.key){return split;}
  let split=eligible && roll();
  self.seen.insert(job.key,split);
  if split{self.originals.insert(job.key,Ball::default());self.queue.push_back(job);}
  split
 }
}
fn roll_split()->bool{
 let splits=|n:usize|n<2;
 #[cfg(test)]
 if let Some(n)=native_tests::next_roll(){return splits(n);}
 crate::random_index(4).is_some_and(splits)
}
static STATE:Mutex<State>=Mutex::new(State::new());
#[derive(Clone,Copy,Default)]
struct Generated{origin:[f32;3],manager:usize,lot:i32,slot:u8,source:u64,map:u32,position:[f32;3],mask:u8,attempted:u8,count:u8,limit:u8,handles:[u32;3]}
thread_local!{static GENERATED:Cell<Generated>=const{Cell::new(Generated{origin:[0.;3],manager:0,lot:0,slot:0,source:0,map:0,position:[0.;3],mask:0,attempted:0,count:0,limit:0,handles:[0;3]})};}
fn root(lot:i32)->Option<(i32,i32,u8)>{ROOTS.binary_search_by_key(&lot,|v|v.0).ok().map(|i|{let r=ROOTS[i];(r.1,r.2,r.3)})}
const FLOOR_LOT:i32=98_100_000;
fn generated(lot:i32)->bool{
 let in_range=|first:i32,dual:bool|lot.checked_sub(first).is_some_and(|n|n>=0&&((n/10)as usize)<ROOTS.len()&&(n%10==0||dual&&n%10==5));
 in_range(98_000_000,true)||in_range(FLOOR_LOT,false)
}
// The native codec transmits the optional 12-byte record at +AC, but omits
// the source lot at +B8. Both [-1,0,0] and [0,0,0] occur for choices.
// Preserve the first word; borrow the other two only when both are empty.
const SOURCE_TAG:u32=u32::from_le_bytes(*b"MApf");
unsafe fn network_choice(p:usize)->bool{
 let owner=ptr::read_unaligned((p+0xaa)as*const u16);
 read_i32(p+0x58)&0x1f==0x12 && owner&0x4000!=0 && (owner>>7)&3!=0 &&
 matches!(read_i32(p+0xac),-1|0)
}
pub unsafe extern "system" fn encode(descriptor:usize,stream:usize)->usize{
 let original:unsafe extern "system" fn(usize,usize)->usize=std::mem::transmute(ENCODE_ORIGINAL.load(Ordering::Acquire));
 if !ACTIVE.load(Ordering::Acquire)||!network_choice(descriptor){return original(descriptor,stream);}
 let lot=read_i32(descriptor+0xb8);
 if root(lot).is_none()||ptr::read_unaligned((descriptor+0xb0)as*const u64)!=0{return original(descriptor,stream);}
 let mut copy=ptr::read_unaligned(descriptor as*const [u64;24]);
 let p=copy.as_mut_ptr()as usize;
 ptr::write_unaligned((p+0xb0)as*mut u32,SOURCE_TAG);
 ptr::write_unaligned((p+0xb4)as*mut i32,lot);
 original(p,stream)
}
pub unsafe extern "system" fn decode(p:usize,stream:usize)->u8{
 let original:unsafe extern "system" fn(usize,usize)->u8=std::mem::transmute(DECODE_ORIGINAL.load(Ordering::Acquire));
 let ok=original(p,stream);
 if ok!=0 && network_choice(p) && read_i32(p+0xb8)==-1 &&
    ptr::read_unaligned((p+0xb0)as*const u32)==SOURCE_TAG {
  let lot=read_i32(p+0xb4);
  if root(lot).is_some(){
   ptr::write_unaligned((p+0xb8)as*mut i32,lot);
   ptr::write_unaligned((p+0xb0)as*mut u64,0);
  }
 }
 ok
}
unsafe fn local(base:usize)->Option<(usize,u8)>{
 let world=read_ptr(base+WORLD_RVA);if world==0{return None;}
 let player=read_ptr(world+0x174e8);if player==0{return None;}
 let game=read_ptr(base+GAME_DATA_RVA);if game==0{return None;}
 let data=read_ptr(game+8);if data==0{return None;}
 let slot=read_i32(data+8).max(1);(1..=3).contains(&slot).then_some((player,slot as u8))
}
// A node may predate hook readiness or have been restored while the player
// singleton was absent. Check the actual existing list before reserving it.
unsafe fn already_existing(manager:usize,key:Key,count:u8)->bool{
 let mut node=read_ptr(manager+0x58);
 for _ in 0..65536{
  if node==0{return false;}
  if Key::from_descriptor(node+0x20,count)==key{return true;}
  node=read_ptr(node);
 }true // A malformed/overlong list must not turn an old ball into a new one.
}
fn fresh_caller(rva:usize)->bool{matches!(rva,0x575a53|0x575b39|0x573fe6|0x57f046|0x57f42c)}
#[unsafe(naked)]
pub unsafe extern "system" fn create(){core::arch::naked_asm!("mov r8,[rsp]","jmp {callback}",callback=sym capture);}
unsafe extern "system" fn capture(manager:usize,descriptor:usize,caller:usize)->usize{
 let original:unsafe extern "system" fn(usize,usize)->usize=std::mem::transmute(CREATE_ORIGINAL.load(Ordering::Acquire));
 if !ACTIVE.load(Ordering::Acquire)||descriptor==0{return original(manager,descriptor);}
 // A public weapon placed from a split choice returns through this same native
 // creator on the host and on the guest who requested it.
 if read_i32(descriptor+0x58)&0x1f==0x11{
  let node=original(manager,descriptor);
  if node!=0{placement::created(manager,descriptor);}
  return node;
 }
 let lot=read_i32(descriptor+0xb8);let Some((a,b,count))=root(lot)else{return original(manager,descriptor);};
 let base=BASE.load(Ordering::Acquire);let Some((player,slot))=local(base)else{return original(manager,descriptor);};
 let flags=ptr::read_unaligned((descriptor+0xaa)as*const u16);
 let source=ptr::read_unaligned((descriptor+0x60)as*const u64);
 let position=ptr::read_unaligned((descriptor+0x78)as*const [f32;3]);
 if manager!=read_ptr(base+ITEMS_RVA)||((flags>>7)&3)as u8!=slot||flags&0x4000==0||
  source as u32==u32::MAX||source==0||!position.iter().all(|p|p.is_finite())||read_i32(descriptor+0x58)&15!=2{return original(manager,descriptor);}
 let key=Key::from_descriptor(descriptor,count);
 // Restoration/late-join packets establish an old group without a new roll.
 // Also remember pre-acquisition groups so subsequent option packets stay old.
 let eligible=fresh_caller(caller.wrapping_sub(base))&&has_effect(player,OWNER,base)&&!has_effect(player,reward_cycle_runtime::RESET,base)&&!already_existing(manager,key,count);
 let job=Job::original(key,[a,b],count,ptr::read_unaligned((descriptor+0x74)as*const u32),position);
 let index=ptr::read((descriptor+0xa9)as*const u8);
 if index>=count{return original(manager,descriptor);}
 {
  let Ok(mut state)=STATE.lock()else{return original(manager,descriptor);};
  if !state.observe(job,eligible,roll_split){drop(state);return original(manager,descriptor);}
  let Some(ball)=state.originals.get_mut(&key)else{return 0;};
  if ball.received&(1<<index)!=0{return 0;}
  ball.received|=1<<index;
  ball.tier=ball.tier.max(option_tier(flags));
 }
 // Keep the native orb and its original flight. It stays reserved until landing.
 let node=original(manager,descriptor);
 if node!=0{
  let handle=ptr::read_unaligned((node+0x78)as*const u32);
  if let Ok(mut state)=STATE.lock(){if let Some(ball)=state.originals.get_mut(&key){
   ball.handles[index as usize]=handle;state.locked.insert(handle);
  }}
 }
 node
}
// Called at the original dispatch only within our synchronous native redraw.
// Always copy the descriptor: the caller owns its buffer and network metadata.
pub unsafe fn route(manager:usize,descriptor:usize,original:unsafe extern "system" fn(usize,usize)->usize)->Option<usize>{
 let scope=GENERATED.with(Cell::get);
 if scope.manager==0||scope.manager!=manager||descriptor==0||read_i32(descriptor+0xb8)!=scope.lot{return None;}
 let owner=ptr::read_unaligned((descriptor+0xaa)as*const u16);
 let index=ptr::read((descriptor+0xa9)as*const u8);
 if ((owner>>7)&3)as u8!=scope.slot||index>=scope.count||scope.limit&(1<<index)==0||scope.mask&(1<<index)!=0||scope.attempted&(1<<index)!=0{return Some(0);}
 GENERATED.with(|c|{let mut s=c.get();s.attempted|=1<<index;c.set(s);});
 let mut copy=ptr::read_unaligned(descriptor as*const [u64;24]);let p=copy.as_mut_ptr()as usize;
 ptr::write_unaligned((p+0x58)as*mut u32,ptr::read_unaligned((p+0x58)as*const u32)&!0x10);
 ptr::write_unaligned((p+0x60)as*mut u64,scope.source);
 ptr::write_unaligned((p+0x74)as*mut u32,scope.map);
 ptr::write_unaligned((p+0x78)as*mut [f32;3],scope.position);
 ptr::write_unaligned((p+0x84)as*mut [f32;3],scope.origin);
 let rarity=if scope.lot>=FLOOR_LOT{1}else{option_tier(owner).min(2)as u16};
 ptr::write_unaligned((p+0xaa)as*mut u16,(owner&!0x1f)|rarity|0x840);
 let node=original(manager,p);
 if node!=0{GENERATED.with(|c|{let mut s=c.get();s.mask|=1<<index;s.handles[index as usize]=ptr::read_unaligned((node+0x78)as*const u32);c.set(s);});}
 Some(node)
}
fn same_partition(a:i32,b:i32)->bool{if generated(a)||generated(b){a==b}else{true}}
unsafe extern "system" fn group_allowed(incoming:usize,candidate:usize)->u8{
 let incoming_lot=read_i32(incoming+0xd8);let candidate_lot=read_i32(candidate+0xd8);
 // All options in a personal Hex choice must remain separate from world rewards.
 if hex_pool::is_template_lot(incoming_lot)!=hex_pool::is_template_lot(candidate_lot){return 0;}
 let incoming_snack=snack::bonus_lot(incoming_lot);let candidate_snack=snack::bonus_lot(candidate_lot);
 if incoming_snack!=candidate_snack{return 0;}
 if incoming_snack{
  let scope=PERSONAL_REWARD.with(Cell::get);
  if scope.manager!=0&&snack::bonus_lot(scope.lot){return scope.nodes.contains(&candidate)as u8;}
 }
 let key_for=|node:usize,lot:i32|root(lot).map(|(_,_,count)|Key::from_descriptor(node+0x20,count));
 let incoming_key=key_for(incoming,incoming_lot);let candidate_key=key_for(candidate,candidate_lot);
 if incoming_key!=candidate_key&&(incoming_key.is_some()||candidate_key.is_some())&&STATE.lock().is_ok_and(|s|
    incoming_key.iter().chain(candidate_key.iter()).any(|k|s.originals.contains_key(k))){return 0;}
 if !same_partition(incoming_lot,candidate_lot){return 0;}
 let draw=GENERATED.with(Cell::get);
 if draw.manager!=0 && draw.lot==incoming_lot{
  let handle=ptr::read_unaligned((candidate+0x78)as*const u32);
  return (0..3).any(|n|draw.mask&(1<<n)!=0&&draw.handles[n]==handle)as u8;
 }
 1
}
#[unsafe(naked)]
pub unsafe extern "system" fn group(){core::arch::naked_asm!(
 "pushfq","sub rsp,0xc8",
 "mov [rsp+0x28],rax","mov [rsp+0x30],rcx","mov [rsp+0x38],rdx",
 "mov [rsp+0x40],r8","mov [rsp+0x48],r9","mov [rsp+0x50],r10","mov [rsp+0x58],r11",
 "movdqu [rsp+0x60],xmm0","movdqu [rsp+0x70],xmm1","movdqu [rsp+0x80],xmm2",
 "movdqu [rsp+0x90],xmm3","movdqu [rsp+0xa0],xmm4","movdqu [rsp+0xb0],xmm5",
 "mov rcx,r8","call {callback}","mov [rsp+0x20],al",
 "movdqu xmm0,[rsp+0x60]","movdqu xmm1,[rsp+0x70]","movdqu xmm2,[rsp+0x80]",
 "movdqu xmm3,[rsp+0x90]","movdqu xmm4,[rsp+0xa0]","movdqu xmm5,[rsp+0xb0]",
 "mov rax,[rsp+0x28]","mov rcx,[rsp+0x30]","mov rdx,[rsp+0x38]",
 "mov r8,[rsp+0x40]","mov r9,[rsp+0x48]","mov r10,[rsp+0x50]","mov r11,[rsp+0x58]",
 "cmp byte ptr [rsp+0x20],0","je 2f",
 "add rsp,0xc8","popfq","jmp qword ptr [rip+{original}]",
 "2:","add rsp,0xc8","popfq","jmp qword ptr [rip+{skip}]",
 callback=sym group_allowed,original=sym GROUP_ORIGINAL,skip=sym GROUP_SKIP
 );}
pub fn reset(){if let Ok(mut s)=STATE.lock(){*s=State::new();}placement::reset();sync::reset();}
// Native 579FF0 chooses the highest displayed rarity across the choices.
fn option_tier(flags:u16)->u8{match flags&15{3=>3,2|9=>2,_=>1}}
pub unsafe extern "system" fn pickup(manager:usize,handle:u32,choice:i32)->usize{
 let base=BASE.load(Ordering::Acquire);
 if ACTIVE.load(Ordering::Acquire)&&manager==read_ptr(base+ITEMS_RVA)&&
    (STATE.lock().is_ok_and(|state|state.locked.contains(&handle))||placement::reserved(manager,handle,base)){return 0;}
 let original:unsafe extern "system" fn(usize,u32,i32)->usize=std::mem::transmute(PICKUP_ORIGINAL.load(Ordering::Acquire));
 original(manager,handle,choice)
}
unsafe fn lookup(items:usize,handle:u32,base:usize)->usize{
 let find:unsafe extern "system" fn(usize,u32)->usize=std::mem::transmute(base+LOOKUP_RVA);find(items,handle)
}
fn enqueue(job:Job){if let Ok(mut state)=STATE.lock(){state.queue.push_back(job);}}
unsafe fn resolve_landing(job:&mut Job,items:usize,base:usize,dt:f32)->bool{
 let original=matches!(job.phase,Phase::Original);
 let ball=match job.phase{
  Phase::Children=>{job.grace=(job.grace-dt).max(0.);if job.grace>0.{enqueue(*job);return false;}return true;},
  Phase::Original=>{let Some(ball)=STATE.lock().ok().and_then(|s|s.originals.get(&job.key).copied())else{return false;};ball},
  Phase::Landing(ball)=>ball,
 };
 job.grace=(job.grace-dt).max(0.);
 if original&&ball.received!=(1<<job.count)-1&&job.grace>0.{enqueue(*job);return false;}
 let mut head=None;let mut tier=ball.tier.max(1);
 for handle in ball.handles.into_iter().filter(|&h|h!=0){
  let node=lookup(items,handle,base);
  if node==0{continue;}
  tier=tier.max(option_tier(ptr::read_unaligned((node+0xca)as*const u16)));
  if read_ptr(node+0x18)==0{head=Some((node,handle));}
 }
 if let Some((node,_))=head{
  // Wait for native activation even when this original has no flight animation.
  // 5835E0 clears the flight flag at the end of the native update.
  let flags=ptr::read_unaligned((node+0xca)as*const u16);
  if ptr::read((node+0xf9)as*const u8)==1||(original&&(flags&0x40==0||flags&0x800==0&&job.grace>0.)){enqueue(*job);return false;}
  job.map=ptr::read_unaligned((node+0x94)as*const u32);
  job.position=ptr::read_unaligned((node+0x98)as*const [f32;3]);
 }
 // Remove only the parent choice group, without invoking the award routine.
 let remove:unsafe extern "system" fn(usize,u32,u8)->u8=std::mem::transmute(base+REMOVE_GROUP_RVA);
 if let Some((_,handle))=head{if remove(items,handle,0)==0{enqueue(*job);return false;}}
 if let Ok(mut s)=STATE.lock(){
  if original{s.originals.remove(&job.key);}
  for h in ball.handles{s.locked.remove(&h);}
 }
 // A missing preview allocation can still deliver the owed rewards. A parent
 // which existed and was later removed by the world must never pay out again.
 if head.is_none()&&ball.handles.iter().any(|&h|h!=0){return false;}
 let tier=tier.min(job.ceiling);
 if tier<=2{
  let floor=FLOOR_LOT+(job.lots[0]-98_000_000);
  job.lots=[floor;2];
 }
 job.ceiling=tier-1;job.phase=Phase::Children;job.grace=0.;true
}
unsafe fn emit_children(mut job:Job,items:usize,player:usize,base:usize){
 let Some((a,b,_))=root(job.key.lot)else{return;};
 let slot=job.key.slot;
 let previous=GENERATED.with(Cell::get);
 let spawn:unsafe extern "system" fn(usize,usize,i32,u8,u8,u8,u8,*const f32)->u8=std::mem::transmute(base+SPAWN_RVA);
 for i in 0..2{if job.wanted[i]!=0&&job.masks[i]&job.wanted[i]==job.wanted[i]{continue;}
  // A partially created ball may already have been claimed before retry.
  // Never replenish that ball after its successfully created options vanish.
  if job.masks[i]!=0{
   if (0..3).any(|n|job.masks[i]&(1<<n)!=0&&lookup(items,job.handles[i][n],base)==0){job.masks[i]=job.wanted[i];continue;}
  }
  let position=job.child_position(i);
  GENERATED.with(|c|c.set(Generated{origin:job.position,manager:items,lot:job.lots[i],slot,source:job.key.source,map:job.map,position,mask:job.masks[i],attempted:0,count:job.count,limit:if job.wanted[i]==0{(1<<job.count)-1}else{job.wanted[i]},handles:job.handles[i]}));
  let p=[position[0],position[1],position[2],0.];spawn(items,player,job.lots[i],0,1,1,0,p.as_ptr());
  let result=GENERATED.with(|c|{let result=c.get();c.set(previous);result});
  job.masks[i]=result.mask;job.handles[i]=result.handles;if job.wanted[i]==0{job.wanted[i]=result.attempted;}
 }
 for i in 0..2{
  if job.scheduled&(1<<i)!=0||job.wanted[i]==0||job.masks[i]&job.wanted[i]!=job.wanted[i]{continue;}
  job.scheduled|=1<<i;
  if job.ceiling==0||!roll_split(){continue;}
  let handles=job.handles[i];
  if !handles.iter().any(|&h|h!=0)||handles.iter().any(|&h|h!=0&&lookup(items,h,base)==0){continue;}
  let mut child=Job::original(job.key,[a,b],job.count,job.map,job.child_position(i));
  child.phase=Phase::Landing(Ball{handles,received:job.masks[i],tier:1});child.ceiling=job.ceiling;child.generation=job.generation+1;
  if let Ok(mut s)=STATE.lock(){s.locked.extend(handles.into_iter().filter(|&h|h!=0));s.queue.push_back(child);}
 }
 if job.scheduled!=3{job.grace=0.25;enqueue(job);}
}
pub unsafe fn update(player:usize,base:usize,dt:f32){
 if has_effect(player,reward_cycle_runtime::RESET,base){reset();return;}
 placement::finish(read_ptr(base+ITEMS_RVA),base);
 let stats=native_actor::stats(player);
 if !has_effect(player,OWNER,base)||stats==0||native_actor::incapacitated(player,stats){return;}
 let Some((_,slot))=local(base)else{return;};let items=read_ptr(base+ITEMS_RVA);
 if items==0||GENERATED.with(Cell::get).manager!=0{return;}
 let dt=if dt.is_finite(){dt.max(0.)}else{0.};
 // Take only work already pending at the start of this update. Each new
 // generation gets its own native flight before it can resolve.
 let work=STATE.lock().map(|mut s|std::mem::take(&mut s.queue)).unwrap_or_default();
 for mut job in work{
  if job.key.slot!=slot{enqueue(job);continue;}
  if resolve_landing(&mut job,items,base,dt){emit_children(job,items,player,base);}
 }
}
#[cfg(test)]mod tests{
 use super::*;
 fn job(n:u64)->Job{let r=ROOTS[0];Job::original(Key{source:n,lot:r.0,slot:1,batch:None},[r.1,r.2],r.3,0,[0.;3])}
 #[test]fn eligible_groups_roll_once_and_misses_stay_original(){
  let mut state=State::new();
  for n in 1..=12{
   let j=job(n);let split=n%2==0;
   assert_eq!(state.observe(j,true,||split),split);
   assert_eq!(state.observe(j,true,||panic!("duplicate reroll")),split);
  }
  assert_eq!(state.queue.len(),6);
  let old=job(13);assert!(!state.observe(old,false,||panic!("old group roll")));
  assert!(!state.observe(old,true,||panic!("old group reroll")));
 }
 #[test]fn native_group_partitions(){
  for &(source,a,b,_)in ROOTS{assert!(root(source).is_some());assert!(generated(a)&&generated(b));assert!(same_partition(a,a));assert!(!same_partition(a,b));assert!(!same_partition(source,a));assert!(same_partition(source,source+10));}
  for caller in [0x573955,0x57ee94,0]{assert!(!fresh_caller(caller));}
 }
}

#[cfg(test)]#[path="potential_fission_native_tests.rs"]mod native_tests;
