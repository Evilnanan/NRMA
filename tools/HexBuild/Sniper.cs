using SoulsFormats;
using System.Security.Cryptography;
using System.Text.Json;
static class Sniper {
 public const int Owner=95004500,Cue=95004501,Root=95004510;
 const int LevelCount=15,RowsPerLevel=3,MagicAttackPerLevel=10;
 static int LevelRoot(int level)=>Root+(level-1)*RowsPerLevel;
 public const int SoundEvent=9101140;
 const int ReleaseSound=526261; // a405/45011, sound type 5
 public static void OwnerRange(PARAM.Row owner){Set(owner,"bowDistRate",200);Set(owner,"stateInfo",168);}
 public static void Volley(Func<string,int,int,PARAM.Row> clone){
  for(int level=1;level<=LevelCount;level++){
   int rootId=LevelRoot(level),flightId=rootId+1,impactId=rootId+2;
   var root=clone("Bullet",10438100,rootId);
   Set(root,"HitBulletID",flightId);
   Set(root,"spEffectIDForShooter",Cue);
   var flight=clone("Bullet",10438101,flightId);
   Set(flight,"life",4.4f);
   Set(flight,"lockShootLimitAng",0);
   Set(flight,"HitBulletID",impactId);
   var impact=clone("Bullet",10438102,impactId);
   Set(impact,"atkId_Bullet",impactId);
   var attack=clone("AtkParam_Pc",43811,impactId);
   Set(attack,"atkMag",Convert.ToInt32(attack["atkMag"].Value)+level*MagicAttackPerLevel);
  }
  var marker=clone("SpEffectParam",8000000,Cue);
  foreach(var (field,value) in new(string,object)[]{("maxHpRate",1f),("effectEndurance",.6f),("spCategory",20),("saveCategory",-1),("isDisableNetSync",1),("iconId",-1),("spEffectTextId_1",-1),("displayIconForPermamentEffect",0),("vfxId",-1)})Set(marker,field,value);
 }
 public static void Update(string inputMod,string outputMod,Func<string,Memory<byte>,PARAM> parse){
  var b=SFUtil.DecryptNightreignRegulation(Path.Combine(inputMod,"regulation.bin"));
  var entries=new[]{"Bullet","AtkParam_Pc","SpEffectParam","SpEffectVfxParam"}.ToDictionary(n=>n,n=>b.Files.Single(f=>Name(f)==n+".param"));
  var tables=entries.ToDictionary(e=>e.Key,e=>parse(e.Key,e.Value.Bytes));
  OwnerRange(tables["SpEffectParam"].Rows.Single(r=>r.ID==Owner));
  Volley((name,from,id)=>{
   var rows=tables[name].Rows;
   var next=new PARAM.Row(rows.Single(r=>r.ID==from)){ID=id};
   rows.RemoveAll(r=>r.ID==id);rows.Add(next);return next;
  });
  var legacy=tables["SpEffectVfxParam"].Rows.SingleOrDefault(r=>r.ID==Cue);
  if(legacy!=null){
   var old=new PARAM.Row(tables["SpEffectVfxParam"].Rows.Single(r=>r.ID==110015)){ID=Cue};
   Set(old,"initSeId",ReleaseSound);
   Check(MmvCompat.SameCells(old,legacy,true),"Unexpected existing Sniper cue VFX");
   tables["SpEffectVfxParam"].Rows.Remove(legacy);
  }
  foreach(var (name,t) in tables){t.Rows=t.Rows.OrderBy(r=>r.ID).ToList();entries[name].Bytes=t.Write();}
  Directory.CreateDirectory(outputMod);SFUtil.EncryptNightreignRegulation(Path.Combine(outputMod,"regulation.bin"),b,b.Compression);
 }
 static string Name(BinderFile file)=>file.Name.Replace('\\','/').Split('/').Last();
 static void Check(bool value,string message){if(!value)throw new InvalidDataException(message);}
 static void Set(PARAM.Row row,string field,object value)=>row[field].Value=Convert.ChangeType(value,row[field].Value.GetType());
 static string Hash(string path)=>Convert.ToHexString(SHA256.HashData(File.ReadAllBytes(path)));
 static bool Removal(EMEVD.Instruction i,int effect)=>i.Bank==2004&&i.ID==21&&i.ArgData.Length==8&&BitConverter.ToInt32(i.ArgData)==20000&&BitConverter.ToInt32(i.ArgData,4)==effect;
 static EMEVD.Instruction I(int bank,int id,params object[] args)=>new(bank,id,args);
 static EMEVD.Instruction SoundInit()=>I(2000,0,0,SoundEvent,0);
 static bool StartsSound(EMEVD.Instruction i)=>i.Bank==2000&&i.ID==0&&i.ArgData.Length>=8&&BitConverter.ToInt32(i.ArgData,4)==SoundEvent;
 static EMEVD.Event SoundWorker(){
  var worker=new EMEVD.Event(SoundEvent,EMEVD.Event.RestBehaviorType.Restart);
  EMEVD.Instruction Marker(bool on)=>I(4,5,(sbyte)0,20000u,Cue,(byte)(on?1:0),(sbyte)0,1f);
  worker.Instructions.AddRange([I(2000,2,(byte)0),Marker(true),I(2010,2,20000u,5,ReleaseSound),Marker(false),I(1000,4,(byte)1)]);
  return worker;
 }
 static void AddSound(EMEVD ev){
  var ctor=ev.Events.Single(e=>e.ID==0);
  var worker=ev.Events.SingleOrDefault(e=>e.ID==SoundEvent);
  if(worker!=null){CheckSound(ev);return;}
  Check(ctor.Parameters.Count==0&&!ctor.Instructions.Any(StartsSound),"Unexpected Sniper sound constructor");
  ctor.Instructions.Insert(0,SoundInit());ev.Events.Add(SoundWorker());
 }
 static void CheckSound(EMEVD ev){
  var init=ev.Events.Single(e=>e.ID==0).Instructions.Single(StartsSound);
  Check(init.Layer==null&&init.ArgData.SequenceEqual(SoundInit().ArgData)&&Cages.Same(ev.Events.Single(e=>e.ID==SoundEvent),SoundWorker()),"Unexpected local Sniper sound event");
 }
 public static void StageSound(string input,string output){
  var ev=EMEVD.Read(input);AddSound(ev);ev.Write(output);
  var read=EMEVD.Read(output);CheckSound(read);
  Check(ev.Events.Count==read.Events.Count&&ev.Events.Zip(read.Events).All(p=>Cages.Same(p.First,p.Second)),"Sniper sound event readback failed");
 }
 public static void Events(EMEVD events){
  AddSound(events);
  foreach(int id in new[]{1111,9100941}){
   var e=events.Events.Single(e=>e.ID==id);Check(e.Parameters.Count==0&&!e.Instructions.Any(i=>Removal(i,Owner)),"Sniper reset already present or parameterized");
   int index=id==1111?9:e.Instructions.FindLastIndex(i=>i.Bank==2004&&i.ID==21)+1;
   Check(index>0&&e.Instructions.Take(index).Any(i=>i.Bank==3),"Missing guarded player-ready anchor");
   e.Instructions.Insert(index,new EMEVD.Instruction(2004,21,new object[]{20000u,Owner}));
  }
 }
 static void ValidateEvents(EMEVD events){CheckSound(events);foreach(int id in new[]{1111,9100941})Check(events.Events.Single(e=>e.ID==id).Instructions.Count(i=>Removal(i,Owner))==1,"Sniper cleanup missing or duplicated");}
 static object Validate(BND4 native,BND4 current,Func<string,Memory<byte>,PARAM> parse){
  PARAM Table(BND4 b,string name)=>parse(name,b.Files.Single(f=>Name(f)==name+".param").Bytes);
  var bullets=Table(current,"Bullet").Rows.ToDictionary(r=>r.ID);
  int Bullet(int id,string field)=>Convert.ToInt32(bullets[id][field].Value);
  var spell=Table(current,"Magic").Rows.Single(r=>r.ID==4381);
  Check(Convert.ToInt32(spell["refId1"].Value)==10438100&&Bullet(10438100,"HitBulletID")==10438101&&
        Bullet(10438101,"numShoot")==4&&Bullet(10438101,"HitBulletID")==10438102&&
        Bullet(10438102,"atkId_Bullet")==43811,"Loretta's Mastery uncharged four-arrow chain changed");
  var reference=Table(native,"SpEffectParam").Rows;
  Check(Convert.ToInt32(reference.Single(r=>r.ID==8600100)["atkOccurrenceSpEffectId"].Value)==8600105&&
        Convert.ToSingle(reference.Single(r=>r.ID==8600105)["effectEndurance"].Value)==.3f,"HP-on-hit timing reference changed");
  var effects=Table(current,"SpEffectParam");var expected=new PARAM.Row(Table(native,"SpEffectParam").Rows.Single(r=>r.ID==8000000)){ID=Owner};
  foreach(var(field,value)in new(string,object)[]{("maxHpRate",1f),("spCategory",20),("iconId",22278),("spEffectTextId_1",Owner),("displayIconForPermamentEffect",0)})Set(expected,field,value);
  OwnerRange(expected);
  var attacks=Table(current,"AtkParam_Pc").Rows.ToDictionary(r=>r.ID);
  for(int level=1;level<=LevelCount;level++){
   int rootId=LevelRoot(level),flightId=rootId+1,impactId=rootId+2;
   var expectedFlight=new PARAM.Row(bullets[10438101]){ID=flightId};
   Set(expectedFlight,"life",4.4f);
   Set(expectedFlight,"lockShootLimitAng",0);
   Set(expectedFlight,"HitBulletID",impactId);
   var expectedImpact=new PARAM.Row(bullets[10438102]){ID=impactId};
   Set(expectedImpact,"atkId_Bullet",impactId);
   var expectedAttack=new PARAM.Row(attacks[43811]){ID=impactId};
   Set(expectedAttack,"atkMag",Convert.ToInt32(expectedAttack["atkMag"].Value)+level*MagicAttackPerLevel);
   Check(Bullet(rootId,"HitBulletID")==flightId&&Bullet(rootId,"spEffectIDForShooter")==Cue&&
         MmvCompat.SameCells(expectedFlight,bullets[flightId],true)&&
         MmvCompat.SameCells(expectedImpact,bullets[impactId],true)&&
         MmvCompat.SameCells(expectedAttack,attacks[impactId],true),
         $"Sniper flight, impact or base magic attack mismatch at level {level}");
  }
  var marker=effects.Rows.Single(r=>r.ID==Cue);
  Check(Convert.ToSingle(marker["effectEndurance"].Value)==.6f&&Convert.ToInt32(marker["vfxId"].Value)==-1&&Convert.ToInt32(marker["isDisableNetSync"].Value)==1,"Invalid local sound marker");
  Check(CageAI.Same(expected,effects.Rows.Single(r=>r.ID==Owner)),"Owner is not the neutral persistent template");
  Check(RewardCatalog.HistoryBit(Owner)==42&&RewardCatalog.HistoricalOwners[41]==95004400,"History slots must not be reused");
  var good=Table(current,"EquipParamGoods").Rows.Single(r=>r.ID==Owner);
  Check(Convert.ToInt32(good["goodsType"].Value)==11&&Convert.ToInt32(good["vagrantItemLotId"].Value)==Owner&&Convert.ToInt32(good["rarity"].Value)==3&&Convert.ToInt32(good["iconId"].Value)==20278,"Reward good mismatch");
  var item=Table(current,"ItemTableParam").Rows.Single(r=>r.ID==RewardCatalog.Table(Owner));
  Check(Convert.ToInt32(item["itemId"].Value)==Owner&&Convert.ToInt32(item["itemCategory"].Value)==1&&Convert.ToInt32(item["chanceWeight"].Value)==1000,"Reward wiring mismatch");
  var attach=Table(current,"AttachEffectParam").Rows.Single(r=>r.ID==Owner);var permanent=Table(current,"PermanentBuffParam").Rows.Single(r=>r.ID==Owner);
  Check(Convert.ToInt32(attach["passiveSpEffectId_1"].Value)==Owner&&Convert.ToInt32(attach["statusIconId"].Value)==22278&&Convert.ToInt32(permanent["iconId"].Value)==21278,"Ability icon/effect mismatch");
  return new{owner=Owner,spell=4381,volleyRoot=Root,flightLife=4.4f,magicAttackPerLevel=MagicAttackPerLevel,bowDistRate=200,arrows=4,conflicts=new[]{95000900},historyBit=42,historyBytes=(RewardCatalog.HistoricalOwners.Length+7)/8,sharedProjectilesUnchanged=true};
 }
 public static void Verify(string root,BND4 native,Func<string,Memory<byte>,PARAM> parse){
  var current=SFUtil.DecryptNightreignRegulation(Path.Combine(root,"dist/mod/regulation.bin"));Validate(native,current,parse);
  string input=Path.Combine(root,"dist/mod/event/common.emevd.dcx");var raw=Environment.GetEnvironmentVariable("NRHEX_HEARTSTEALER_COMMON_RAW");
  if(!OperatingSystem.IsWindows()&&raw!=null){Check(Hash(input).Equals(Environment.GetEnvironmentVariable("NRHEX_HEARTSTEALER_COMMON_SHA256"),StringComparison.OrdinalIgnoreCase),"Raw common does not match installed package");input=raw;}
  ValidateEvents(EMEVD.Read(input));Console.WriteLine("PASS: installed Sniper owner, neutral attack, extended range, 15 level-scaled four-arrow chains, reward wiring and guarded resets");
 }
 public static void Stage(string root,BND4 native,Func<string,Memory<byte>,PARAM> parse,Hex[] hexes){
  string input=Path.Combine(root,"dist/mod/regulation.bin"),dir=Path.Combine(root,"work/sniper/staged");Directory.CreateDirectory(dir);
  var b=SFUtil.DecryptNightreignRegulation(input);var before=b.Files.ToDictionary(f=>f.ID,f=>f.Bytes.ToArray());
  var names=new[]{"Bullet","AtkParam_Pc","SpEffectVfxParam","SpEffectParam","AttachEffectParam","PermanentBuffParam","EquipParamGoods","ItemTableParam"};
  var tables=names.ToDictionary(n=>n,n=>parse(n,b.Files.Single(f=>Name(f)==n+".param").Bytes));
  var originals=names.ToDictionary(n=>n,n=>parse(n,native.Files.Single(f=>Name(f)==n+".param").Bytes));var added=names.ToDictionary(n=>n,n=>new HashSet<int>());
  PARAM.Row Add(string name,int from,int id){Check(!tables[name].Rows.Any(r=>r.ID==id),$"Sniper ID collision: {name}/{id}");var r=new PARAM.Row(originals[name].Rows.First(r=>r.ID==from)){ID=id};tables[name].Rows.Add(r);added[name].Add(id);return r;}
  var h=hexes.Single(h=>h.Id==Owner);var s=Add("SpEffectParam",8000000,Owner);
  foreach(var(f,v)in new(string,object)[]{("maxHpRate",1f),("spCategory",20),("iconId",h.SmallIcon),("spEffectTextId_1",Owner),("displayIconForPermamentEffect",0)})Set(s,f,v);
  OwnerRange(s);Volley(Add);
  var a=Add("AttachEffectParam",8000000,Owner);foreach(var(f,v)in new[]{("passiveSpEffectId_1",Owner),("attachTextId",Owner),("statusIconId",h.SmallIcon),("compatibilityId",-1),("overrideBaseEffectId",Owner),("overrideEffectId",-1),("isNumericEffect",0),("displayPercentageSymbol",0),("displayedModifierValue",0)})Set(a,f,v);
  var p=Add("PermanentBuffParam",8000000,Owner);foreach(var(f,v)in new[]{("spEffectId",Owner),("iconId",h.LargeIcon),("displayNumericValue",2),("displayPercentageSymbol",0),("numericValue",0)})Set(p,f,v);
  var g=Add("EquipParamGoods",8000001,Owner);Set(g,"vagrantItemLotId",Owner);Set(g,"iconId",h.LargeIcon-1000);Set(g,"rarity",3);
  var t=Add("ItemTableParam",4200000,RewardCatalog.Table(Owner));Set(t,"disableParam_NT",0);Set(t,"itemId",Owner);Set(t,"chanceWeight",1000);
  foreach(string n in names){Check(tables[n].Rows.Count<65536,"PARAM row capacity exceeded");tables[n].Rows=tables[n].Rows.OrderBy(r=>r.ID).ToList();b.Files.Single(f=>Name(f)==n+".param").Bytes=tables[n].Write();}
  string output=Path.Combine(dir,"regulation.bin");SFUtil.EncryptNightreignRegulation(output,b,b.Compression);var final=SFUtil.DecryptNightreignRegulation(output);
  Check(final.Version==b.Version&&final.Compression==b.Compression&&final.Files.Count==b.Files.Count,"Regulation metadata changed");
  foreach(var f in final.Files){var expected=b.Files.Single(v=>v.ID==f.ID);Check(f.Name==expected.Name&&f.Flags==expected.Flags&&f.Bytes.Span.SequenceEqual(expected.Bytes.Span),"Regulation readback mismatch");var n=names.SingleOrDefault(n=>Name(f)==n+".param");if(n==null){Check(f.Bytes.Span.SequenceEqual(before[f.ID]),"Unrelated table changed");continue;}var old=parse(n,before[f.ID]);var now=parse(n,f.Bytes);Check(now.Rows.Count==old.Rows.Count+added[n].Count,"Row count mismatch");Check(old.Rows.OrderBy(r=>r.ID).Zip(now.Rows.Where(r=>!added[n].Contains(r.ID)).OrderBy(r=>r.ID)).All(p=>CageAI.Same(p.First,p.Second)),"Existing parameter row changed");}
  var audit=Validate(native,final,parse);RewardCatalog.Write(Path.Combine(dir,"hex_pool.rs"),hexes);
  string eventInput=Path.Combine(root,"dist/mod/event/common.emevd.dcx"),eventOutput=Path.Combine(dir,"common.emevd.dcx");var ev=EMEVD.Read(eventInput);var oldEv=EMEVD.Read(eventInput);Events(ev);ev.Write(eventOutput);var actualEv=EMEVD.Read(eventOutput);ValidateEvents(actualEv);
  Check(actualEv.Format==oldEv.Format&&actualEv.Compression==oldEv.Compression&&actualEv.Events.Count==oldEv.Events.Count+1&&actualEv.StringData.SequenceEqual(oldEv.StringData)&&actualEv.LinkedFileOffsets.SequenceEqual(oldEv.LinkedFileOffsets),"Event metadata changed");
  foreach(var old in oldEv.Events){var actual=actualEv.Events.Single(e=>e.ID==old.ID);Check(Cages.Same(ev.Events.Single(e=>e.ID==old.ID),actual),"Event readback mismatch");if(old.ID==0)actual.Instructions.RemoveAll(StartsSound);if(old.ID is 1111 or 9100941)actual.Instructions.RemoveAll(i=>Removal(i,Owner));Check(Cages.Same(old,actual),"Unrelated event instructions/parameters changed");}
  HexTexts.Stage(root,hexes,Path.Combine(dir,"texts"),allRewards:true,onlyIds:new[]{Owner});
  File.WriteAllText(Path.Combine(dir,"validation.json"),JsonSerializer.Serialize(new{inputSha256=Hash(input),outputSha256=Hash(output),eventInputSha256=Hash(eventInput),eventOutputSha256=Hash(eventOutput),hexCount=hexes.Length,addedRows=added.ToDictionary(p=>p.Key,p=>p.Value.Order().ToArray()),rowCounts=tables.ToDictionary(p=>p.Key,p=>p.Value.Rows.Count),audit,existingRowsAndUnrelatedTablesUnchanged=true,localSoundEvent=SoundEvent,gameplayVerified=false,multiplayerVerified=false},new JsonSerializerOptions{WriteIndented=true}));
  Console.WriteLine("PASS: Sniper owner, native Loretta volley, append-only reward and two guarded resets staged");
 }
}
