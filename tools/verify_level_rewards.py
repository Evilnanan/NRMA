"""Read-only native ABI, map lifecycle and saved receipt validation."""
from pathlib import Path
import hashlib, json, struct, re
import pefile
root = Path(__file__).resolve().parents[1]
exe = Path(r'D:/Program Files (x86)/Steam/steamapps/common/ELDEN RING NIGHTREIGN/Game/nightreign.exe')
raw = exe.read_bytes()
assert hashlib.sha256(raw).hexdigest() == '9545e8732919bb6a63904cb73223641b3c4f9a67869be5722612acca413e8b3a'
pe = pefile.PE(data=raw, fast_load=True)
def read(rva, n):
    off = pe.get_offset_from_rva(rva)
    return raw[off:off+n]
def check(rva, s):
    expected = bytes.fromhex(s)
    assert read(rva, len(expected)) == expected, hex(rva)
def singleton(rva):
    return rva + 7 + struct.unpack('<i', read(rva+3, 4))[0]
assert singleton(0x601302) == singleton(0x20edf6) == singleton(0x1fd598) == 0x3c078d0
check(0x601309, '48 8B 68 08 48 85 ED')
check(0x601316, '8B 75 68')  # Native level-loss code, not NPC reward/drop level.
check(0x20ee00, '48 8B 58 08')
check(0x20ee24, '8B 43 68')  # Same level serialized into native player data.
check(0x5759e0, '48 89 5C 24 08 57 48 83 EC 20 48 8B FA 48 8B D9')
check(0x575b39, '48 8B 5C 24 30 48 83 C4 20 5F C3')  # Preserves local creator RAX.
check(0x576e57, '89 85 88 00 00 00')  # Original lot -> descriptor+B8.
check(0x576c96, '66 33 C2 66 83 E0 0F')  # Native lot rarity -> descriptor+AA low nibble.
source=(root/'src/lib.rs').read_text(encoding='utf8')
level_source=(root/'src/level_rewards.rs').read_text(encoding='utf8')
levels=[int(v.strip()) for v in re.search(r'pub const LEVELS: \[i32; 4\] = \[([^\]]+)\]',level_source).group(1).split(',')]
assert levels==[2,7,10,13]
delivery=source[source.index('unsafe fn update_level_rewards'):source.index('unsafe extern "system" fn on_update')]
# Regression audit: queued native effects must never block reward creation.
assert 'if spawn_personal(player,base,chosen) {\n  reward_cycle_runtime::commit(player,base,next_seen);\n  apply(player,receipt,0);' in delivery
assert 'has_effect(player,receipt,base)' not in delivery
assert 'if CALLBACK_LOCK.swap(true,Ordering::AcqRel)' in delivery
assert 'now.saturating_add(250)' in delivery
assert 'unclaimed_hex_ball(base)' in delivery
assert 'choose_reward(player,base,None)' in delivery
assert 'terrain_exclusions(unavailable,reward_terrain::exclude_cage(reward_terrain::great_hollow(base)))' in source
assert 'level_rewards::REACHED.iter()' in delivery
spawn=source.split('unsafe fn spawn_personal(',1)[1].split('unsafe fn spawn_personal_lot(',1)[0]
assert spawn.rstrip().endswith('created>0\n}')  # A published partial ball must not be retried.
assert 'if !changed&&now<due{return;}' in delivery
assert 'state.changed(player,level)' in delivery
assert 'level_rewards::queued(reached,receipts,alive,expedition,pending_ball)' in delivery
events = json.loads((root/'.work/rewards/level-reward-events.json').read_text(encoding='utf8'))
native_maps = {bytes.fromhex(i['hex'])[1:6] for i in events['nativeExamples'] if i['Bank']==3}
assert b'\x01\x0a\x00\x00\x00' in native_maps and b'\x01\x0b\x00\x00\x00' in native_maps
assert any(i['Bank']==2004 and i['instruction']==21 for i in events['nativeExamples'])
gate, reset = sorted((e for e in events['events'] if e['ID']!=1111), key=lambda e:e['ID'])
deployment = next(e for e in events['events'] if e['ID']==1111)
original = next(e for e in json.loads((root/'data/reference/rewards/deployment-events.json').read_text(encoding='utf8')) if e['file']=='common.emevd.dcx' and e['ID']==1111)
def read_reset_block(event, start, receipts):
    # Other abilities share this guarded block. Inspect the level receipts and
    # reconstruct the enclosing event without any player effect instructions.
    instructions=event['instructions']
    end=start
    found=[]
    while end<len(instructions):
        op=instructions[end]
        if (op['Bank'],op['ID']) not in [(2004,8),(2004,21)]:break
        target,effect=struct.unpack('<Ii',bytes.fromhex(op['hex']))
        assert target==20000
        if 95001000<=effect<=95001013:found.append((op['ID'],effect))
        end+=1
    assert found==[(21,v) for v in receipts]
    return instructions[:start]+instructions[end:]

receipts=[*range(95001000,95001004),*range(95001010,95001014)]
assert deployment['Parameters']==[] and read_reset_block(deployment,9,receipts)==original['instructions']
hub_receipts=[*receipts[:4],95001004,*receipts[4:]]
hub_flow=read_reset_block(reset,4,hub_receipts)
for event in [gate, reset]:
    assert event['Parameters'] == [] and event['RestBehavior'] == 1
    assert event['instructions'][0] == {'Bank':2000, 'ID':2, 'hex':'00000000'}
    assert event['instructions'][-1] == {'Bank':1000, 'ID':4, 'hex':'01000000'}
gi=gate['instructions']; ri=reset['instructions']
assert [(i['Bank'], i['ID']) for i in gi] == [(2000,2),(3,8),(3,8),(0,0),(2004,8),(1001,0),(1000,4)]
assert [bytes.fromhex(i['hex'])[:6] for i in gi[1:3]] == [bytes([1,0,10,0,0,0]), bytes([1,0,11,0,0,0])]
assert struct.unpack('<f', bytes.fromhex(gi[5]['hex']))[0] == .5
assert [bytes.fromhex(i['hex'])[:6] for i in ri[1:3]] == [bytes([255,1,10,0,0,0]), bytes([255,1,11,0,0,0])]
assert [(i['Bank'],i['ID']) for i in hub_flow]==[(2000,2),(3,8),(3,8),(0,0),(3,8),(3,8),(0,0),(1000,4)]
assert hub_flow[3]['hex']=='0001FF00' and hub_flow[6]['hex']=='00010100'
rarities = json.loads(Path(r'D:/Program Files/Smithbox/Assets/PARAM/NR/Param Enums/RARITY.json').read_text(encoding='utf8'))
(root/'dist/reports/level-reward-validation.json').write_text(json.dumps(dict(version='0.14.6',levels=levels,nativePlayerLevelOffset='GameDataMan+8 -> +68',nativeLocalSlotOffset='GameDataMan+8 -> +8',rarity=3,nativeRarityEnum=rarities,localEventsReadBack=True,nativeFreshDeploymentGuardAndResetReadBack=True,returnToHubClearsOnlyCustomReceipts=True,nativeIntegerReturnAndDescriptorOffsetsVerified=True,gameplayVerified=False,multiplayerVerified=False),ensure_ascii=False,indent=2),encoding='utf8')
print('PASS: native level, slot, descriptor ABI, gold rarity and local hub-reset event readback.')
