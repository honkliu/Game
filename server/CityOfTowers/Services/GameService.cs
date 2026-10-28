using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Microsoft.EntityFrameworkCore;
using CityOfTowers.Data;
using CityOfTowers.Domain;

namespace CityOfTowers.Services;

public class GameService
{
    private readonly AppDb _db;
    private readonly IServiceProvider _services;
    public GameService(AppDb db, IServiceProvider services)
    {
        _db = db;
        _services = services;
    }

    /// <summary>Resolved lazily: AiService depends on GameService, so injecting it directly would cycle.</summary>
    private AiService Ai => _services.GetRequiredService<AiService>();

    /// <summary>Message produced by the most recent move, e.g. "enter-town:{id}".</summary>
    public string? LastMoveMessage { get; private set; }

    private static readonly JsonSerializerOptions Json = new() { PropertyNamingPolicy = JsonNamingPolicy.CamelCase };

    // ---------------------------------------------------------------- helpers
    public static string NormaliseAnswer(string s)
    {
        var sb = new StringBuilder();
        var lastSpace = false;
        foreach (var ch in s.Trim().ToLowerInvariant())
        {
            if (char.IsLetterOrDigit(ch)) { sb.Append(ch); lastSpace = false; }
            else if (char.IsWhiteSpace(ch) && !lastSpace) { sb.Append(' '); lastSpace = true; }
        }
        return sb.ToString().Trim();
    }

    public static string HashAnswer(string answer) =>
        Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(NormaliseAnswer(answer))));

    // ---------------------------------------------------------------- army
    public record ArmySlot(string? Creature, int Count);

    public static ArmySlot[] ParseArmy(string json)
    {
        try
        {
            var raw = JsonSerializer.Deserialize<List<ArmySlot?>>(json, Json) ?? [];
            var slots = new ArmySlot[7];
            for (var i = 0; i < 7; i++)
                slots[i] = i < raw.Count && raw[i] != null ? raw[i]! : new ArmySlot(null, 0);
            return slots;
        }
        catch
        {
            return Enumerable.Range(0, 7).Select(_ => new ArmySlot(null, 0)).ToArray();
        }
    }

    public static string SerializeArmy(ArmySlot[] slots) => JsonSerializer.Serialize(slots, Json);

    public static bool AddToArmy(ArmySlot[] slots, string creature, int count)
    {
        for (var i = 0; i < slots.Length; i++)
            if (slots[i].Creature == creature) { slots[i] = slots[i] with { Count = slots[i].Count + count }; return true; }
        for (var i = 0; i < slots.Length; i++)
            if (slots[i].Creature == null) { slots[i] = new ArmySlot(creature, count); return true; }
        return false;
    }

    public static int ArmyStrength(ArmySlot[] slots) =>
        slots.Where(s => s.Creature != null)
             .Sum(s => (Creatures.All.TryGetValue(s.Creature!, out var c) ? c.AiValue : 100) * s.Count);

    // ---------------------------------------------------------------- fog
    public static byte[] ParseFog(string json, int size)
    {
        try
        {
            var arr = JsonSerializer.Deserialize<byte[]>(json);
            if (arr != null && arr.Length == size) return arr;
        }
        catch { }
        return new byte[size];
    }

    public static string SerializeFog(byte[] fog) => JsonSerializer.Serialize(fog);

    /// <summary>Radius a town lets its owner see, and Cover of Darkness's shroud radius (VCMI DARKNESS 20).</summary>
    public const int TownSight = 5;
    public const int DarknessRadius = 20;

    public static void RecomputeFog(byte[] fog, int w, int h, IEnumerable<Hero> heroes, IEnumerable<Town>? towns = null)
    {
        for (var i = 0; i < fog.Length; i++) if (fog[i] == 2) fog[i] = 1;

        var eyes = heroes.Select(hero =>
        {
            var skills = ParseSkills(hero.SkillsJson);
            return (hero.X, hero.Y, R: 5 + 2 * (skills.TryGetValue("scouting", out var sc) ? sc : 0));
        }).Concat((towns ?? []).Select(t => (t.X, t.Y, R: TownSight)));

        foreach (var (cx, cy, radius) in eyes)
        {
            for (var y = cy - radius; y <= cy + radius; y++)
            for (var x = cx - radius; x <= cx + radius; x++)
            {
                if (x < 0 || y < 0 || x >= w || y >= h) continue;
                var dx = x - cx;
                var dy = y - cy;
                if (dx * dx + dy * dy > radius * radius) continue;
                fog[y * w + x] = 2;
            }
        }
    }

    /// <summary>Necropolis Cover of Darkness: re-shrouds the land around the town for everyone else, except what they see now.</summary>
    public static void ApplyDarkness(byte[] fog, int w, int h, Town dark)
    {
        var r = DarknessRadius;
        for (var y = dark.Y - r; y <= dark.Y + r; y++)
        for (var x = dark.X - r; x <= dark.X + r; x++)
        {
            if (x < 0 || y < 0 || x >= w || y >= h) continue;
            var dx = x - dark.X;
            var dy = y - dark.Y;
            if (dx * dx + dy * dy > r * r) continue;
            if (fog[y * w + x] == 1) fog[y * w + x] = 0;
        }
    }

    public static Dictionary<string, int> ParseSkills(string json)
    {
        try { return JsonSerializer.Deserialize<Dictionary<string, int>>(json, Json) ?? []; }
        catch { return []; }
    }

    public static string[] ParseStringArray(string json)
    {
        try { return JsonSerializer.Deserialize<string[]>(json, Json) ?? []; }
        catch { return []; }
    }

    // ---------------------------------------------------------------- hero
    public static Hero CreateHero(Guid gameId, Guid playerId, SeededPlayer sp, int x, int y)
    {
        var def = Heroes.All[sp.HeroKey];
        var skills = def.Skills.ToDictionary(s => s, _ => 1);
        var hero = new Hero
        {
            GameId = gameId,
            PlayerId = playerId,
            HeroKey = def.Key,
            Name = def.Name,
            Faction = def.Faction,
            Level = 1,
            Experience = 0,
            Attack = def.Attack,
            Defense = def.Defense,
            Power = def.Power,
            Knowledge = def.Knowledge,
            X = x,
            Y = y,
            SkillsJson = JsonSerializer.Serialize(skills, Json),
            SpellsJson = "[]",
            // magic heroes start with a spellbook, as in HoMM3
            ArtifactsJson = def.Might ? "[]" : JsonSerializer.Serialize(new[] { TownRules.Spellbook }, Json)
        };
        hero.MaxMana = 10 + hero.Knowledge * 10;
        hero.Mana = hero.MaxMana;
        hero.MaxMovement = MaxMovement(hero);
        hero.Movement = hero.MaxMovement;

        var tiers = Creatures.BaseTiers(sp.Faction);
        var army = Enumerable.Range(0, 7).Select(_ => new ArmySlot(null, 0)).ToArray();
        if (tiers.Length > 0) army[0] = new ArmySlot(tiers[0], 25);
        if (tiers.Length > 1) army[1] = new ArmySlot(tiers[1], 8);
        hero.ArmyJson = SerializeArmy(army);
        return hero;
    }

    public static int MaxMovement(Hero hero)
    {
        var skills = ParseSkills(hero.SkillsJson);
        var m = 1500;
        if (skills.TryGetValue("logistics", out var log)) m += 150 * log;
        return m;
    }

    // ---------------------------------------------------------------- game creation
    public async Task<Game> CreateGameAsync(string name, int capacity, int turnSeconds, int turnLimit, int mapSize, int aiPlayers = 0)
    {
        capacity = Math.Clamp(capacity, 2, 3);
        if (turnSeconds is not (30 or 60 or 300 or 86400)) turnSeconds = 300;
        if (turnLimit != 0) turnLimit = Math.Clamp(turnLimit, 10, 500);
        mapSize = Math.Clamp(mapSize, 36, 144);

        var riddle = Riddles.Pick(Random.Shared);
        var map = MapGenerator.Generate(mapSize, mapSize, Seeded.Players.Length, Random.Shared.Next());

        var game = new Game
        {
            Name = string.IsNullOrWhiteSpace(name) ? "The Three Cities" : name.Trim(),
            HostUserId = Guid.Empty,
            Capacity = capacity,
            TurnSeconds = turnSeconds,
            TurnLimit = turnLimit,
            RiddleId = Array.IndexOf(Riddles.All, riddle),
            PuzzleAnswerHash = HashAnswer(riddle.Answer),
            FragmentsJson = JsonSerializer.Serialize(riddle.Fragments),
            MapWidth = mapSize,
            MapHeight = mapSize,
            MapJson = JsonSerializer.Serialize(map, Json),
            State = GameState.InProgress,
            Day = 1,
            Week = 1,
            Month = 1,
            CurrentTurn = 1,
            TurnStartedAt = DateTime.UtcNow
        };

        var rng = new Random();
        for (var attempt = 0; attempt < 500; attempt++)
        {
            var gx = rng.Next(2, mapSize - 2);
            var gy = rng.Next(2, mapSize - 2);
            if (map.At(gx, gy) is { Passable: true, ObjectKind: null } && Math.Min(Math.Max(Math.Abs(gx - 8), Math.Abs(gy - 8)),
                    Math.Min(Math.Max(Math.Abs(gx - (mapSize - 9)), Math.Abs(gy - 8)), Math.Max(Math.Abs(gx - 8), Math.Abs(gy - (mapSize - 9))))) > 3)
            {
                game.GrailX = gx;
                game.GrailY = gy;
                break;
            }
        }

        _db.Games.Add(game);
        await _db.SaveChangesAsync();

        var starts = new (int x, int y)[] { (8, 8), (mapSize - 9, 8), (8, mapSize - 9) };
        for (var i = 0; i < Seeded.Players.Length; i++)
        {
            var sp = Seeded.Players[i];
            var (sx, sy) = starts[i];

            var player = new Player
            {
                GameId = game.Id,
                UserId = Guid.Empty,
                Faction = sp.Faction,
                CityName = sp.CityName,
                // the host plays seat 0; the last `aiPlayers` seats are computer-controlled
                IsAi = aiPlayers > 0 && i >= Seeded.Players.Length - aiPlayers,
                Wood = 20, Ore = 20, Mercury = 5, Sulfur = 5, Crystal = 5, Gems = 5, Gold = 20000
            };
            _db.Players.Add(player);
            await _db.SaveChangesAsync();

            // a HoMM3 starting town: Village Hall, Fort, Tavern and the first two dwellings
            string[] startBuildings = ["villageHall", "fort", "tavern", "dwelling1", "dwelling2"];
            var town = new Town
            {
                GameId = game.Id,
                PlayerId = player.Id,
                Name = sp.CityName,
                Faction = sp.Faction,
                X = sx,
                Y = sy,
                BuildingsJson = JsonSerializer.Serialize(startBuildings, Json),
                AvailableJson = JsonSerializer.Serialize(new Dictionary<string, int>
                {
                    ["1"] = TownBuildings.WeeklyGrowth(sp.Faction, startBuildings, 1),
                    ["2"] = TownBuildings.WeeklyGrowth(sp.Faction, startBuildings, 2)
                }, Json),
                GarrisonJson = SerializeArmy(Enumerable.Range(0, 7).Select(_ => new ArmySlot(null, 0)).ToArray()),
                TavernJson = JsonSerializer.Serialize(
                    Heroes.ForFaction(sp.Faction).Where(h => h.Key != sp.HeroKey).Select(h => h.Key).Take(2).ToArray(), Json),
                MageGuildJson = "[]"
            };
            _db.Towns.Add(town);

            var tile = map.At(sx, sy);
            tile.ObjectKind = "town";
            tile.ObjectData = town.Id.ToString();
            tile.Owner = i;
            tile.Passable = false;
            Footprint.Stamp(map, sx, sy, "town", null);
            var gate = map.At(sx, sy + 1);
            gate.ObjectKind = null;
            gate.Passable = true;

            // the starting hero stands in the town's gate, as in HoMM3
            var hero = CreateHero(game.Id, player.Id, sp, sx, sy);
            _db.Heroes.Add(hero);

            var fog = new byte[mapSize * mapSize];
            RecomputeFog(fog, mapSize, mapSize, [hero], [town]);
            player.FogJson = SerializeFog(fog);
        }

        game.MapJson = JsonSerializer.Serialize(map, Json);
        _db.SharedProjects.AddRange(SharedProjectCatalog.Seed(game.Id));
        await _db.SaveChangesAsync();
        return game;
    }

    // ---------------------------------------------------------------- map
    public Task<AdventureMap> LoadMapAsync(Game game) =>
        Task.FromResult(JsonSerializer.Deserialize<AdventureMap>(game.MapJson, Json) ?? new AdventureMap());

    public async Task SaveMapAsync(Game game, AdventureMap map)
    {
        game.MapJson = JsonSerializer.Serialize(map, Json);
        await _db.SaveChangesAsync();
    }

    // ---------------------------------------------------------------- movement
    /// <summary>
    /// Where a hero appears at a town (hired, or Town Portal): inside the town if the gate is free,
    /// else the nearest open tile outside it — two heroes never share a tile.
    /// </summary>
    public async Task<(int x, int y)> ArrivalSpotAsync(Game game, int tx, int ty, Guid? exceptHero = null)
    {
        var occupied = (await _db.Heroes.Where(h => h.GameId == game.Id && h.Id != exceptHero)
            .Select(h => new { h.X, h.Y }).ToListAsync()).Select(h => (h.X, h.Y)).ToHashSet();
        if (!occupied.Contains((tx, ty))) return (tx, ty);
        var map = await LoadMapAsync(game);
        var seen = new HashSet<(int, int)> { (tx, ty) };
        var queue = new Queue<(int x, int y)>();
        queue.Enqueue((tx, ty));
        while (queue.Count > 0)
        {
            var (cx, cy) = queue.Dequeue();
            foreach (var (dx, dy) in new[] { (0, 1), (-1, 1), (1, 1), (-1, 0), (1, 0), (0, -1), (-1, -1), (1, -1) })
            {
                var n = (cx + dx, cy + dy);
                if (!map.InBounds(n.Item1, n.Item2) || !seen.Add(n)) continue;
                var t = map.At(n.Item1, n.Item2);
                if (!t.Passable) { if (t.ObjectKind is Footprint.Block or "town") queue.Enqueue(n); continue; }
                if (!occupied.Contains(n)) return n;
                queue.Enqueue(n);
            }
        }
        return (tx, ty);
    }

    private static bool Corner(MapTile t) => Pathfinder.Pinches(t);

    public async Task<(bool ok, string? error, Hero? hero)> MoveHeroAsync(Guid heroId, string playerId, List<(int x, int y)> path)
    {
        var hero = await _db.Heroes.FirstOrDefaultAsync(h => h.Id == heroId);
        if (hero == null) return (false, "Hero not found.", null);

        var player = await _db.Players.FirstOrDefaultAsync(p => p.Id == hero.PlayerId);
        if (player == null) return (false, "Player not found.", null);

        var sp = Seeded.Find(playerId);
        if (sp == null || sp.Faction != player.Faction) return (false, "Not your hero.", null);

        var game = await _db.Games.FirstAsync(g => g.Id == hero.GameId);
        if (game.State != GameState.InProgress) return (false, "The game is not in progress.", null);
        if (hero.PendingEncounterJson != null) return (false, "Resolve the encounter first.", null);

        var map = await LoadMapAsync(game);
        var others = await _db.Heroes.Where(h => h.GameId == hero.GameId && h.Id != hero.Id)
            .Select(h => new { h.X, h.Y }).ToListAsync();
        var moved = false;
        var acted = false;
        string? blocked = null;
        LastMoveMessage = null;

        foreach (var (x, y) in path)
        {
            if (!map.InBounds(x, y)) { blocked = "Cannot move there."; break; }
            var dx = x - hero.X;
            var dy = y - hero.Y;
            if (Math.Max(Math.Abs(dx), Math.Abs(dy)) != 1) { blocked = "Heroes move one tile at a time."; break; }
            if (others.Any(o => o.X == x && o.Y == y)) { blocked = "Another hero stands there."; break; }

            var tile = map.At(x, y);
            var cost = Terrain.MoveCost(tile.Terrain);

            // Mines, monster stacks and obelisks are visited from the neighbouring tile. A town is
// different: the hero steps onto its own town's tile and stands inside it, as in HoMM3.
            if (!tile.Passable && tile.ObjectKind is "mine" or "monster" or "town" or "obelisk")
            {
                if (tile.ObjectKind == "town")
                {
                    var townId = Guid.TryParse(tile.ObjectData, out var tid) ? tid : (Guid?)null;
                    var town = townId == null ? null : await _db.Towns.FirstOrDefaultAsync(t => t.Id == townId);
                    var townOwner = town == null ? null : await _db.Players.FirstOrDefaultAsync(p => p.Id == town.PlayerId);
                    if (town != null && townOwner != null && townOwner.Faction == player.Faction)
                    {
                        // own town: walk onto the tile so the hero is visibly inside
                        if (hero.Movement < cost) { blocked = "Not enough movement left today."; break; }
                        hero.Movement -= cost;
                        hero.X = x;
                        hero.Y = y;
                        moved = true;
                        LastMoveMessage = $"enter-town:{town.Id}";
                        VisitOwnTown(game, hero, town);
                        break;
                    }
                }
                if (tile.ObjectKind != "town")
                {
                    if (hero.Movement < cost) { blocked = "Not enough movement left today."; break; }
                    hero.Movement -= cost;
                }
                var (_, adjMsg) = await InteractAsync(game, map, player, hero, tile, x, y);
                if (adjMsg != null) LastMoveMessage = adjMsg;
                acted = true;
                break;
            }

            if (!tile.Passable) { blocked = "Cannot move there."; break; }
            if (dx != 0 && dy != 0 && (Corner(map.At(hero.X + dx, hero.Y)) || Corner(map.At(hero.X, hero.Y + dy))))
            { blocked = "Cannot cut that corner."; break; }
            if (hero.Movement < cost) { blocked = "Not enough movement left today."; break; }

            hero.Movement -= cost;
            hero.X = x;
            hero.Y = y;
            moved = true;

            var (stop, msg) = await InteractAsync(game, map, player, hero, tile, x, y);
            if (msg != null) LastMoveMessage = msg;
            if (stop) break;
        }

        if (moved || acted)
        {
            var fog = ParseFog(player.FogJson, map.Width * map.Height);
            var heroes = await _db.Heroes.Where(h => h.PlayerId == player.Id).ToListAsync();
            var ownTowns = await _db.Towns.Where(t => t.PlayerId == player.Id).ToListAsync();
            RecomputeFog(fog, map.Width, map.Height, heroes, ownTowns);
            player.FogJson = SerializeFog(fog);
            await SaveMapAsync(game, map);
            return (true, null, hero);
        }

        return (false, blocked ?? "Cannot move there.", hero);
    }

    /// <summary>A hero entering its own town learns the guild's spells and, in a Castle, visits the Stables.</summary>
    public static void VisitOwnTown(Game game, Hero hero, Town town)
    {
        TownRules.LearnGuildSpells(hero, town);
        TownRules.VisitStables(hero, game, ParseStringArray(town.BuildingsJson));
    }

    private async Task<(bool stop, string? message)> InteractAsync(Game game, AdventureMap map, Player player, Hero hero, MapTile tile, int tx, int ty)
    {
        switch (tile.ObjectKind)
        {
            case "resource":
            {
                var parts = (tile.ObjectData ?? "").Split(':');
                var type = parts[0];
                var amount = parts.Length > 1 ? int.Parse(parts[1]) : 0;
                AddResource(player, type, amount);
                tile.ObjectKind = null;
                tile.ObjectData = null;
                return (false, (string?)$"Found {amount} {type}.");
            }
            case "treasure":
            {
                var parts = (tile.ObjectData ?? "").Split(':');
                var gold = parts.Length > 0 ? int.Parse(parts[0]) : 0;
                var xp = parts.Length > 1 ? int.Parse(parts[1]) : 0;
                player.Gold += gold;
                GainExperience(hero, xp);
                tile.ObjectKind = null;
                tile.ObjectData = null;
                return (false, (string?)$"Found {gold} gold and {xp} experience.");
            }
            case "mine":
            {
                var ownerIndex = Array.FindIndex(Seeded.Players, p => p.Faction == player.Faction);
                // Owner is a bitmask of the seats working this mine, so several cities can share it.
                var bit = 1 << ownerIndex;
                if ((tile.Owner & bit) != 0)
                    return (false, (string?)null);
                tile.Owner |= bit;
                return (false, (string?)$"Claimed the {tile.ObjectData} mine.");
            }
            case "artifact":
            {
                var arts = ParseStringArray(hero.ArtifactsJson).ToList();
                var found = tile.ObjectData ?? "artifact";
                // HoMM3 holds 14 paper-doll artifacts (excluding war machines/spellbook) plus a 64-item backpack
                if (arts.Count(a => !TownRules.IsGear(a)) >= 14 + 64) return (false, (string?)"The backpack is full.");
                arts.Add(found);
                hero.ArtifactsJson = JsonSerializer.Serialize(arts, Json);
                tile.ObjectKind = null;
                tile.ObjectData = null;
                return (false, (string?)$"Found the {Artifacts.Of(found).Name}.");
            }
            case "shrine":
            {
                // HoMM3: a shrine teaches nothing to a hero without a spellbook, and stays put
                if (!TownRules.HasGear(hero, TownRules.Spellbook))
                    return (false, (string?)"You need a spellbook to learn the shrine's spell.");
                var spells = ParseStringArray(hero.SpellsJson).ToList();
                var spell = tile.ObjectData ?? "spell";
                if (!spells.Contains(spell)) spells.Add(spell);
                hero.SpellsJson = JsonSerializer.Serialize(spells, Json);
                tile.ObjectKind = null;
                tile.ObjectData = null;
                return (false, (string?)"Learned a spell.");
            }
            case "monster":
            {
                // peaceful encounter: stop beside the stack and let the player choose
                var parts = (tile.ObjectData ?? "").Split(':');
                var creature = parts[0];
                var count = parts.Length > 1 && int.TryParse(parts[1], out var n) ? n : 5;
                hero.PendingEncounterJson = JsonSerializer.Serialize(
                    new { creature, count, x = tx, y = ty }, Json);
                return (true, (string?)"A wandering band blocks the way.");
            }
            case "town":
            {
                // stepping up to a town enters it: the client opens the town screen
                var townId = Guid.TryParse(tile.ObjectData, out var tid) ? tid : (Guid?)null;
                if (townId == null) return (true, (string?)null);
                var town = await _db.Towns.FirstOrDefaultAsync(t => t.Id == townId);
                if (town == null) return (true, (string?)null);
                var owner = await _db.Players.FirstOrDefaultAsync(p => p.Id == town.PlayerId);
                var mine = owner != null && owner.Faction == player.Faction;
                return (true, (string?)(mine ? $"enter-town:{townId}" : $"visit-town:{townId}"));
            }
            case "obelisk":
            {
                // walking up opens the Obelisk; its Visit action (VisitObeliskAsync) reveals the
                // next fragment, so a walk-up and a visit never count twice
                return (true, (string?)"obelisk:open");
            }
            case "grail":
            {
                // digging takes the rest of the day
                game.GrailFound = true;
                game.GrailFinderPlayerId = player.Id;
                player.Precious = true;
                hero.Movement = 0;
                tile.ObjectKind = null;
                tile.ObjectData = null;
                return (true, (string?)"You dig up the Grail!");
            }
            default:
                return (false, (string?)null);
        }
    }

    private static void AddResource(Player p, string type, int amount)
    {
        switch (type)
        {
            case "wood": p.Wood += amount; break;
            case "ore": p.Ore += amount; break;
            case "mercury": p.Mercury += amount; break;
            case "sulfur": p.Sulfur += amount; break;
            case "crystal": p.Crystal += amount; break;
            case "gems": p.Gems += amount; break;
            case "gold": p.Gold += amount; break;
        }
    }

    public static void GainExperience(Hero hero, int xp)
    {
        hero.Experience += xp;
        var newLevel = Experience.LevelFor(hero.Experience);
        while (hero.Level < newLevel)
        {
            hero.Level++;
            var rng = Random.Shared;
            if (rng.Next(2) == 0)
            {
                switch (rng.Next(4))
                {
                    case 0: hero.Attack++; break;
                    case 1: hero.Defense++; break;
                    case 2: hero.Power++; break;
                    default: hero.Knowledge++; hero.MaxMana += 10; break;
                }
            }
            else
            {
                var skills = ParseSkills(hero.SkillsJson);
                var upgradable = skills.Where(kv => kv.Value < 3).Select(kv => kv.Key).ToList();
                if (upgradable.Count > 0)
                {
                    var pick = upgradable[rng.Next(upgradable.Count)];
                    skills[pick]++;
                    hero.SkillsJson = JsonSerializer.Serialize(skills, Json);
                }
                else hero.Attack++;
            }
        }
    }

    // ---------------------------------------------------------------- turns
    // TickAsync is called both by the background TurnTicker and by the _test/nextday
    // endpoint. Two concurrent ticks load the same Game row and both write it, so the
    // second SaveChanges finds 0 rows affected and throws DbUpdateConcurrencyException.
    private static readonly SemaphoreSlim _tickLock = new(1, 1);

    public async Task<List<Guid>> TickAsync()
    {
        await _tickLock.WaitAsync();
        try
        {
            return await TickCoreAsync();
        }
        finally
        {
            _tickLock.Release();
        }
    }

    private async Task<List<Guid>> TickCoreAsync()
    {
        var now = DateTime.UtcNow;
        var advanced = new List<Guid>();
        var games = await _db.Games.Where(g => g.State == GameState.InProgress).ToListAsync();

        foreach (var game in games)
        {
            if ((now - game.TurnStartedAt).TotalSeconds < game.TurnSeconds) continue;

            game.CurrentTurn++;
            game.TurnStartedAt = now;
            game.Day++;
            if (game.Day > 7) { game.Day = 1; game.Week++; }
            if (game.Week > 4) { game.Week = 1; game.Month++; }

            var players = await _db.Players.Where(p => p.GameId == game.Id).ToListAsync();
            var towns = await _db.Towns.Where(t => t.GameId == game.Id).ToListAsync();
            var map = await LoadMapAsync(game);
            var caravan = await _db.SharedProjects.FirstOrDefaultAsync(sp =>
                sp.GameId == game.Id && sp.Completed && sp.Name == SharedProjectCatalog.CaravanRoad);
            var caravanMembers = caravan == null ? [] : ParseStringArray(caravan.ContributorsJson);

            foreach (var p in players)
            {
                p.BuiltThisTurn = false;
                p.WrongAnswers = 0;
                if (caravanMembers.Contains(p.Id.ToString())) p.Gold += SharedProjectCatalog.CaravanGoldPerDay;

                var town = towns.FirstOrDefault(t => t.PlayerId == p.Id);
                if (town != null)
                {
                    var built = ParseStringArray(town.BuildingsJson);
                    p.Gold += TownBuildings.GoldIncome(built);
                    if (built.Contains("resourceSilo"))
                        foreach (var (r, v) in TownBuildings.SiloProduction(town.Faction))
                            TownRules.Add(p, r, v);
                }

                var ownerIndex = Array.FindIndex(Seeded.Players, s => s.Faction == p.Faction);
                foreach (var t in map.Tiles)
                {
                    if (t.ObjectKind != "mine" || (t.Owner & (1 << ownerIndex)) == 0) continue;
                    switch (t.ObjectData)
                    {
                        case "wood": p.Wood += 2; break;
                        case "ore": p.Ore += 2; break;
                        case "gold": p.Gold += 1000; break;
                        case "mercury": p.Mercury++; break;
                        case "sulfur": p.Sulfur++; break;
                        case "crystal": p.Crystal++; break;
                        case "gems": p.Gems++; break;
                    }
                }

                var heroes = await _db.Heroes.Where(h => h.PlayerId == p.Id).ToListAsync();
                foreach (var h in heroes)
                {
                    // Castle Stables' bonus lasts until the end of the week it was granted in
                    h.MaxMovement = MaxMovement(h) + (TownRules.StabledThisWeek(h, game) ? 400 : 0);
                    h.Movement = h.MaxMovement;
                }

                // towns keep their surroundings in sight; Cover of Darkness re-shrouds rivals' land
                var mapFog = ParseFog(p.FogJson, map.Width * map.Height);
                RecomputeFog(mapFog, map.Width, map.Height, heroes, towns.Where(t => t.PlayerId == p.Id));
                foreach (var dark in towns.Where(t => t.PlayerId != p.Id && ParseStringArray(t.BuildingsJson).Contains("special1")
                                                      && t.Faction == "necropolis"))
                    ApplyDarkness(mapFog, map.Width, map.Height, dark);
                p.FogJson = SerializeFog(mapFog);
            }

            if (game.Day == 1)
            {
                foreach (var town in towns)
                {
                    var built = ParseStringArray(town.BuildingsJson);
                    var avail = JsonSerializer.Deserialize<Dictionary<string, int>>(town.AvailableJson, Json) ?? [];
                    for (var tier = 1; tier <= 7; tier++)
                    {
                        if (!TownBuildings.HasDwelling(built, tier)) continue;
                        var k = tier.ToString();
                        avail[k] = (avail.TryGetValue(k, out var cur) ? cur : 0) + TownBuildings.WeeklyGrowth(town.Faction, built, tier);
                    }
                    town.AvailableJson = JsonSerializer.Serialize(avail, Json);

                    var townOwner = players.FirstOrDefault(p => p.Id == town.PlayerId);
                    if (townOwner != null && town.Faction == "rampart")
                    {
                        // Mystic Pond: 1-4 of a random rare resource each week
                        if (built.Contains("special1"))
                        {
                            string[] rare = ["mercury", "sulfur", "crystal", "gems"];
                            TownRules.Add(townOwner, rare[Random.Shared.Next(rare.Length)], Random.Shared.Next(1, 5));
                        }
                        // Treasury: +10% of the gold held at the start of the week
                        if (built.Contains("special3")) townOwner.Gold += townOwner.Gold / 10;
                    }

                    // the tavern offers two fresh heroes each day, never one already hired
                    if (built.Contains("tavern"))
                    {
                        var owner = players.FirstOrDefault(p => p.Id == town.PlayerId);
                        var hired = owner == null
                            ? []
                            : (await _db.Heroes.Where(h => h.PlayerId == owner.Id).Select(h => h.HeroKey).ToListAsync());
                        var pool = Heroes.ForFaction(town.Faction)
                            .Where(h => !hired.Contains(h.Key))
                            .OrderBy(_ => Random.Shared.Next())
                            .Take(2)
                            .Select(h => h.Key)
                            .ToArray();
                        town.TavernJson = JsonSerializer.Serialize(pool, Json);
                    }
                }
            }

            if (game.TurnLimit > 0 && game.CurrentTurn > game.TurnLimit)
                game.State = GameState.Finished;

            try
            {
                await _db.SaveChangesAsync();
            }
            catch (DbUpdateConcurrencyException)
            {
                // the game was deleted or finished by another request while this tick ran;
                // drop the pending changes for it and move on
                foreach (var entry in _db.ChangeTracker.Entries().Where(e => e.State != EntityState.Unchanged).ToList())
                    entry.State = EntityState.Detached;
                continue;
            }

            // computer seats take their turn after the day has rolled over
            if (game.State == GameState.InProgress && players.Any(p => p.IsAi))
                await Ai.PlayAsync(game);

            await SaveMapAsync(game, map);
            advanced.Add(game.Id);
        }

        if (advanced.Count > 0) await _db.SaveChangesAsync();
        return advanced;
    }

    // ---------------------------------------------------------------- obelisk
    public async Task<(int visits, string[] fragments, string? error)> VisitObeliskAsync(Guid gameId, string playerId)
    {
        var sp = Seeded.Find(playerId);
        if (sp == null) return (0, [], "Unknown player.");

        var player = await _db.Players.FirstOrDefaultAsync(p => p.GameId == gameId && p.Faction == sp.Faction);
        if (player == null) return (0, [], "You are not in this game.");

        var game = await _db.Games.FirstAsync(g => g.Id == gameId);
        if (game.State != GameState.InProgress) return (player.ObeliskVisits, [], "The game is not in progress.");

        var all = ParseStringArray(game.FragmentsJson);
        if (player.ObeliskVisits >= all.Length)
            return (player.ObeliskVisits, all, "You have seen every fragment.");

        player.ObeliskVisits++;
        if (player.ObeliskVisits >= all.Length) await RevealGrailAsync(game);
        await _db.SaveChangesAsync();
        return (player.ObeliskVisits, all.Take(player.ObeliskVisits).ToArray(), null);
    }

    /// <summary>Marks the Grail tile on the map so a hero can walk there and dig.</summary>
    private async Task RevealGrailAsync(Game game)
    {
        if (game.GrailRevealed || game.GrailFound) return;
        game.GrailRevealed = true;
        var map = await LoadMapAsync(game);
        if (game.GrailX >= 0 && game.GrailY >= 0 && map.InBounds(game.GrailX, game.GrailY))
        {
            var t = map.At(game.GrailX, game.GrailY);
            t.ObjectKind = "grail";
            t.ObjectData = null;
            t.Passable = true;
            game.MapJson = JsonSerializer.Serialize(map, Json);
        }
    }

    public async Task<(bool correct, string? error)> SolveAsync(Guid gameId, string playerId, string answer)
    {
        var sp = Seeded.Find(playerId);
        if (sp == null) return (false, "Unknown player.");

        var player = await _db.Players.FirstOrDefaultAsync(p => p.GameId == gameId && p.Faction == sp.Faction);
        if (player == null) return (false, "You are not in this game.");

        var game = await _db.Games.FirstAsync(g => g.Id == gameId);
        if (game.State != GameState.InProgress) return (false, "The game is not in progress.");
        if (player.WrongAnswers >= 10) return (false, "Locked until next turn.");

        if (HashAnswer(answer) == game.PuzzleAnswerHash)
        {
            // Solving the puzzle reveals where to dig; the Grail itself must be dug up.
            await RevealGrailAsync(game);
            await _db.SaveChangesAsync();
            return (true, null);
        }

        player.WrongAnswers++;
        await _db.SaveChangesAsync();
        return (false, "That is not the answer. Try again.");
    }
}