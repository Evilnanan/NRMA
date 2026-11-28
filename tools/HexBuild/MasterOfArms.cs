using SoulsFormats;

static class MasterOfArms {
    public const int Owner = 95001400;
    const int NativeSkillBonus = 8350000;
    const float SkillAttackRate = 1.2f;
    static readonly string[] Categories = { "magicSubCategoryChange1", "magicSubCategoryChange2" };

    public static void Configure(PARAM.Row owner, PARAM.Row native) {
        if (Convert.ToInt32(native[Categories[0]].Value) != 112 ||
            Convert.ToInt32(native[Categories[1]].Value) != 111 ||
            Erosion.AttackRates.Any(field => Convert.ToSingle(native[field].Value) != 1.15f))
            throw new InvalidDataException("Review the native weapon skill attack bonus");
        foreach (string field in Categories) owner[field].Value = native[field].Value;
        foreach (string field in Erosion.AttackRates) owner[field].Value = SkillAttackRate;
    }

    public static void Verify(PARAM.Row owner) {
        if (owner.ID != Owner || Convert.ToInt32(owner[Categories[0]].Value) != 112 ||
            Convert.ToInt32(owner[Categories[1]].Value) != 111 ||
            Erosion.AttackRates.Any(field => Convert.ToSingle(owner[field].Value) != SkillAttackRate) ||
            Convert.ToSingle(owner["artsConsumptionRate"].Value) != 0f ||
            Convert.ToSingle(owner["magicConsumptionRate"].Value) != 1f ||
            Convert.ToSingle(owner["miracleConsumptionRate"].Value) != 1f)
            throw new InvalidDataException("Master of Arms skill bonus or FP rate differs");
    }

    public static PARAM.Row NativeBonus(PARAM effects) => effects.Rows.Single(r => r.ID == NativeSkillBonus);
}
