namespace CityOfTowers.Domain;

/// <summary>
/// The three fixed players. There is no authentication: a client identifies
/// itself with the X-Player-Id header, and these are the only valid values.
/// </summary>
public record SeededPlayer(string Id, string Name, string Faction, string CityName, string HeroKey);

public static class Seeded
{
    public static readonly SeededPlayer[] Players =
    [
        new("alice", "Alice", "castle", "Alice's Keep", "christian"),
        new("bob", "Bob", "rampart", "Bob's Grove", "ryland"),
        new("carol", "Carol", "necropolis", "Carol's Crypt", "vidomina")
    ];

    public static SeededPlayer? Find(string? id) =>
        Players.FirstOrDefault(p => string.Equals(p.Id, id, StringComparison.OrdinalIgnoreCase));
}