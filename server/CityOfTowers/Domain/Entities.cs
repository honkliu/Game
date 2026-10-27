using System.ComponentModel.DataAnnotations;

namespace CityOfTowers.Domain;

public enum GameState { Waiting, InProgress, Finished }

public class Game
{
    public Guid Id { get; set; } = Guid.NewGuid();
    [MaxLength(60)] public string Name { get; set; } = "";
    public Guid HostUserId { get; set; }
    public int Capacity { get; set; } = 3;
    public int TurnSeconds { get; set; } = 300;
    public int TurnLimit { get; set; } = 60;
    public GameState State { get; set; } = GameState.Waiting;

    public int Day { get; set; } = 1;
    public int Week { get; set; } = 1;
    public int Month { get; set; } = 1;
    public int CurrentTurn { get; set; } = 1;

    public DateTime TurnStartedAt { get; set; } = DateTime.UtcNow;
    public Guid? WinnerUserId { get; set; }

    public int MapWidth { get; set; } = 72;
    public int MapHeight { get; set; } = 72;
    public string MapJson { get; set; } = "";

    public int RiddleId { get; set; }
    public string PuzzleAnswerHash { get; set; } = "";
    public string FragmentsJson { get; set; } = "[]";
    public int GrailX { get; set; } = -1;
    public int GrailY { get; set; } = -1;
    public bool GrailRevealed { get; set; }
    public bool GrailFound { get; set; }
    public Guid? GrailFinderPlayerId { get; set; }

    public DateTime CreatedAt { get; set; } = DateTime.UtcNow;
}

public class Player
{
    public Guid Id { get; set; } = Guid.NewGuid();
    public Guid GameId { get; set; }
    public Guid UserId { get; set; }
    [MaxLength(20)] public string Faction { get; set; } = "castle";
    [MaxLength(40)] public string CityName { get; set; } = "";
    public bool Precious { get; set; }
    /// <summary>Computer-controlled seat: the turn ticker plays it.</summary>
    public bool IsAi { get; set; }
    public int ObeliskVisits { get; set; }
    public bool BuiltThisTurn { get; set; }
    public int WrongAnswers { get; set; }
    public DateTime JoinedAt { get; set; } = DateTime.UtcNow;

    public int Wood { get; set; }
    public int Ore { get; set; }
    public int Mercury { get; set; }
    public int Sulfur { get; set; }
    public int Crystal { get; set; }
    public int Gems { get; set; }
    public int Gold { get; set; }

    public string FogJson { get; set; } = "";
}

public class Hero
{
    public Guid Id { get; set; } = Guid.NewGuid();
    public Guid GameId { get; set; }
    public Guid PlayerId { get; set; }
    [MaxLength(30)] public string HeroKey { get; set; } = "";
    [MaxLength(30)] public string Name { get; set; } = "";
    [MaxLength(20)] public string Faction { get; set; } = "castle";

    public int Level { get; set; } = 1;
    public int Experience { get; set; }
    public int Attack { get; set; }
    public int Defense { get; set; }
    public int Power { get; set; }
    public int Knowledge { get; set; }

    public int Mana { get; set; }
    public int MaxMana { get; set; }
    public int Movement { get; set; }
    public int MaxMovement { get; set; }

    public int X { get; set; }
    public int Y { get; set; }

    public string ArmyJson { get; set; } = "[]";
    public string SkillsJson { get; set; } = "{}";
    public string SpellsJson { get; set; } = "[]";
    public string ArtifactsJson { get; set; } = "[]";
    /// <summary>Set when the hero is standing on a monster tile awaiting a choice.</summary>
    public string? PendingEncounterJson { get; set; }
}

public class Town
{
    public Guid Id { get; set; } = Guid.NewGuid();
    public Guid GameId { get; set; }
    public Guid PlayerId { get; set; }
    [MaxLength(40)] public string Name { get; set; } = "";
    [MaxLength(20)] public string Faction { get; set; } = "castle";
    public int X { get; set; }
    public int Y { get; set; }

    public string BuildingsJson { get; set; } = "[]";
    public string AvailableJson { get; set; } = "{}";
    public string GarrisonJson { get; set; } = "[]";
    public string TavernJson { get; set; } = "[]";
    public string MageGuildJson { get; set; } = "[]";
}

public class Post
{
    public Guid Id { get; set; } = Guid.NewGuid();
    public Guid TownId { get; set; }
    [MaxLength(120)] public string Title { get; set; } = "";
    [MaxLength(20000)] public string Body { get; set; } = "";
    [MaxLength(500)] public string? ImageUrl { get; set; }
    public DateTime CreatedAt { get; set; } = DateTime.UtcNow;
    public DateTime UpdatedAt { get; set; } = DateTime.UtcNow;
}

public class Comment
{
    public Guid Id { get; set; } = Guid.NewGuid();
    public Guid PostId { get; set; }
    public Guid AuthorUserId { get; set; }
    [MaxLength(2000)] public string Body { get; set; } = "";
    public DateTime CreatedAt { get; set; } = DateTime.UtcNow;
}

public class WallMessage
{
    public Guid Id { get; set; } = Guid.NewGuid();
    public Guid TownId { get; set; }
    public Guid AuthorUserId { get; set; }
    public Guid? ParentId { get; set; }
    [MaxLength(2000)] public string Body { get; set; } = "";
    public DateTime CreatedAt { get; set; } = DateTime.UtcNow;
}

public class TowerMessage
{
    public Guid Id { get; set; } = Guid.NewGuid();
    public Guid TownId { get; set; }
    public Guid AuthorUserId { get; set; }
    [MaxLength(2000)] public string Body { get; set; } = "";
    public DateTime CreatedAt { get; set; } = DateTime.UtcNow;
}

public class ObeliskMessage
{
    public Guid Id { get; set; } = Guid.NewGuid();
    public Guid GameId { get; set; }
    public Guid AuthorUserId { get; set; }
    [MaxLength(2000)] public string Body { get; set; } = "";
    public DateTime CreatedAt { get; set; } = DateTime.UtcNow;
}