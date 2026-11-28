using SoulsFormats;
using System.Diagnostics;
using System.Text.RegularExpressions;

static class BattleMage {
    public const int Owner = 95007100, Speed = Owner + 1, NativeSpeed = 8330401;
    const int AnimationFamilyStride = 1_000_000;
    static readonly KeyValuePair<int, int>[] SharedExtensions = new Dictionary<int, int> {
        [4040] = 107, [4050] = 107, [4090] = 113, [4210] = 124, [4220] = 107,
        [4410] = 14, [4500] = 113, [4720] = 120, [4721] = 120,
        [4800] = 117, [4820] = 117, [5000] = 103, [5001] = 103,
        [6001] = 25, [6010] = 17, [6020] = 17, [6100] = 17, [6110] = 19, [6210] = 17,
        [6230] = 22, [6240] = 99, [6270] = 105, [6400] = 28, [6410] = 29,
        [6820] = 40, [6900] = 36, [7200] = 74, [7210] = 26, [7320] = 89, [7500] = 70
    }.ToArray();
    static readonly KeyValuePair<int, int>[] MmvExtensions = new Dictionary<int, int> {
        [2004310] = 163, [2004500] = 107, [2004510] = 107, [2004700] = 120,
        [2004710] = 120, [2005000] = 143, [2007410] = 120,
        [6908] = 65, [2006710] = 149, [2006900] = 139, [2007210] = 144,
        [2007700] = 148, [2007710] = 154, [2007730] = 147, [2007800] = 153
    }.ToArray();

    public static Dictionary<int, int> Extensions(bool mmv = false) =>
        (mmv ? SharedExtensions.Concat(MmvExtensions) : SharedExtensions).ToDictionary();
    static IEnumerable<int> ExtensionFamilies(bool mmv) =>
        Extensions(mmv).Values.Distinct().Order();
    static int FastRelease(int family) => family is 19 or 26 or 65 or 74 ? 45010 : 45011;
    // These fast releases omit both early native movement flags in each hand.
    // Keep this in sync with battle_mage.rs::full_body_chain.
    static bool FullBody(int family) => family is 14 or 65 or 74 or 120 or 144 or 147;

    static string Name(BinderFile file) => Path.GetFileName(file.Name);
    static void Check(bool valid, string error) { if (!valid) throw new InvalidDataException(error); }
    static bool AddedFamily(string file) =>
        file.Length == 8 && file[0] == 'a' && file.EndsWith(".tae") &&
        int.TryParse(file.AsSpan(1, 3), out int family) &&
        (SharedExtensions.Any(p => p.Value == family - 400) ||
            MmvExtensions.Any(p => p.Value == family - 400));
    public static bool AddedAnimation(string file, long id) =>
        AddedFamily(file) && id is 45020 or 45030 or 45120 or 45130;
    public static bool AddedWindow(string file, long id, TAE.Event e) =>
        AddedFamily(file) && id is 45010 or 45011 or 45110 or 45111 && IsWindow(e);

    static bool IsWindow(TAE.Event e) => e.Type == 66 &&
        BitConverter.ToInt32(e.GetParameterBytes(false)) is 100600 or 100601 or 100605 or 100606;

    static void AddMarker(TAE.Animation animation, TAE.Event template, int marker, float start, float end) {
        var bytes = template.GetParameterBytes(false).ToArray();
        BitConverter.GetBytes(marker).CopyTo(bytes, 0);
        var group = new TAE.EventGroup(66) { GroupData = template.Group!.GroupData };
        animation.EventGroups.Add(group);
        animation.Events.Add(new TAE.Event(start, end, 66, template.Unk04, bytes, false) { Group = group });
    }

    static float Recovery(TAE.Animation animation, float lastShot) => animation.Events.Where(e => e.Type == 0 &&
        BitConverter.ToInt32(e.GetParameterBytes(false)) == 29 && e.StartTime >= lastShot && e.EndTime > e.StartTime)
        .Min(e => e.StartTime);

    static void AddWindow(TAE.Animation animation, TAE.Event template, int marker, bool retimeGoldenArcs = false) {
        // The next cast request can already be consumed by the time the
        // recovery flag appears. Publish after the last release event and
        // keep the window through the native recovery.
        var releases = animation.Events.Where(e => e.Type == 64).ToArray();
        float lastShot = releases.Max(e => e.EndTime);
        var existing = animation.Events.SingleOrDefault(IsWindow);
        if (retimeGoldenArcs) {
            // The MMV held cast uses the right-hand marker in both animations,
            // opening on the first frame of the final wave.
            Check(existing is not null && BitConverter.ToInt32(existing.GetParameterBytes(false)) == 100600 &&
                existing.StartTime == releases.Max(e => e.StartTime) && existing.EndTime > lastShot,
                $"Golden Arcs native combo window changed in {animation.ID}");
            var bytes = existing!.GetParameterBytes(false).ToArray();
            BitConverter.GetBytes(marker).CopyTo(bytes, 0);
            existing.SetParameterBytes(false, bytes);
            existing.StartTime = lastShot;
            return;
        }
        Check(existing is null, $"Existing combo window in {animation.ID}");
        var recovery = animation.Events.Where(e => e.Type == 0 &&
            BitConverter.ToInt32(e.GetParameterBytes(false)) == 29 &&
            e.StartTime >= lastShot && e.EndTime > e.StartTime)
            .OrderBy(e => e.StartTime).ThenByDescending(e => e.EndTime).FirstOrDefault()
            ?? throw new InvalidDataException($"No native recovery after spell fire in {animation.ID}");
        AddMarker(animation, template, marker, lastShot, Math.Min(recovery.StartTime + 0.5f, recovery.EndTime));
    }

    static void MarkFullBody(TAE.Animation animation) {
        var window = animation.Events.Single(IsWindow);
        var end = Recovery(animation, window.StartTime);
        // Native MoveStart respects this marker until the existing recovery.
        bool Marker(TAE.Event e) => e.Type == 66 && BitConverter.ToInt32(e.GetParameterBytes(false)) == 100200;
        var existing = animation.Events.SingleOrDefault(Marker);
        if (existing is null) AddMarker(animation, window, 100200, 0, end);
        else Check(existing.StartTime == 0 && existing.EndTime == end, $"Invalid movement lock in {animation.ID}");
    }

    public static void ExtendTimelines(BND4 binder, bool mmv) {
        var reference = TAE.Read(binder.Files.Single(f => Name(f) == "a511.tae").Bytes);
        var template = reference.Animations.Single(a => a.ID == 45020).Events.Single(IsWindow);
        foreach (int motion in ExtensionFamilies(mmv)) {
            int family = 400 + motion;
            var file = binder.Files.Single(f => Name(f) == $"a{family}.tae");
            var timeline = TAE.Read(file.Bytes);
            foreach (int hand in new[] { 0, 100 }) {
                var held = timeline.Animations.Single(a => a.ID == 45010 + hand);
                var fast = timeline.Animations.Single(a => a.ID == FastRelease(motion) + hand);
                var header = (TAE.Animation.AnimMiniHeader.Standard)fast.MiniHeader;
                int import = header.ImportsHKX ? header.ImportHKXSourceAnimID :
                    checked(family * AnimationFamilyStride + (int)fast.ID);
                Check(import / AnimationFamilyStride is >= 400 and < 600 &&
                    import % AnimationFamilyStride is 45010 or 45011 or 45110 or 45111,
                    $"Unexpected release motion import for {family}/{fast.ID}: {import}");
                int marker = hand == 0 ? 100600 : 100605;
                foreach (int stage in new[] { 2, 3 }) {
                    int id = 45000 + stage * 10 + hand;
                    Check(timeline.Animations.All(a => a.ID != id), $"Spell continuation collision in {family}/{id}");
                    var followUp = new TAE.Animation(id, new TAE.Animation.AnimMiniHeader.Standard {
                        ImportsHKX = true, ImportHKXSourceAnimID = import
                    }, fast.AnimFileName);
                    followUp.EventGroups.AddRange(fast.EventGroups);
                    followUp.Events.AddRange(fast.Events);
                    AddWindow(followUp, template, stage == 2 ? marker + 1 : marker);
                    if (FullBody(motion)) MarkFullBody(followUp);
                    timeline.Animations.Add(followUp);
                }
                AddWindow(held, template, marker, retimeGoldenArcs: motion == 148);
                if (fast != held) AddWindow(fast, template, marker);
            }
            timeline.Animations.Sort((a, b) => a.ID.CompareTo(b.ID));
            file.Bytes = timeline.Write();
            var reopened = TAE.Read(file.Bytes);
            Check(timeline.Animations.Count == reopened.Animations.Count,
                $"Spell timeline count roundtrip failed in {family}");
            foreach (var (expected, actual) in timeline.Animations.Zip(reopened.Animations)) {
                Check(actual.ID == expected.ID && actual.Events.Count == expected.Events.Count,
                    $"Spell timeline roundtrip failed in {family}/{expected.ID}");
                foreach (var (a, b) in expected.Events.Zip(actual.Events)) {
                    var bytes = a.GetParameterBytes(false);
                    var readback = b.GetParameterBytes(false);
                    // SoulsFormats can adjust zero alignment on the first event.
                    Check(a.Type == b.Type && a.StartTime == b.StartTime && a.EndTime == b.EndTime &&
                        a.Group?.GroupType == b.Group?.GroupType && bytes.Take(readback.Length)
                            .SequenceEqual(readback.Take(bytes.Length)) &&
                        bytes.Skip(readback.Length).Concat(readback.Skip(bytes.Length)).All(x => x == 0),
                        $"Spell event roundtrip failed in {family}/{expected.ID}");
                }
                if (AddedAnimation(Name(file), actual.ID)) {
                    var from = (TAE.Animation.AnimMiniHeader.Standard)expected.MiniHeader;
                    var to = (TAE.Animation.AnimMiniHeader.Standard)actual.MiniHeader;
                    Check(to.ImportsHKX && to.ImportHKXSourceAnimID == from.ImportHKXSourceAnimID &&
                        actual.Events.Count(IsWindow) == 1, $"Spell motion alias roundtrip failed in {family}/{actual.ID}");
                }
            }
        }
        Console.WriteLine($"PASS: two continuation stages per hand added to {ExtensionFamilies(mmv).Count()} spell families");
    }

    public static void StageTimelines(string source, string output, bool mmv) {
        var binder = BND4.Read(source);
        ExtendTimelines(binder, mmv);
        WriteBinder(binder, output);
    }

    public static void StageFullBodyMarkers(string source, string output, bool mmv) {
        var binder = BND4.Read(source);
        foreach (int family in ExtensionFamilies(mmv).Where(FullBody)) {
            var file = binder.Files.Single(f => Name(f) == $"a{400 + family}.tae");
            var timeline = TAE.Read(file.Bytes);
            foreach (int id in new[] { 45020, 45030, 45120, 45130 })
                MarkFullBody(timeline.Animations.Single(a => a.ID == id));
            file.Bytes = timeline.Write();
        }
        WriteBinder(binder, output);
    }

    public static void StageBehavior(string root, string source, string output, string work, bool mmv) {
        var binder = BND4.Read(source);
        var graph = binder.Files.Single(f => f.ID == 7000500 && Name(f) == "c0000.hkx");
        Directory.CreateDirectory(work);
        string input = Path.Combine(work, "before.hkx"), patched = Path.Combine(work, "after.hkx");
        File.WriteAllBytes(input, graph.Bytes.ToArray());
        var start = new ProcessStartInfo("python") { UseShellExecute = false, CreateNoWindow = true };
        foreach (string arg in new[] { Path.Combine(root, "tools/build_battle_mage_behavior.py"), input, patched })
            start.ArgumentList.Add(arg);
        foreach (int family in ExtensionFamilies(mmv))
            start.ArgumentList.Add(FormattableString.Invariant($"{400 + family}:{FastRelease(family)}"));
        using var process = Process.Start(start) ?? throw new IOException("Could not start spell behavior builder");
        process.WaitForExit();
        Check(process.ExitCode == 0, "Battle Mage graph patch failed");
        graph.Bytes = File.ReadAllBytes(patched);
        WriteBinder(binder, output);
    }

    static void WriteBinder(BND4 binder, string output) {
        Directory.CreateDirectory(Path.GetDirectoryName(output)!);
        binder.Write(output);
        var actual = BND4.Read(output);
        Check(actual.Version == binder.Version && actual.Compression == binder.Compression &&
            actual.Files.Count == binder.Files.Count && actual.Files.Zip(binder.Files).All(p =>
                p.First.ID == p.Second.ID && p.First.Name == p.Second.Name && p.First.Flags == p.Second.Flags &&
                p.First.Bytes.Span.SequenceEqual(p.Second.Bytes.Span)), $"Binder roundtrip failed: {output}");
    }

    public static void SpeedEffect(PARAM.Row row) {
        if (Convert.ToInt32(row["dexterityCancelSystemOnlyAddDexterity"].Value) != 60 ||
            Convert.ToInt32(row["saveCategory"].Value) != -1 ||
            Convert.ToInt32(row["iconId"].Value) != -1 ||
            Convert.ToInt32(row["spEffectTextId_1"].Value) != -1)
            throw new InvalidDataException("Review native spellcasting speed +2");
        // Only the private effect replaces itself. Equipped copies still participate natively.
        row["spCategory"].Value = Convert.ChangeType(20, row["spCategory"].Value.GetType());
    }

    public static void StageSpeed(string input, string output, Func<string, Memory<byte>, PARAM> parse) {
        var binder = SFUtil.DecryptNightreignRegulation(input);
        var file = binder.Files.Single(f => Path.GetFileName(f.Name) == "SpEffectParam.param");
        var effects = parse("SpEffectParam", file.Bytes);
        var row = new PARAM.Row(effects.Rows.Single(r => r.ID == NativeSpeed)) { ID = Speed };
        SpeedEffect(row);
        var existing = effects.Rows.SingleOrDefault(r => r.ID == Speed);
        if (existing != null && !CageAI.Same(existing, row))
            throw new InvalidDataException("Battle Mage hidden effect ID is occupied");
        if (existing == null) effects.Rows.Add(row);
        effects.Rows = effects.Rows.OrderBy(r => r.ID).ToList();
        file.Bytes = effects.Write();
        SFUtil.EncryptNightreignRegulation(output, binder, binder.Compression);
        var reopened = SFUtil.DecryptNightreignRegulation(output);
        if (reopened.Version != binder.Version || reopened.Compression != binder.Compression ||
            reopened.Files.Count != binder.Files.Count ||
            !binder.Files.Zip(reopened.Files).All(p => p.First.ID == p.Second.ID &&
                p.First.Name == p.Second.Name && p.First.Flags == p.Second.Flags &&
                p.First.Bytes.Span.SequenceEqual(p.Second.Bytes.Span)))
            throw new InvalidDataException("Battle Mage parameter roundtrip failed");
        Console.WriteLine("PASS: hidden native casting speed +2 cloned; all other parameter tables unchanged");
    }

    public static void Events(EMEVD events) {
        foreach (int id in new[] { 1111, 9100941 }) {
            var reset = events.Events.Single(e => e.ID == id);
            int at = reset.Instructions.FindIndex(i => i.Bank == 2004 && i.ID == 21 &&
                i.ArgData.Length == 8 && BitConverter.ToInt32(i.ArgData, 4) == FlowingSwordsman.Owner);
            if (at < 0 || reset.Parameters.Count != 0) throw new InvalidDataException("Missing battle mage cleanup anchor");
            reset.Instructions.Insert(at, new EMEVD.Instruction(2004, 21, new object[] { 20000u, Owner }));
        }
    }

    public static SortedDictionary<int, int> ComboMotions(string path) {
        var motions = new SortedDictionary<int, int>();
        foreach (var file in BND4.Read(path).Files) {
            var name = Regex.Match(file.Name, @"a(\d{3})\.tae$", RegexOptions.IgnoreCase);
            if (!name.Success || int.Parse(name.Groups[1].Value) - 400 is not (>= 0 and <= byte.MaxValue)) continue;
            int motion = int.Parse(name.Groups[1].Value) - 400;
            // Keep the thorn families out of Battle Mage only, including MMV's Impenetrable Thorns.
            if (motion is 81 or 82 or 156) continue;
            var casts = TAE.Read(file.Bytes).Animations;
            // Every offered stage must open the next native input window.
            // A valid release event alone only makes a clip a possible finisher.
            bool CanContinue(int stage) => casts.Any(a => a.ID % 100000 == stage && HasComboWindow(a));
            if (!CanContinue(45010) || !CanContinue(45020)) continue;
            motions.Add(motion, CanContinue(45030) ? 3 : 2);
        }
        return motions;
    }

    static bool HasComboWindow(TAE.Animation animation) => animation.Events.Any(IsWindow);

}
