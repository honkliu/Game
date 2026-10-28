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
        builder.Services.AddScoped(_ => new AppDb(new DbContextOptionsBuilder<AppDb>().UseSqlite(current!.Connection).Options));
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
            Require((await Player(f)).Gold == 59 && (await Town(f)).GarrisonJson == "[]", "Unaffordable purchase mutated state.");
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
            Require((await Player(f)).Gold == 20000, "Rejected upgrade charged gold.");
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
            Require((await Player(f)).Gold == 20000, "Rejected trade changed gold.");
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
        await Case("hire-hero", "Hire a tavern hero", "Pay 2000 gold, add the chosen hero and remove that offer.", async f =>
        {
            f.Hero.X = 2; f.Hero.Y = 3; await f.SaveMap();
            await Post($"/api/towns/{f.Town.Id}/hire", new { heroKey = "sorsha" });
            Require((await Player(f)).Gold == 18000 && await f.Db.Heroes.CountAsync() == 2, "Hiring cost or hero count is incorrect.");
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
            f.Town.MageGuildJson = Strings("magicArrow", "visions", "prayer");
            var learned = TownRules.LearnGuildSpells(f.Hero, f.Town);
            Require(learned.Contains("magicArrow") && learned.Contains("visions") && !learned.Contains("prayer"), "Guild ignored Wisdom.");
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
        await Case("mine-owner-consent", "Visit another player's mine", "First discovery claims it; another player cannot grant themselves ownership or sharing.", async f =>
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
            Require(arranged.Worn.TryGetValue("leftHand", out var key) && key == "artifact1", "Shield did not occupy the shield slot.");
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
            await MoveObject(f, "shrine", "prayer", false);
            Require(!GameService.ParseStringArray((await Hero(f)).SpellsJson).Contains("prayer"), "Shrine taught a third-level spell without Wisdom.");
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
            Require(!(await Player(f)).Precious, "Grail structure did not consume the Grail.");
        });
        await Case("movement-sand", "Move across sand", "Sand has the same 150-point base terrain cost as snow before Pathfinding.", f =>
        {
            Require(Terrain.MoveCost("sand") == 150, $"Sand movement cost is {Terrain.MoveCost("sand")}.");
            return Task.CompletedTask;
        });
        await Case("movement-pathfinding", "Apply the Pathfinding skill", "Basic Pathfinding reduces a snow step from 150 to 125 points.", async f =>
        {
            f.Hero.X = 2; f.Hero.Y = 3; f.Hero.SkillsJson = "{\"pathfinding\":1}";
            f.Map.At(2, 4).Terrain = "snow"; await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 4 } } });
            Require((await Hero(f)).Movement == 1375, $"Expected 1375 movement, got {(await Hero(f)).Movement}.");
        });
        await Case("hero-level", "Gain a hero level", "Each level grants one primary-skill point and one secondary-skill advancement.", f =>
        {
            var before = f.Hero.Attack + f.Hero.Defense + f.Hero.Power + f.Hero.Knowledge;
            GameService.GainExperience(f.Hero, 1000);
            var after = f.Hero.Attack + f.Hero.Defense + f.Hero.Power + f.Hero.Knowledge;
            Require(f.Hero.Level == 2 && after == before + 1 && GameService.ParseSkills(f.Hero.SkillsJson).Count > 0, "Level-up grants only a primary OR secondary advancement rather than both.");
            return Task.CompletedTask;
        });
        await Case("spell-flight", "Cast Fly", "Fly must enable a day-long flight movement mode, not acknowledge an inert spell.", async f =>
        {
            f.Hero.X = 2; f.Hero.Y = 3;
            f.Map.At(2, 4).ObjectKind = "tree"; f.Map.At(2, 4).Passable = false;
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
            });
        await Case("shrine-learn", "Learn from a shrine and revisit it", "An eligible visitor learns once; the structure persists and the hero remains adjacent.", async f =>
        {
            TownRules.AddGear(f.Hero, TownRules.Spellbook); f.Hero.SkillsJson = "{\"wisdom\":1}";
            await MoveObject(f, "shrine", "prayer", false);
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 4 } } });
            var h = await Hero(f);
            var map = JsonSerializer.Deserialize<AdventureMap>((await f.Db.Games.AsNoTracking().SingleAsync()).MapJson, Json)!;
            Require(GameService.ParseStringArray(h.SpellsJson).Count(s => s == "prayer") == 1
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
        await Case("spell-door-land", "Dimension Door landing validation", "Solid, out-of-bounds, and unseen destinations are rejected without spending mana.", async f =>
        {
            TownRules.AddGear(f.Hero, TownRules.Spellbook); f.Hero.SpellsJson = Strings("dimensionDoor"); f.Hero.Mana = 100;
            f.Player.FogJson = GameService.SerializeFog(new byte[49]); await f.SaveMap();
            foreach (var (x, y) in new[] { (-1, 5), (2, 2), (5, 5) })
                await Post($"/api/heroes/{f.Hero.Id}/cast", new { spell = "dimensionDoor", x, y }, 400);
            Require((await Hero(f)).Mana == 100, "Invalid teleport spent mana.");
        });
        await Case("stables-week", "Visit Castle Stables", "Gain 400 movement once per week; daily refresh keeps it until the week ends.", async f =>
        {
            f.Town.BuildingsJson = Strings("villageHall", "fort", "special2");
            f.Hero.X = 2; f.Hero.Y = 3; await f.SaveMap();
            await Post($"/api/heroes/{f.Hero.Id}/move", new { path = new[] { new { x = 2, y = 2 } } });
            Require((await Hero(f)).MaxMovement == 1900, "Stables did not increase the daily budget.");
            await Tick(f);
            Require((await Hero(f)).Movement == 1900, "Stables bonus was not restored the next day.");
            for (var i = 0; i < 6; i++) await Tick(f);
            Require((await Hero(f)).MaxMovement == 1500, "Stables bonus leaked into the next week.");
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
        await app.StopAsync();
        Console.WriteLine("ACTION_RESULTS " + JsonSerializer.Serialize(results, Json));
        return failures == 0 ? 0 : 1;
    }

    private static string Strings(params string[] values) => JsonSerializer.Serialize(values);
    private static string Army(params (string Creature, int Count)[] stacks) =>
        GameService.SerializeArmy(stacks.Select(s => new GameService.ArmySlot(s.Creature, s.Count))
            .Concat(Enumerable.Repeat(new GameService.ArmySlot(null, 0), 7 - stacks.Length)).ToArray());
}
