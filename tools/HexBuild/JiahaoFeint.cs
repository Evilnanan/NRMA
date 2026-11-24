using SoulsFormats;

// The Special1 selector also contains unrelated attacks. A native fencing feint
// has both the evasive movement effect and the two-part avoid window.
static class JiahaoFeint {
    const int FeintMovement = 430, BackStart = 6999701;
    const short ExtendedAvoid = 290;

    static bool Candidate(long id) => id is 30950 or 32950 or 35950 or 30100 or 32100 or 35100;
    static bool Effect(TAE.Event e, int id) => e.Type == 67 && BitConverter.ToInt32(e.GetParameterBytes(false)) == id;
    static bool Avoid(TAE.Event e, short state) {
        var p = e.GetParameterBytes(false);
        return e.Type == 0 && p.Length >= 16 && BitConverter.ToInt32(p) == 8 && BitConverter.ToInt16(p, 14) == state;
    }

    // Regenerate these from each variant's own action when merging MMV.
    public static bool IsExtra(long clip, TAE.Event e) => Candidate(clip) &&
        (Avoid(e, JiahaoFollowup.OwnerState) || Effect(e, BackStart));

    static TAE.Event Copy(TAE.Event source, float start, float end, byte[] data) =>
        new(start, end, source.Type, source.Unk04, data, false) {
            Group = new TAE.EventGroup(source.Group.GroupType) { GroupData = source.Group.GroupData }
        };

    public static int Apply(BND4 bundle) {
        int added = 0, clips = 0;
        foreach (var file in bundle.Files.Where(f => f.Name.EndsWith(".tae"))) {
            var tae = TAE.Read(file.Bytes);
            var additions = new Dictionary<int, List<TAE.Event>>();
            foreach (var (clip, idx) in tae.Animations.Select((a, i) => (a, i))) {
                if (!Candidate(clip.ID)) continue;
                var movement = clip.Events.SingleOrDefault(e => Effect(e, FeintMovement) && e.StartTime == 0);
                if (movement == null) continue;
                var early = clip.Events.Single(e => Avoid(e, 0) && e.StartTime == 0);
                var extended = clip.Events.Single(e => Avoid(e, ExtendedAvoid) && e.StartTime == early.EndTime);
                if (extended.EndTime != movement.EndTime || extended.EndTime <= extended.StartTime)
                    throw new InvalidDataException($"Unexpected native feint window: {file.Name}/{clip.ID}");
                clips++;
                var iframe = extended.GetParameterBytes(false).ToArray();
                BitConverter.GetBytes(JiahaoFollowup.OwnerState).CopyTo(iframe, 14);
                var pulse = movement.GetParameterBytes(false).ToArray();
                BitConverter.GetBytes(BackStart).CopyTo(pulse, 0);
                foreach (var e in new[] { Copy(extended, extended.StartTime, extended.EndTime, iframe),
                                          Copy(movement, 0, 1f/30, pulse) }) {
                    var existing = clip.Events.SingleOrDefault(x => x.Type == e.Type && IsExtra(clip.ID, x));
                    if (existing != null) {
                        if (existing.StartTime != e.StartTime || existing.EndTime != e.EndTime ||
                            !existing.GetParameterBytes(false).SequenceEqual(e.GetParameterBytes(false)))
                            throw new InvalidDataException($"Mismatched Jiahao feint event: {file.Name}/{clip.ID}");
                        continue;
                    }
                    if (!additions.ContainsKey(idx)) additions[idx] = [];
                    additions[idx].Add(e);
                    added++;
                }
            }
            if (additions.Count > 0) file.Bytes = TaeOverlay.Add(file.Bytes.ToArray(), additions);
        }
        if (clips == 0) throw new InvalidDataException("No native fencing feints found");
        Console.WriteLine($"Jiahao: {clips} feint clips validated, {added} events added.");
        return added;
    }
}
