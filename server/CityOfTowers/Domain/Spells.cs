namespace CityOfTowers.Domain;

/// <summary>
/// Spell catalogue. Levels 1-5 match the Mage Guild tiers.
/// </summary>
public record SpellDef(string Key, string Name, int Level, string School, int Cost, string Description);

public static class Spells
{
    public static readonly Dictionary<string, SpellDef> All = new()
    {
        // ---- level 1
        ["bless"] = new("bless", "Bless", 1, "water", 5, "Raises the army's attack for the rest of the day."),
        ["curse"] = new("curse", "Curse", 1, "fire", 5, "Lowers an enemy stack's attack."),
        ["haste"] = new("haste", "Haste", 1, "air", 5, "Increases the army's speed."),
        ["slow"] = new("slow", "Slow", 1, "earth", 5, "Reduces an enemy stack's speed."),
        ["magicArrow"] = new("magicArrow", "Magic Arrow", 1, "all", 5, "Deals direct damage to one stack."),

        // ---- level 2
        ["blessing"] = new("blessing", "Blessing", 2, "water", 8, "Stronger Bless; affects every stack."),
        ["fireball"] = new("fireball", "Fireball", 2, "fire", 8, "Damages a target and its neighbours."),
        ["lightning"] = new("lightning", "Lightning Bolt", 2, "air", 8, "Heavy damage to a single stack."),
        ["stoneSkin"] = new("stoneSkin", "Stone Skin", 2, "earth", 8, "Raises the army's defense."),
        ["visions"] = new("visions", "Visions", 2, "all", 8, "Reveals the strength of a wandering stack."),

        // ---- level 3
        ["prayer"] = new("prayer", "Prayer", 3, "water", 12, "Raises attack, defense and speed."),
        ["inferno"] = new("inferno", "Inferno", 3, "fire", 12, "Damages every enemy stack."),
        ["chainLightning"] = new("chainLightning", "Chain Lightning", 3, "air", 12, "Damage that jumps between stacks."),
        ["animateDead"] = new("animateDead", "Animate Dead", 3, "earth", 12, "Restores fallen creatures to a stack."),
        ["teleport"] = new("teleport", "Teleport", 3, "all", 12, "Moves a stack anywhere on the field."),

        // ---- level 4
        ["townPortal"] = new("townPortal", "Town Portal", 4, "water", 16, "Returns the hero to the nearest town."),
        ["berserk"] = new("berserk", "Berserk", 4, "fire", 16, "Makes an enemy stack attack its neighbours."),
        ["fly"] = new("fly", "Fly", 4, "air", 16, "Lets the hero cross impassable terrain."),
        ["implosion"] = new("implosion", "Implosion", 4, "earth", 16, "Massive damage to a single stack."),
        ["waterWalk"] = new("waterWalk", "Water Walk", 4, "all", 16, "Lets the hero cross water."),

        // ---- level 5
        ["dimensionDoor"] = new("dimensionDoor", "Dimension Door", 5, "water", 20, "Teleports the hero across the map."),
        ["armageddon"] = new("armageddon", "Armageddon", 5, "fire", 20, "Damages every stack on the field."),
        ["fly5"] = new("fly5", "Fly", 5, "air", 20, "Greater flight; longer range."),
        ["resurrection"] = new("resurrection", "Resurrection", 5, "earth", 20, "Returns a destroyed stack to the field."),
        ["sacrifice"] = new("sacrifice", "Sacrifice", 5, "all", 20, "Trades one stack's health for another's."),
    };

    public static SpellDef[] OfLevel(int level) =>
        All.Values.Where(s => s.Level == level).OrderBy(s => s.Key, StringComparer.Ordinal).ToArray();

    /// <summary>
    /// Spells a newly built guild level adds, as in HoMM3: 5 level-1 spells, 4 level-2,
    /// 3 level-3, 2 level-4 and 1 level-5, chosen at random from that level.
    /// </summary>
    public static string[] ForGuildLevel(int level, Random rng)
    {
        if (level is < 1 or > 5) return [];
        return OfLevel(level).OrderBy(_ => rng.Next()).Take(6 - level).Select(s => s.Key).ToArray();
    }

    /// <summary>Highest spell level a hero can learn: 2 without Wisdom, then 3/4/5 for Basic/Advanced/Expert.</summary>
    public static int MaxLearnable(int wisdom) => 2 + Math.Clamp(wisdom, 0, 3);
}