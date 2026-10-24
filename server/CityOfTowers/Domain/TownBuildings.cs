namespace CityOfTowers.Domain;

/// <summary>
/// Town building definitions. The tree is shared across factions; the dwelling
/// names and creature tiers differ per faction.
/// </summary>
public record TownBuildingDef(
    string Key,
    string Name,
    string Category,
    int Gold,
    int Wood,
    int Ore,
    int Mercury,
    int Sulfur,
    int Crystal,
    int Gems,
    string[] Requires,
    string Description,
    int Tier = 0);

public static class TownBuildings
{
    public static readonly Dictionary<string, TownBuildingDef> All = new();

    private static void Add(TownBuildingDef b) => All[b.Key] = b;

    static TownBuildings()
    {
        // ---- halls
        Add(new("villageHall", "Village Hall", "Hall", 0, 0, 0, 0, 0, 0, 0, [], "Provides 500 gold per day."));
        Add(new("townHall", "Town Hall", "Hall", 500, 5, 0, 0, 0, 0, 0, ["villageHall"], "Provides 1000 gold per day."));
        Add(new("cityHall", "City Hall", "Hall", 1000, 5, 5, 0, 0, 0, 0, ["townHall"], "Provides 2000 gold per day."));
        Add(new("capitol", "Capitol", "Hall", 5000, 10, 10, 0, 0, 0, 0, ["cityHall"], "Provides 4000 gold per day."));

        // ---- fortifications
        Add(new("fort", "Fort", "Fortification", 5000, 20, 20, 0, 0, 0, 0, [], "Adds fortifications and a moat."));
        Add(new("citadel", "Citadel", "Fortification", 2500, 0, 5, 0, 0, 0, 0, ["fort"], "Adds a tower; +50% creature growth."));
        Add(new("castle", "Castle", "Fortification", 5000, 10, 10, 0, 0, 0, 0, ["citadel"], "Adds two towers; +100% creature growth."));

        // ---- dwellings (tier 1..7)
        var dwellingCosts = new (int gold, int wood, int ore, int crystal, int gems, int sulfur)[]
        {
            (500, 5, 0, 0, 0, 0),
            (1000, 5, 0, 0, 0, 0),
            (1000, 5, 5, 0, 0, 0),
            (2000, 5, 5, 0, 0, 0),
            (2000, 5, 5, 5, 0, 0),
            (3000, 5, 5, 0, 5, 0),
            (5000, 10, 10, 0, 0, 10)
        };
        for (var t = 1; t <= 7; t++)
        {
            var c = dwellingCosts[t - 1];
            Add(new("dwelling" + t, "Dwelling " + t, "Dwelling",
                c.gold, c.wood, c.ore, 0, c.sulfur, c.crystal, c.gems,
                t > 1 ? ["dwelling" + (t - 1)] : [],
                "Allows recruiting tier " + t + " creatures.", t));
            Add(new("upg" + t, "Upgraded Dwelling " + t, "Dwelling",
                1000 + t * 500, 0, 5, 0, 0, 0, 0,
                ["dwelling" + t],
                "Upgrades tier " + t + " creatures to their improved form.", t));
        }

        // ---- mage guild
        Add(new("mageGuild1", "Mage Guild Level 1", "Mage Guild", 2000, 5, 5, 0, 0, 0, 0, [], "Teaches level 1 spells."));
        Add(new("mageGuild2", "Mage Guild Level 2", "Mage Guild", 1000, 5, 5, 0, 0, 4, 0, ["mageGuild1"], "Teaches level 2 spells."));
        Add(new("mageGuild3", "Mage Guild Level 3", "Mage Guild", 1000, 5, 5, 0, 0, 0, 6, ["mageGuild2"], "Teaches level 3 spells."));
        Add(new("mageGuild4", "Mage Guild Level 4", "Mage Guild", 1000, 5, 5, 0, 6, 0, 0, ["mageGuild3"], "Teaches level 4 spells."));
        Add(new("mageGuild5", "Mage Guild Level 5", "Mage Guild", 1000, 5, 5, 6, 0, 0, 0, ["mageGuild4"], "Teaches level 5 spells."));

        // ---- civic
        Add(new("marketplace", "Marketplace", "Civic", 500, 5, 0, 0, 0, 0, 0, [], "Trade resources."));
        Add(new("tavern", "Tavern", "Civic", 500, 5, 0, 0, 0, 0, 0, [], "Hire additional heroes."));
        Add(new("blacksmith", "Blacksmith", "Civic", 1000, 0, 5, 0, 0, 0, 0, [], "Sells war machines."));
        Add(new("resourceSilo", "Resource Silo", "Civic", 5000, 5, 5, 0, 0, 0, 0, ["marketplace"], "+1 of each resource per day."));

        // ---- grail
        Add(new("grail", "Grail Structure", "Grail", 0, 0, 0, 0, 0, 0, 0, [], "Requires the Grail. +5000 gold/day, +2 growth."));
    }

    /// <summary>Gold income per day for a town with the given buildings.</summary>
    public static int GoldIncome(IEnumerable<string> built)
    {
        var set = built.ToHashSet();
        var grail = set.Contains("grail") ? 5000 : 0;
        if (set.Contains("capitol")) return 4000 + grail;
        if (set.Contains("cityHall")) return 2000 + grail;
        if (set.Contains("townHall")) return 1000 + grail;
        return 500 + grail;
    }

    /// <summary>Growth multiplier from fortifications.</summary>
    public static double GrowthMultiplier(IEnumerable<string> built)
    {
        var set = built.ToHashSet();
        if (set.Contains("castle")) return 2.0;
        if (set.Contains("citadel")) return 1.5;
        return 1.0;
    }

    /// <summary>Whether a dwelling for the given tier is built.</summary>
    public static bool HasDwelling(IEnumerable<string> built, int tier) =>
        built.Contains("dwelling" + tier);

    /// <summary>Whether the upgraded dwelling for the given tier is built.</summary>
    public static bool HasUpgrade(IEnumerable<string> built, int tier) =>
        built.Contains("upg" + tier);
}