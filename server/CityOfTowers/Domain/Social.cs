namespace CityOfTowers.Domain;

/// <summary>
/// A peaceful encounter with a wandering monster stack. There is no combat:
/// the player may observe (gain experience), recruit (pay gold), or pass.
/// </summary>
public record EncounterOption(string Kind, string Label, string Description, int Cost, int Reward);

public static class Encounters
{
    /// <summary>Experience gained for observing a stack.</summary>
    public static int ObserveReward(int aiValue, int count) => Math.Max(10, aiValue * count / 20);

    /// <summary>Gold cost to recruit a stack.</summary>
    public static int RecruitCost(int gold, int count) => gold * count;

    /// <summary>Whether the hero's army has room for another stack.</summary>
    public static bool HasRoom(string armyJson)
    {
        var slots = Services.GameService.ParseArmy(armyJson);
        return slots.Any(s => s.Creature == null);
    }
}

/// <summary>
/// A shared project: a monument any player may contribute to. When complete it
/// grants a benefit to every contributor. This replaces alliance warfare.
/// </summary>
public class SharedProject
{
    public Guid Id { get; set; } = Guid.NewGuid();
    public Guid GameId { get; set; }
    public string Name { get; set; } = "";
    public string Description { get; set; } = "";
    public int RequiredGold { get; set; }
    public int RequiredWood { get; set; }
    public int RequiredOre { get; set; }
    public int ContributedGold { get; set; }
    public int ContributedWood { get; set; }
    public int ContributedOre { get; set; }
    public bool Completed { get; set; }
    /// <summary>JSON array of player ids who contributed.</summary>
    public string ContributorsJson { get; set; } = "[]";
}

/// <summary>
/// A map annotation: a marker one player places for others to see.
/// </summary>
public class MapAnnotation
{
    public Guid Id { get; set; } = Guid.NewGuid();
    public Guid GameId { get; set; }
    public Guid AuthorPlayerId { get; set; }
    public int X { get; set; }
    public int Y { get; set; }
    public string Text { get; set; } = "";
    public DateTime CreatedAt { get; set; } = DateTime.UtcNow;
}

/// <summary>
/// A message sent from one hero to another when they meet.
/// </summary>
public class HeroMessage
{
    public Guid Id { get; set; } = Guid.NewGuid();
    public Guid GameId { get; set; }
    public Guid FromPlayerId { get; set; }
    public Guid ToPlayerId { get; set; }
    public string Body { get; set; } = "";
    public DateTime CreatedAt { get; set; } = DateTime.UtcNow;
}

/// <summary>
/// A voluntary resource trade offered by one player to another.
/// </summary>
public class TradeOffer
{
    public Guid Id { get; set; } = Guid.NewGuid();
    public Guid GameId { get; set; }
    public Guid FromPlayerId { get; set; }
    public Guid ToPlayerId { get; set; }
    public string GiveResource { get; set; } = "";
    public int GiveAmount { get; set; }
    public string WantResource { get; set; } = "";
    public int WantAmount { get; set; }
    /// <summary>pending | accepted | declined</summary>
    public string Status { get; set; } = "pending";
    public DateTime CreatedAt { get; set; } = DateTime.UtcNow;
}