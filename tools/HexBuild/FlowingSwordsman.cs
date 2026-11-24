using SoulsFormats;

static class FlowingSwordsman {
    public const int Owner = 95005900, Receipt = Owner + 1, Lot = 95559000;
    const int Flag = 29;
    static void Set(PARAM.Row r,string key,object v) =>
        r[key].Value = Convert.ChangeType(v,r[key].Value.GetType());
    public static void Mechanics(Func<string,int,int,PARAM.Row> clone, PARAM lots) {
        var receipt = clone("SpEffectParam",8000000,Receipt);
        Set(receipt,"maxHpRate",1f); Set(receipt,"saveCategory",10); Set(receipt,"dontDeleteOnDead",1);
        Set(receipt,"iconId",-1); Set(receipt,"spEffectTextId_1",-1); Set(receipt,"isDisableNetSync",1);
        OnyxMastery.WeaponLot(clone,lots,Lot,7060000);
        for(int i=0;i<3;i++){
            var row=clone("SpEffectParam",7037604+i,Owner+2+i);
            // Each threshold has its own effect. The private meter accounts for decay.
            Set(row,"stateInfo",0); Set(row,"accumuVal",0); Set(row,"spCategory",20);
        }
    }
    // The native chain drains through the base listener, each live 1.5 s tier,
    // and the instant overflow effect at 60. Route their ticks to a private
    // effect consumed by the owning player's counter.
    public static void Decay(PARAM effects, Func<string,int,int,PARAM.Row> clone) {
        int debit=Owner+5, overflow=Owner+6;
        var tag=clone("SpEffectParam",7037607,debit);
        var excess=clone("SpEffectParam",7037607,overflow);
        foreach(var row in new[]{tag,excess}) {
            Set(row,"stateInfo",0); Set(row,"accumuVal",0); Set(row,"spCategory",20);
            Set(row,"isDisableNetSync",1);
        }
        DecayTimings(effects);
        Set(effects[Owner],"cycleOccurrenceSpEffectId",debit);
        for(int i=0;i<3;i++) {
            var row=effects[Owner+2+i];
            Set(row,"cycleOccurrenceSpEffectId",debit);
        }
        Set(excess,"cycleOccurrenceSpEffectId",debit);
    }

    // Native weapon successive-attack effect. The second and third tiers
    // have shorter drain intervals than the separate 7037600 effect family.
    public static void DecayTimings(PARAM effects) {
        Set(effects[Owner],"motionInterval",effects[312501]["motionInterval"].Value);
        for(int i=0;i<3;i++)
            foreach(var field in new[]{"effectEndurance","motionInterval"})
                Set(effects[Owner+2+i],field,effects[312505+i][field].Value);
    }

    public static bool IsFeintGate(string name,long clip,TAE.Event e) {
        if(name!="a28.tae" || clip is not (30950 or 32950) || e.Type!=0) return false;
        var p=e.GetParameterBytes(false);
        return p.Length>=16 && BitConverter.ToInt32(p)==8 &&
               BitConverter.ToInt16(p,14)==(clip==30950?25059:25060);
    }
    public static void FeintWindows(BND4 bundle) {
        var file=bundle.Files.Single(f=>Path.GetFileName(f.Name)=="a28.tae");
        var tae=TAE.Read(file.Bytes);
        var additions=new Dictionary<int,List<TAE.Event>>();
        foreach(long id in new long[]{30950,32950}) {
            int index=tae.Animations.FindIndex(a=>a.ID==id && a.Events.Count>0);
            var clip=tae.Animations[index];
            TAE.Event Avoid(short state)=>clip.Events.Single(e=>e.Type==0 &&
                BitConverter.ToInt32(e.GetParameterBytes(false))==8 && BitConverter.ToInt16(e.GetParameterBytes(false),14)==state);
            var first=Avoid(0); var tail=Avoid(290);
            if(first.StartTime!=0 || first.EndTime!=tail.StartTime || Math.Abs(tail.StartTime*30-7)>.001f ||
               Math.Abs(tail.EndTime*30-10)>.001f)throw new InvalidDataException("Review native curved sword feint duration");
            var bytes=tail.GetParameterBytes(false).ToArray();
            BitConverter.GetBytes((short)(id==30950?25059:25060)).CopyTo(bytes,14);
            var existing=clip.Events.SingleOrDefault(e=>IsFeintGate("a28.tae",id,e));
            if(existing!=null) {
                if(existing.StartTime!=tail.StartTime || existing.EndTime!=tail.EndTime || !bytes.SequenceEqual(existing.GetParameterBytes(false)))
                    throw new InvalidDataException("Conflicting Flowing feint gate");
                continue;
            }
            additions[index]=[new TAE.Event(tail.StartTime,tail.EndTime,0,tail.Unk04,bytes,false){
                Group=new TAE.EventGroup(tail.Group.GroupType){GroupData=tail.Group.GroupData}}];
        }
        if(additions.Count>0)file.Bytes=TaeOverlay.Add(file.Bytes.ToArray(),additions);
    }
    public static void Events(EMEVD events) {
        foreach(var id in new long[]{1111,9100941}){
            var e=events.Events.Single(e=>e.ID==id);
            int at=e.Instructions.FindIndex(i=>i.Bank==2004 && i.ID==21 && i.ArgData.Length==8 &&
                BitConverter.ToInt32(i.ArgData,4)==PhantomCombo.Owner);
            if(at<0 || e.Parameters.Count!=0)throw new InvalidDataException("Missing ability cleanup");
            e.Instructions.InsertRange(at,Enumerable.Range(Owner,5).Select(effect=>
                new EMEVD.Instruction(2004,21,new object[]{20000u,effect})));
        }
    }
    public static bool IsMarker(string name,TAE.Event e)=>name=="a157.tae" && e.Type==227 && BitConverter.ToInt32(e.GetParameterBytes(false))==Flag;
    public static void ChargeWindows(BND4 bundle){
        var file=bundle.Files.Single(f=>Path.GetFileName(f.Name)=="a157.tae");
        var additions=new Dictionary<int,List<TAE.Event>>();
        var tae=TAE.Read(file.Bytes);
        foreach(var id in new long[]{30500,30510,32500,32510}){
            int i=tae.Animations.FindIndex(a=>a.ID==id);
            var clip=tae.Animations[i];
            var window=clip.Events.Single(e=>e.Type==67 && BitConverter.ToInt32(e.GetParameterBytes(false))==100280);
            var prior=clip.Events.SingleOrDefault(e=>IsMarker("a157.tae",e));
            if(prior!=null){
                if(prior.StartTime!=window.StartTime || prior.EndTime!=window.EndTime)throw new InvalidDataException("Conflicting flowing charge flag");
                continue;
            }
            var bytes=new byte[16];BitConverter.GetBytes(Flag).CopyTo(bytes,0);
            additions[i]=[new TAE.Event(window.StartTime,window.EndTime,227,0,bytes,false){Group=new TAE.EventGroup(227)}];
        }
        if(additions.Count>0)file.Bytes=TaeOverlay.Add(file.Bytes.ToArray(),additions);
    }
    public static void ChargeFile(string path){
        var bundle=BND4.Read(path);ChargeWindows(bundle);FeintWindows(bundle);bundle.Write(path);
    }
}
