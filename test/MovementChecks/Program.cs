using System.Text.Json;
using CityOfTowers.Data;
using CityOfTowers.Domain;
using CityOfTowers.Services;
using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;

if (args.FirstOrDefault() == "actions") return await ActionChecks.Run();

var failed = 0;
var checks = 0;
void Check(string name, bool ok)
{
    checks++;
    Console.WriteLine($"{(ok ? "PASS" : "FAIL")} {name}");
    if (!ok) failed++;
}

foreach (var kind in new[] { "block", "town", "mine", "monster", "obelisk", "shrine", "tree" })
{
    await using var f = await Fixture.Create();
    f.Map.At(2, 1).ObjectKind = kind;
    f.Map.At(2, 1).Passable = false;
    var path = Pathfinder.FindPath(f.Map, 1, 1, 2, 2);
    Check($"path detours around {kind} corner", path is { Count: > 2 });
    await f.SaveMap();
    var move = await f.Service.MoveHeroAsync(f.Hero.Id, "alice", [(2, 2)]);
    Check($"server blocks diagonal across {kind}", !move.ok && f.Hero.X == 1 && f.Hero.Y == 1 && f.Hero.Movement == 1500);
}

foreach (var kind in new[] { "town", "mine", "monster", "obelisk", "shrine" })
{
    await using var f = await Fixture.Create();
    f.Map.At(2, 1).ObjectKind = "block";
    f.Map.At(2, 1).Passable = false;
    var target = f.Map.At(2, 2);
    target.ObjectKind = kind;
    target.Passable = false;
    target.ObjectData = kind switch
    {
        "town" => f.Town.Id.ToString(),
        "mine" => "wood",
        "monster" => "pikeman:5",
        _ => null
    };
    target.Owner = 0;
    await f.SaveMap();
    var move = await f.Service.MoveHeroAsync(f.Hero.Id, "alice", [(2, 2)]);
    Check($"cannot interact with {kind} through a blocked corner",
        !move.ok && f.Hero.X == 1 && f.Hero.Y == 1 && f.Hero.Movement == 1500
        && f.Hero.PendingEncounterJson == null && f.Service.LastMoveMessage == null);
}

{
    await using var f = await Fixture.Create();
    f.Db.Heroes.Add(new Hero { GameId = f.Game.Id, PlayerId = f.Player.Id, X = 2, Y = 1 });
    await f.Db.SaveChangesAsync();
    var move = await f.Service.MoveHeroAsync(f.Hero.Id, "alice", [(2, 2)]);
    Check("cannot cut diagonally through another hero", !move.ok && f.Hero.Movement == 1500);
}

{
    await using var f = await Fixture.Create();
    var gate = f.Map.At(2, 2);
    gate.ObjectKind = "town";
    gate.ObjectData = f.Town.Id.ToString();
    gate.Passable = false;
    Footprint.Stamp(f.Map, 2, 2, "town", null);
    f.Hero.X = 2;
    f.Hero.Y = 3;
    await f.SaveMap();
    var enter = await f.Service.MoveHeroAsync(f.Hero.Id, "alice", [(2, 2), (2, 1)]);
    Check("enter own gate and stop instead of passing through town",
        enter.ok && f.Hero.X == 2 && f.Hero.Y == 2 && f.Service.LastMoveMessage == $"enter-town:{f.Town.Id}");
    var diagonalExit = await f.Service.MoveHeroAsync(f.Hero.Id, "alice", [(3, 3)]);
    Check("cannot exit town diagonally through its wall", !diagonalExit.ok && f.Hero.X == 2 && f.Hero.Y == 2);
    var exit = await f.Service.MoveHeroAsync(f.Hero.Id, "alice", [(2, 3)]);
    Check("can leave town through the open gate", exit.ok && f.Hero.X == 2 && f.Hero.Y == 3);
    var wall = await f.Service.MoveHeroAsync(f.Hero.Id, "alice", [(3, 2)]);
    Check("town wall itself remains impassable", !wall.ok && f.Hero.X == 2 && f.Hero.Y == 3);
}

{
    await using var f = await Fixture.Create();
    var monster = f.Map.At(2, 1);
    monster.ObjectKind = "monster";
    monster.ObjectData = "pikeman:5";
    monster.Passable = false;
    await f.SaveMap();
    var visit = await f.Service.MoveHeroAsync(f.Hero.Id, "alice", [(2, 1), (3, 1)]);
    Check("approaching a creature stops beside it, not beyond it",
        visit.ok && f.Hero.X == 1 && f.Hero.Y == 1 && f.Hero.PendingEncounterJson != null);
}

{
    await using var f = await Fixture.Create();
    var shrine = f.Map.At(2, 1);
    shrine.ObjectKind = "shrine";
    shrine.ObjectData = "magicArrow";
    f.Hero.ArtifactsJson = JsonSerializer.Serialize(new[] { TownRules.Spellbook });
    await f.SaveMap();
    var loaded = await f.Service.LoadMapAsync(f.Game);
    Check("old saves treat shrines as solid structures", !loaded.At(2, 1).Passable);
    var visit = await f.Service.MoveHeroAsync(f.Hero.Id, "alice", [(2, 1), (3, 1)]);
    var after = await f.Service.LoadMapAsync(f.Game);
    Check("shrine visit teaches without walking through or removing the structure",
        visit.ok && f.Hero.X == 1 && f.Hero.Y == 1
        && GameService.ParseStringArray(f.Hero.SpellsJson).Contains("magicArrow")
        && after.At(2, 1).ObjectKind == "shrine" && !after.At(2, 1).Passable);
    f.Hero.ArtifactsJson = "[]";
    f.Hero.SpellsJson = "[]";
    await f.Db.SaveChangesAsync();
    var noBook = await f.Service.MoveHeroAsync(f.Hero.Id, "alice", [(2, 1)]);
    Check("shrine without a spellbook still blocks movement", noBook.ok && f.Hero.X == 1 && f.Hero.Y == 1 && f.Hero.SpellsJson == "[]");
}

{
    await using var f = await Fixture.Create();
    f.Player.IsAi = true;
    f.Player.BuiltThisTurn = true;
    f.Hero.X = 2;
    f.Hero.Y = 2;
    var gate = f.Map.At(2, 2);
    gate.ObjectKind = "town";
    gate.ObjectData = f.Town.Id.ToString();
    gate.Passable = false;
    Footprint.Stamp(f.Map, 2, 2, "town", null);
    var mine = f.Map.At(5, 4);
    mine.ObjectKind = "mine";
    mine.ObjectData = "ore";
    mine.Owner = 0;
    mine.Passable = false;
    Footprint.Stamp(f.Map, 5, 4, "mine", "ore");
    await f.SaveMap();
    await new AiService(f.Db, f.Service).PlayAsync(f.Game);
    Check("computer hero leaves through its gate instead of cutting through walls",
        f.Hero.X == 2 && f.Hero.Y == 3 && f.Hero.Movement < 1500);
}

{
    await using var f = await Fixture.Create();
    f.Map.At(2, 1).ObjectKind = "resource";
    f.Map.At(2, 1).ObjectData = "wood:5";
    await f.SaveMap();
    var move = await f.Service.MoveHeroAsync(f.Hero.Id, "alice", [(2, 2)]);
    Check("open diagonals beside passable pickups still work", move.ok && f.Hero.X == 2 && f.Hero.Y == 2);
}

Console.WriteLine($"{checks - failed}/{checks} movement checks passed");
return failed == 0 ? 0 : 1;

sealed class Fixture : IAsyncDisposable
{
    private readonly SqliteConnection _connection;
    private readonly ServiceProvider _services = new ServiceCollection().BuildServiceProvider();
    private static readonly JsonSerializerOptions Json = new() { PropertyNamingPolicy = JsonNamingPolicy.CamelCase };
    public AppDb Db { get; }
    public SqliteConnection Connection => _connection;
    public AdventureMap Map { get; } = new() { Width = 7, Height = 7, Tiles = Enumerable.Range(0, 49).Select(_ => new MapTile()).ToList() };
    public Game Game { get; } = new() { State = GameState.InProgress, MapWidth = 7, MapHeight = 7 };
    public Player Player { get; } = new();
    public Hero Hero { get; } = new() { X = 1, Y = 1, Movement = 1500, MaxMovement = 1500 };
    public Town Town { get; } = new() { X = 2, Y = 2 };
    public GameService Service { get; }

    private Fixture(SqliteConnection connection)
    {
        _connection = connection;
        Db = new AppDb(new DbContextOptionsBuilder<AppDb>().UseSqlite(connection).Options);
        Service = new GameService(Db, _services);
        Player.GameId = Game.Id;
        Hero.GameId = Game.Id;
        Hero.PlayerId = Player.Id;
        Town.GameId = Game.Id;
        Town.PlayerId = Player.Id;
    }

    public static async Task<Fixture> Create()
    {
        var connection = new SqliteConnection("Data Source=:memory:");
        await connection.OpenAsync();
        var fixture = new Fixture(connection);
        await fixture.Db.Database.EnsureCreatedAsync();
        fixture.Db.AddRange(fixture.Game, fixture.Player, fixture.Hero, fixture.Town);
        await fixture.SaveMap();
        return fixture;
    }

    public async Task SaveMap()
    {
        Game.MapJson = JsonSerializer.Serialize(Map, Json);
        await Db.SaveChangesAsync();
    }

    public async ValueTask DisposeAsync()
    {
        await Db.DisposeAsync();
        await _connection.DisposeAsync();
        await _services.DisposeAsync();
    }
}
