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
    public GameService(AppDb db) => _db = db;

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

    public static void RecomputeFog(byte[] fog, int w, int h, IEnumerable<Hero> heroes)
    {
        for (var i = 0; i < fog.Length; i++) if (fog[i] == 2) fog[i] = 1;

        foreach (var hero in heroes)
        {
            var skills = ParseSkills(hero.SkillsJson);
            var radius = 5 + 2 * (skills.TryGetValue("scouting", out var sc) ? sc : 0);
            for (var y = hero.Y - radius; y <= hero.Y + radius; y++)
            for (var x = hero.X - radius; x <= hero.X + radius; x++)
            {
                if (x < 0 || y < 0 || x >= w || y >= h) continue;
                var dx = x - hero.X;
                var dy = y - hero.Y;
                if (dx * dx + dy * dy > radius * radius) continue;
                fog[y * w + x] = 2;
            }
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
            ArtifactsJson = "[]"
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
    public async Task<Game> CreateGameAsync(string name, int capacity, int turnSeconds, int turnLimit, int mapSize)
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
            if (map.At(gx, gy).Passable)
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
                Wood = 20, Ore = 20, Mercury = 5, Sulfur = 5, Crystal = 5, Gems = 5, Gold = 20000
            };
            _db.Players.Add(player);
            await _db.SaveChangesAsync();

            var town = new Town
            {
                GameId = game.Id,
                PlayerId = player.Id,
                Name = sp.CityName,
                Faction = sp.Faction,
                X = sx,
                Y = sy,
                BuildingsJson = JsonSerializer.Serialize(new[] { "villageHall", "dwelling1", "dwelling2", "tavern", "marketplace" }, Json),
                AvailableJson = JsonSerializer.Serialize(new Dictionary<string, int> { ["1"] = 28, ["2"] = 16 }, Json),
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

            var hx = sx + 1 < mapSize ? sx + 1 : sx - 1;
            var hero = CreateHero(game.Id, player.Id, sp, hx, sy);
            _db.Heroes.Add(hero);

            var fog = new byte[mapSize * mapSize];
            RecomputeFog(fog, mapSize, mapSize, [hero]);
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

        foreach (var (x, y) in path)
        {
            if (!map.InBounds(x, y)) { blocked = "Cannot move there."; break; }
            var dx = x - hero.X;
            var dy = y - hero.Y;
            if (Math.Max(Math.Abs(dx), Math.Abs(dy)) != 1) { blocked = "Heroes move one tile at a time."; break; }
            if (others.Any(o => o.X == x && o.Y == y)) { blocked = "Another hero stands there."; break; }

            var tile = map.At(x, y);
            var cost = Terrain.MoveCost(tile.Terrain);

            // Mines, monster stacks and towns are visited from the neighbouring tile.
            if (!tile.Passable && tile.ObjectKind is "mine" or "monster" or "town")
            {
                if (tile.ObjectKind != "town")
                {
                    if (hero.Movement < cost) { blocked = "Not enough movement left today."; break; }
                    hero.Movement -= cost;
                }
                await InteractAsync(game, map, player, hero, tile, x, y);
                acted = true;
                break;
            }

            if (!tile.Passable) { blocked = "Cannot move there."; break; }
            if (dx != 0 && dy != 0 && (!map.At(hero.X + dx, hero.Y).Passable || !map.At(hero.X, hero.Y + dy).Passable))
            { blocked = "Cannot cut that corner."; break; }
            if (hero.Movement < cost) { blocked = "Not enough movement left today."; break; }

            hero.Movement -= cost;
            hero.X = x;
            hero.Y = y;
            moved = true;

            var (stop, _) = await InteractAsync(game, map, player, hero, tile, x, y);
            if (stop) break;
        }

        if (moved || acted)
        {
            var fog = ParseFog(player.FogJson, map.Width * map.Height);
            var heroes = await _db.Heroes.Where(h => h.PlayerId == player.Id).ToListAsync();
            RecomputeFog(fog, map.Width, map.Height, heroes);
            player.FogJson = SerializeFog(fog);
            await SaveMapAsync(game, map);
            return (true, null, hero);
        }

        return (false, blocked ?? "Cannot move there.", hero);
    }

    private Task<(bool stop, string? message)> InteractAsync(Game game, AdventureMap map, Player player, Hero hero, MapTile tile, int tx, int ty)
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
                return Task.FromResult((false, (string?)$"Found {amount} {type}."));
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
                return Task.FromResult((false, (string?)$"Found {gold} gold and {xp} experience."));
            }
            case "mine":
            {
                var ownerIndex = Array.FindIndex(Seeded.Players, p => p.Faction == player.Faction);
                if (tile.Owner == -1)
                {
                    tile.Owner = ownerIndex;
                    return Task.FromResult((false, (string?)$"Claimed the {tile.ObjectData} mine."));
                }
                return Task.FromResult((false, (string?)(tile.Owner == ownerIndex ? null : "Another city already works this mine.")));
            }
            case "artifact":
            {
                var arts = ParseStringArray(hero.ArtifactsJson).ToList();
                if (arts.Count < 8) arts.Add(tile.ObjectData ?? "artifact");
                hero.ArtifactsJson = JsonSerializer.Serialize(arts, Json);
                tile.ObjectKind = null;
                tile.ObjectData = null;
                return Task.FromResult((false, (string?)"Found an artifact."));
            }
            case "shrine":
            {
                var spells = ParseStringArray(hero.SpellsJson).ToList();
                var spell = tile.ObjectData ?? "spell";
                if (!spells.Contains(spell)) spells.Add(spell);
                hero.SpellsJson = JsonSerializer.Serialize(spells, Json);
                tile.ObjectKind = null;
                tile.ObjectData = null;
                return Task.FromResult((false, (string?)"Learned a spell."));
            }
            case "monster":
            {
                // peaceful encounter: stop beside the stack and let the player choose
                var parts = (tile.ObjectData ?? "").Split(':');
                var creature = parts[0];
                var count = parts.Length > 1 && int.TryParse(parts[1], out var n) ? n : 5;
                hero.PendingEncounterJson = JsonSerializer.Serialize(
                    new { creature, count, x = tx, y = ty }, Json);
                return Task.FromResult((true, (string?)"A wandering band blocks the way."));
            }
            case "town":
                return Task.FromResult((true, (string?)null));
            case "grail":
            {
                // digging takes the rest of the day
                game.GrailFound = true;
                game.GrailFinderPlayerId = player.Id;
                player.Precious = true;
                hero.Movement = 0;
                tile.ObjectKind = null;
                tile.ObjectData = null;
                return Task.FromResult((true, (string?)"You dig up the Grail!"));
            }
            default:
                return Task.FromResult((false, (string?)null));
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
    public async Task<List<Guid>> TickAsync()
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
                    {
                        p.Wood++; p.Ore++; p.Mercury++; p.Sulfur++; p.Crystal++; p.Gems++;
                    }
                }

                var ownerIndex = Array.FindIndex(Seeded.Players, s => s.Faction == p.Faction);
                foreach (var t in map.Tiles)
                {
                    if (t.ObjectKind != "mine" || t.Owner != ownerIndex) continue;
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
                    h.MaxMovement = MaxMovement(h);
                    h.Movement = h.MaxMovement;
                }
            }

            if (game.Day == 1)
            {
                foreach (var town in towns)
                {
                    var built = ParseStringArray(town.BuildingsJson);
                    var mult = TownBuildings.GrowthMultiplier(built);
                    var avail = JsonSerializer.Deserialize<Dictionary<string, int>>(town.AvailableJson, Json) ?? [];
                    var tiers = Creatures.BaseTiers(town.Faction);
                    for (var tier = 1; tier <= 7; tier++)
                    {
                        if (!TownBuildings.HasDwelling(built, tier)) continue;
                        var key = tiers[tier - 1];
                        var growth = (int)Math.Floor(Creatures.All[key].Growth * mult) + (built.Contains("grail") ? 2 : 0);
                        var k = tier.ToString();
                        avail[k] = (avail.TryGetValue(k, out var cur) ? cur : 0) + growth;
                    }
                    town.AvailableJson = JsonSerializer.Serialize(avail, Json);
                }
            }

            if (game.TurnLimit > 0 && game.CurrentTurn > game.TurnLimit)
                game.State = GameState.Finished;

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