namespace CityOfTowers.Domain;

/// <summary>Hero roster per faction. Might heroes favour Attack/Defense; magic heroes Power/Knowledge.</summary>
public record HeroDef(string Key, string Name, string Faction, bool Might, int Attack, int Defense, int Power, int Knowledge, string[] Skills);

public static class Heroes
{
    public static readonly Dictionary<string, HeroDef> All = new()
    {
        // Castle
        ["christian"] = new("christian", "Christian", "castle", true, 2, 2, 1, 1, ["pathfinding", "leadership"]),
        ["adela"] = new("adela", "Adela", "castle", false, 1, 1, 2, 2, ["wisdom", "diplomacy"]),
        ["sorsha"] = new("sorsha", "Sorsha", "castle", true, 2, 2, 1, 1, ["leadership", "luck"]),
        ["cuthbert"] = new("cuthbert", "Cuthbert", "castle", false, 1, 1, 2, 2, ["wisdom", "earth"]),

        // Rampart
        ["ryland"] = new("ryland", "Ryland", "rampart", true, 2, 2, 1, 1, ["leadership", "pathfinding"]),
        ["clancy"] = new("clancy", "Clancy", "rampart", false, 1, 1, 2, 2, ["wisdom", "earth"]),
        ["mephala"] = new("mephala", "Mephala", "rampart", true, 2, 2, 1, 1, ["armorer", "leadership"]),
        ["elle"] = new("elle", "Elle", "rampart", false, 1, 1, 2, 2, ["wisdom", "water"]),

        // Tower
        ["tyris"] = new("tyris", "Tyris", "tower", true, 2, 2, 1, 1, ["leadership", "luck"]),
        ["solmyr"] = new("solmyr", "Solmyr", "tower", false, 1, 1, 2, 2, ["wisdom", "air"]),
        ["piquedram"] = new("piquedram", "Piquedram", "tower", true, 2, 2, 1, 1, ["artillery", "leadership"]),
        ["cyra"] = new("cyra", "Cyra", "tower", false, 1, 1, 2, 2, ["wisdom", "water"]),

        // Inferno
        ["rashka"] = new("rashka", "Rashka", "inferno", true, 2, 2, 1, 1, ["leadership", "artillery"]),
        ["pyre"] = new("pyre", "Pyre", "inferno", false, 1, 1, 2, 2, ["wisdom", "fire"]),
        ["marius"] = new("marius", "Marius", "inferno", true, 2, 2, 1, 1, ["leadership", "tactics"]),
        ["xarfax"] = new("xarfax", "Xarfax", "inferno", false, 1, 1, 2, 2, ["wisdom", "fire"]),

        // Necropolis
        ["vidomina"] = new("vidomina", "Vidomina", "necropolis", true, 2, 2, 1, 1, ["leadership", "pathfinding"]),
        ["thant"] = new("thant", "Thant", "necropolis", false, 1, 1, 2, 2, ["wisdom", "earth"]),
        ["straker"] = new("straker", "Straker", "necropolis", true, 2, 2, 1, 1, ["leadership", "armorer"]),
        ["sephinroth"] = new("sephinroth", "Sephinroth", "necropolis", false, 1, 1, 2, 2, ["wisdom", "earth"]),

        // Dungeon
        ["lorelei"] = new("lorelei", "Lorelei", "dungeon", true, 2, 2, 1, 1, ["leadership", "tactics"]),
        ["alamar"] = new("alamar", "Alamar", "dungeon", false, 1, 1, 2, 2, ["wisdom", "water"]),
        ["ajit"] = new("ajit", "Ajit", "dungeon", true, 2, 2, 1, 1, ["leadership", "luck"]),
        ["jeddite"] = new("jeddite", "Jeddite", "dungeon", false, 1, 1, 2, 2, ["wisdom", "earth"]),

        // Stronghold
        ["crag"] = new("crag", "Crag Hack", "stronghold", true, 2, 2, 1, 1, ["leadership", "artillery"]),
        ["gundula"] = new("gundula", "Gundula", "stronghold", false, 1, 1, 2, 2, ["wisdom", "fire"]),
        ["jabarkas"] = new("jabarkas", "Jabarkas", "stronghold", true, 2, 2, 1, 1, ["leadership", "pathfinding"]),
        ["saurug"] = new("saurug", "Saurug", "stronghold", false, 1, 1, 2, 2, ["wisdom", "earth"]),

        // Fortress
        ["tazar"] = new("tazar", "Tazar", "fortress", true, 2, 2, 1, 1, ["leadership", "resistance"]),
        ["alagar"] = new("alagar", "Alagar", "fortress", false, 1, 1, 2, 2, ["wisdom", "water"]),
        ["bron"] = new("bron", "Bron", "fortress", true, 2, 2, 1, 1, ["leadership", "armorer"]),
        ["mirlanda"] = new("mirlanda", "Mirlanda", "fortress", false, 1, 1, 2, 2, ["wisdom", "earth"]),

        // Conflux
        ["pasis"] = new("pasis", "Pasis", "conflux", true, 2, 2, 1, 1, ["leadership", "luck"]),
        ["luna"] = new("luna", "Luna", "conflux", false, 1, 1, 2, 2, ["wisdom", "fire"]),
        ["ignatius"] = new("ignatius", "Ignatius", "conflux", true, 2, 2, 1, 1, ["leadership", "tactics"]),
        ["ciele"] = new("ciele", "Ciele", "conflux", false, 1, 1, 2, 2, ["wisdom", "water"])
    };

    public static HeroDef[] ForFaction(string faction) =>
        All.Values.Where(h => h.Faction == faction).ToArray();
}

/// <summary>Secondary skills and their levels.</summary>
public static class Skills
{
    public static readonly string[] All =
    [
        "pathfinding", "archery", "logistics", "scouting", "diplomacy", "navigation",
        "leadership", "luck", "ballistics", "eagleEye", "estates", "wisdom",
        "mysticism", "intelligence", "sorcery", "resistance", "firstAid", "artillery",
        "learning", "tactics", "armorer", "air", "earth", "fire", "water"
    ];

    public static readonly string[] Names =
    [
        "Pathfinding", "Archery", "Logistics", "Scouting", "Diplomacy", "Navigation",
        "Leadership", "Luck", "Ballistics", "Eagle Eye", "Estates", "Wisdom",
        "Mysticism", "Intelligence", "Sorcery", "Resistance", "First Aid", "Artillery",
        "Learning", "Tactics", "Armorer", "Air Magic", "Earth Magic", "Fire Magic", "Water Magic"
    ];

    public static string Name(string key)
    {
        var i = Array.IndexOf(All, key);
        return i >= 0 ? Names[i] : key;
    }

    public static readonly string[] Levels = ["None", "Basic", "Advanced", "Expert"];
}

/// <summary>Experience thresholds (HoMM3 table).</summary>
public static class Experience
{
    private static readonly int[] Thresholds =
    [
        0, 1000, 2000, 3200, 4600, 6200, 8000, 10000, 12200, 14700,
        17500, 20600, 24320, 28784, 34000, 40000, 47000, 55000, 64000, 74000,
        85000, 97000, 110000, 124000, 139000, 155000, 172000, 190000, 209000, 229000,
        250000, 272000, 295000, 319000, 344000, 370000, 397000, 425000, 454000, 484000,
        515000, 547000, 580000, 614000, 649000, 685000, 722000, 760000, 799000, 839000,
        880000, 922000, 965000, 1009000, 1054000, 1100000, 1147000, 1195000, 1244000, 1294000,
        1345000, 1397000, 1450000, 1504000, 1559000, 1615000, 1672000, 1730000, 1789000, 1849000,
        1910000, 1972000, 2035000, 2099000, 2164000
    ];

    public static int LevelFor(int xp)
    {
        var lvl = 1;
        for (var i = 1; i < Thresholds.Length; i++)
        {
            if (xp >= Thresholds[i]) lvl = i + 1;
            else break;
        }
        return lvl;
    }

    public static int NextThreshold(int level) =>
        level < Thresholds.Length ? Thresholds[level] : Thresholds[^1];
}

/// <summary>Terrain types and their movement costs.</summary>
public static class Terrain
{
    public static readonly string[] Types =
        ["dirt", "sand", "grass", "snow", "swamp", "rough", "subterranean", "lava", "water", "rock"];

    public static int MoveCost(string t, int pathfinding = 0)
    {
        var cost = t switch
        {
            "rough" => 125,
            "snow" or "sand" => 150,
            "swamp" => 175,
            "water" or "rock" => 0,
            _ => 100
        };
        return cost == 0 ? 0 : Math.Max(100, cost - 25 * Math.Clamp(pathfinding, 0, 3));
    }

    public static bool Passable(string t) => t != "water" && t != "rock";
}