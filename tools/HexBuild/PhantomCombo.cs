using SoulsFormats;

static class PhantomCombo
{
    public const int Owner = 95005800;
    public const int Trigger = Owner + 1, Reset = Owner + 2;

    static void Set(PARAM.Row row, string field, object value) =>
        row[field].Value = Convert.ChangeType(value, row[field].Value.GetType());

    public static void ConfigureOwner(PARAM.Row owner, PARAM.Row native)
    {
        foreach (var field in new[] { "stateInfo", "motionInterval", "accumuVal", "accumuOverVal",
                                      "accumuUnderVal", "accumuUnderFireId", "dedupeAccumuTrigger" })
            Set(owner, field, native[field].Value);
        Set(owner, "accumuOverFireId", Trigger);
        if (Convert.ToInt32(owner["stateInfo"].Value) != 304 || Convert.ToInt32(owner["accumuOverVal"].Value) != 32)
            throw new InvalidDataException("Native successive attack slot 2 changed");
    }

    public static void Mechanics(Func<string,int,int,PARAM.Row> clone)
    {
        var trigger = clone("SpEffectParam", 6903, Trigger);
        Set(trigger, "stateInfo", 0);
        Set(trigger, "accumuVal", 0);
        var reset = clone("SpEffectParam", 6903, Reset);
        Set(reset, "stateInfo", 304);
        Set(reset, "accumuVal", -999);
        Set(reset, "isDisableNetSync", 1);
        Set(trigger, "isDisableNetSync", 1);
    }

    public static void Events(EMEVD events)
    {
        foreach (var id in new long[] { 1111, 9100941 })
        {
            var e = events.Events.Single(e => e.ID == id);
            int at = e.Instructions.FindIndex(i => i.Bank == 2004 && i.ID == 21 &&
                i.ArgData.Length >= 8 && BitConverter.ToInt32(i.ArgData, 4) == ColdJuice.Owner);
            if (at < 0 || e.Parameters.Count > 0) throw new InvalidDataException("Missing guarded ability cleanup");
            e.Instructions.InsertRange(at, Enumerable.Range(Owner,3).Select(effect =>
                new EMEVD.Instruction(2004, 21, new object[] { 20000u, effect })));
        }
    }
}
