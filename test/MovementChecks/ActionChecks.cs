using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using CityOfTowers.Api;
using CityOfTowers.Data;
using CityOfTowers.Domain;
using CityOfTowers.Services;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;

static class ActionChecks
{
    private static readonly JsonSerializerOptions Json = new() { PropertyNamingPolicy = JsonNamingPolicy.CamelCase };
    private static void Require(bool ok, string detail)
    {
        if (!ok) throw new InvalidOperationException(detail);
    }

    public static async Task<int> Run()
    {
        Fixture? current = null;
        var builder = WebApplication.CreateBuilder();
        builder.WebHost.UseUrls("http://127.0.0.1:0");
        builder.Logging.ClearProviders();
        builder.Services.AddScoped(_ => new AppDb(new DbContextOptionsBuilder<AppDb>().UseSqlite(current!.Connection.ConnectionString).Options));
        builder.Services.AddScoped<GameService>();
        builder.Services.AddScoped<AiService>();
        builder.Services.AddScoped<SocialService>();
        builder.Services.AddSingleton<WsHub>();
        builder.Services.AddSingleton<RateLimiter>();
        builder.Services.AddSignalR();
        await using var app = builder.Build();
        app.MapApi();
        await app.StartAsync();
        using var client = new HttpClient { BaseAddress = new Uri(app.Urls.Single()) };
        var results = new List<object>();
        var failures = 0;

        async Task Case(string id, string action, string expected, Func<Fixture, Task> test, string kind = "Preserve")
        {
            await using var f = await Fixture.Create();
            current = f;
            f.Hero.X = f.Town.X;
            f.Hero.Y = f.Town.Y;
            f.Hero.Name = "Christian";
            f.Hero.HeroKey = "christian";
            f.Hero.ArmyJson = Army(("pikeman", 25), ("archer", 8));
            f.Hero.Knowledge = 1;
            f.Hero.Mana = f.Hero.MaxMana = 10;
            f.Player.Gold = 20000;
            f.Player.Wood = f.Player.Ore = f.Player.Mercury = f.Player.Sulfur = f.Player.Crystal = f.Player.Gems = 20;
            f.Town.Name = "Steadwick";
            f.Town.BuildingsJson = Strings("villageHall", "fort", "tavern", "dwelling1", "dwelling2");
            f.Town.AvailableJson = "{\"1\":14,\"2\":9}";
            f.Town.TavernJson = Strings("sorsha", "catherine");
            f.Game.TurnLimit = 0;
            f.Game.FragmentsJson = Strings(Enumerable.Range(1, 10).Select(i => $"fragment {i}").ToArray());
            f.Game.GrailX = f.Game.GrailY = 5;
            var gate = f.Map.At(f.Town.X, f.Town.Y);
            gate.ObjectKind = "town";
            gate.ObjectData = f.Town.Id.ToString();
            gate.Passable = false;
            Footprint.Stamp(f.Map, f.Town.X, f.Town.Y, "town", null);
            await f.SaveMap();
            var status = "PASS";
            var detail = "";
            try { await test(f); }
            catch (Exception e) { status = "FAIL"; detail = e.Message; failures++; }
            results.Add(new { id, action, expected, kind, status, detail });
            Console.WriteLine($"{status} {id}: {action}{(detail.Length > 0 ? " — " + detail : "")}");
            current = null;
        }

        async Task<JsonElement> Request(string method, string path, object? body = null, int status = 200, string player = "alice")
        {
            using var request = new HttpRequestMessage(new HttpMethod(method), path);
            request.Headers.Add("X-Player-Id", player);
            if (body != null) request.Content = JsonContent.Create(body);
            using var response = await client.SendAsync(request);
            var text = await response.Content.ReadAsStringAsync();
            Require((int)response.StatusCode == status, $"{method} {path}: expected {status}, got {(int)response.StatusCode}: {text}");
            return text.Length == 0 ? default : JsonSerializer.Deserialize<JsonElement>(text);
        }
        Task<JsonElement> Post(string path, object? body = null, int status = 200, string player = "alice") => Request("POST", path, body, status, player);
        async Task<int> ConcurrentPost(string path, object body, string player)
        {
            using var request = new HttpRequestMessage(HttpMethod.Post, path) { Content = JsonContent.Create(body) };
            request.Headers.Add("X-Player-Id", player);
            using var response = await client.SendAsync(request);
            return (int)response.StatusCode;
        }
        async Task<Hero> Hero(Fixture f) => await f.Db.Heroes.AsNoTracking().SingleAsync(h => h.Id == f.Hero.Id);
        async Task<Player> Player(Fixture f) => await f.Db.Players.AsNoTracking().SingleAsync(p => p.Id == f.Player.Id);
        async Task<Town> Town(Fixture f) => await f.Db.Towns.AsNoTracking().SingleAsync(t => t.Id == f.Town.Id);
        async Task MoveObject(Fixture f, string kind, string? data, bool passable = true)
        {
            f.Hero.X = 2; f.Hero.Y = 3;
            var tile = f.Map.At(2, 4);
            tile.ObjectKind = kind; tile.ObjectData = data; tile.Passable = passable; tile.Owner = 0;
            await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 4 } } });
        }
        async Task Tick(Fixture f) => await Post($"/api/games/{f.Game.Id}/endturn");

        string ArtifactKey(int id) => Enumerable.Range(0, 60).Select(n => $"artifact{n}").Single(k => Artifacts.IdOf(k) == id);
        Hero Companion(Fixture f, string army) => new() { GameId = f.Game.Id, PlayerId = f.Player.Id,
            Name = "Sorsha", HeroKey = "sorsha", Faction = "castle", X = 3, Y = 2,
            ArmyJson = army, Knowledge = 1, Mana = 10, MaxMana = 10 };

        await Case("spell-native-metadata", "Use original utility spell schools, levels and mana costs",
            "The original seventy spell definitions are retained as metadata, but only the eight preserved/adapted utility effects are castable.", async f =>
        {
            Require(Spells.All.Count == 70 && Spells.All.Values.Count(s => s.Adventure) == 8
                && Spells.All["townPortal"].School == "earth" && Spells.All["dimensionDoor"].School == "air"
                && Spells.All["fireball"].Level == 3 && Spells.All["implosion"].Level == 5 && Spells.All["resurrection"].Level == 4,
                "The catalogue contains incorrect levels or schools.");
            foreach (var (key, plain, trained) in new[] { ("haste", 6, 5), ("cure", 6, 5), ("visions", 4, 2), ("townPortal", 16, 12), ("dimensionDoor", 25, 20), ("fly", 20, 15) })
            {
                var spell = Spells.All[key];
                Require(Spells.ManaCost(spell, new Dictionary<string, int>()) == plain
                    && Spells.ManaCost(spell, new Dictionary<string, int> { ["air"] = 3, ["earth"] = 3, ["water"] = 3, ["fire"] = 3 }) == trained, $"{key} uses an invented mana cost.");
            }
            foreach (var faction in new[] { "castle", "rampart", "necropolis" })
            for (var level = 1; level <= 5; level++)
            {
                var stocked = Spells.ForGuildLevel(level, new Random(level), faction);
                var expected = faction == "castle" && level == 5 ? 0 : 6 - level;
                Require(stocked.Length == expected && stocked.Distinct().Count() == stocked.Length
                    && stocked.All(key => Spells.All[key].Level == level),
                    $"{faction} guild {level}: expected {expected} eligible spells, received {stocked.Length}.");
            }
            var catalogue = await Request("GET", "/api/spells");
            Require(catalogue.GetArrayLength() == 70
                && catalogue.EnumerateArray().Count(s => s.GetProperty("adventure").GetBoolean()) == 8
                && catalogue.EnumerateArray().All(s => s.GetProperty("manaCosts").GetArrayLength() == 4 && s.GetProperty("id").GetInt32() >= 0),
                "The HTTP catalogue omitted the native spellbook metadata.");
        });
        await Case("spell-removed", "Reject every unsupported or combat-only spell before payment",
            "Retained legacy spells cannot become paid no-op casts in a peaceful game.", async f =>
        {
            f.Hero.SpellsJson = Strings(Spells.All.Keys.ToArray()); TownRules.AddGear(f.Hero, TownRules.Spellbook);
            f.Hero.Mana = 100; await f.SaveMap();
            foreach (var spell in Spells.All.Values.Where(s => !s.Adventure))
                await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = spell.Key }, 400);
            Require((await Hero(f)).Mana == 100 && (await Hero(f)).Movement == 1500, "A removed spell charged mana or movement.");
        }, "Remove");
        foreach (var mastery in new[] { 0, 1, 2, 3 })
            await Case($"spell-portal-{mastery}", $"Town Portal at Earth mastery {mastery}",
                "No/Basic chooses the nearest own town; Advanced/Expert requires an explicit own town; movement is 300/200 and mana is 16/12.", async f =>
            {
                var far = new Town { GameId = f.Game.Id, PlayerId = f.Player.Id, X = 5, Y = 5, Name = "Far town" };
                f.Db.Towns.Add(far); f.Hero.X = 2; f.Hero.Y = 3;
                f.Hero.SkillsJson = JsonSerializer.Serialize(new { earth = mastery }); f.Hero.SpellsJson = Strings("townPortal");
                f.Hero.Mana = 100; TownRules.AddGear(f.Hero, TownRules.Spellbook); await f.SaveMap();
                if (mastery >= 2) await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "townPortal" }, 400);
                await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "townPortal", townId = mastery >= 2 ? far.Id : (Guid?)null });
                var h = await Hero(f);
                Require(h.X == (mastery >= 2 ? 5 : 2) && h.Y == (mastery >= 2 ? 5 : 2)
                    && h.Movement == (mastery == 3 ? 1300 : 1200) && h.Mana == (mastery == 0 ? 84 : 88), "Town Portal ignored destination, mastery or remaining movement.");
                var player = await Player(f);
                Require(GameService.ParseFog(player.FogJson, 49)[h.Y * 7 + h.X] == 2, "Teleport did not refresh the hero's actual sight.");
            });
        await Case("spell-portal-guards", "Reject an occupied nearest town and insufficient portal movement",
            "Basic Town Portal does not silently skip an occupied closest town or appear beside its gate; rejection costs nothing.", async f =>
        {
            var other = Companion(f, Army(("griffin", 1))); other.X = other.Y = 2;
            f.Db.Heroes.Add(other); f.Db.Towns.Add(new Town { GameId = f.Game.Id, PlayerId = f.Player.Id, X = 5, Y = 5 });
            f.Hero.X = 2; f.Hero.Y = 3; f.Hero.SpellsJson = Strings("townPortal"); f.Hero.Mana = 100;
            TownRules.AddGear(f.Hero, TownRules.Spellbook); await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "townPortal" }, 400);
            f.Db.Heroes.Remove(other); f.Hero.Movement = 299; await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "townPortal" }, 400);
            Require((await Hero(f)).Mana == 100 && (await Hero(f)).X == 2 && (await Hero(f)).Y == 3, "A rejected portal moved or charged the hero.");
        });
        foreach (var mastery in new[] { 0, 1, 2, 3 })
            await Case($"spell-door-limit-{mastery}", $"Dimension Door daily limit at Air mastery {mastery}",
                "Allow one/two/three/four casts, charge 300/200 movement, reject excess casts without payment and reset tomorrow.", async f =>
            {
                f.Hero.SpellsJson = Strings("dimensionDoor"); f.Hero.Mana = 100;
                f.Hero.SkillsJson = JsonSerializer.Serialize(new { air = mastery }); TownRules.AddGear(f.Hero, TownRules.Spellbook); await f.SaveMap();
                var targets = new[] { (4, 4), (5, 4), (4, 5), (5, 5) };
                for (var n = 0; n <= mastery; n++)
                    await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "dimensionDoor", x = targets[n].Item1, y = targets[n].Item2 });
                var before = await Hero(f);
                Require(before.Movement == 1500 - (mastery + 1) * (mastery == 3 ? 200 : 300)
                    && GameService.Effects(before).DoorCasts == mastery + 1, "The daily limit or movement price is incorrect.");
                await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "dimensionDoor", x = 3, y = 5 }, 400);
                Require((await Hero(f)).Mana == before.Mana, "An excess cast spent mana.");
                await Tick(f);
                Require(GameService.Effects(await Hero(f)).DoorCasts == 0, "The daily cast counter did not reset.");
                await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "dimensionDoor", x = 3, y = 5 });
            });
        await Case("spell-door-range", "Enforce the original 9-by-8 teleport radius",
            "Reject a clear in-bounds point just outside either axis, accept the extreme legal corner, and allow the last positive movement point.", async f =>
        {
            f.Map.Width = f.Map.Height = 25; f.Map.Tiles = Enumerable.Range(0, 625).Select(_ => new MapTile()).ToList();
            f.Game.MapWidth = f.Game.MapHeight = 25; f.Hero.X = f.Hero.Y = 12;
            f.Hero.Movement = 1; f.Hero.Mana = 100; f.Hero.SpellsJson = Strings("dimensionDoor");
            TownRules.AddGear(f.Hero, TownRules.Spellbook); await f.SaveMap();
            foreach (var (x, y) in new[] { (22, 12), (12, 21) })
                await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "dimensionDoor", x, y }, 400);
            await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "dimensionDoor", x = 21, y = 20 });
            Require((await Hero(f)).Movement == 0 && (await Hero(f)).Mana == 75
                && (await Hero(f)).X == 21 && (await Hero(f)).Y == 20, "The legal range boundary or last-point cost is incorrect.");
        });
        foreach (var mastery in new[] { 0, 1, 2, 3 })
            await Case($"spell-visions-{mastery}", $"Reveal nearby details with Visions mastery {mastery}",
                "Show exact monsters, add hero armies at Advanced and garrisons at Expert; do not reveal unseen land and expire tomorrow.", async f =>
            {
                var bob = new Player { GameId = f.Game.Id, Faction = "rampart" }; f.Db.Players.Add(bob);
                var enemy = Companion(f, Army(("griffin", 7))); enemy.PlayerId = bob.Id; enemy.X = 4; enemy.Y = 3; f.Db.Heroes.Add(enemy);
                f.Hero.X = 2; f.Hero.Y = 3; f.Hero.Power = 1; f.Hero.Mana = 100;
                f.Hero.SpellsJson = Strings("visions"); f.Hero.SkillsJson = JsonSerializer.Serialize(new { earth = mastery });
                TownRules.AddGear(f.Hero, TownRules.Spellbook);
                f.Map.At(2, 4).ObjectKind = "monster"; f.Map.At(2, 4).ObjectData = "pikeman:3"; f.Map.At(2, 4).Passable = false;
                f.Map.At(6, 6).ObjectKind = "monster"; f.Map.At(6, 6).ObjectData = "angel:99";
                f.Town.GarrisonJson = Army(("archer", 4));
                var fog = Enumerable.Repeat((byte)2, 49).ToArray(); fog[48] = 0; f.Player.FogJson = GameService.SerializeFog(fog); await f.SaveMap();
                await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "visions" });
                var targets = await Request("GET", $"/api/heroes/{f.Hero.Id}/visions");
                var list = targets.EnumerateArray().ToArray();
                Require(list.Length == 1 + (mastery >= 2 ? 1 : 0) + (mastery >= 3 ? 1 : 0)
                    && list.Single(v => v.GetProperty("kind").GetString() == "monster").GetProperty("army")[0].GetProperty("count").GetInt32() == 3,
                    "Visions returned no useful exact data, ignored mastery or exposed an unseen target.");
                Require((await Hero(f)).Mana == (mastery == 0 ? 96 : 98) && (await Player(f)).FogJson == f.Player.FogJson, "Visions changed fog or charged the wrong school.");
                await Tick(f);
                Require((await Request("GET", $"/api/heroes/{f.Hero.Id}/visions")).GetArrayLength() == 0, "Visions lasted into the next day.");
            }, "Adapt");
        await Case("hero-scouting", "Apply native Scouting and only equipped vision artifacts",
            "Hero sight is five plus one/two/three from Scouting; Speculum and Spyglass add one each only while worn.", f =>
        {
            f.Hero.X = f.Hero.Y = 10;
            for (var mastery = 0; mastery <= 3; mastery++)
            {
                f.Hero.SkillsJson = JsonSerializer.Serialize(new { scouting = mastery });
                var fog = new byte[625]; GameService.RecomputeFog(fog, 25, 25, [f.Hero]);
                Require(fog[10 * 25 + 15 + mastery] == 2 && fog[10 * 25 + 16 + mastery] == 0, "Scouting does not match the native radius.");
            }
            f.Hero.SkillsJson = "{}"; f.Hero.ArtifactsJson = Strings(ArtifactKey(52), ArtifactKey(53));
            var equipped = new byte[625]; GameService.RecomputeFog(equipped, 25, 25, [f.Hero]);
            Require(equipped[10 * 25 + 17] == 2, "Equipped vision artifacts did not add sight.");
            Artifacts.Save(f.Hero, new(new() { ["misc1"] = ArtifactKey(52) }, [ArtifactKey(53)]));
            var backpack = new byte[625]; GameService.RecomputeFog(backpack, 25, 25, [f.Hero]);
            Require(backpack[10 * 25 + 16] == 2 && backpack[10 * 25 + 17] == 0, "A backpack artifact still added sight.");
            return Task.CompletedTask;
        });

        await Case("mine-share", "Let a mine owner share and revoke access",
            "Sharing leaves ownership unchanged, pays every city half, and revocation restores the owner's full yield.", async f =>
        {
            var bob = new Player { GameId = f.Game.Id, Faction = "rampart" }; f.Db.Players.Add(bob);
            f.Map.At(2, 4).ObjectKind = "mine"; f.Map.At(2, 4).ObjectData = "gold"; f.Map.At(2, 4).Owner = 1;
            f.Db.Towns.Remove(f.Town); f.Player.Gold = 0; await f.SaveMap();
            await Post($"/api/games/{f.Game.Id}/mine-sharing", new { x = 2, y = 4, shared = true }, 204);
            await Tick(f);
            Require((await Player(f)).Gold == 500 && (await f.Db.Players.AsNoTracking().SingleAsync(p => p.Id == bob.Id)).Gold == 500,
                "Sharing did not split the daily yield.");
            await Post($"/api/games/{f.Game.Id}/mine-sharing", new { x = 2, y = 4, shared = false }, 204);
            await Tick(f);
            Require((await Player(f)).Gold == 1500 && (await f.Db.Players.AsNoTracking().SingleAsync(p => p.Id == bob.Id)).Gold == 500,
                "Revocation did not restore exclusive full yield.");
            var map = JsonSerializer.Deserialize<AdventureMap>((await f.Db.Games.AsNoTracking().SingleAsync()).MapJson, Json)!;
            Require(map.At(2, 4).Owner == 1 && !map.At(2, 4).MineShared, "Sharing changed ownership or was not saved.");
        }, "Adapt");
        await Case("mine-half-yield", "Accumulate fractional rare-resource mine income",
            "Every two half-yields produce one spendable unit, with no rounding loss across revocation.", async f =>
        {
            var bob = new Player { GameId = f.Game.Id, Faction = "rampart" }; f.Db.Players.Add(bob);
            f.Player.Crystal = 0; f.Town.BuildingsJson = "[]";
            var mine = f.Map.At(2, 4); mine.ObjectKind = "mine"; mine.ObjectData = "crystal"; mine.Owner = 1; await f.SaveMap();
            await Post($"/api/games/{f.Game.Id}/mine-sharing", new { x = 2, y = 4, shared = true }, 204);
            await Tick(f);
            var first = await Player(f);
            Require(first.Crystal == 0 && GameService.ParseSkills(first.MineRemaindersJson)["crystal"] == 1, "The first half-unit was rounded away.");
            await Post($"/api/games/{f.Game.Id}/mine-sharing", new { x = 2, y = 4, shared = false }, 204);
            await Tick(f);
            await Post($"/api/games/{f.Game.Id}/mine-sharing", new { x = 2, y = 4, shared = true }, 204);
            await Tick(f);
            Require((await Player(f)).Crystal == 2 && (await f.Db.Players.AsNoTracking().SingleAsync(p => p.Id == bob.Id)).Crystal == 1
                && GameService.ParseSkills((await Player(f)).MineRemaindersJson)["crystal"] == 0, "A stored half-unit was lost, doubled or paid to the wrong city.");
        }, "Adapt");
        await Case("mine-share-guards", "Reject sharing by a visitor or after game end",
            "Only the owner can authorize sharing; unclaimed/non-mine coordinates are not authority.", async f =>
        {
            f.Db.Players.Add(new Player { GameId = f.Game.Id, Faction = "rampart" });
            var mine = f.Map.At(2, 4); mine.ObjectKind = "mine"; mine.ObjectData = "wood"; mine.Owner = 1; await f.SaveMap();
            await Post($"/api/games/{f.Game.Id}/mine-sharing", new { x = 2, y = 4, shared = true }, 400, "bob");
            await Post($"/api/games/{f.Game.Id}/mine-sharing", new { x = 1, y = 1, shared = true }, 400);
            await Post($"/api/games/{f.Game.Id}/mine-sharing", new { x = -1, y = 4, shared = true }, 400);
            f.Game.State = GameState.Finished; await f.SaveMap();
            await Post($"/api/games/{f.Game.Id}/mine-sharing", new { x = 2, y = 4, shared = true }, 400);
        }, "Adapt");
        await Case("endgame-agreement", "Require unanimous human agreement and allow withdrawal",
            "A single vote cannot end a three-player game; withdrawal persists and only the final unanimous vote ends it.", async f =>
        {
            var bob = new Player { GameId = f.Game.Id, Faction = "rampart" };
            var carol = new Player { GameId = f.Game.Id, Faction = "necropolis" }; f.Db.AddRange(bob, carol); await f.SaveMap();
            var path = $"/api/games/{f.Game.Id}/agreement";
            await Post(path, new { agree = true });
            await Post(path, new { agree = true }, 200, "bob");
            await Post(path, new { agree = false });
            await Post(path, new { agree = true }, 200, "carol");
            var pending = await f.Db.Games.AsNoTracking().SingleAsync();
            Require(pending.State == GameState.InProgress && GameService.ParseStringArray(pending.EndAgreementJson).Length == 2,
                "The game ended without the withdrawn vote.");
            await Post(path, new { agree = true });
            var ended = await f.Db.Games.AsNoTracking().SingleAsync();
            Require(ended.State == GameState.Finished && ended.CurrentTurn == 1, "Unanimous agreement did not end the game without advancing the day.");
            await Post(path, new { agree = false }, 400);
        }, "Adapt");
        await Case("endgame-ai-agreement", "Auto-consent computer seats after all humans agree",
            "Computers neither block human unanimity nor cast a human vote.", async f =>
        {
            f.Db.Players.Add(new Player { GameId = f.Game.Id, Faction = "rampart", IsAi = true }); await f.SaveMap();
            await Post($"/api/games/{f.Game.Id}/agreement", new { agree = true }, 400, "bob");
            await Post($"/api/games/{f.Game.Id}/agreement", new { agree = true });
            Require((await f.Db.Games.AsNoTracking().SingleAsync()).State == GameState.Finished, "The computer seat blocked a unanimous human agreement.");
        }, "Adapt");
        await Case("hero-level-choice", "Persist and explicitly choose a secondary skill",
            "The primary point is immediate; an unoffered skill is rejected and only one offered skill advances.", async f =>
        {
            f.Hero.SkillsJson = "{\"pathfinding\":1,\"leadership\":1}";
            GameService.GainExperience(f.Hero, 1000); await f.SaveMap();
            var pending = HeroProgression.Pending(f.Hero)!;
            Require(pending.Choices.Length == 2 && pending.Choices.Select(c => c.Key).Distinct().Count() == 2
                && pending.Choices.All(c => Skills.Adventure.Contains(c.Key)), "The offer is not two distinct supported skills.");
            await Post($"/api/heroes/{f.Hero.Id}/level-up", new { skill = "offence" }, 400);
            await Post($"/api/heroes/{f.Hero.Id}/level-up", new { skill = pending.Choices[0].Key });
            var saved = await Hero(f); var skills = GameService.ParseSkills(saved.SkillsJson);
            Require(saved.PendingLevelJson == null && skills[pending.Choices[0].Key] == pending.Choices[0].Level
                && skills["leadership"] == 1, "The chosen skill was not persisted or a retained combat skill was changed.");
            await Post($"/api/heroes/{f.Hero.Id}/level-up", new { skill = pending.Choices[0].Key }, 400);
        }, "Adapt");
        await Case("hero-level-queue", "Resolve a multi-level experience award in order",
            "Every level keeps its own primary point and persisted offer; later choices are not skipped.", async f =>
        {
            var primary = f.Hero.Attack + f.Hero.Defense + f.Hero.Power + f.Hero.Knowledge;
            GameService.GainExperience(f.Hero, 3200); await f.SaveMap();
            for (var level = 2; level <= 4; level++)
            {
                var h = await Hero(f); var pending = HeroProgression.Pending(h)!;
                Require(h.Level == level && pending.Level == level, "A pending level was skipped.");
                await Post($"/api/heroes/{f.Hero.Id}/level-up", new { skill = pending.Choices.FirstOrDefault()?.Key });
            }
            var saved = await Hero(f);
            Require(saved.Level == 4 && saved.PendingLevelJson == null
                && saved.Attack + saved.Defense + saved.Power + saved.Knowledge == primary + 3, "Multi-level choices duplicated or lost primary points.");
        });
        await Case("hero-level-cap", "Respect eight skills and retain old combat skills",
            "A full eight-skill hero only upgrades supported existing skills; legacy combat skills and the inactive Fire school receive no invented offers.", async f =>
        {
            f.Hero.SkillsJson = JsonSerializer.Serialize(Skills.Adventure.Take(8).ToDictionary(k => k, _ => 1));
            GameService.GainExperience(f.Hero, 1000);
            var pending = HeroProgression.Pending(f.Hero)!;
            Require(pending.Choices.All(c => c.Level == 2 && GameService.ParseSkills(f.Hero.SkillsJson).ContainsKey(c.Key)), "A ninth skill was offered.");
            f.Hero.Level = 1; f.Hero.Experience = 0; f.Hero.PendingLevelJson = null;
            var old = new[] { "archery", "leadership", "luck", "ballistics", "artillery", "offence", "armorer", "fire" };
            f.Hero.SkillsJson = JsonSerializer.Serialize(old.ToDictionary(k => k, _ => 1));
            GameService.GainExperience(f.Hero, 1000); await f.SaveMap();
            Require(HeroProgression.Pending(f.Hero)!.Choices.Length == 0, "An ineffective skill was offered.");
            await Post($"/api/heroes/{f.Hero.Id}/level-up", new { skill = (string?)null });
            var saved = await Hero(f);
            Require(old.All(k => GameService.ParseSkills(saved.SkillsJson)[k] == 1), "Legacy skill names or mastery were lost.");
        }, "Adapt");
        await Case("hero-native-progression", "Use original class profiles and experience thresholds",
            "Class-specific starting stats and native low/high probabilities replace Might/Magic placeholders; the exponential experience curve is retained.", f =>
        {
            Require(NativeRules.Data.Classes.Length == 18 && NativeRules.Data.Classes.All(c => c.Low.Sum() == 100 && c.High.Sum() == 100),
                "Native primary weights are incomplete.");
            var knight = NativeRules.Classes["knight"]; var cleric = NativeRules.Classes["cleric"];
            Require(knight.Low.SequenceEqual(new[] { 35, 45, 10, 10 }) && knight.High.SequenceEqual(new[] { 30, 30, 20, 20 })
                && cleric.Initial.SequenceEqual(new[] { 1, 0, 2, 2 }) && Heroes.All["clancy"].Might && !Heroes.All["vidomina"].Might,
                "Hero classes still use placeholder stats or the wrong class.");
            Require(Experience.NextThreshold(14) == 34140 && Experience.NextThreshold(15) == 40567
                && Experience.NextThreshold(16) == 48279, "Later experience thresholds do not follow the original curve.");
            return Task.CompletedTask;
        });
        await Case("hero-obligatory-offers", "Offer Wisdom and a magic school at the native class interval",
            "Knight Wisdom is offered by level six; magic classes receive a school by level three; obligatory offers never bypass a class's zero learning weight.", f =>
        {
            f.Hero.Level = 5; f.Hero.SkillsJson = "{\"pathfinding\":1,\"_wisdomOffer\":5}";
            GameService.GainExperience(f.Hero, 6200);
            Require(HeroProgression.Pending(f.Hero)!.Choices.Any(c => c.Key == "wisdom"), "The Knight did not receive its obligatory Wisdom offer.");
            f.Hero.HeroKey = "adela"; f.Hero.Level = 2; f.Hero.Experience = 0; f.Hero.PendingLevelJson = null;
            f.Hero.SkillsJson = "{\"wisdom\":3,\"_schoolOffer\":2}"; GameService.GainExperience(f.Hero, 2000);
            Require(HeroProgression.Pending(f.Hero)!.Choices.Any(c => c.Key is "air" or "earth" or "fire" or "water"), "The Cleric did not receive its obligatory magic-school offer.");
            f.Hero.HeroKey = "crag"; f.Hero.Level = 1; f.Hero.Experience = 0; f.Hero.PendingLevelJson = null;
            f.Hero.SkillsJson = "{\"air\":3,\"earth\":3,\"fire\":3,\"wisdom\":3,\"logistics\":3,\"scouting\":3,\"intelligence\":3,\"_schoolOffer\":3}";
            GameService.GainExperience(f.Hero, 1000);
            Require(HeroProgression.Pending(f.Hero)!.Choices.All(c => c.Key != "water"
                && NativeRules.ClassOf(f.Hero).Secondary.GetValueOrDefault(c.Key) > 0),
                "The Barbarian was offered a school with zero native learning probability.");
            return Task.CompletedTask;
        });
        await Case("hero-learning-estates", "Apply preserved Learning and Estates effects",
            "Learning grants five percent per mastery and Estates pays 125/250/500 daily.", async f =>
        {
            f.Hero.SkillsJson = "{\"learning\":3,\"estates\":3}";
            GameService.GainExperience(f.Hero, 100); await f.SaveMap();
            Require(f.Hero.Experience == 115, "Expert Learning did not grant fifteen percent.");
            var before = f.Player.Gold; f.Db.Towns.Remove(f.Town); await f.SaveMap(); await Tick(f);
            Require((await Player(f)).Gold == before + 500, "Expert Estates did not pay five hundred gold.");
        });

        await Case("army-heroes", "Move, split, merge and swap armies between meeting heroes",
            "Counts are conserved in both directions; both heroes retain an army.", async f =>
        {
            var other = Companion(f, Army(("griffin", 7))); f.Db.Heroes.Add(other); await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/exchange", new { otherHeroId = other.Id, fromSlot = 0, toSlot = 1, count = 5 });
            await Post($"/api/heroes/{f.Hero.Id}/exchange", new { otherHeroId = other.Id, fromSlot = 0, toSlot = 1, count = 0 });
            await Post($"/api/heroes/{f.Hero.Id}/exchange", new { otherHeroId = other.Id, fromSlot = 1, toSlot = 0 });
            var faster = await Hero(f);
            Require(faster.MaxMovement == 1700 && faster.Movement == 1500,
                "Exchanging away the slowest stack did not update future movement without refilling today's points.");
            await Post($"/api/heroes/{other.Id}/exchange", new { otherHeroId = f.Hero.Id, fromSlot = 1, toSlot = 0, count = 4 });
            var a = GameService.ParseArmy((await Hero(f)).ArmyJson);
            var b = GameService.ParseArmy((await f.Db.Heroes.AsNoTracking().SingleAsync(h => h.Id == other.Id)).ArmyJson);
            Require(a[0] == new GameService.ArmySlot("pikeman", 4) && a[1] == new GameService.ArmySlot("griffin", 7)
                && b[0] == new GameService.ArmySlot("archer", 8) && b[1] == new GameService.ArmySlot("pikeman", 21),
                "The exchange lost creatures or used the wrong slots.");
        });
        await Case("army-heroes-guards", "Reject remote, foreign and last-stack hero transfers",
            "Only adjacent own heroes can exchange; invalid transfers are atomic.", async f =>
        {
            var other = Companion(f, Army(("griffin", 7))); f.Db.Heroes.Add(other);
            f.Hero.ArmyJson = Army(("pikeman", 5)); await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/exchange", new { otherHeroId = other.Id, fromSlot = 0, toSlot = 1 }, 400);
            await Post($"/api/heroes/{f.Hero.Id}/exchange", new { otherHeroId = other.Id, fromSlot = 0, toSlot = 0, count = -1 }, 400);
            other.X = 5; await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/exchange", new { otherHeroId = other.Id }, 400);
            other.X = 3; other.PlayerId = Guid.NewGuid(); await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/exchange", new { otherHeroId = other.Id }, 400);
            Require(GameService.ParseArmy((await Hero(f)).ArmyJson)[0].Count == 5, "A rejected exchange changed an army.");
        });
        await Case("hero-scholar", "Share spells when owned heroes meet",
            "Scholar respects its level, the recipient's Wisdom and spellbooks; learning works both ways.", async f =>
        {
            var other = Companion(f, Army(("griffin", 7)));
            f.Hero.SkillsJson = "{\"scholar\":3}";
            f.Hero.ArtifactsJson = other.ArtifactsJson = Strings("spellBook");
            f.Hero.SpellsJson = Strings("townPortal", "magicArrow");
            other.SpellsJson = Strings("viewAir"); f.Db.Heroes.Add(other); await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/exchange", new { otherHeroId = other.Id });
            var recipient = await f.Db.Heroes.AsNoTracking().SingleAsync(h => h.Id == other.Id);
            Require(GameService.ParseStringArray(recipient.SpellsJson).Contains("magicArrow")
                && !GameService.ParseStringArray(recipient.SpellsJson).Contains("townPortal")
                && GameService.ParseStringArray((await Hero(f)).SpellsJson).Contains("viewAir"), "Scholar ignored Wisdom or did not teach in both directions.");
            await f.Db.Entry(other).ReloadAsync(); other.SkillsJson = "{\"wisdom\":2}"; await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/exchange", new { otherHeroId = other.Id });
            Require(GameService.ParseStringArray((await f.Db.Heroes.AsNoTracking().SingleAsync(h => h.Id == other.Id)).SpellsJson).Contains("townPortal"),
                "Advanced Wisdom did not permit a level-four Scholar spell.");
        });
        await Case("army-dismiss", "Confirm troop dismissal without emptying a hero",
            "Unconfirmed, foreign and last-stack dismissal fails; confirmed dismissal removes only its stack.", async f =>
        {
            f.Hero.ArmyJson = Army(("angel", 25), ("dwarf", 8)); await f.SaveMap();
            var path = $"/api/heroes/{f.Hero.Id}/dismiss-stack";
            await Post(path, new { slot = 1, confirmed = false }, 400);
            await Post(path, new { slot = 1, confirmed = true }, 400, "bob");
            await Post(path, new { slot = 1, confirmed = true });
            await Post(path, new { slot = 0, confirmed = true }, 400);
            var army = GameService.ParseArmy((await Hero(f)).ArmyJson);
            Require(army[0].Count == 25 && army[1].Creature == null, "Dismissal removed the wrong stack.");
            var faster = await Hero(f);
            Require(faster.MaxMovement == 2000 && faster.Movement == 1500,
                "Dismissing the slowest stack did not refresh the movement maximum without restoring points.");
        });
        await Case("hero-dismiss", "Confirm hero dismissal and guard a townless final hero",
            "A hero may be dismissed while a town remains, but not without confirmation or as the last townless hero.", async f =>
        {
            var path = $"/api/heroes/{f.Hero.Id}/dismiss";
            await Post(path, new { confirmed = false }, 400);
            f.Db.Towns.Remove(f.Town); await f.SaveMap();
            await Post(path, new { confirmed = true }, 400);
            var other = Companion(f, Army(("griffin", 7))); f.Db.Heroes.Add(other); await f.SaveMap();
            await Post(path, new { confirmed = true }, 204);
            Require(!await f.Db.Heroes.AsNoTracking().AnyAsync(h => h.Id == f.Hero.Id)
                && await f.Db.Heroes.CountAsync() == 1, "Hero dismissal did not preserve the other hero.");
        });
        await Case("artifact-equipment", "Equip, unequip and swap persistent artifact slots",
            "Only worn artifacts apply bonuses; duplicates and reserved gear survive; reducing max mana does not erase stored mana.", async f =>
        {
            var head = ArtifactKey(19); var weapon = ArtifactKey(7);
            f.Hero.ArtifactsJson = Strings("spellBook", head, weapon, weapon);
            f.Hero.Mana = 25; f.Hero.MaxMana = 20; await f.SaveMap();
            var path = $"/api/heroes/{f.Hero.Id}/artifacts";
            await Post(path, new { from = "head", to = "rightHand" }, 400);
            var result = await Post(path, new { from = "head", to = "backpack:1" });
            var dto = result.GetProperty("hero");
            Require(dto.GetProperty("knowledge").GetInt32() == 1 && dto.GetProperty("maxMana").GetInt32() == 10
                && dto.GetProperty("mana").GetInt32() == 25 && dto.GetProperty("spellbook").GetBoolean(), "Backpack artifact applied a bonus or mana/gear was lost.");
            var again = await Request("GET", $"/api/heroes/{f.Hero.Id}");
            Require(!again.GetProperty("worn").TryGetProperty("head", out _), "Reading the hero auto-equipped an explicitly unequipped artifact.");
            await Post(path, new { from = "backpack:1", to = "head" });
            var saved = await Hero(f);
            Require(saved.MaxMana == 20 && TownRules.Artifacts(saved).Count(k => k == weapon) == 2, "Re-equipping lost duplicates or did not restore maximum mana.");
            await Post(path, new { from = "rightHand", to = "backpack:0" });
            Require(TownRules.Artifacts(await Hero(f)).Length == 3, "Swapping duplicate keys deleted an artifact.");
        });
        await Case("artifact-exchange", "Transfer and swap artifacts between meeting heroes",
            "Both inventories and bonuses update atomically; remote or foreign heroes cannot receive artifacts.", async f =>
        {
            var other = Companion(f, Army(("griffin", 7))); f.Db.Heroes.Add(other);
            f.Hero.ArtifactsJson = Strings(ArtifactKey(19));
            other.ArtifactsJson = Strings(ArtifactKey(20)); await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/artifacts", new { from = "head", to = "head", toHeroId = other.Id });
            var one = await Hero(f); var two = await f.Db.Heroes.AsNoTracking().SingleAsync(h => h.Id == other.Id);
            Require(Artifacts.Layout(one).Worn["head"] == ArtifactKey(20) && Artifacts.Layout(two).Worn["head"] == ArtifactKey(19)
                && one.MaxMana == 30 && two.MaxMana == 20, "Exchanged artifacts did not swap or recalculate bonuses.");
            await f.Db.Entry(other).ReloadAsync(); other.X = 5; await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/artifacts", new { from = "head", to = "backpack:0", toHeroId = other.Id }, 400);
        });
        await Case("artifact-backpack", "Reorder a backpack and preserve manual layout on pickup",
            "Backpack positions persist, an unavailable slot is rejected, and a full backpack cannot consume a map artifact.", async f =>
        {
            var weapon = ArtifactKey(7); var otherWeapon = ArtifactKey(8);
            f.Hero.ArtifactsJson = Strings(weapon, weapon, otherWeapon); await f.SaveMap();
            var path = $"/api/heroes/{f.Hero.Id}/artifacts";
            await Post(path, new { from = "backpack:0", to = "backpack:1" });
            Require(Artifacts.Layout(await Hero(f)).Backpack.SequenceEqual(new[] { otherWeapon, weapon }), "Backpack order did not persist.");
            await Post(path, new { from = "backpack:1", to = "backpack:3" }, 400);
            await f.Db.Entry(f.Hero).ReloadAsync();
            f.Hero.ArtifactsJson = Strings(Enumerable.Repeat(weapon, 65).ToArray());
            f.Hero.ArtifactLayoutJson = "{}"; await f.SaveMap();
            await MoveObject(f, "artifact", otherWeapon);
            Require((await Hero(f)).ArtifactsJson == Strings(Enumerable.Repeat(weapon, 65).ToArray())
                && JsonSerializer.Deserialize<AdventureMap>((await f.Db.Games.AsNoTracking().SingleAsync()).MapJson, Json)!.At(2, 4).ObjectKind == "artifact",
                "A full backpack lost or consumed the pickup.");
        });

        await Case("social-endgame", "Reject pending economics after the game ends", "Trade responses, project contributions and pending creature actions cannot change finished games.", async f =>
        {
            var bob = new Player { GameId = f.Game.Id, Faction = "rampart", Gold = 5000, Ore = 10 };
            var offer = new TradeOffer { GameId = f.Game.Id, FromPlayerId = f.Player.Id, ToPlayerId = bob.Id,
                GiveResource = "gold", GiveAmount = 100, WantResource = "ore", WantAmount = 1 };
            var project = new SharedProject { GameId = f.Game.Id, Name = "Finished project", RequiredGold = 100 };
            f.Db.AddRange(bob, offer, project);
            await MoveObject(f, "monster", "pikeman:3", false);
            f.Game.State = GameState.Finished; await f.SaveMap();
            await Post($"/api/trades/{offer.Id}/respond", new { accept = true }, 400, "bob");
            await Post($"/api/projects/{project.Id}/contribute", new { gold = 100, wood = 0, ore = 0 }, 400);
            foreach (var action in new[] { "observe", "recruit", "pass" })
                await Post($"/api/heroes/{f.Hero.Id}/encounter", new { action }, 400);
            await Post($"/api/games/{f.Game.Id}/trades", new { toPlayerId = bob.Id.ToString(), give = "gold", giveAmount = 1, want = "ore", wantAmount = 1 }, 400);
            Require((await Player(f)).Gold == 20000 && (await Hero(f)).PendingEncounterJson != null
                && (await f.Db.TradeOffers.AsNoTracking().SingleAsync()).Status == "pending"
                && (await f.Db.SharedProjects.AsNoTracking().SingleAsync()).ContributedGold == 0, "A finished game accepted a pending mutation.");
        }, "Adapt");
        await Case("social-concurrent-trade", "Accept one offer concurrently twice", "Exactly one acceptance transfers resources; a replay cannot duplicate either side.", async f =>
        {
            var bob = new Player { GameId = f.Game.Id, Faction = "rampart", Ore = 10 };
            var offer = new TradeOffer { GameId = f.Game.Id, FromPlayerId = f.Player.Id, ToPlayerId = bob.Id,
                GiveResource = "gold", GiveAmount = 100, WantResource = "ore", WantAmount = 1 };
            f.Db.AddRange(bob, offer); await f.SaveMap();
            var statuses = await Task.WhenAll(Enumerable.Range(0, 2).Select(_ => ConcurrentPost($"/api/trades/{offer.Id}/respond", new { accept = true }, "bob")));
            Require(statuses.Order().SequenceEqual(new[] { 204, 400 }), $"Concurrent accepts returned {string.Join(',', statuses)}.");
            var recipient = await f.Db.Players.AsNoTracking().SingleAsync(p => p.Id == bob.Id);
            Require((await Player(f)).Gold == 19900 && (await Player(f)).Ore == 21 && recipient.Gold == 100 && recipient.Ore == 9,
                "Concurrent acceptance duplicated or lost resources.");
        }, "Adapt");
        await Case("social-concurrent-project", "Complete a project with concurrent contributions", "Only the contribution completing the project is charged; the completed project cannot be funded twice.", async f =>
        {
            var bob = new Player { GameId = f.Game.Id, Faction = "rampart", Gold = 2000 };
            f.Player.Gold = 2000;
            var project = new SharedProject { GameId = f.Game.Id, Name = "Concurrent project", RequiredGold = 100 };
            f.Db.AddRange(bob, project); await f.SaveMap();
            var statuses = await Task.WhenAll(new[] { "alice", "bob" }.Select(player =>
                ConcurrentPost($"/api/projects/{project.Id}/contribute", new { gold = 100, wood = 0, ore = 0 }, player)));
            Require(statuses.Order().SequenceEqual(new[] { 204, 400 }), $"Concurrent funding returned {string.Join(',', statuses)}.");
            var saved = await f.Db.SharedProjects.AsNoTracking().SingleAsync();
            Require(saved.Completed && saved.ContributedGold == 100
                && await f.Db.Players.SumAsync(p => p.Gold) == 3900
                && GameService.ParseStringArray(saved.ContributorsJson).Length == 1, "Project charged twice or lost a contribution.");
        }, "Adapt");
        await Case("social-concurrent-claim", "Visit one neutral mine concurrently", "The first visitor claims it; the second cannot overwrite ownership or gain a second full yield.", async f =>
        {
            var bob = new Player { GameId = f.Game.Id, Faction = "rampart" };
            var visitor = new Hero { GameId = f.Game.Id, PlayerId = bob.Id, Faction = "rampart",
                X = 2, Y = 5, Movement = 1500, MaxMovement = 1500, ArmyJson = Army(("pikeman", 5)) };
            f.Db.AddRange(bob, visitor);
            f.Hero.X = 2; f.Hero.Y = 3;
            var mine = f.Map.At(2, 4); mine.ObjectKind = "mine"; mine.ObjectData = "gold"; mine.Owner = 0; mine.Passable = false;
            await f.SaveMap();
            var move = new { path = new[] { new { x = 2, y = 4 } } };
            var statuses = await Task.WhenAll(ConcurrentPost($"/api/heroes/{f.Hero.Id}/move", move, "alice"),
                ConcurrentPost($"/api/heroes/{visitor.Id}/move", move, "bob"));
            Require(statuses.All(s => s == 200), $"Concurrent visits returned {string.Join(',', statuses)}.");
            var map = JsonSerializer.Deserialize<AdventureMap>((await f.Db.Games.AsNoTracking().SingleAsync()).MapJson, Json)!;
            var owner = map.At(2, 4).Owner;
            Require(owner is 1 or 2, "Concurrent visits created shared or unowned ownership.");
            await Tick(f);
            Require((await Player(f)).Gold == 20500 + (owner == 1 ? 1000 : 0)
                && (await f.Db.Players.AsNoTracking().SingleAsync(p => p.Id == bob.Id)).Gold == (owner == 2 ? 1000 : 0),
                "Concurrent mine visits awarded duplicate daily income.");
        }, "Adapt");
        await Case("game-join", "Enter an existing seeded-player game", "Repeated entry returns the same seat; unknown players are rejected without adding seats.", async f =>
        {
            for (var i = 0; i < 2; i++)
            {
                var joined = await Post($"/api/games/{f.Game.Id}/join");
                Require(joined.GetProperty("playerId").GetGuid() == f.Player.Id, "Entry returned a different seat.");
            }
            await Post($"/api/games/{f.Game.Id}/join", null, 400, "unknown");
            Require(await f.Db.Players.CountAsync() == 1, "Entry duplicated a seeded-player seat.");
        }, "Adapt");
        await Case("recruit-base", "Recruit a base creature", "Exact gold cost and weekly-pool debit; creatures join the garrison.", async f =>
        {
            await Post($"/api/towns/{f.Town.Id}/recruit", new { tier = 1, count = 3 });
            Require((await Player(f)).Gold == 19820, "3 Pikemen must cost 180 gold.");
            var town = await Town(f);
            Require(GameService.ParseArmy(town.GarrisonJson)[0] == new GameService.ArmySlot("pikeman", 3), "Recruits must join the garrison.");
            Require(JsonSerializer.Deserialize<Dictionary<string, int>>(town.AvailableJson)!["1"] == 11, "Pool must fall by exactly 3.");
        });
        await Case("recruit-upgraded", "Recruit base and upgraded forms", "An upgraded dwelling offers both forms from the same pool.", async f =>
        {
            f.Town.BuildingsJson = Strings("villageHall", "fort", "dwelling1", "upg1"); await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/recruit", new { tier = 1, count = 3, creature = "halberdier" });
            await Post($"/api/towns/{f.Town.Id}/recruit", new { tier = 1, count = 2, creature = "pikeman" });
            Require((await Player(f)).Gold == 19655, "3 Halberdiers plus 2 Pikemen cost 345 gold.");
            Require(JsonSerializer.Deserialize<Dictionary<string, int>>((await Town(f)).AvailableJson)!["1"] == 9, "Both forms must use the same pool.");
        });
        await Case("recruit-locked", "Recruit from an unbuilt dwelling", "Reject unbuilt dwellings and unavailable upgrades without charging.", async f =>
        {
            await Post($"/api/towns/{f.Town.Id}/recruit", new { tier = 7, count = 1 }, 400);
            await Post($"/api/towns/{f.Town.Id}/recruit", new { tier = 1, count = 1, creature = "halberdier" }, 400);
            Require((await Player(f)).Gold == 20000, "Rejected recruitment charged resources.");
        });
        await Case("recruit-capacity", "Recruit with full armies", "Use a matching stack or empty slot; reject when neither army has room, with no charge.", async f =>
        {
            var full = Army(("archer", 1), ("griffin", 1), ("swordsman", 1), ("monk", 1), ("cavalier", 1), ("angel", 1), ("halberdier", 1));
            f.Town.GarrisonJson = full; f.Hero.ArmyJson = full; await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/recruit", new { tier = 1, count = 1 }, 400);
            Require((await Player(f)).Gold == 20000 && (await Town(f)).GarrisonJson == full, "A rejected recruit changed resources or stacks.");
        });
        await Case("recruit-fallback", "Recruit with a full garrison", "A visiting hero with capacity receives recruits only when the garrison has no room.", async f =>
        {
            f.Town.GarrisonJson = Army(("archer", 1), ("griffin", 1), ("swordsman", 1), ("monk", 1), ("cavalier", 1), ("angel", 1), ("halberdier", 1));
            await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/recruit", new { tier = 1, count = 2 });
            Require(GameService.ParseArmy((await Hero(f)).ArmyJson)[0].Count == 27, "Fallback recruits did not reach the visitor.");
        });
        await Case("recruit-amount", "Recruit an exact requested quantity", "Reject zero, negative, and over-pool quantities; never silently change the purchase.", async f =>
        {
            foreach (var count in new[] { 0, -1, 15 })
                await Post($"/api/towns/{f.Town.Id}/recruit", new { tier = 1, count }, 400);
            Require((await Player(f)).Gold == 20000, "Invalid recruitment changed gold.");
        });
        await Case("recruit-poor", "Recruit without sufficient resources", "Reject an unaffordable purchase with no pool or army change.", async f =>
        {
            f.Player.Gold = 59; await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/recruit", new { tier = 1, count = 1 }, 400);
            var town = await Town(f);
            Require((await Player(f)).Gold == 59 && town.GarrisonJson == "[]" && town.AvailableJson == f.Town.AvailableJson, "Unaffordable purchase mutated state.");
        });
        await Case("town-ownership", "Use another player's town", "Peaceful visitors cannot recruit, build, exchange armies, upgrade or buy the owner's gear.", async f =>
        {
            foreach (var (action, body) in new (string, object)[] {
                ("recruit", new { tier = 1, count = 1 }), ("build", new { buildingKey = "townHall" }),
                ("army", new { fromArmy = "garrison", fromSlot = 0, toArmy = "hero", toSlot = 0 }),
                ("upgrade", new { army = "garrison", slot = 0 }), ("blacksmith", new { }), ("spellbook", new { }) })
                await Post($"/api/towns/{f.Town.Id}/{action}", body, 400, "bob");
        }, "Adapt");
        foreach (var operation in new[] { "move", "merge", "swap", "split" })
            await Case($"army-{operation}", $"Army {operation}", "Move, combine, swap or split stacks without losing or creating creatures.", async f =>
            {
                f.Town.GarrisonJson = operation switch {
                    "merge" => Army(("pikeman", 5)), "swap" => Army(("griffin", 5)), _ => "[]" };
                await f.SaveMap();
                await Post($"/api/towns/{f.Town.Id}/army", new { fromArmy = "hero", fromSlot = 0, toArmy = "garrison", toSlot = 0, count = operation == "split" ? 7 : 0 });
                var h = GameService.ParseArmy((await Hero(f)).ArmyJson);
                var g = GameService.ParseArmy((await Town(f)).GarrisonJson);
                Require(g[0].Creature == "pikeman" && g[0].Count == (operation == "merge" ? 30 : operation == "split" ? 7 : 25), "Destination stack is incorrect.");
                Require(operation == "swap" ? h[0] == new GameService.ArmySlot("griffin", 5) : operation == "split" ? h[0].Count == 18 : h[0].Creature == null, "Source stack is incorrect.");
            });
        await Case("army-last", "Keep a hero's last troop", "Reject removal of the final troop and leave both armies unchanged.", async f =>
        {
            f.Hero.ArmyJson = Army(("pikeman", 1)); await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/army", new { fromArmy = "hero", fromSlot = 0, toArmy = "garrison", toSlot = 0 }, 400);
            Require((await Hero(f)).ArmyJson == f.Hero.ArmyJson && (await Town(f)).GarrisonJson == "[]", "Rejected transfer changed an army.");
        });
        await Case("army-reject-atomic", "Reject an illegal army transfer atomically", "The shared transfer rule must not mutate either army when it returns an error.", f =>
        {
            var from = GameService.ParseArmy(Army(("pikeman", 1))); var to = GameService.ParseArmy("[]");
            var error = TownRules.Transfer(from, 0, to, 0, 0, true, false);
            Require(error != null && from[0].Count == 1 && to[0].Creature == null, "Transfer mutated arrays before rejecting the last troop.");
            return Task.CompletedTask;
        });
        await Case("army-away", "Exchange troops while away from town", "A hero must physically be in town to exchange troops with its garrison.", async f =>
        {
            f.Hero.X = 6; f.Hero.Y = 6; await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/army", new { fromArmy = "hero", fromSlot = 0, toArmy = "garrison", toSlot = 0 }, 400);
        });
        await Case("army-rearrange", "Rearrange a hero's own army", "Moving or splitting within the seven slots preserves total creatures.", async f =>
        {
            await Post($"/api/heroes/{f.Hero.Id}/army", new { fromArmy = "hero", fromSlot = 0, toArmy = "hero", toSlot = 6, count = 4 });
            var a = GameService.ParseArmy((await Hero(f)).ArmyJson);
            Require(a[0].Count == 21 && a[6] == new GameService.ArmySlot("pikeman", 4), "Hero split is incorrect.");
        });
        await Case("upgrade-price", "Upgrade a creature stack", "The upgraded dwelling is required; charge cost difference times the entire stack.", async f =>
        {
            f.Town.BuildingsJson = Strings("villageHall", "fort", "dwelling1", "upg1"); await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/upgrade", new { army = "hero", slot = 0 });
            Require(GameService.ParseArmy((await Hero(f)).ArmyJson)[0] == new GameService.ArmySlot("halberdier", 25), "Wrong upgraded stack.");
            Require((await Player(f)).Gold == 19625, "Upgrade must cost 25 × 15 gold.");
        });
        await Case("upgrade-locked", "Upgrade without an upgraded dwelling", "Reject without consuming creatures or resources.", async f =>
        {
            await Post($"/api/towns/{f.Town.Id}/upgrade", new { army = "hero", slot = 0 }, 400);
            Require((await Player(f)).Gold == 20000 && (await Hero(f)).ArmyJson == f.Hero.ArmyJson, "Rejected upgrade changed gold or creatures.");
        });
        await Case("upgrade-army", "Select an army to upgrade", "Invalid army selectors must be rejected, not interpreted as the garrison.", async f =>
        {
            f.Town.GarrisonJson = Army(("pikeman", 2)); f.Town.BuildingsJson = Strings("dwelling1", "upg1"); await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/upgrade", new { army = "wrong", slot = 0 }, 400);
        });
        await Case("market-trade", "Trade minerals at a marketplace", "One market buys one wood for 2500 gold and sells one wood for 25 gold.", async f =>
        {
            f.Town.BuildingsJson = Strings("villageHall", "marketplace"); await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/trade", new { from = "gold", to = "wood", amount = 1 }, 204);
            Require((await Player(f)).Gold == 17500 && (await Player(f)).Wood == 21, "Buy rate or resource delta is incorrect.");
            await Post($"/api/towns/{f.Town.Id}/trade", new { from = "wood", to = "gold", amount = 1 }, 204);
            Require((await Player(f)).Gold == 17525 && (await Player(f)).Wood == 20, "Sell rate is incorrect.");
        });
        await Case("market-amount", "Validate marketplace quantities", "Zero, negative, and unaffordable large orders must fail with no resource change.", async f =>
        {
            f.Town.BuildingsJson = Strings("marketplace"); await f.SaveMap();
            foreach (var amount in new[] { 0, -1, int.MaxValue })
                await Post($"/api/towns/{f.Town.Id}/trade", new { from = "gold", to = "wood", amount }, 400);
            Require((await Player(f)).Gold == 20000 && (await Player(f)).Wood == 20, "Rejected trade changed resources.");
        });
        await Case("build-once", "Build one structure per day", "Prerequisites and costs apply; a second building on the same day is rejected.", async f =>
        {
            await Post($"/api/towns/{f.Town.Id}/build", new { buildingKey = "townHall" }, 204);
            await Post($"/api/towns/{f.Town.Id}/build", new { buildingKey = "blacksmith" }, 400);
            Require((await Player(f)).Gold == 17500, "Town Hall cost is incorrect.");
        });
        await Case("build-prerequisites", "Respect a structure's prerequisites", "Reject buildings whose prerequisites are absent without charging resources.", async f =>
        {
            await Post($"/api/towns/{f.Town.Id}/build", new { buildingKey = "capitol" }, 400);
            Require((await Player(f)).Gold == 20000, "Rejected structure changed resources.");
        });
        await Case("dwelling-growth", "Advance a dwelling's weekly pool", "An existing Guardhouse gains exactly 14 Pikemen at the week boundary.", async f =>
        {
            var old = JsonSerializer.Deserialize<Dictionary<string, int>>(f.Town.AvailableJson)!["1"];
            for (var i = 0; i < 7; i++) await Tick(f);
            var count = JsonSerializer.Deserialize<Dictionary<string, int>>((await Town(f)).AvailableJson)!["1"];
            Require(count == old + 14, $"Expected {old + 14} Pikemen, got {count}.");
        });
        await Case("town-income", "Collect daily town income", "A Village Hall adds exactly 500 gold at the day boundary.", async f =>
        {
            await Tick(f);
            Require((await Player(f)).Gold == 20500, "Village Hall income is incorrect.");
        });
        await Case("hire-hero", "Hire a tavern hero", "Pay the original 2500 gold, add the chosen hero and remove that offer; 2499 is insufficient.", async f =>
        {
            f.Hero.X = 2; f.Hero.Y = 3; f.Player.Gold = 2499; await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/hire", new { heroKey = "sorsha" }, 400);
            Require((await Player(f)).Gold == 2499 && await f.Db.Heroes.CountAsync() == 1, "An unaffordable hire changed gold or the roster.");
            f.Player.Gold = 20000; await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/hire", new { heroKey = "sorsha" });
            Require((await Player(f)).Gold == 17500 && await f.Db.Heroes.CountAsync() == 2, "Hiring cost or hero count is incorrect.");
            Require(!GameService.ParseStringArray((await Town(f)).TavernJson).Contains("sorsha"), "Hired hero remains offered.");
        });
        await Case("hire-gate", "Hire while the visiting gate is occupied", "Only one visiting hero fits in town; hiring must not spawn a hero outside the town.", async f =>
        {
            await Post($"/api/towns/{f.Town.Id}/hire", new { heroKey = "sorsha" }, 400);
        });
        await Case("spellbook-buy", "Buy and learn from a Mage Guild", "A visitor pays 500 gold for a spellbook and learns eligible stocked spells.", async f =>
        {
            f.Town.BuildingsJson = Strings("mageGuild1"); f.Town.MageGuildJson = Strings("magicArrow", "haste"); await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/spellbook");
            var h = await Hero(f);
            Require((await Player(f)).Gold == 19500 && TownRules.HasGear(h, TownRules.Spellbook)
                && GameService.ParseStringArray(h.SpellsJson).Length == 2, "Book purchase or learning is incorrect.");
        });
        await Case("guild-wisdom", "Respect Wisdom when learning guild spells", "Without Wisdom, learn levels one and two, but not level three.", f =>
        {
            TownRules.AddGear(f.Hero, TownRules.Spellbook);
            f.Town.MageGuildJson = Strings("magicArrow", "visions", "animateDead");
            var learned = TownRules.LearnGuildSpells(f.Hero, f.Town);
            Require(learned.Contains("magicArrow") && learned.Contains("visions") && !learned.Contains("animateDead"), "Guild ignored Wisdom.");
            return Task.CompletedTask;
        });
        await Case("guild-rest", "Rest overnight at a Mage Guild", "A visiting hero with a spellbook regains full spell points after a day.", async f =>
        {
            f.Town.BuildingsJson = Strings("villageHall", "mageGuild1"); f.Hero.Mana = 0;
            TownRules.AddGear(f.Hero, TownRules.Spellbook); await f.SaveMap();
            await Tick(f);
            Require((await Hero(f)).Mana == 10, "Overnight Mage Guild visit did not restore mana.");
        });
        await Case("blacksmith", "Buy a faction's war machine", "A present hero pays once; duplicate purchases are rejected.", async f =>
        {
            f.Town.BuildingsJson = Strings("blacksmith"); await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/blacksmith");
            await Post($"/api/towns/{f.Town.Id}/blacksmith", null, 400);
            Require(TownRules.HasGear(await Hero(f), "ballista") && (await Player(f)).Gold == 17500, "War machine purchase is incorrect.");
        });
        foreach (var resource in TownRules.Resources)
            await Case($"resource-{resource}", $"Collect a {resource} pile", "Credit the exact quantity once and remove the pile.", async f =>
            {
                await MoveObject(f, "resource", $"{resource}:5");
                var p = await Player(f);
                Require(TownRules.Get(p, resource) == (resource == "gold" ? 20005 : 25), "Pile yielded the wrong quantity.");
                var g = await f.Db.Games.AsNoTracking().SingleAsync();
                var map = JsonSerializer.Deserialize<AdventureMap>(g.MapJson, Json)!;
                Require(map.At(2, 4).ObjectKind == null, "Collected pile was not removed.");
            });
        foreach (var resource in TownRules.Resources)
            await Case($"mine-{resource}", $"Claim and collect from a {resource} mine", "Claiming preserves the structure; pay its exact daily yield once.", async f =>
            {
                await MoveObject(f, "mine", resource, false);
                await Tick(f);
                var delta = resource == "gold" ? 1500 : resource is "wood" or "ore" ? 2 : 1;
                Require(TownRules.Get(await Player(f), resource) == (resource == "gold" ? 20000 : 20) + delta, "Mine daily yield is incorrect.");
                var g = await f.Db.Games.AsNoTracking().SingleAsync();
                var map = JsonSerializer.Deserialize<AdventureMap>(g.MapJson, Json)!;
                Require(map.At(2, 4).ObjectKind == "mine" && map.At(2, 4).Owner == 1, "Mine structure or claim was lost.");
            });
        await Case("resource-route-stop", "Stop a walking route at the first pickup", "Only the first pile is collected and reported; later route objects remain untouched.", async f =>
        {
            f.Hero.X = 2; f.Hero.Y = 3;
            foreach (var y in new[] { 4, 5 })
            {
                var tile = f.Map.At(2, y);
                tile.ObjectKind = "resource"; tile.ObjectData = "gold:500"; tile.Passable = true;
            }
            await f.SaveMap();
            using var request = new HttpRequestMessage(HttpMethod.Post, $"/api/heroes/{f.Hero.Id}/move");
            request.Headers.Add("X-Player-Id", "alice");
            request.Content = JsonContent.Create(new { path = new[] { new { x = 2, y = 4 }, new { x = 2, y = 5 } } });
            using var response = await client.SendAsync(request);
            Require(response.IsSuccessStatusCode, "Pickup route was rejected.");
            var notices = JsonSerializer.Deserialize<JsonElement>(response.Headers.GetValues("X-Move-Events").Single());
            Require(notices.GetArrayLength() == 1 && notices[0].GetProperty("key").GetString() == "gold"
                && notices[0].GetProperty("amount").GetInt32() == 500, "Pickup component was missing, duplicated or incorrect.");
            var hero = await Hero(f);
            var saved = JsonSerializer.Deserialize<AdventureMap>((await f.Db.Games.AsNoTracking().SingleAsync()).MapJson, Json)!;
            Require(hero.Y == 4 && hero.Movement == 1400 && (await Player(f)).Gold == 20500
                && saved.At(2, 4).ObjectKind == null && saved.At(2, 5).ObjectKind == "resource", "Route ran through a pickup or collected later objects.");
        });
        await Case("flight-landing-pickup-stop", "Stop after a flight landing pickup", "Landing collects its pile but cannot also visit the adjacent structure in the same move.", async f =>
        {
            f.Hero.X = 2; f.Hero.Y = 3;
            var pile = f.Map.At(2, 4);
            pile.ObjectKind = "resource"; pile.ObjectData = "gold:500"; pile.Passable = true;
            var shrine = f.Map.At(2, 5);
            shrine.ObjectKind = "shrine"; shrine.ObjectData = "magicArrow"; shrine.Passable = false;
            f.Player.FogJson = GameService.SerializeFog(Enumerable.Repeat((byte)2, 49).ToArray());
            TownRules.AddGear(f.Hero, TownRules.Spellbook);
            f.Hero.SpellsJson = Strings("fly"); f.Hero.Mana = 100;
            await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "fly" });
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 4 }, new { x = 2, y = 5 } } });
            var hero = await Hero(f);
            Require(hero.Y == 4 && hero.Movement == 1360 && (await Player(f)).Gold == 20500
                && !GameService.ParseStringArray(hero.SpellsJson).Contains("magicArrow"), "Flight chained a second interaction after its landing pickup.");
        });
        await Case("mine-owner-consent", "Visit another player's mine", "A visitor cannot grant themselves ownership or sharing of a claimed mine.", async f =>
        {
            var bob = new Player { GameId = f.Game.Id, Faction = "rampart" }; f.Db.Players.Add(bob);
            f.Hero.X = 2; f.Hero.Y = 3;
            var tile = f.Map.At(2, 4); tile.ObjectKind = "mine"; tile.ObjectData = "wood"; tile.Passable = false; tile.Owner = 2;
            await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 4 } } });
            var g = await f.Db.Games.AsNoTracking().SingleAsync();
            Require(JsonSerializer.Deserialize<AdventureMap>(g.MapJson, Json)!.At(2, 4).Owner == 2, "A visitor added itself to the mine's owners without consent.");
        }, "Adapt");
        await Case("treasure-choice", "Open a gold treasure chest", "A chest must not automatically grant both gold and experience.", async f =>
        {
            await MoveObject(f, "treasure", "1500:1000");
            var h = await Hero(f); var p = await Player(f);
            Require(!(p.Gold > 20000 && h.Experience > 0), "Chest granted both gold and experience.");
        });
        await Case("artifact-pickup", "Pick up an artifact", "Remove the map object and place the artifact into its matching slot or backpack.", async f =>
        {
            await MoveObject(f, "artifact", "artifact1");
            var h = await Hero(f); var arranged = Artifacts.Arrange(TownRules.Artifacts(h));
            var map = JsonSerializer.Deserialize<AdventureMap>((await f.Db.Games.AsNoTracking().SingleAsync()).MapJson, Json)!;
            Require(arranged.Worn.TryGetValue("leftHand", out var key) && key == "artifact1"
                && map.At(2, 4).ObjectKind == null, "Shield did not occupy its slot or remain removed from the map.");
        });
        await Case("shrine-book", "Visit a shrine without a spellbook", "Do not teach a spell, remove the shrine, or move through it.", async f =>
        {
            await MoveObject(f, "shrine", "magicArrow", false);
            var h = await Hero(f);
            Require(h.X == 2 && h.Y == 3 && GameService.ParseStringArray(h.SpellsJson).Length == 0, "A hero without a spellbook learned or crossed the shrine.");
        });
        await Case("shrine-wisdom", "Learn a shrine's higher-level spell", "A third-level shrine requires Basic Wisdom as well as a spellbook.", async f =>
        {
            TownRules.AddGear(f.Hero, TownRules.Spellbook);
            await MoveObject(f, "shrine", "animateDead", false);
            Require(!GameService.ParseStringArray((await Hero(f)).SpellsJson).Contains("animateDead"), "Shrine taught a third-level spell without Wisdom.");
        });
        foreach (var action in new[] { "observe", "recruit", "pass" })
            await Case($"encounter-{action}", $"Peaceful creature encounter: {action}", "Resolve the pending encounter; observe/pass preserve the obstacle, recruitment charges and removes it.", async f =>
            {
                await MoveObject(f, "monster", "pikeman:3", false);
                await Post($"/api/heroes/{f.Hero.Id}/encounter", new { action });
                var h = await Hero(f); var p = await Player(f);
                Require(h.PendingEncounterJson == null, "Encounter did not clear.");
                var g = await f.Db.Games.AsNoTracking().SingleAsync();
                var tile = JsonSerializer.Deserialize<AdventureMap>(g.MapJson, Json)!.At(2, 4);
                Require(action == "recruit" ? tile.ObjectKind == null && p.Gold < 20000 && GameService.ParseArmy(h.ArmyJson)[0].Count == 28
                    : tile.ObjectKind == "monster" && p.Gold == 20000 && (action != "observe" || h.Experience > 0), "Encounter had the wrong creature, obstacle or resource effect.");
            }, "Adapt");
        await Case("encounter-strength", "Reject an expedition that is too weak", "Observe needs half the stack's strength; recruit needs full strength.", async f =>
        {
            f.Hero.ArmyJson = Army(("pikeman", 1));
            await MoveObject(f, "monster", "pikeman:10", false);
            await Post($"/api/heroes/{f.Hero.Id}/encounter", new { action = "observe" }, 400);
            await Post($"/api/heroes/{f.Hero.Id}/encounter", new { action = "recruit" }, 400);
            Require((await Hero(f)).PendingEncounterJson != null, "Failed attempt cleared the encounter.");
        }, "Adapt");
        await Case("encounter-offer", "Load the server-authoritative joining offer", "Pending encounters expose the actual creature, count, gold price and strength eligibility, without changing gold or position.", async f =>
        {
            await MoveObject(f, "monster", "pikeman:5", false);
            var dto = await Request("GET", $"/api/heroes/{f.Hero.Id}");
            var offer = dto.GetProperty("encounterOffer");
            Require(offer.GetProperty("creature").GetString() == "pikeman"
                && offer.GetProperty("name").GetString() == Creatures.All["pikeman"].Name
                && offer.GetProperty("count").GetInt32() == 5
                && offer.GetProperty("gold").GetInt32() == Creatures.All["pikeman"].Gold * 5
                && offer.GetProperty("canRecruit").GetBoolean(), "Joining offer does not match the authoritative encounter.");
            Require((await Player(f)).Gold == 20000 && (await Hero(f)).Y == 3, "Reading the offer mutated the adventure state.");
        }, "Adapt");
        await Case("encounter-offer-weak", "Load a non-joining creature encounter", "The offer's eligibility uses the same strength rule as recruitment; acknowledging it leaves the creature solid.", async f =>
        {
            f.Hero.ArmyJson = Army(("pikeman", 1));
            await MoveObject(f, "monster", "pikeman:10", false);
            var dto = await Request("GET", $"/api/heroes/{f.Hero.Id}");
            Require(!dto.GetProperty("encounterOffer").GetProperty("canRecruit").GetBoolean(), "A weaker army was offered recruitment.");
            await Post($"/api/heroes/{f.Hero.Id}/encounter", new { action = "pass" });
            Require((await Hero(f)).PendingEncounterJson == null && (await Hero(f)).Y == 3, "Acknowledgment moved through the creature.");
        }, "Adapt");
        await Case("encounter-offer-legacy", "Read absent or malformed legacy encounter metadata", "Hero queries remain usable and omit an offer for malformed, empty, unknown or absent encounter state.", async f =>
        {
            foreach (var pending in new string?[] { null, "{}", "{", "null", "{\"creature\":\"missing\",\"count\":1}" })
            {
                f.Hero.PendingEncounterJson = pending; await f.SaveMap();
                var dto = await Request("GET", $"/api/heroes/{f.Hero.Id}");
                Require(dto.GetProperty("encounterOffer").ValueKind == JsonValueKind.Null, "Legacy encounter produced an invalid offer.");
            }
        }, "Adapt");
        await Case("encounter-merge-full", "Accept joining creatures with seven occupied matching stacks", "A full army can merge joining creatures into its matching stack and pays the quoted price exactly once.", async f =>
        {
            f.Hero.ArmyJson = Army(("pikeman", 25), ("archer", 8), ("griffin", 5), ("swordsman", 5), ("monk", 5), ("cavalier", 5), ("angel", 1));
            await MoveObject(f, "monster", "pikeman:5", false);
            var dto = await Request("GET", $"/api/heroes/{f.Hero.Id}");
            var price = dto.GetProperty("encounterOffer").GetProperty("gold").GetInt32();
            await Post($"/api/heroes/{f.Hero.Id}/encounter", new { action = "recruit" });
            await Post($"/api/heroes/{f.Hero.Id}/encounter", new { action = "recruit" }, 400);
            Require(GameService.ParseArmy((await Hero(f)).ArmyJson)[0].Count == 30
                && (await Player(f)).Gold == 20000 - price, "A full matching army was rejected, or the joining price was charged more than once.");
        }, "Adapt");
        await Case("obelisk-reveal", "Visit the Obelisk and reveal the Grail", "Ten fragments reveal the Grail tile without ending the game.", async f =>
        {
            for (var i = 0; i < 10; i++) await Post($"/api/games/{f.Game.Id}/obelisk/visit");
            var game = await f.Db.Games.AsNoTracking().SingleAsync();
            Require(game.GrailRevealed && game.State == GameState.InProgress
                && JsonSerializer.Deserialize<AdventureMap>(game.MapJson, Json)!.At(5, 5).ObjectKind == "grail", "Grail reveal is incorrect.");
        }, "Adapt");
        await Case("grail-dig", "Dig the revealed Grail", "Award the Grail, remove its tile and spend remaining movement; do not end the game.", async f =>
        {
            f.Game.GrailRevealed = true;
            await MoveObject(f, "grail", null);
            Require((await Player(f)).Precious && (await Hero(f)).Movement == 0
                && (await f.Db.Games.AsNoTracking().SingleAsync()).State == GameState.InProgress, "Digging did not award the Grail or spend movement.");
        }, "Adapt");
        await Case("grail-build", "Build a Grail structure", "Require and consume the Grail when constructing its unique structure.", async f =>
        {
            f.Player.Precious = true; await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/build", new { buildingKey = "grail" }, 204);
            Require(!(await Player(f)).Precious && GameService.ParseStringArray((await Town(f)).BuildingsJson).Contains("grail")
                && (await f.Db.Games.AsNoTracking().SingleAsync()).State == GameState.InProgress, "Grail structure did not consume the Grail, was not built, or ended the game.");
        });
        await Case("native-creature-reference", "Compare all 126 creatures with original numerical tables",
            "Every retained creature matches native recruitment resources, statistics, growth and its upgrade price difference.", f =>
        {
            string Normalize(string s) => new(s.Where(char.IsLetter).Select(char.ToLowerInvariant).ToArray());
            foreach (var creature in Creatures.All.Values)
            {
                var native = NativeRules.Data.Creatures.Single(c => Normalize(c.Name) == Normalize(creature.Name));
                Require(new[] { creature.Wood, creature.Mercury, creature.Ore, creature.Sulfur, creature.Crystal, creature.Gems, creature.Gold }
                    .SequenceEqual(native.Cost), $"{creature.Key}: recruitment resource cost differs from CRTRAITS.");
                Require(new[] { creature.Attack, creature.Defense, creature.DamageMin, creature.DamageMax, creature.Health, creature.Speed, creature.Growth }
                    .SequenceEqual(new[] { native.Attack, native.Defense, native.Damage[0], native.Damage[1], native.Hp, native.Speed, native.Growth }),
                    $"{creature.Key}: creature statistics differ from CRTRAITS.");
                if (creature.UpgradeOf is { } baseKey)
                {
                    var source = Creatures.All[baseKey];
                    var expected = native.Cost.Zip(NativeRules.Data.Creatures.Single(c => Normalize(c.Name) == Normalize(source.Name)).Cost,
                        (upgraded, original) => Math.Max(0, upgraded - original)).ToArray();
                    var actual = TownRules.UpgradeCost(source, creature, 3);
                    Require(new[] { "wood", "mercury", "ore", "sulfur", "crystal", "gems", "gold" }
                        .Select(r => actual.GetValueOrDefault(r)).SequenceEqual(expected.Select(value => value * 3)),
                        $"{creature.Key}: upgrade resources differ from the original.");
                }
            }
            Require(Creatures.All.Count == 126, "Reference coverage must include all nine retained creature factions.");
            return Task.CompletedTask;
        });
        await Case("native-building-reference", "Compare every supported purchasable building with BUILDING.TXT",
            "All three towns' guilds, halls, fortifications, dwellings, upgrades and specials use original seven-resource prices.", f =>
        {
            string Normalize(string s) => new(s.Where(char.IsLetter).Select(char.ToLowerInvariant).ToArray());
            var shared = new Dictionary<string, string>
            {
                ["townHall"] = "HALL_TOWN_ID", ["cityHall"] = "HALL_CITY_ID", ["capitol"] = "HALL_CAPITOL_ID",
                ["fort"] = "CASTLE_FORT_ID", ["citadel"] = "CASTLE_CITADEL_ID", ["castle"] = "CASTLE_CASTLE_ID",
                ["tavern"] = "TAVERN_ID", ["marketplace"] = "MARKETPLACE_ID", ["resourceSilo"] = "MARKETPLACE_SILO_ID", ["blacksmith"] = "BLACKSMITH_ID"
            };
            foreach (var faction in TownBuildings.Factions)
            foreach (var building in TownBuildings.For(faction).Values.Where(b => b.Key is not ("villageHall" or "grail")))
            {
                NativeRules.Building native;
                if (building.Key.StartsWith("mageGuild"))
                    native = NativeRules.Data.Buildings.Single(b => b.Name == (building.Key == "mageGuild1" ? "MAGE_GUILD_ID" : $"MAGE_GUILD{building.Key[9..]}_ID"));
                else if (shared.TryGetValue(building.Key, out var name))
                    native = NativeRules.Data.Buildings.Single(b => b.Name == name);
                else if (building.Tier > 0)
                {
                    var key = Creatures.BaseTiers(faction)[building.Tier - 1];
                    if (building.Key.StartsWith("upg")) key = Creatures.UpgradeOf(key)!;
                    native = NativeRules.Data.Buildings.Single(b => b.Section.Equals(faction, StringComparison.OrdinalIgnoreCase)
                        && Normalize(b.Name == "Black Lord" ? "Dread Knight" : b.Name) == Normalize(Creatures.All[key].Name));
                }
                else
                {
                    var alias = (faction, building.Key) switch
                    {
                        ("castle", "horde1") => "Griffin Horde", ("castle", "special3") => "Tavern Upgrade",
                        ("rampart", "special1") => "Mystic Garden", ("rampart", "special2") => "Rainbow",
                        ("rampart", "horde1") => "Dwarf Horde", ("rampart", "horde2") => "Treefolk Horde",
                        ("necropolis", "horde1") => "Skeleton Horde", _ => building.Name
                    };
                    native = NativeRules.Data.Buildings.First(b => b.Section.Equals(faction, StringComparison.OrdinalIgnoreCase) && b.Name == alias);
                }
                Require(new[] { building.Wood, building.Mercury, building.Ore, building.Sulfur, building.Crystal, building.Gems, building.Gold }
                    .SequenceEqual(native.Cost), $"{faction}/{building.Key}: differs from BUILDING.TXT.");
            }
            return Task.CompletedTask;
        });
        await Case("special-growth-reference", "Apply every supported horde, fortification and Grail growth bonus",
            "The Grail adds half of total growth after fortifications and horde bonuses, including low-growth tier-seven rounding.", async f =>
        {
            foreach (var faction in TownBuildings.Factions)
            for (var tier = 1; tier <= 7; tier++)
            foreach (var fortification in new[] { "fort", "citadel", "castle" })
            foreach (var grail in new[] { false, true })
            {
                var built = new[] { $"dwelling{tier}", fortification, "horde1", "horde2" }.ToList();
                if (grail) built.Add("grail");
                var baseGrowth = Creatures.All[Creatures.BaseTiers(faction)[tier - 1]].Growth;
                var bonus = (faction, tier) switch { ("castle", 3) => 3, ("rampart", 2) => 4, ("rampart", 5) => 2, ("necropolis", 1) => 6, _ => 0 };
                var expected = baseGrowth + (fortification == "castle" ? baseGrowth : fortification == "citadel" ? baseGrowth / 2 : 0) + bonus;
                if (grail) expected += expected / 2;
                Require(TownBuildings.WeeklyGrowth(faction, built, tier) == expected, $"{faction} tier {tier} {fortification} grail={grail}: growth differs.");
            }
            f.Game.Day = 7; f.Town.BuildingsJson = Strings("dwelling7", "castle", "grail"); f.Town.AvailableJson = "{\"7\":0}";
            await f.SaveMap(); await Tick(f);
            Require(JsonSerializer.Deserialize<Dictionary<string, int>>((await Town(f)).AvailableJson)!["7"] == 3, "The weekly pool did not receive three Angels.");
        });
        await Case("special-pond", "Pay Mystic Pond only at the week boundary",
            "Exactly one rare resource gains one to four units; no daily payout or replacement peaceful bonus is invented.", async f =>
        {
            f.Town.Faction = "rampart"; f.Town.BuildingsJson = Strings("special1"); f.Game.Day = 7;
            await f.SaveMap(); await Tick(f);
            var player = await Player(f);
            var gains = new[] { player.Mercury - 20, player.Sulfur - 20, player.Crystal - 20, player.Gems - 20 };
            Require(gains.Count(g => g > 0) == 1 && gains.Sum() is >= 1 and <= 4, "Mystic Pond produced the wrong resources or quantity.");
            await Tick(f);
            var tomorrow = await Player(f);
            Require(new[] { tomorrow.Mercury, tomorrow.Sulfur, tomorrow.Crystal, tomorrow.Gems }
                .SequenceEqual(new[] { player.Mercury, player.Sulfur, player.Crystal, player.Gems }), "Mystic Pond paid on a non-weekly day.");
        });
        await Case("special-darkness", "Preserve active sight inside Cover of Darkness",
            "Re-shroud explored memory within radius twenty, without erasing what a hero currently sees or tiles outside the radius.", f =>
        {
            var fog = Enumerable.Repeat((byte)1, 41 * 41).ToArray();
            fog[20 * 41 + 21] = 2;
            GameService.ApplyDarkness(fog, 41, 41, new Town { X = 20, Y = 20 });
            Require(fog[20 * 41 + 21] == 2 && fog[20 * 41 + 40] == 0 && fog[0] == 1, "Cover of Darkness erased current sight or used the wrong radius.");
            return Task.CompletedTask;
        });
        await Case("special-combat-only", "Retain combat-only buildings without invented adventure effects",
            "Brotherhood, Fountain of Fortune and Necromancy Amplifier do not create gold, creatures, movement or replacement skill bonuses.", async f =>
        {
            f.Town.Faction = "castle"; f.Town.BuildingsJson = Strings("special3");
            foreach (var faction in new[] { "rampart", "necropolis" })
                f.Db.Towns.Add(new Town { GameId = f.Game.Id, PlayerId = f.Player.Id, Faction = faction, X = 5, Y = 5,
                    BuildingsJson = Strings("special2") });
            var skills = f.Hero.SkillsJson; var army = f.Hero.ArmyJson;
            await f.SaveMap(); await Tick(f);
            var hero = await Hero(f);
            Require((await Player(f)).Gold == 20000 && hero.ArmyJson == army && hero.SkillsJson == skills,
                "A combat-only building created a peaceful replacement bonus.");
        });
        await Case("special-treasury-income", "Pay every town's income and opening-balance Treasury interest",
            "Two Treasuries each pay ten percent of the same pre-income balance, not compounded interest on today's income.", async f =>
        {
            f.Game.Day = 7; f.Player.Gold = 10000;
            f.Town.Faction = "rampart"; f.Town.BuildingsJson = Strings("villageHall", "special3", "resourceSilo");
            f.Db.Towns.Add(new Town { GameId = f.Game.Id, PlayerId = f.Player.Id, Faction = "rampart", X = 5, Y = 5,
                BuildingsJson = Strings("villageHall", "special3", "resourceSilo") });
            await f.SaveMap(); await Tick(f);
            var player = await Player(f);
            Require(player.Gold == 13000 && player.Crystal == 22, $"Expected 13000 gold and 22 crystal, got {player.Gold}/{player.Crystal}.");
        });
        await Case("artifact-sight", "Refresh visibility immediately when a sight artifact is equipped or removed",
            "Only equipped Speculum/Spyglass expands active sight; unequipping retains explored memory but not active visibility.", async f =>
        {
            f.Map.Width = f.Map.Height = f.Game.MapWidth = f.Game.MapHeight = 21;
            f.Map.Tiles = Enumerable.Range(0, 441).Select(_ => new MapTile()).ToList();
            f.Hero.X = f.Hero.Y = 10; f.Hero.SkillsJson = "{}"; f.Hero.ArtifactsJson = Strings("artifact15");
            Artifacts.Save(f.Hero, new(new(), ["artifact15"]));
            f.Player.FogJson = GameService.SerializeFog(new byte[441]); await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/artifacts", new { from = "backpack:0", to = "misc1" });
            Require(GameService.ParseFog((await Player(f)).FogJson, 441)[10 * 21 + 16] == 2, "Equipping Speculum did not reveal its extra radius.");
            await Post($"/api/heroes/{f.Hero.Id}/artifacts", new { from = "misc1", to = "backpack:0" });
            Require(GameService.ParseFog((await Player(f)).FogJson, 441)[10 * 21 + 16] == 1, "Backpack Speculum retained equipped sight.");
        });
        await Case("movement-source", "Charge the terrain being left, not the destination",
            "Leaving snow for grass spends 150; entering snow from grass spends 100.", async f =>
        {
            f.Hero.X = 2; f.Hero.Y = 3; f.Hero.SkillsJson = "{}";
            f.Map.At(2, 3).Terrain = "snow"; await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 4 } } });
            Require((await Hero(f)).Movement == 1350, "Movement used the destination terrain.");
        });
        await Case("movement-diagonal", "Charge native diagonal rounding",
            "Diagonal costs are floor(source cost times sqrt(2)): 141/176/212/247.", async f =>
        {
            foreach (var (terrain, expected) in new[] { ("grass", 141), ("rough", 176), ("snow", 212), ("swamp", 247) })
                Require(Terrain.MoveCost(terrain, diagonal: true) == expected, $"{terrain}: wrong diagonal rounding.");
            f.Hero.X = 2; f.Hero.Y = 3; await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 3, y = 4 } } });
            Require((await Hero(f)).Movement == 1359, "Actual movement ignored the diagonal charge.");
        });
        await Case("movement-last-step", "Allow the final diagonal when a straight move remains",
            "125 movement can pay the last 141-point grass diagonal, consuming the remainder.", async f =>
        {
            f.Hero.X = 2; f.Hero.Y = 3; f.Hero.Movement = 125; await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 3, y = 4 } } });
            Require((await Hero(f)).X == 3 && (await Hero(f)).Movement == 0, "Last-step diagonal exception was not applied.");
        });
        await Case("movement-exhaustion", "Exhaust unusable movement after stepping",
            "A 50-point remainder on snow cannot fund another straight move and becomes zero.", async f =>
        {
            f.Hero.X = 2; f.Hero.Y = 3; f.Hero.Movement = 150; f.Map.At(2, 4).Terrain = "snow"; await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 4 } } });
            Require((await Hero(f)).Movement == 0, "Unusable movement survived the step.");
        });
        await Case("movement-native-terrain", "Use army-native terrain, including mixed native factions",
            "All stacks must be native to the source terrain; hero class/faction does not grant the exemption.", async f =>
        {
            foreach (var (faction, terrain) in new[] { ("castle", "grass"), ("rampart", "grass"), ("tower", "snow"), ("inferno", "lava"),
                ("necropolis", "dirt"), ("dungeon", "subterranean"), ("stronghold", "rough"), ("fortress", "swamp"), ("conflux", "grass") })
            {
                var h = new Hero { ArmyJson = Army((Creatures.BaseTiers(faction)[0], 1)) };
                Require(GameService.NativeTerrain(h) == terrain && Terrain.MoveCost(terrain, nativeTerrain: GameService.NativeTerrain(h)) == 100,
                    $"{faction}: wrong native terrain.");
            }
            Require(GameService.NativeTerrain(new Hero { ArmyJson = Army(("pikeman", 1), ("centaur", 1)) }) == "grass"
                && GameService.NativeTerrain(new Hero { ArmyJson = Army(("pikeman", 1), ("gnoll", 1)) }) == null, "Mixed-army native terrain is wrong.");
            f.Hero.X = 2; f.Hero.Y = 3; f.Hero.ArmyJson = Army(("gnoll", 1)); f.Map.At(2, 3).Terrain = "swamp"; await f.SaveMap();
            var detail = await Request("GET", $"/api/heroes/{f.Hero.Id}");
            Require(detail.GetProperty("nativeTerrain").GetString() == "swamp", "The hero detail omitted its actual army-native terrain.");
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 4 } } });
            Require((await Hero(f)).Movement == 1400, "A Castle hero's Fortress army did not receive its native-terrain exemption.");
        });
        await Case("spell-door-water", "Apply the original SoD wrong-terrain Dimension Door failure",
            "A water target fails to move but spends mana, movement and one daily cast.", async f =>
        {
            f.Hero.X = 2; f.Hero.Y = 3; f.Hero.SpellsJson = Strings("dimensionDoor"); f.Hero.Mana = 100;
            f.Hero.SkillsJson = "{}"; TownRules.AddGear(f.Hero, TownRules.Spellbook);
            f.Map.At(2, 4).Terrain = "water"; f.Map.At(2, 4).Passable = false; await f.SaveMap();
            var response = await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "dimensionDoor", x = 2, y = 4 });
            var h = await Hero(f);
            Require(h.X == 2 && h.Y == 3 && h.Mana == 75 && h.Movement == 1200 && GameService.Effects(h).DoorCasts == 1
                && response.GetProperty("message").GetString() == "Dimension Door failed!", "SoD failure did not preserve position and spend casting resources.");
        });
        await Case("movement-sand", "Move across sand", "Sand has the same 150-point base terrain cost as snow before Pathfinding.", f =>
        {
            Require(Terrain.MoveCost("sand") == 150, $"Sand movement cost is {Terrain.MoveCost("sand")}.");
            return Task.CompletedTask;
        });
        await Case("movement-pathfinding", "Apply the Pathfinding skill", "Basic Pathfinding reduces a snow step from 150 to 125 points.", async f =>
        {
            f.Hero.X = 2; f.Hero.Y = 3; f.Hero.SkillsJson = "{\"pathfinding\":1}";
            f.Map.At(2, 3).Terrain = "snow"; await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 4 } } });
            Require((await Hero(f)).Movement == 1375, $"Expected 1375 movement, got {(await Hero(f)).Movement}.");
        });
        await Case("hero-level", "Gain a hero level", "Each level grants one primary point and persists its secondary-skill choice rather than choosing for the player.", f =>
        {
            var before = f.Hero.Attack + f.Hero.Defense + f.Hero.Power + f.Hero.Knowledge;
            GameService.GainExperience(f.Hero, 1000);
            var after = f.Hero.Attack + f.Hero.Defense + f.Hero.Power + f.Hero.Knowledge;
            Require(f.Hero.Level == 2 && after == before + 1 && HeroProgression.Pending(f.Hero)?.Choices.Length == 2,
                "Level-up did not grant one primary point and a pending choice.");
            return Task.CompletedTask;
        });
        await Case("spell-flight", "Cast Fly", "Fly must enable a day-long flight movement mode, not acknowledge an inert spell.", async f =>
        {
            f.Hero.X = 2; f.Hero.Y = 3;
            f.Map.At(2, 4).ObjectKind = "tree"; f.Map.At(2, 4).Passable = false;
            f.Player.FogJson = GameService.SerializeFog(Enumerable.Repeat((byte)2, 49).ToArray());
            TownRules.AddGear(f.Hero, TownRules.Spellbook); f.Hero.SpellsJson = Strings("fly"); f.Hero.Mana = 100; await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "fly" });
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 4 }, new { x = 2, y = 5 } } });
            Require((await Hero(f)).Y == 5, "Fly did not cross the obstacle and land on open ground.");
            await Tick(f);
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 4 }, new { x = 2, y = 3 } } }, 400);
            Require((await Hero(f)).Y == 5, "Flight did not expire at the day boundary.");
        });
        await Case("spell-door-occupied", "Cast Dimension Door onto another hero", "Reject occupied destinations without spending mana.", async f =>
        {
            TownRules.AddGear(f.Hero, TownRules.Spellbook); f.Hero.SpellsJson = Strings("dimensionDoor"); f.Hero.Mana = 100;
            f.Db.Heroes.Add(new Hero { GameId = f.Game.Id, PlayerId = f.Player.Id, X = 5, Y = 5 }); await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "dimensionDoor", x = 5, y = 5 }, 400);
            Require((await Hero(f)).Mana == 100 && (await Hero(f)).X == 2 && (await Hero(f)).Y == 2, "Rejected teleport spent mana or moved the hero.");
        });
        await Case("game-finished", "Attempt economic actions after the game ends", "Building, recruitment, trading, hiring and army actions must not change a finished game.", async f =>
        {
            f.Game.State = GameState.Finished;
            f.Town.BuildingsJson = Strings("villageHall", "fort", "tavern", "dwelling1", "upg1", "marketplace", "blacksmith", "mageGuild1");
            await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/recruit", new { tier = 1, count = 1 }, 400);
            await Post($"/api/towns/{f.Town.Id}/trade", new { from = "wood", to = "gold", amount = 1 }, 400);
            await Post($"/api/towns/{f.Town.Id}/build", new { buildingKey = "townHall" }, 400);
            await Post($"/api/towns/{f.Town.Id}/army", new { fromArmy = "hero", fromSlot = 0, toArmy = "garrison", toSlot = 0 }, 400);
            await Post($"/api/towns/{f.Town.Id}/garrison", new { direction = "toGarrison", slot = 0 }, 400);
            await Post($"/api/towns/{f.Town.Id}/upgrade", new { army = "hero", slot = 0 }, 400);
            await Post($"/api/towns/{f.Town.Id}/blacksmith", null, 400);
            await Post($"/api/towns/{f.Town.Id}/spellbook", null, 400);
            await Post($"/api/heroes/{f.Hero.Id}/army", new { fromSlot = 0, toSlot = 6, count = 2 }, 400);
            await Post($"/api/games/{f.Game.Id}/endturn", null, 400);
            f.Hero.X = 2; f.Hero.Y = 3; await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/hire", new { heroKey = "sorsha" }, 400);
            Require((await Player(f)).Gold == 20000, "Finished game changed resources.");
        });
        await Case("recruit-minerals", "Recruit a rare-resource creature", "An Angel costs 3000 gold and one gem; missing gems reject the purchase.", async f =>
        {
            f.Town.BuildingsJson = Strings("fort", "dwelling7"); f.Town.AvailableJson = "{\"7\":2}";
            f.Player.Gems = 0; await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/recruit", new { tier = 7, count = 1 }, 400);
            Require((await Player(f)).Gold == 20000, "Missing gems still charged gold.");
            f.Player.Gems = 2; await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/recruit", new { tier = 7, count = 1 });
            Require((await Player(f)).Gold == 17000 && (await Player(f)).Gems == 1, "Angel's full resource price is incorrect.");
        });
        await Case("army-invalid", "Reject invalid army operations", "Invalid slots, selectors, negative/oversized counts, and partial swaps leave armies unchanged.", async f =>
        {
            f.Town.GarrisonJson = Army(("griffin", 3)); await f.SaveMap();
            foreach (var request in new[] {
                new { fromArmy = "wrong", fromSlot = 0, toArmy = "garrison", toSlot = 0, count = 0 },
                new { fromArmy = "hero", fromSlot = -1, toArmy = "garrison", toSlot = 0, count = 0 },
                new { fromArmy = "hero", fromSlot = 0, toArmy = "garrison", toSlot = 7, count = 0 },
                new { fromArmy = "hero", fromSlot = 0, toArmy = "garrison", toSlot = 0, count = -1 },
                new { fromArmy = "hero", fromSlot = 0, toArmy = "garrison", toSlot = 0, count = 26 },
                new { fromArmy = "hero", fromSlot = 0, toArmy = "garrison", toSlot = 0, count = 2 } })
                await Post($"/api/towns/{f.Town.Id}/army", request, 400);
            Require((await Hero(f)).ArmyJson == f.Hero.ArmyJson && (await Town(f)).GarrisonJson == f.Town.GarrisonJson, "Rejected operation changed an army.");
        });
        await Case("army-legacy", "Transfer through the garrison shortcut", "Moving creatures to the garrison and back conserves both stacks.", async f =>
        {
            await Post($"/api/towns/{f.Town.Id}/garrison", new { direction = "toGarrison", slot = 0, count = 3 });
            await Post($"/api/towns/{f.Town.Id}/garrison", new { direction = "toHero", slot = 0, count = 3 });
            Require(GameService.ParseArmy((await Hero(f)).ArmyJson)[0].Count == 25
                && GameService.ParseArmy((await Town(f)).GarrisonJson).All(s => s.Count == 0), "Round-trip transfer changed creature totals.");
        });
        await Case("upgrade-garrison", "Upgrade the town garrison", "Upgrading retains count and charges the exact price difference.", async f =>
        {
            f.Town.BuildingsJson = Strings("dwelling1", "upg1"); f.Town.GarrisonJson = Army(("pikeman", 2)); await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/upgrade", new { army = "garrison", slot = 0 });
            Require(GameService.ParseArmy((await Town(f)).GarrisonJson)[0] == new GameService.ArmySlot("halberdier", 2)
                && (await Player(f)).Gold == 19970, "Garrison upgrade changed count or charged the wrong price.");
        });
        await Case("build-new-pool", "Build a new dwelling", "A new Guardhouse makes one base week's supply immediately available.", async f =>
        {
            f.Town.BuildingsJson = Strings("villageHall", "fort"); f.Town.AvailableJson = "{}"; await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/build", new { buildingKey = "dwelling1" }, 204);
            Require(JsonSerializer.Deserialize<Dictionary<string, int>>((await Town(f)).AvailableJson)!["1"] == 14, "New dwelling did not initialize its creature pool.");
        });
        foreach (var (structure, growth) in new[] { ("citadel", 21), ("castle", 28) })
            await Case($"growth-{structure}", $"Advance a week with a {structure}", "Fortifications increase weekly Pikeman growth without discarding the existing pool.", async f =>
            {
                f.Town.BuildingsJson = Strings("villageHall", "fort", "dwelling1", structure); await f.SaveMap();
                for (var i = 0; i < 7; i++) await Tick(f);
                Require(JsonSerializer.Deserialize<Dictionary<string, int>>((await Town(f)).AvailableJson)!["1"] == 14 + growth, "Fortification growth is incorrect.");
            });
        await Case("hire-limits", "Validate tavern hiring", "Reject missing taverns, unknown offers, insufficient gold, and an eight-hero roster.", async f =>
        {
            f.Hero.X = 2; f.Hero.Y = 3; f.Town.BuildingsJson = Strings("villageHall"); await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/hire", new { heroKey = "sorsha" }, 400);
            f.Town.BuildingsJson = Strings("tavern"); await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/hire", new { heroKey = "not-a-hero" }, 400);
            f.Player.Gold = 1999; await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/hire", new { heroKey = "sorsha" }, 400);
            f.Player.Gold = 20000;
            for (var i = 0; i < 7; i++) f.Db.Heroes.Add(new Hero { GameId = f.Game.Id, PlayerId = f.Player.Id, X = 6, Y = i });
            await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/hire", new { heroKey = "sorsha" }, 400);
            Require((await Player(f)).Gold == 20000 && await f.Db.Heroes.CountAsync() == 8, "Rejected hire changed the roster or gold.");
        });
        await Case("mana-capacity", "Calculate a hero's spell-point capacity", "Knowledge grants ten points each; Intelligence adds 25/50/100 percent.", f =>
        {
            f.Hero.Knowledge = 2;
            foreach (var (level, expected) in new[] { (0, 20), (1, 25), (2, 30), (3, 40) })
            {
                f.Hero.SkillsJson = level == 0 ? "{}" : $"{{\"intelligence\":{level}}}";
                Require(GameService.ManaCapacity(f.Hero) == expected, "Knowledge or Intelligence capacity is incorrect.");
            }
            return Task.CompletedTask;
        });
        foreach (var (level, restored) in new[] { (0, 1), (1, 2), (2, 3), (3, 4) })
            await Case($"mana-daily-{level}", $"Regenerate spell points (Mysticism {level})", "Outside a Mage Guild, recover the skill's daily amount without exceeding capacity.", async f =>
            {
                f.Hero.X = 6; f.Hero.Y = 6; f.Hero.Mana = 0; f.Hero.SkillsJson = $"{{\"mysticism\":{level}}}"; await f.SaveMap();
                await Tick(f);
                Require((await Hero(f)).Mana == restored, "Daily spell-point regeneration is incorrect.");
                f.Hero.Mana = 9; await f.SaveMap();
                await Tick(f);
                Require((await Hero(f)).Mana == 10, "Daily regeneration exceeded the hero's spell-point capacity.");
            });
        await Case("shrine-learn", "Learn from a shrine and revisit it", "An eligible visitor learns once; the structure persists and the hero remains adjacent.", async f =>
        {
            TownRules.AddGear(f.Hero, TownRules.Spellbook); f.Hero.SkillsJson = "{\"wisdom\":1}";
            await MoveObject(f, "shrine", "animateDead", false);
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 4 } } });
            var h = await Hero(f);
            var map = JsonSerializer.Deserialize<AdventureMap>((await f.Db.Games.AsNoTracking().SingleAsync()).MapJson, Json)!;
            Require(GameService.ParseStringArray(h.SpellsJson).Count(s => s == "animateDead") == 1
                && h.Y == 3 && !map.At(2, 4).Passable && map.At(2, 4).ObjectKind == "shrine", "Shrine duplicated a spell or lost its obstacle.");
        });
        await Case("mine-repeat", "Revisit your own mine", "A repeat visit neither pays a second daily yield nor removes the mine.", async f =>
        {
            await MoveObject(f, "mine", "wood", false);
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 4 } } });
            Require((await Player(f)).Wood == 20, "Visiting the mine paid income before the day boundary.");
        }, "Adapt");
        await Case("encounter-invalid", "Reject invalid or unaffordable encounters", "Unknown choices and unaffordable recruitment retain the obstacle and pending choice.", async f =>
        {
            f.Player.Gold = 0; await MoveObject(f, "monster", "pikeman:3", false);
            await Post($"/api/heroes/{f.Hero.Id}/encounter", new { action = "fight" }, 400);
            await Post($"/api/heroes/{f.Hero.Id}/encounter", new { action = "recruit" }, 400);
            Require((await Hero(f)).PendingEncounterJson != null && (await Player(f)).Gold == 0, "Rejected encounter changed state.");
        }, "Adapt");
        await Case("spell-guards", "Validate spell casting", "Unknown/unlearned spells, missing spellbooks and insufficient mana are rejected without cost.", async f =>
        {
            await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "missing" }, 400);
            await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "visions" }, 400);
            f.Hero.SpellsJson = Strings("visions"); await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "visions" }, 400);
            TownRules.AddGear(f.Hero, TownRules.Spellbook); f.Hero.Mana = 0; await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "visions" }, 400);
            Require((await Hero(f)).Mana == 0, "Rejected spell changed mana.");
        });
        await Case("spell-door-land", "Dimension Door landing validation", "Solid and out-of-bounds destinations are rejected without spending mana; unexplored clear land remains legal in SoD.", async f =>
        {
            TownRules.AddGear(f.Hero, TownRules.Spellbook); f.Hero.SpellsJson = Strings("dimensionDoor"); f.Hero.Mana = 100;
            f.Player.FogJson = GameService.SerializeFog(new byte[49]); await f.SaveMap();
            foreach (var (x, y) in new[] { (-1, 5), (2, 2) })
                await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "dimensionDoor", x, y }, 400);
            Require((await Hero(f)).Mana == 100, "Invalid teleport spent mana.");
            await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "dimensionDoor", x = 5, y = 5 });
            Require((await Hero(f)).Mana == 75 && (await Hero(f)).X == 5 && (await Hero(f)).Movement == 1200, "SoD Dimension Door rejected unexplored clear land or used the wrong cost.");
        });
        await Case("stables-week", "Visit Castle Stables", "Gain 400 movement once per week; daily refresh keeps it until the week ends.", async f =>
        {
            f.Town.BuildingsJson = Strings("villageHall", "fort", "special2");
            f.Hero.Movement = f.Hero.MaxMovement = GameService.MaxMovement(f.Hero);
            f.Hero.X = 2; f.Hero.Y = 3; await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 2 } } });
            Require((await Hero(f)).MaxMovement == 1960, "Stables did not increase the daily budget.");
            var movement = (await Hero(f)).Movement;
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 3 }, new { x = 2, y = 2 } } });
            Require((await Hero(f)).MaxMovement == 1960 && (await Hero(f)).Movement < movement, "Repeat visits stacked the weekly Stables bonus.");
            await Tick(f);
            Require((await Hero(f)).Movement == 1960, "Stables bonus was not restored the next day.");
            for (var i = 0; i < 6; i++) await Tick(f);
            Require((await Hero(f)).MaxMovement == 1560, "Stables bonus leaked into the next week.");
        });
        await Case("transform-stack", "Use the Skeleton Transformer", "A living visitor stack becomes the same number of Skeletons; undead cannot be transformed again.", async f =>
        {
            f.Town.Faction = "necropolis"; f.Town.BuildingsJson = Strings("special3"); await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/transform", new { army = "hero", slot = 0 });
            Require(GameService.ParseArmy((await Hero(f)).ArmyJson)[0] == new GameService.ArmySlot("skeleton", 25), "Transformer did not preserve the stack count.");
            await Post($"/api/towns/{f.Town.Id}/transform", new { army = "hero", slot = 0 }, 400);
        });
        await Case("tower-post-comment", "Publish a journal entry and comment", "The owner publishes in a built Tower; a comment is stored under the selected post.", async f =>
        {
            f.Town.BuildingsJson = Strings("mageGuild1"); await f.SaveMap();
            var post = await Post($"/api/towns/{f.Town.Id}/posts", new { title = "Expedition", body = "Found a mine." });
            var postId = post.GetProperty("postId").GetGuid();
            await Post($"/api/posts/{postId}/comments", new { body = "Recorded." });
            var comments = await Request("GET", $"/api/posts/{postId}/comments");
            Require(comments.GetArrayLength() == 1 && comments[0].GetProperty("body").GetString() == "Recorded.", "Comment was not persisted under the journal entry.");
        }, "Adapt");
        await Case("tower-capacity", "Respect a Tower's journal capacity", "A first-level Tower accepts three entries, then rejects a fourth without inserting it.", async f =>
        {
            f.Town.BuildingsJson = Strings("mageGuild1"); await f.SaveMap();
            for (var i = 0; i < 3; i++)
                await Post($"/api/towns/{f.Town.Id}/posts", new { title = $"Entry {i}", body = "Notes." });
            await Post($"/api/towns/{f.Town.Id}/posts", new { title = "Overflow", body = "Notes." }, 400);
            Require(await f.Db.Posts.CountAsync() == 3, "Rejected entry changed the Tower's posts.");
        }, "Adapt");
        await Case("movement-army-speed", "Derive daily movement from the army", "Replacing the slowest creature with a faster stack changes the next day's base movement budget.", f =>
        {
            f.Hero.ArmyJson = Army(("pikeman", 1));
            var slow = GameService.MaxMovement(f.Hero);
            f.Hero.ArmyJson = Army(("angel", 1));
            Require(GameService.MaxMovement(f.Hero) > slow, "Daily movement ignores the slowest creature in the army.");
            return Task.CompletedTask;
        });
        await Case("spell-haste", "Use peaceful Haste", "The contract adapts Haste into an adventure movement benefit, not an inert combat cast.", async f =>
        {
            TownRules.AddGear(f.Hero, TownRules.Spellbook); f.Hero.SpellsJson = Strings("haste"); await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "haste" });
            Require((await Hero(f)).Movement > 1500, "Haste spent mana without granting an adventure movement benefit.");
        }, "Adapt");
        foreach (var key in new[] { "viewAir", "viewEarth", "cure" })
            await Case($"spell-contract-{key}", $"Provide the contract's {key} utility", "The contract's utility must exist and must not be acknowledged as an inert combat spell.", async f =>
            {
                Require(Spells.All.ContainsKey(key), $"{key} is absent from the spell catalogue.");
                TownRules.AddGear(f.Hero, TownRules.Spellbook); f.Hero.SpellsJson = Strings(key); f.Hero.Mana = 100; await f.SaveMap();
                var result = await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = key });
                Require(!result.GetProperty("message").GetString()!.Contains("no effect"), "The utility is acknowledged but inert.");
            }, "Adapt");
        foreach (var choice in new[] { "gold", "experience" })
            await Case($"treasure-{choice}", $"Choose treasure {choice}", "Persist the pending choice; grant only the selected reward and reject replay.", async f =>
            {
                await MoveObject(f, "treasure", "1500:1000");
                var pending = await Request("GET", $"/api/heroes/{f.Hero.Id}");
                Require(pending.GetProperty("pendingTreasure").GetProperty("gold").GetInt32() == 1500
                    && (await Player(f)).Gold == 20000 && (await Hero(f)).Experience == 0, "Opening did not persist an unpaid choice.");
                await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 5 } } }, 400);
                await Post($"/api/heroes/{f.Hero.Id}/treasure", new { choice });
                await Post($"/api/heroes/{f.Hero.Id}/treasure", new { choice }, 400);
                Require((await Player(f)).Gold == (choice == "gold" ? 21500 : 20000)
                    && (await Hero(f)).Experience == (choice == "experience" ? 1000 : 0)
                    && (await Hero(f)).PendingTreasureJson == null, "Reward was doubled, replayed or not cleared.");
                var saved = await f.Db.Games.AsNoTracking().SingleAsync();
                Require(!(await f.Service.LoadMapAsync(saved)).Tiles.Any(t => t.ObjectKind == "treasure"), "Claimed chest remained on the map.");
            });
        await Case("treasure-leave", "Leave and revisit a chest", "Cancellation pays nothing and retains the chest for a later choice.", async f =>
        {
            await MoveObject(f, "treasure", "1500:1000");
            await Post($"/api/heroes/{f.Hero.Id}/treasure", new { choice = "both" }, 400);
            await Post($"/api/heroes/{f.Hero.Id}/treasure", new { choice = "gold" }, 400, "bob");
            await Post($"/api/heroes/{f.Hero.Id}/treasure", new { choice = "leave" });
            Require((await Player(f)).Gold == 20000 && (await Hero(f)).Experience == 0, "Leaving awarded a reward.");
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 5 }, new { x = 2, y = 4 } } });
            await Post($"/api/heroes/{f.Hero.Id}/treasure", new { choice = "gold" });
            Require((await Player(f)).Gold == 21500, "Retained chest could not be collected later.");
        });
        await Case("treasure-concurrent", "Submit conflicting chest choices concurrently", "Exactly one gold/experience request succeeds; both rewards can never be paid.", async f =>
        {
            await MoveObject(f, "treasure", "1500:1000");
            async Task<int> Choose(string choice)
            {
                using var request = new HttpRequestMessage(HttpMethod.Post, $"/api/heroes/{f.Hero.Id}/treasure");
                request.Headers.Add("X-Player-Id", "alice");
                request.Content = JsonContent.Create(new { choice });
                using var response = await client.SendAsync(request);
                return (int)response.StatusCode;
            }
            var codes = await Task.WhenAll(Choose("gold"), Choose("experience"));
            Require(codes.Order().SequenceEqual(new[] { 200, 400 }), $"Conflicting choices returned {string.Join(',', codes)}.");
            var gold = (await Player(f)).Gold;
            var xp = (await Hero(f)).Experience;
            Require(gold == 21500 && xp == 0 || gold == 20000 && xp == 1000, "Concurrent choices paid both rewards or neither.");
        });
        foreach (var kind in new[] { "town", "monster", "mine", "shrine", "obelisk", "block", "tree", "resource", "treasure" })
            await Case($"flight-cross-{kind}", $"Fly across {kind}", "Crossing does not interact, remove, claim or make the object walkable.", async f =>
            {
                f.Hero.X = 2; f.Hero.Y = 3;
                var tile = f.Map.At(2, 4);
                tile.ObjectKind = kind;
                tile.Owner = 0;
                tile.ObjectData = kind == "resource" ? "gold:500" : kind == "monster" ? "pikeman:5" : "1500:1000";
                tile.Passable = kind is "resource" or "treasure";
                f.Player.FogJson = GameService.SerializeFog(Enumerable.Repeat((byte)2, 49).ToArray());
                TownRules.AddGear(f.Hero, TownRules.Spellbook);
                f.Hero.SpellsJson = Strings("fly"); f.Hero.Mana = 100;
                await f.SaveMap();
                await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "fly" });
                await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 4 }, new { x = 2, y = 5 } } });
                var hero = await Hero(f);
                var saved = await f.Db.Games.AsNoTracking().SingleAsync();
                var after = (await f.Service.LoadMapAsync(saved)).At(2, 4);
                Require(hero.Y == 5 && hero.Movement == 1220 && hero.PendingEncounterJson == null && hero.PendingTreasureJson == null
                    && (await Player(f)).Gold == 20000 && after.ObjectKind == kind && after.Owner == 0
                    && (kind is "resource" or "treasure" || !after.Passable), "Flight landed on, collected, altered or mispriced a crossed object.");
            });
        await Case("flight-landings", "Validate flight landings", "Reject unseen, occupied and obstructed landings without spending movement; retain a legal affordable prefix.", async f =>
        {
            f.Hero.X = 2; f.Hero.Y = 3; f.Hero.Movement = 200;
            f.Map.At(2, 4).ObjectKind = "tree"; f.Map.At(2, 4).Passable = false;
            var fog = Enumerable.Repeat((byte)2, 49).ToArray(); fog[6 * 7 + 2] = 0;
            f.Player.FogJson = GameService.SerializeFog(fog);
            f.Db.Heroes.Add(new Hero { GameId = f.Game.Id, PlayerId = f.Player.Id, X = 5, Y = 5 });
            TownRules.AddGear(f.Hero, TownRules.Spellbook); f.Hero.SpellsJson = Strings("fly"); f.Hero.Mana = 100; await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "fly" });
            foreach (var point in new[] { new { x = 2, y = 4 }, new { x = 2, y = 6 }, new { x = 5, y = 5 } })
                await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { point } }, 400);
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 4 }, new { x = 2, y = 5 } } }, 400);
            Require((await Hero(f)).Movement == 200 && (await Hero(f)).Y == 3, "Rejected flight stranded the hero or spent movement.");
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 3, y = 3 }, new { x = 4, y = 3 } } });
            Require((await Hero(f)).X == 3 && (await Hero(f)).Movement == 60, "Flight did not stop at the affordable legal landing.");
        });
        await Case("flight-visit", "Land beside and visit a creature", "Flight can approach a creature but the hero remains on adjacent ground.", async f =>
        {
            f.Hero.X = 2; f.Hero.Y = 3;
            f.Map.At(2, 5).ObjectKind = "monster"; f.Map.At(2, 5).ObjectData = "pikeman:5"; f.Map.At(2, 5).Passable = false;
            f.Player.FogJson = GameService.SerializeFog(Enumerable.Repeat((byte)2, 49).ToArray());
            TownRules.AddGear(f.Hero, TownRules.Spellbook); f.Hero.SpellsJson = Strings("fly5"); f.Hero.Mana = 100; await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "fly5" });
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 4 }, new { x = 2, y = 5 } } });
            Require((await Hero(f)).Y == 4 && (await Hero(f)).PendingEncounterJson != null, "Flight could not visit without occupying the creature.");
        });
        await Case("flight-invalid-route", "Reject malformed flight approaches", "An out-of-bounds approach to a solid destination returns a rejection, not a server error.", async f =>
        {
            f.Hero.X = 2; f.Hero.Y = 3;
            f.Map.At(2, 5).ObjectKind = "monster"; f.Map.At(2, 5).ObjectData = "pikeman:5"; f.Map.At(2, 5).Passable = false;
            f.Player.FogJson = GameService.SerializeFog(Enumerable.Repeat((byte)2, 49).ToArray());
            TownRules.AddGear(f.Hero, TownRules.Spellbook); f.Hero.SpellsJson = Strings("fly"); f.Hero.Mana = 100; await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "fly" });
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = -1, y = 4 }, new { x = 2, y = 5 } } }, 400);
            Require((await Hero(f)).Movement == 1500 && (await Hero(f)).Y == 3, "Malformed flight changed movement or position.");
        });
        await Case("shrine-legacy-fly", "Learn Fly from an old saved shrine", "Legacy fly5 keys still teach the canonical spell without making the shrine passable.", async f =>
        {
            TownRules.AddGear(f.Hero, TownRules.Spellbook); f.Hero.SkillsJson = "{\"wisdom\":3}";
            await MoveObject(f, "shrine", "fly5", false);
            Require(GameService.ParseStringArray((await Hero(f)).SpellsJson).SequenceEqual(new[] { "fly" })
                && (await Hero(f)).Y == 3, "The legacy shrine did not teach canonical Fly from adjacent ground.");
        });
        foreach (var mastery in new[] { 0, 1, 2, 3 })
            await Case($"flight-mastery-{mastery}", $"Fly at Air mastery {mastery}", "Mana and diagonal movement use 20/15 mana and 40/20/0 percent movement penalties.", async f =>
            {
                f.Hero.X = 2; f.Hero.Y = 3;
                f.Hero.SkillsJson = JsonSerializer.Serialize(new { air = mastery });
                f.Player.FogJson = GameService.SerializeFog(Enumerable.Repeat((byte)2, 49).ToArray());
                TownRules.AddGear(f.Hero, TownRules.Spellbook); f.Hero.SpellsJson = Strings("fly"); f.Hero.Mana = 100; await f.SaveMap();
                await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "fly" });
                await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 3, y = 4 } } });
                var cost = 141 * (mastery >= 3 ? 100 : mastery >= 2 ? 120 : 140) / 100;
                Require((await Hero(f)).Movement == 1500 - cost && (await Hero(f)).Mana == (mastery == 0 ? 80 : 85), "Flight mastery cost differs from its actual effect.");
            });
        await Case("movement-speed-table", "Use the slowest stack and Logistics", "A mixed army uses speed 4 = 1560; speed 12 caps at 2000, with 10/20/30 percent Logistics.", async f =>
        {
            f.Hero.ArmyJson = Army(("angel", 1), ("pikeman", 1));
            Require(GameService.MaxMovement(f.Hero) == 1560, "Mixed army did not use the slowest speed.");
            foreach (var mastery in new[] { 1, 2, 3 })
            {
                f.Hero.SkillsJson = JsonSerializer.Serialize(new { logistics = mastery });
                Require(GameService.MaxMovement(f.Hero) == 1560 * (100 + mastery * 10) / 100, "Logistics multiplier is incorrect.");
            }
            f.Hero.SkillsJson = "{}"; f.Hero.ArmyJson = Army(("angel", 1)); await f.SaveMap();
            await Tick(f);
            Require((await Hero(f)).Movement == 2000 && (await Hero(f)).MaxMovement == 2000, "Next-day movement did not use the changed army.");
        });
        foreach (var mastery in new[] { 0, 1, 2, 3 })
            await Case($"haste-day-{mastery}", $"Apply Haste at Air mastery {mastery}", "Grant 300/500 once, persist it, reject recasting without spending mana, and expire tomorrow.", async f =>
            {
                f.Hero.SkillsJson = JsonSerializer.Serialize(new { air = mastery });
                TownRules.AddGear(f.Hero, TownRules.Spellbook); f.Hero.SpellsJson = Strings("haste"); f.Hero.Mana = 100; await f.SaveMap();
                await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "haste" });
                await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "haste" }, 400);
                var hero = await Hero(f); var bonus = mastery >= 2 ? 500 : 300;
                Require(hero.Movement == 1500 + bonus && hero.MaxMovement == 1500 + bonus && GameService.Effects(hero).HasteBonus == bonus
                    && hero.Mana == (mastery == 0 ? 94 : 95), "Haste stacked, used the wrong bonus or charged a rejected recast.");
                await Tick(f);
                Require((await Hero(f)).Movement == 1560 && GameService.Effects(await Hero(f)).HasteBonus == 0, "Haste survived the next-day refresh.");
            }, "Adapt");
        await Case("cure-penalty", "Cure a negative adventure effect", "Remove the penalty without refilling movement, and preserve beneficial Fly/Haste effects.", async f =>
        {
            TownRules.AddGear(f.Hero, TownRules.Spellbook); f.Hero.SpellsJson = Strings("cure"); f.Hero.Mana = 100;
            f.Hero.SkillsJson = "{\"water\":1}";
            GameService.SetEffects(f.Hero, new AdventureEffects(f.Game.CurrentTurn, 40, f.Game.CurrentTurn, 300, 400));
            f.Hero.Movement = 500; f.Hero.MaxMovement = 1460; await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "cure" });
            var hero = await Hero(f); var effects = GameService.Effects(hero);
            Require(hero.Movement == 500 && hero.MaxMovement == 1860 && hero.Mana == 95 && effects.MovementPenalty == 0
                && effects.FlightTurn == f.Game.CurrentTurn && effects.HasteBonus == 300, "Cure refilled movement, removed beneficial effects or retained the penalty.");
        }, "Adapt");
        foreach (var key in new[] { "viewAir", "viewEarth" })
        foreach (var mastery in new[] { 0, 1, 2, 3 })
            await Case($"{key}-mastery-{mastery}", $"Survey with {key} mastery {mastery}", "Reveal only the mastery-appropriate locations and terrain, without changing exploration.", async f =>
            {
                f.Map.At(5, 5).ObjectKind = "artifact"; f.Map.At(5, 5).ObjectData = "artifact1";
                f.Map.At(6, 6).ObjectKind = "resource"; f.Map.At(6, 6).ObjectData = "wood:5";
                f.Map.At(5, 1).ObjectKind = "mine"; f.Map.At(5, 1).ObjectData = "ore"; f.Map.At(5, 1).Passable = false;
                f.Hero.SkillsJson = JsonSerializer.Serialize(new Dictionary<string, int> { [key == "viewAir" ? "air" : "earth"] = mastery });
                TownRules.AddGear(f.Hero, TownRules.Spellbook); f.Hero.SpellsJson = Strings(key); f.Hero.Mana = 100;
                var beforeFog = f.Player.FogJson; await f.SaveMap();
                var result = await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = key });
                var view = result.GetProperty("view");
                var kinds = view.GetProperty("locations").EnumerateArray().Select(p => p.GetProperty("kind").GetString()).Order().ToArray();
                var expected = key == "viewAir" ? mastery >= 3 ? new[] { "artifact", "hero", "town" }
                    : mastery >= 2 ? new[] { "artifact", "hero" } : new[] { "artifact" }
                    : mastery >= 2 ? new[] { "mine", "resource" } : new[] { "resource" };
                Require(kinds.SequenceEqual(expected.Order()), $"Wrong survey markers: {string.Join(',', kinds)}.");
                Require(view.GetProperty("terrain").EnumerateArray().Count(t => t.ValueKind != JsonValueKind.Null) == (key == "viewEarth" && mastery == 3 ? 49 : 0)
                    && (await Player(f)).FogJson == beforeFog && (await Hero(f)).Mana == (mastery == 0 ? 98 : 99), "Survey leaked terrain, changed exploration or charged the wrong mana.");
            });
        await app.StopAsync();
        Console.WriteLine("ACTION_RESULTS " + JsonSerializer.Serialize(results, Json));
        return failures == 0 ? 0 : 1;
    }

    private static string Strings(params string[] values) => JsonSerializer.Serialize(values);
    private static string Army(params (string Creature, int Count)[] stacks) =>
        GameService.SerializeArmy(stacks.Select(s => new GameService.ArmySlot(s.Creature, s.Count))
            .Concat(Enumerable.Repeat(new GameService.ArmySlot(null, 0), 7 - stacks.Length)).ToArray());
}
