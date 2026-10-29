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

{
    await using var f = await Fixture.Create();
    var town = f.Map.At(2, 2);
    town.ObjectKind = "town";
    town.ObjectData = f.Town.Id.ToString();
    town.Passable = false;
    f.Hero.X = 5; f.Hero.Y = 2;
    await f.SaveMap();
    var move = await f.Service.MoveHeroAsync(f.Hero.Id, "alice", [(4, 2), (3, 2), (2, 2)]);
    Check("old saves without town footprints cannot walk through the city",
        !move.ok && f.Hero.X == 5 && f.Hero.Y == 2 && f.Hero.Movement == 1500);
    var loaded = await f.Service.LoadMapAsync(f.Game);
    Check("old saves rebuild the entire solid town footprint",
        Footprint.Offsets("town", null).All(o => !loaded.At(2 + o.dx, 2 + o.dy).Passable));
}

{
    await using var f = await Fixture.Create();
    var monster = f.Map.At(2, 1);
    monster.ObjectKind = "monster"; monster.ObjectData = "pikeman:5"; monster.Passable = true;
    await f.SaveMap();
    var move = await f.Service.MoveHeroAsync(f.Hero.Id, "alice", [(2, 1), (3, 1)]);
    Check("old saves with walk-through creatures stop the hero beside the creature",
        move.ok && f.Hero.X == 1 && f.Hero.Y == 1 && f.Hero.PendingEncounterJson != null);
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

foreach (var (kind, data, expected) in new (string, string?, int)[]
{
    ("town", null, 13), ("mine", "wood", 6), ("mine", "ore", 6), ("mine", "gems", 6),
    ("mine", "gold", 3), ("mine", "crystal", 3), ("mine", "mercury", 3), ("mine", "sulfur", 3)
})
{
    await using var f = await Fixture.Create();
    var anchor = f.Map.At(3, 3);
    anchor.ObjectKind = kind; anchor.ObjectData = kind == "town" ? f.Town.Id.ToString() : data; anchor.Passable = false;
    Footprint.Stamp(f.Map, 3, 3, kind, data);
    var offsets = Footprint.Offsets(kind, data).ToHashSet();
    Check($"{kind} {data} uses its native ground footprint, not a generic rectangle", offsets.Count + 1 == expected);
    var walkable = true;
    foreach (var (dx, dy) in offsets.Where(o => o != (0, 0)))
    {
        f.Hero.X = 3 + dx; f.Hero.Y = 4 + dy;
        await f.SaveMap();
        var result = await f.Service.MoveHeroAsync(f.Hero.Id, "alice", [(3 + dx, 3 + dy)]);
        walkable &= !result.ok && f.Hero.Y == 4 + dy;
    }
    Check($"{kind} {data} blocks walking onto every non-entry body tile", walkable);
    if (kind == "town")
        Check("town decorative upper corners are not invisible walls",
            f.Map.At(1, 1).Passable && f.Map.At(5, 1).Passable && !f.Map.At(3, 1).Passable);
}
foreach (var (kind, terrain) in new[] { ("tree", "grass"), ("tree", "dirt"), ("tree", "snow"), ("rock", "grass"), ("mountain", "grass") })
{
    await using var f = await Fixture.Create();
    var obstacle = f.Map.At(3, 3);
    obstacle.ObjectKind = kind; obstacle.Terrain = terrain; obstacle.Passable = false;
    var pickup = f.Map.At(2, 3);
    pickup.ObjectKind = "resource"; pickup.ObjectData = "gold:500";
    await f.SaveMap();
    var loaded = await f.Service.LoadMapAsync(f.Game);
    var sprite = loaded.At(3, 3).Sprite;
    Check($"{terrain} {kind} selects art whose solid body fits without covering a pickup or road",
        sprite != null && ObjectGeometry.Blocked(sprite).ToHashSet().SetEquals([(0, 0)])
        && loaded.At(2, 3).Passable && loaded.At(2, 3).ObjectKind == "resource" && loaded.At(4, 3).Passable);
}

for (var seed = 0; seed < 25; seed++)
{
    const int size = 72;
    var map = MapGenerator.Generate(size, size, 3, seed);
    var reachable = true;
    foreach (var (sx, sy) in new[] { (8, 8), (size - 9, 8), (8, size - 9) })
    {
        map.At(sx, sy).ObjectKind = "town"; map.At(sx, sy).Passable = false;
        Footprint.Stamp(map, sx, sy, "town", null);
        var seen = Enumerable.Range(0, size * size).Select(i => (x: i % size, y: i / size))
            .Where(p => map.At(p.x, p.y).ObjectKind == "obelisk" && (p.x - sx) * (p.x - sx) + (p.y - sy) * (p.y - sy) <= 25);
        reachable &= seen.Any(p => Pathfinder.FindPath(map, sx, sy + 1, p.x, p.y) != null);
    }
    if (!reachable) { Check($"seed {seed}: every start sees a reachable obelisk on day one", false); break; }
    if (seed == 24) Check("every generated start sees a reachable obelisk on day one", true);
}

{
    await using var f = await Fixture.Create();
    f.Map.At(f.Town.X, f.Town.Y).ObjectKind = "town";
    await f.SaveMap();
    var first = await f.Service.EnsureStartObelisksAsync();
    var second = await f.Service.EnsureStartObelisksAsync();
    var loaded = await f.Service.LoadMapAsync(f.Game);
    Check("old saves gain one obelisk inside each town's sight, once",
        first >= 1 && second == 0 && loaded.Tiles.Count(t => t.ObjectKind == "obelisk") == first);
}

foreach (var size in new[] { 36, 72, 144 })
{
    var ok = true;
    for (var seed = 0; seed < 10 && ok; seed++)
    {
        var map = MapGenerator.Generate(size, size, 3, seed);
        var starts = new[] { (8, 8), (size - 9, 8), (8, size - 9) }.Select(s =>
        {
            map.At(s.Item1, s.Item2).ObjectKind = "town"; map.At(s.Item1, s.Item2).Passable = false;
            Footprint.Stamp(map, s.Item1, s.Item2, "town", null);
            map.At(s.Item1, s.Item2 + 1).ObjectKind = null; map.At(s.Item1, s.Item2 + 1).Passable = true;
            return new Town { PlayerId = Guid.NewGuid(), X = s.Item1, Y = s.Item2 };
        }).ToList();
        var neutral = GameService.PlaceNeutralTowns(Guid.NewGuid(), map, starts, new HashSet<(int x, int y)>(),
            GameService.NeutralTownCount(size), new Random(seed));
        ok = neutral.Count >= 1 && neutral.All(t => t.PlayerId == Guid.Empty && map.At(t.X, t.Y).ObjectKind == "town"
            && map.At(t.X, t.Y).Owner == -1 && starts.All(s => Math.Max(Math.Abs(s.X - t.X), Math.Abs(s.Y - t.Y)) >= 6)
            && starts.Any(s => Pathfinder.FindPath(map, s.X, s.Y + 1, t.X, t.Y + 1) != null));
        if (!ok) Check($"{size}x{size} seed {seed}: an unowned, reachable town is placed", false);
    }
    if (ok) Check($"{size}x{size} maps place unowned towns away from starts and reachable from them", true);
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
        var connection = new SqliteConnection($"Data Source=movement-{Guid.NewGuid():N};Mode=Memory;Cache=Shared");
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
