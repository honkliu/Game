using System.Text.Json;
using Microsoft.EntityFrameworkCore;
using CityOfTowers.Data;
using CityOfTowers.Domain;
using CityOfTowers.Services;

namespace CityOfTowers.Api;

public static partial class Endpoints
{
    private static readonly JsonSerializerOptions Json = new() { PropertyNamingPolicy = JsonNamingPolicy.CamelCase };

    public static void MapApi(this WebApplication app)
    {
        app.Use(async (context, next) =>
        {
            var serialized = context.Request.Path.StartsWithSegments("/api")
                && (!HttpMethods.IsGet(context.Request.Method)
                    || context.Request.Path.Value!.EndsWith("/map", StringComparison.Ordinal)
                    || context.Request.Path.Value!.EndsWith("/visions", StringComparison.Ordinal));
            if (!serialized) { await next(context); return; }
            await GameService.MutationLock.WaitAsync(context.RequestAborted);
            try { await next(context); }
            finally { GameService.MutationLock.Release(); }
        });
        app.MapHeroActions();
        app.MapPeacefulActions();
        // ------------------------------------------------------------ players
        app.MapGet("/api/players", () => Results.Ok(Seeded.Players.Select(p => new
        {
            id = p.Id, name = p.Name, faction = p.Faction, cityName = p.CityName
        })));

        // ------------------------------------------------------------ games
        app.MapGet("/api/games", async (AppDb db) =>
        {
            var games = await db.Games.ToListAsync();
            var players = await db.Players.ToListAsync();
            return Results.Ok(games.Select(g => new
            {
                id = g.Id,
                name = g.Name,
                state = g.State.ToString(),
                day = g.Day,
                week = g.Week,
                month = g.Month,
                turn = g.CurrentTurn,
                players = players.Count(p => p.GameId == g.Id)
            }));
        });

        app.MapPost("/api/games", async (CreateGameReq req, GameService games) =>
        {
            var game = await games.CreateGameAsync(
                req.Name ?? "The Three Cities",
                req.Capacity <= 0 ? 3 : req.Capacity,
                req.TurnSeconds <= 0 ? 300 : req.TurnSeconds,
                req.TurnLimit,
                req.MapSize <= 0 ? 72 : req.MapSize,
                Math.Clamp(req.AiPlayers, 0, 2));
            return Results.Ok(new { gameId = game.Id });
        });

        // Take a seat in an existing game. Idempotent: a player who already has a seat
        // gets it back, so the lobby's Enter button is safe to press repeatedly.
        app.MapPost("/api/games/{id:guid}/join", async (Guid id, AppDb db, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });

            var game = await db.Games.FirstOrDefaultAsync(g => g.Id == id);
            if (game == null) return Results.NotFound(new { error = "That game no longer exists." });

            var existing = await db.Players.FirstOrDefaultAsync(p => p.GameId == id && p.Faction == sp.Faction);
            if (existing != null) return Results.Ok(new { playerId = existing.Id });

            // the seat is created with the game, so a missing one means the faction is not in this game
            return Results.BadRequest(new { error = "That city is not part of this game." });
        });

        app.MapGet("/api/games/{id:guid}", async (Guid id, AppDb db) =>
        {
            var game = await db.Games.FirstOrDefaultAsync(g => g.Id == id);
            if (game == null) return Results.NotFound(new { error = "That game no longer exists." });

            var players = await db.Players.Where(p => p.GameId == id).ToListAsync();
            var towns = await db.Towns.Where(t => t.GameId == id).ToListAsync();
            var heroes = await db.Heroes.Where(h => h.GameId == id).ToListAsync();

            return Results.Ok(new
            {
                id = game.Id,
                name = game.Name,
                state = game.State.ToString(),
                day = game.Day,
                week = game.Week,
                month = game.Month,
                turn = game.CurrentTurn,
                turnSeconds = game.TurnSeconds,
                turnLimit = game.TurnLimit,
                turnStartedAt = game.TurnStartedAt,
                mapWidth = game.MapWidth,
                mapHeight = game.MapHeight,
                winnerUserId = game.WinnerUserId,
                endAgreement = GameService.ParseStringArray(game.EndAgreementJson),
                grailRevealed = game.GrailRevealed,
                grailX = game.GrailRevealed ? game.GrailX : (int?)null,
                grailY = game.GrailRevealed ? game.GrailY : (int?)null,
                players = players.Select(p => new
                {
                    playerId = p.Id,
                    name = Seeded.Players.FirstOrDefault(s => s.Faction == p.Faction)?.Name ?? "?",
                    faction = p.Faction,
                    cityName = p.CityName,
                    isAi = p.IsAi,
                    precious = p.Precious,
                    obeliskVisits = p.ObeliskVisits,
                    resources = new
                    {
                        wood = p.Wood, ore = p.Ore, mercury = p.Mercury, sulfur = p.Sulfur,
                        crystal = p.Crystal, gems = p.Gems, gold = p.Gold
                    },
                    townId = towns.FirstOrDefault(t => t.PlayerId == p.Id)?.Id,
                    heroIds = heroes.Where(h => h.PlayerId == p.Id).Select(h => h.Id)
                })
            });
        });

        // ------------------------------------------------------------ map
        app.MapGet("/api/games/{id:guid}/map", async (Guid id, AppDb db, GameService games, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });

            var game = await db.Games.FirstOrDefaultAsync(g => g.Id == id);
            if (game == null) return Results.NotFound();

            var player = await db.Players.FirstOrDefaultAsync(p => p.GameId == id && p.Faction == sp.Faction);
            if (player == null) return Results.NotFound();

            var map = await games.LoadMapAsync(game);
            var fog = GameService.ParseFog(player.FogJson, map.Width * map.Height);
            var heroes = await db.Heroes.Where(h => h.GameId == id).ToListAsync();
            var towns = await db.Towns.Where(t => t.GameId == id).ToListAsync();

            var tiles = new List<object>(map.Tiles.Count);
            for (var i = 0; i < map.Tiles.Count; i++)
            {
                var t = map.Tiles[i];
                var f = fog[i];
                if (f == 0)
                {
                    tiles.Add(new { terrain = (string?)null, variant = 0, objectKind = (string?)null, objectData = (string?)null, owner = -1, passable = false, fog = 0 });
                    continue;
                }
                tiles.Add(new
                {
                    terrain = t.Terrain,
                    variant = t.Variant,
                    objectKind = t.ObjectKind,
                    objectData = t.ObjectData,
                    sprite = t.Sprite,
                    owner = t.Owner,
                    mineShared = t.MineShared,
                    passable = t.Passable,
                    fog = f
                });
            }

            // Like HoMM3: other heroes are seen only in sight, other towns once explored.
            bool InSight(int x, int y) => map.InBounds(x, y) && fog[y * map.Width + x] == 2;
            bool Explored(int x, int y) => map.InBounds(x, y) && fog[y * map.Width + x] > 0;

            return Results.Ok(new
            {
                width = map.Width,
                height = map.Height,
                tiles,
                heroes = heroes.Where(h => h.PlayerId == player.Id || InSight(h.X, h.Y)).Select(h => new
                {
                    id = h.Id,
                    name = h.Name,
                    faction = h.Faction,
                    x = h.X,
                    y = h.Y,
                    level = h.Level,
                    movement = h.Movement,
                    maxMovement = h.MaxMovement,
                    nativeTerrain = GameService.NativeTerrain(h),
                    isMine = h.PlayerId == player.Id
                }),
                towns = towns.Where(t => t.PlayerId == player.Id || Explored(t.X, t.Y)).Select(t => new
                {
                    id = t.Id,
                    name = t.Name,
                    faction = t.Faction,
                    x = t.X,
                    y = t.Y,
                    isMine = t.PlayerId == player.Id,
                    neutral = t.PlayerId == Guid.Empty
                })
            });
        });

        // ------------------------------------------------------------ heroes
        app.MapGet("/api/games/{id:guid}/heroes", async (Guid id, AppDb db, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });

            var player = await db.Players.FirstOrDefaultAsync(p => p.GameId == id && p.Faction == sp.Faction);
            if (player == null) return Results.NotFound();

            var heroes = await db.Heroes.Where(h => h.PlayerId == player.Id).ToListAsync();
            return Results.Ok(heroes.Select(HeroDto));
        });

        app.MapGet("/api/heroes/{heroId:guid}", async (Guid heroId, AppDb db) =>
        {
            var hero = await db.Heroes.FirstOrDefaultAsync(h => h.Id == heroId);
            if (hero == null) return Results.NotFound();
            return Results.Ok(HeroDto(hero));
        });

        app.MapPost("/api/heroes/{heroId:guid}/move", async (Guid heroId, MoveReq req, GameService games, WsHub ws, HttpContext ctx) =>
        {
            var playerId = ctx.Request.Headers["X-Player-Id"].ToString();
            var path = (req.Path ?? []).Select(p => (p.X, p.Y)).ToList();
            var (ok, error, hero) = await games.MoveHeroAsync(heroId, playerId, path);
            if (!ok) return Results.BadRequest(new { error });
            await ws.BroadcastAsync(hero!.GameId, "heroMoved", new { heroId = hero.Id, x = hero.X, y = hero.Y });
            // the hero payload is unchanged; a move notice (enter-town, pickup) rides in a header
            if (games.LastMoveMessage != null) ctx.Response.Headers["X-Move-Message"] = games.LastMoveMessage;
            if (games.LastMoveNotices.Count > 0)
                ctx.Response.Headers["X-Move-Events"] = JsonSerializer.Serialize(games.LastMoveNotices, Json);
            return Results.Ok(HeroDto(hero!));
        });

        app.MapPost("/api/heroes/{heroId:guid}/treasure", async (Guid heroId, TreasureReq req, GameService games, AppDb db, WsHub ws, HttpContext ctx) =>
        {
            var (ok, error, message) = await games.ResolveTreasureAsync(heroId, ctx.Request.Headers["X-Player-Id"].ToString(), req.Choice ?? "");
            if (!ok) return Results.BadRequest(new { error });
            var hero = await db.Heroes.FirstAsync(h => h.Id == heroId);
            await ws.BroadcastAsync(hero.GameId, "heroMoved", new { heroId });
            return Results.Ok(new { message, notices = games.LastMoveNotices });
        });

        // ------------------------------------------------------------ towns
        app.MapGet("/api/towns/{townId:guid}", async (Guid townId, AppDb db, HttpContext ctx) =>
        {
            var town = await db.Towns.FirstOrDefaultAsync(t => t.Id == townId);
            if (town == null) return Results.NotFound();
            if (await TownHiddenAsync(db, townId, ctx.Request.Headers["X-Player-Id"]))
                return Results.BadRequest(new { error = "You have not discovered that town yet." });

            var player = await db.Players.FirstOrDefaultAsync(p => p.Id == town.PlayerId);
            var built = GameService.ParseStringArray(town.BuildingsJson);
            var avail = JsonSerializer.Deserialize<Dictionary<string, int>>(town.AvailableJson, Json) ?? [];
            var tiers = Creatures.BaseTiers(town.Faction);
            var owner = player == null ? null : Seeded.Players.FirstOrDefault(p => p.Faction == player.Faction);
            var selected = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            var tree = TownBuildings.For(town.Faction);
            var (machine, machinePrice) = TownBuildings.Blacksmith.TryGetValue(town.Faction, out var bs) ? bs : ("ballista", 2500);

            return Results.Ok(new
            {
                id = town.Id,
                gameId = town.GameId,
                name = town.Name,
                ownerName = player == null ? "Neutral" : owner?.Name ?? "?",
                isOwner = player != null && selected?.Faction == player.Faction,
                faction = town.Faction,
                x = town.X,
                y = town.Y,
                buildings = built,
                builtThisTurn = player?.BuiltThisTurn ?? false,
                postSlots = SocialService.TowerSlots(built),
                goldIncome = TownBuildings.GoldIncome(built),
                growthMultiplier = TownBuildings.GrowthMultiplier(built),
                marketCount = await MarketCountAsync(db, town.GameId, town.PlayerId),
                available = Enumerable.Range(1, 7).Select(tier => new
                {
                    tier,
                    creature = tiers.Length >= tier ? tiers[tier - 1] : null,
                    upgraded = tiers.Length >= tier ? Creatures.UpgradeOf(tiers[tier - 1]) : null,
                    count = avail.TryGetValue(tier.ToString(), out var c) ? c : 0,
                    dwellingBuilt = TownBuildings.HasDwelling(built, tier),
                    upgradeBuilt = TownBuildings.HasUpgrade(built, tier),
                    growth = TownBuildings.WeeklyGrowth(town.Faction, built, tier),
                    dwellingName = tree.TryGetValue((TownBuildings.HasUpgrade(built, tier) ? "upg" : "dwelling") + tier, out var dn) ? dn.Name : ""
                }),
                blacksmith = new { machine, name = TownBuildings.MachineNames[machine], gold = machinePrice },
                garrison = GameService.ParseArmy(town.GarrisonJson),
                heroInTown = player == null ? null : await HeroInTownAsync(db, player.Id, town.X, town.Y),
                tavern = GameService.ParseStringArray(town.TavernJson),
                mageGuild = GameService.ParseStringArray(town.MageGuildJson).Select(Spells.CanonicalKey).Distinct(),
                resources = new
                {
                    wood = player?.Wood ?? 0, ore = player?.Ore ?? 0, mercury = player?.Mercury ?? 0, sulfur = player?.Sulfur ?? 0,
                    crystal = player?.Crystal ?? 0, gems = player?.Gems ?? 0, gold = player?.Gold ?? 0
                }
            });
        });

        app.MapPost("/api/towns/{townId:guid}/build", async (Guid townId, BuildReq req, AppDb db, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });

            var town = await db.Towns.FirstOrDefaultAsync(t => t.Id == townId);
            if (town == null) return Results.NotFound();

            var player = await db.Players.FirstOrDefaultAsync(p => p.Id == town.PlayerId);
            if (player == null || player.Faction != sp.Faction) return Results.BadRequest(new { error = "You can only build in your own town." });

            var game = await db.Games.FirstAsync(g => g.Id == town.GameId);
            if (game.State != GameState.InProgress) return Results.BadRequest(new { error = "The game is not in progress." });
            if (player.BuiltThisTurn) return Results.BadRequest(new { error = "You have already built this turn." });

            var key = req.BuildingKey ?? "";
            var tree = TownBuildings.For(town.Faction);
            if (!tree.TryGetValue(key, out var def)) return Results.BadRequest(new { error = "Unknown building." });
            if (key == "grail" && !player.Precious)
                return Results.BadRequest(new { error = "Requires the Grail. Dig it up first." });

            var built = GameService.ParseStringArray(town.BuildingsJson).ToList();
            if (built.Contains(key)) return Results.BadRequest(new { error = "You have already built that." });

            foreach (var r in def.Requires)
                if (!built.Contains(r))
                    return Results.BadRequest(new { error = "Requires: " + (tree.TryGetValue(r, out var rd) ? rd.Name : r) + "." });

            if (player.Gold < def.Gold || player.Wood < def.Wood || player.Ore < def.Ore ||
                player.Mercury < def.Mercury || player.Sulfur < def.Sulfur ||
                player.Crystal < def.Crystal || player.Gems < def.Gems)
                return Results.BadRequest(new { error = "Not enough resources." });

            player.Gold -= def.Gold;
            player.Wood -= def.Wood;
            player.Ore -= def.Ore;
            player.Mercury -= def.Mercury;
            player.Sulfur -= def.Sulfur;
            player.Crystal -= def.Crystal;
            player.Gems -= def.Gems;

            built.Add(key);
            town.BuildingsJson = JsonSerializer.Serialize(built, Json);
            player.BuiltThisTurn = true;
            if (key == "grail") player.Precious = false;

            // HoMM3: a new dwelling opens with its first week's creatures; a horde adds its bonus at once
            var avail = JsonSerializer.Deserialize<Dictionary<string, int>>(town.AvailableJson, Json) ?? [];
            if (key.StartsWith("dwelling"))
                avail[def.Tier.ToString()] = TownBuildings.WeeklyGrowth(town.Faction, built, def.Tier);
            else if (TownBuildings.Hordes.TryGetValue(town.Faction, out var hordes) && hordes.TryGetValue(key, out var horde))
                avail[horde.Tier.ToString()] = (avail.TryGetValue(horde.Tier.ToString(), out var cur) ? cur : 0) + horde.Bonus;
            town.AvailableJson = JsonSerializer.Serialize(avail, Json);

            TownRules.StockGuild(town, key);
            var visitor = await db.Heroes.FirstOrDefaultAsync(h => h.PlayerId == player.Id && h.X == town.X && h.Y == town.Y);
            if (visitor != null) GameService.VisitOwnTown(game, visitor, town);

            await db.SaveChangesAsync();
            return Results.NoContent();
        });

        app.MapPost("/api/towns/{townId:guid}/recruit", async (Guid townId, RecruitReq req, AppDb db, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });

            var town = await db.Towns.FirstOrDefaultAsync(t => t.Id == townId);
            if (town == null) return Results.NotFound();

            var player = await db.Players.FirstOrDefaultAsync(p => p.Id == town.PlayerId);
            if (player == null || player.Faction != sp.Faction) return Results.BadRequest(new { error = "You can only recruit in your own town." });
            if (await InactiveGameAsync(db, town.GameId) is { } inactive) return inactive;

            var built = GameService.ParseStringArray(town.BuildingsJson);
            if (req.Tier is < 1 or > 7 || !TownBuildings.HasDwelling(built, req.Tier))
                return Results.BadRequest(new { error = "That dwelling is not built." });

            // an upgraded dwelling recruits either form from the same weekly pool
            var tiers = Creatures.BaseTiers(town.Faction);
            var baseKey = tiers[req.Tier - 1];
            var upgKey = Creatures.UpgradeOf(baseKey);
            var creatureKey = string.IsNullOrEmpty(req.Creature) ? baseKey : req.Creature;
            if (creatureKey != baseKey && creatureKey != upgKey)
                return Results.BadRequest(new { error = "That dwelling does not house that creature." });
            if (creatureKey == upgKey && !TownBuildings.HasUpgrade(built, req.Tier))
                return Results.BadRequest(new { error = "Upgrade the dwelling first." });

            var avail = JsonSerializer.Deserialize<Dictionary<string, int>>(town.AvailableJson, Json) ?? [];
            var k = req.Tier.ToString();
            var have = avail.TryGetValue(k, out var c) ? c : 0;
            if (req.Count > have) return Results.BadRequest(new { error = "Not that many creatures are available." });
            var count = req.Count;
            if (count <= 0) return Results.BadRequest(new { error = "No creatures available." });

            var def = Creatures.All[creatureKey];
            var cost = TownRules.CostOf(def, count);
            if (!TownRules.CanAfford(player, cost)) return Results.BadRequest(new { error = "Not enough resources." });

            var hero = await db.Heroes.FirstOrDefaultAsync(h =>
                h.PlayerId == player.Id && h.X == town.X && h.Y == town.Y);
            // HoMM3 recruits into the garrison; a visiting hero takes them only if the garrison is full
            var garrison = GameService.ParseArmy(town.GarrisonJson);
            if (GameService.AddToArmy(garrison, creatureKey, count))
                town.GarrisonJson = GameService.SerializeArmy(garrison);
            else
            {
                var army = hero == null ? null : GameService.ParseArmy(hero.ArmyJson);
                if (army == null || !GameService.AddToArmy(army, creatureKey, count))
                    return Results.BadRequest(new { error = "There is no room for them in the garrison." });
                hero!.ArmyJson = GameService.SerializeArmy(army);
                await RefreshArmyMovementAsync(db, hero);
            }

            TownRules.Pay(player, cost);
            avail[k] = have - count;
            town.AvailableJson = JsonSerializer.Serialize(avail, Json);

            await db.SaveChangesAsync();
            return Results.Ok(new { recruited = count, creature = creatureKey });
        });

        app.MapPost("/api/towns/{townId:guid}/trade", async (Guid townId, TradeReq req, AppDb db, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });

            var town = await db.Towns.FirstOrDefaultAsync(t => t.Id == townId);
            if (town == null) return Results.NotFound();
            var player = await db.Players.FirstOrDefaultAsync(p => p.Id == town.PlayerId);
            if (player == null || player.Faction != sp.Faction) return Results.BadRequest(new { error = "Not your town." });
            if (await InactiveGameAsync(db, town.GameId) is { } inactive) return inactive;

            var built = GameService.ParseStringArray(town.BuildingsJson);
            if (!built.Contains("marketplace")) return Results.BadRequest(new { error = "No Marketplace built." });

            var from = req.From ?? "";
            var to = req.To ?? "";
            var lots = req.Amount;
            if (lots <= 0) return Results.BadRequest(new { error = "Trade amount must be positive." });
            if (!TownRules.IsResource(from) || !TownRules.IsResource(to)) return Results.BadRequest(new { error = "Unknown resource." });
            if (from == to) return Results.BadRequest(new { error = "Cannot trade a resource for itself." });

            // amount is the number of lots at the HoMM3 rate, which depends on how many Marketplaces the player owns
            var markets = await MarketCountAsync(db, town.GameId, town.PlayerId);
            var (give, get) = TownRules.MarketRate(from, to, markets);
            var have = GetRes(player, from);
            var totalGive = (long)give * lots;
            var totalGet = (long)get * lots;
            if (have < totalGive) return Results.BadRequest(new { error = "Not enough " + from + "." });
            if ((long)GetRes(player, to) + totalGet > int.MaxValue) return Results.BadRequest(new { error = "That trade is too large." });
            SetRes(player, from, have - (int)totalGive);
            SetRes(player, to, GetRes(player, to) + (int)totalGet);

            await db.SaveChangesAsync();
            return Results.NoContent();
        });

        app.MapPost("/api/towns/{townId:guid}/hire", async (Guid townId, HireReq req, AppDb db, GameService games, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });

            var town = await db.Towns.FirstOrDefaultAsync(t => t.Id == townId);
            if (town == null) return Results.NotFound();
            var player = await db.Players.FirstOrDefaultAsync(p => p.Id == town.PlayerId);
            if (player == null || player.Faction != sp.Faction) return Results.BadRequest(new { error = "Not your town." });
            if (await InactiveGameAsync(db, town.GameId) is { } inactive) return inactive;

            var built = GameService.ParseStringArray(town.BuildingsJson);
            if (!built.Contains("tavern")) return Results.BadRequest(new { error = "No Tavern built." });
            if (await db.Heroes.AnyAsync(h => h.GameId == town.GameId && h.X == town.X && h.Y == town.Y))
                return Results.BadRequest(new { error = "A hero is already visiting this town." });
            if (player.Gold < 2500) return Results.BadRequest(new { error = "Not enough gold (2500)." });

            var pool = GameService.ParseStringArray(town.TavernJson).ToList();
            var key = req.HeroKey ?? "";
            if (!pool.Contains(key)) return Results.BadRequest(new { error = "That hero is not available." });

            var count = await db.Heroes.CountAsync(h => h.PlayerId == player.Id);
            if (count >= 8) return Results.BadRequest(new { error = "You already have 8 heroes." });

            player.Gold -= 2500;
            pool.Remove(key);
            town.TavernJson = JsonSerializer.Serialize(pool, Json);

            var hireGame = await db.Games.FirstAsync(g => g.Id == town.GameId);
            var (hx, hy) = await games.ArrivalSpotAsync(hireGame, town.X, town.Y);
            var hero = GameService.CreateHero(town.GameId, player.Id, sp with { HeroKey = key }, hx, hy);
            if (hx == town.X && hy == town.Y) GameService.VisitOwnTown(hireGame, hero, town);
            db.Heroes.Add(hero);
            await db.SaveChangesAsync();
            return Results.Ok(new { heroId = hero.Id });
        });

        // Move a stack between the hero standing in town and the garrison.
        app.MapPost("/api/towns/{townId:guid}/garrison", async (Guid townId, GarrisonReq req, AppDb db, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });

            var town = await db.Towns.FirstOrDefaultAsync(t => t.Id == townId);
            if (town == null) return Results.NotFound();
            var player = await db.Players.FirstOrDefaultAsync(p => p.Id == town.PlayerId);
            if (player == null || player.Faction != sp.Faction) return Results.BadRequest(new { error = "Not your town." });
            if (await InactiveGameAsync(db, town.GameId) is { } inactive) return inactive;

            var hero = await db.Heroes.FirstOrDefaultAsync(h =>
                h.PlayerId == player.Id && h.X == town.X && h.Y == town.Y);
            if (hero == null) return Results.BadRequest(new { error = "No hero is in town." });

            var direction = req.Direction ?? "";
            if (direction is not ("toGarrison" or "toHero"))
                return Results.BadRequest(new { error = "Unknown direction." });
            if (req.Slot < 0 || req.Slot > 6) return Results.BadRequest(new { error = "Unknown slot." });

            var heroArmy = GameService.ParseArmy(hero.ArmyJson);
            var garrison = GameService.ParseArmy(town.GarrisonJson);
            var from = direction == "toGarrison" ? heroArmy : garrison;
            var to = direction == "toGarrison" ? garrison : heroArmy;

            var stack = from[req.Slot];
            if (stack.Creature == null || stack.Count <= 0)
                return Results.BadRequest(new { error = "That slot is empty." });

            var count = req.Count <= 0 ? stack.Count : Math.Min(req.Count, stack.Count);
            if (!GameService.AddToArmy(to, stack.Creature, count))
                return Results.BadRequest(new { error = "The destination army is full." });

            from[req.Slot] = stack with { Count = stack.Count - count };
            if (from[req.Slot].Count <= 0) from[req.Slot] = new GameService.ArmySlot(null, 0);
            if (direction == "toGarrison" && heroArmy.All(s => s.Creature == null || s.Count <= 0))
                return Results.BadRequest(new { error = "A hero must keep at least one stack." });

            hero.ArmyJson = GameService.SerializeArmy(heroArmy);
            await RefreshArmyMovementAsync(db, hero);
            town.GarrisonJson = GameService.SerializeArmy(garrison);
            await db.SaveChangesAsync();
            return Results.Ok(new { moved = count, creature = stack.Creature });
        });

        // HoMM3 army exchange between the visiting hero and the garrison (or within either):
        // move, merge, swap or split a stack into a chosen slot.
        app.MapPost("/api/towns/{townId:guid}/army", async (Guid townId, ArmyMoveReq req, AppDb db, HttpContext ctx) =>
        {
            var (town, player, err) = await OwnTownAsync(db, ctx, townId);
            if (err != null) return err;
            var hero = await db.Heroes.FirstOrDefaultAsync(h => h.PlayerId == player!.Id && h.X == town!.X && h.Y == town.Y);
            if ((req.FromArmy == "hero" || req.ToArmy == "hero") && hero == null)
                return Results.BadRequest(new { error = "No hero is in town." });
            if (req.FromArmy is not ("hero" or "garrison") || req.ToArmy is not ("hero" or "garrison"))
                return Results.BadRequest(new { error = "Unknown army." });

            var heroArmy = hero == null ? null : GameService.ParseArmy(hero.ArmyJson);
            var garrison = GameService.ParseArmy(town!.GarrisonJson);
            var from = req.FromArmy == "hero" ? heroArmy! : garrison;
            var to = req.ToArmy == "hero" ? heroArmy! : garrison;
            var error = TownRules.Transfer(from, req.FromSlot, to, req.ToSlot, req.Count,
                req.FromArmy == "hero", req.ToArmy == "hero");
            if (error != null) return Results.BadRequest(new { error });

            if (hero != null)
            {
                hero.ArmyJson = GameService.SerializeArmy(heroArmy!);
                await RefreshArmyMovementAsync(db, hero);
            }
            town.GarrisonJson = GameService.SerializeArmy(garrison);
            await db.SaveChangesAsync();
            return Results.Ok(new { hero = heroArmy, garrison });
        });

        // Rearrange or split stacks within a hero's own army (the hero screen).
        app.MapPost("/api/heroes/{heroId:guid}/army", async (Guid heroId, ArmyMoveReq req, AppDb db, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });
            var hero = await db.Heroes.FirstOrDefaultAsync(h => h.Id == heroId);
            if (hero == null) return Results.NotFound();
            var player = await db.Players.FirstAsync(p => p.Id == hero.PlayerId);
            if (player.Faction != sp.Faction) return Results.BadRequest(new { error = "Not your hero." });
            if (await InactiveGameAsync(db, hero.GameId) is { } inactive) return inactive;

            var army = GameService.ParseArmy(hero.ArmyJson);
            var error = TownRules.Transfer(army, req.FromSlot, army, req.ToSlot, req.Count, true, true);
            if (error != null) return Results.BadRequest(new { error });
            hero.ArmyJson = GameService.SerializeArmy(army);
            await db.SaveChangesAsync();
            return Results.Ok(new { army });
        });

        // Upgrade a stack in the hero's army or the garrison (needs the upgraded dwelling; costs the price difference).
        app.MapPost("/api/towns/{townId:guid}/upgrade", async (Guid townId, StackReq req, AppDb db, HttpContext ctx) =>
        {
            var (town, player, err) = await OwnTownAsync(db, ctx, townId);
            if (err != null) return err;
            if (req.Army is not ("hero" or "garrison")) return Results.BadRequest(new { error = "Unknown army." });
            var hero = await db.Heroes.FirstOrDefaultAsync(h => h.PlayerId == player!.Id && h.X == town!.X && h.Y == town.Y);
            if (req.Army == "hero" && hero == null) return Results.BadRequest(new { error = "No hero is in town." });
            var army = req.Army == "hero" ? GameService.ParseArmy(hero!.ArmyJson) : GameService.ParseArmy(town!.GarrisonJson);
            if (req.Slot is < 0 or > 6) return Results.BadRequest(new { error = "Unknown slot." });
            var stack = army[req.Slot];
            if (stack.Creature == null || !Creatures.All.TryGetValue(stack.Creature, out var from))
                return Results.BadRequest(new { error = "That slot is empty." });
            var upKey = Creatures.UpgradeOf(stack.Creature);
            if (upKey == null || from.Faction != town!.Faction)
                return Results.BadRequest(new { error = "This town cannot upgrade that creature." });
            if (!TownBuildings.HasUpgrade(GameService.ParseStringArray(town.BuildingsJson), from.Tier))
                return Results.BadRequest(new { error = "Build the upgraded dwelling first." });

            var cost = TownRules.UpgradeCost(from, Creatures.All[upKey], stack.Count);
            if (!TownRules.CanAfford(player!, cost)) return Results.BadRequest(new { error = "Not enough resources." });
            TownRules.Pay(player!, cost);
            army[req.Slot] = stack with { Creature = upKey };
            if (req.Army == "hero")
            {
                hero!.ArmyJson = GameService.SerializeArmy(army);
                await RefreshArmyMovementAsync(db, hero);
            }
            else town.GarrisonJson = GameService.SerializeArmy(army);
            await db.SaveChangesAsync();
            return Results.Ok(new { creature = upKey, cost });
        });

        // Blacksmith: sell the faction's war machine to the visiting hero.
        app.MapPost("/api/towns/{townId:guid}/blacksmith", async (Guid townId, AppDb db, HttpContext ctx) =>
        {
            var (town, player, err) = await OwnTownAsync(db, ctx, townId);
            if (err != null) return err;
            if (!GameService.ParseStringArray(town!.BuildingsJson).Contains("blacksmith"))
                return Results.BadRequest(new { error = "No Blacksmith built." });
            var hero = await db.Heroes.FirstOrDefaultAsync(h => h.PlayerId == player!.Id && h.X == town.X && h.Y == town.Y);
            if (hero == null) return Results.BadRequest(new { error = "A hero must visit the town to buy a war machine." });
            var (machine, price) = TownBuildings.Blacksmith[town.Faction];
            if (TownRules.HasGear(hero, machine))
                return Results.BadRequest(new { error = $"{hero.Name} already has a {TownBuildings.MachineNames[machine]}." });
            if (player!.Gold < price) return Results.BadRequest(new { error = "Not enough gold." });
            player.Gold -= price;
            TownRules.AddGear(hero, machine);
            await db.SaveChangesAsync();
            return Results.Ok(new { machine });
        });

        // Mage Guild: a visiting hero without a spellbook may buy one for 500 gold.
        app.MapPost("/api/towns/{townId:guid}/spellbook", async (Guid townId, AppDb db, HttpContext ctx) =>
        {
            var (town, player, err) = await OwnTownAsync(db, ctx, townId);
            if (err != null) return err;
            if (!GameService.ParseStringArray(town!.BuildingsJson).Contains("mageGuild1"))
                return Results.BadRequest(new { error = "No Mage Guild built." });
            var hero = await db.Heroes.FirstOrDefaultAsync(h => h.PlayerId == player!.Id && h.X == town.X && h.Y == town.Y);
            if (hero == null) return Results.BadRequest(new { error = "No hero is in town." });
            if (TownRules.HasGear(hero, TownRules.Spellbook)) return Results.BadRequest(new { error = "The hero already has a spellbook." });
            if (player!.Gold < 500) return Results.BadRequest(new { error = "Not enough gold (500)." });
            player.Gold -= 500;
            TownRules.AddGear(hero, TownRules.Spellbook);
            var learned = TownRules.LearnGuildSpells(hero, town);
            await db.SaveChangesAsync();
            return Results.Ok(new { learned });
        });

        // Necropolis Skeleton Transformer: turns a living stack into as many Skeletons.
        app.MapPost("/api/towns/{townId:guid}/transform", async (Guid townId, StackReq req, AppDb db, HttpContext ctx) =>
        {
            var (town, player, err) = await OwnTownAsync(db, ctx, townId);
            if (err != null) return err;
            if (town!.Faction != "necropolis" || !GameService.ParseStringArray(town.BuildingsJson).Contains("special3"))
                return Results.BadRequest(new { error = "No Skeleton Transformer built." });
            var hero = await db.Heroes.FirstOrDefaultAsync(h => h.PlayerId == player!.Id && h.X == town.X && h.Y == town.Y);
            if (hero == null) return Results.BadRequest(new { error = "A hero must visit the town." });
            if (req.Slot is < 0 or > 6) return Results.BadRequest(new { error = "Unknown slot." });
            var army = GameService.ParseArmy(hero.ArmyJson);
            var stack = army[req.Slot];
            if (stack.Creature == null || !Creatures.All.TryGetValue(stack.Creature, out var c))
                return Results.BadRequest(new { error = "That slot is empty." });
            if (c.Faction == "necropolis" || c.Abilities.Contains("undead")) return Results.BadRequest(new { error = "The undead cannot be transformed." });
            army[req.Slot] = stack with { Creature = "skeleton" };
            hero.ArmyJson = GameService.SerializeArmy(army);
            await RefreshArmyMovementAsync(db, hero);
            await db.SaveChangesAsync();
            return Results.Ok(new { creature = "skeleton", count = stack.Count });
        });

        // Legacy combat spells remain learnable catalogue entries, never paid no-op casts.
        app.MapPost("/api/heroes/{heroId:guid}/cast", async (Guid heroId, CastReq req, AppDb db, GameService games, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });

            var hero = await db.Heroes.FirstOrDefaultAsync(h => h.Id == heroId);
            if (hero == null) return Results.NotFound();
            var player = await db.Players.FirstAsync(p => p.Id == hero.PlayerId);
            if (player.Faction != sp.Faction) return Results.BadRequest(new { error = "Not your hero." });

            if (hero.PendingEncounterJson != null || hero.PendingTreasureJson != null || hero.PendingLevelJson != null)
                return Results.BadRequest(new { error = "Resolve the pending encounter or treasure choice first." });
            var key = Spells.CanonicalKey(req.Spell ?? "");
            if (!Spells.All.TryGetValue(key, out var spell)) return Results.BadRequest(new { error = "Unknown spell." });
            if (!spell.Adventure) return Results.BadRequest(new { error = "That spell is not available in this peaceful game." });
            var known = GameService.ParseStringArray(hero.SpellsJson).Select(Spells.CanonicalKey);
            if (!known.Contains(key)) return Results.BadRequest(new { error = "Your hero does not know that spell." });
            if (!TownRules.HasGear(hero, TownRules.Spellbook)) return Results.BadRequest(new { error = "Your hero has no spellbook." });
            var skills = GameService.ParseSkills(hero.SkillsJson);
            var manaCost = Spells.ManaCost(spell, skills);
            if (hero.Mana < manaCost) return Results.BadRequest(new { error = "Not enough spell points." });

            var game = await db.Games.FirstAsync(g => g.Id == hero.GameId);
            if (game.State != GameState.InProgress) return Results.BadRequest(new { error = "The game is not in progress." });

            string message;
            object? view = null;
            var effects = GameService.Effects(hero);
            switch (key)
            {
                case "fly":
                    var air = Math.Clamp(skills.GetValueOrDefault("air"), 0, 3);
                    GameService.SetEffects(hero, effects with
                    {
                        FlightTurn = game.CurrentTurn,
                        FlightPenalty = air >= 3 ? 0 : air >= 2 ? 20 : 40
                    });
                    message = "You can fly until tomorrow. Choose a legal landing tile.";
                    break;
                case "haste":
                    if (effects.HasteTurn == game.CurrentTurn)
                        return Results.BadRequest(new { error = "Haste is already active today." });
                    var haste = skills.GetValueOrDefault("air") >= 2 ? 500 : 300;
                    GameService.SetEffects(hero, effects with { HasteTurn = game.CurrentTurn, HasteBonus = haste });
                    hero.MaxMovement += haste;
                    hero.Movement += haste;
                    message = $"Haste grants {haste} movement for today.";
                    break;
                case "cure":
                    GameService.SetEffects(hero, effects with { MovementPenalty = 0 });
                    hero.MaxMovement = GameService.MaxMovement(hero) + (TownRules.StabledThisWeek(hero, game) ? 400 : 0)
                        + (effects.HasteTurn == game.CurrentTurn ? effects.HasteBonus : 0);
                    message = effects.MovementPenalty > 0 ? "Cure removes the negative movement effect." : "Cure finds no negative adventure effects.";
                    break;
                case "viewAir":
                case "viewEarth":
                {
                    var map = await games.LoadMapAsync(game);
                    var mastery = Math.Clamp(skills.GetValueOrDefault(spell.School), 0, 3);
                    var locations = new List<object>();
                    for (var y = 0; y < map.Height; y++)
                    for (var x = 0; x < map.Width; x++)
                    {
                        var tile = map.At(x, y);
                        var show = key == "viewAir" ? tile.ObjectKind == "artifact"
                            : tile.ObjectKind == "resource" || mastery >= 2 && tile.ObjectKind == "mine";
                        if (!show) continue;
                        var name = tile.ObjectKind == "artifact" ? Artifacts.Of(tile.ObjectData ?? "").Name
                            : (tile.ObjectData ?? "").Split(':')[0] + (tile.ObjectKind == "mine" ? " mine" : "");
                        locations.Add(new { kind = tile.ObjectKind, x, y, name });
                    }
                    if (key == "viewAir" && mastery >= 2)
                        foreach (var seen in await db.Heroes.Where(h => h.GameId == game.Id).ToListAsync())
                            locations.Add(new { kind = "hero", x = seen.X, y = seen.Y, name = seen.Name });
                    if (key == "viewAir" && mastery >= 3)
                        foreach (var seen in await db.Towns.Where(t => t.GameId == game.Id).ToListAsync())
                            locations.Add(new { kind = "town", x = seen.X, y = seen.Y, name = seen.Name });
                    var fog = GameService.ParseFog(player.FogJson, map.Width * map.Height);
                    view = new
                    {
                        width = map.Width, height = map.Height,
                        terrain = map.Tiles.Select((t, i) => key == "viewEarth" && mastery >= 3 || fog[i] > 0 ? t.Terrain : null),
                        locations
                    };
                    message = $"{spell.Name} reveals {locations.Count} locations on the world view.";
                    break;
                }
                case "townPortal":
                {
                    var mastery = Spells.Mastery(spell, skills);
                    var towns = await db.Towns.Where(t => t.PlayerId == player.Id).ToListAsync();
                    var town = mastery >= 2 ? towns.FirstOrDefault(t => t.Id == req.TownId)
                        : towns.OrderBy(t => (long)(t.X - hero.X) * (t.X - hero.X) + (long)(t.Y - hero.Y) * (t.Y - hero.Y)).ThenBy(t => t.Id).FirstOrDefault();
                    if (town == null) return Results.BadRequest(new { error = mastery >= 2 ? "Choose an unoccupied town." : "You have no town." });
                    if (await db.Heroes.AnyAsync(h => h.GameId == game.Id && h.X == town.X && h.Y == town.Y))
                        return Results.BadRequest(new { error = "That town already has a visiting hero." });
                    var cost = mastery >= 3 ? 200 : 300;
                    if (hero.Movement < cost) return Results.BadRequest(new { error = "Not enough movement points." });
                    hero.X = town.X; hero.Y = town.Y; hero.Movement -= cost;
                    GameService.VisitOwnTown(game, hero, town);
                    message = $"You return to {town.Name}.";
                    break;
                }
                case "dimensionDoor":
                {
                    if (req.X is not { } tx || req.Y is not { } ty)
                        return Results.BadRequest(new { error = "Choose a destination." });
                    var mastery = Spells.Mastery(spell, skills);
                    var casts = effects.DoorTurn == game.CurrentTurn ? effects.DoorCasts : 0;
                    if (casts >= mastery + 1) return Results.BadRequest(new { error = "Dimension Door has reached its daily limit." });
                    if (hero.Movement <= 0) return Results.BadRequest(new { error = "Not enough movement points." });
                    if (Math.Abs((long)tx - hero.X) > 9 || Math.Abs((long)ty - hero.Y) > 8)
                        return Results.BadRequest(new { error = "That destination is out of range." });
                    var map = await games.LoadMapAsync(game);
                    if (!map.InBounds(tx, ty) || (!map.At(tx, ty).Passable && map.At(tx, ty).Terrain != "water")
                        || map.At(tx, ty).Terrain == "rock"
                        || map.At(tx, ty).ObjectKind != null || (tx == hero.X && ty == hero.Y))
                        return Results.BadRequest(new { error = "You cannot appear there." });
                    if (await db.Heroes.AnyAsync(h => h.GameId == game.Id && h.Id != hero.Id && h.X == tx && h.Y == ty))
                        return Results.BadRequest(new { error = "Another hero stands there." });
                    var failed = map.At(tx, ty).Terrain == "water";
                    if (!failed) { hero.X = tx; hero.Y = ty; }
                    hero.Movement = Math.Max(0, hero.Movement - (mastery >= 3 ? 200 : 300));
                    GameService.SetEffects(hero, effects with { DoorTurn = game.CurrentTurn, DoorCasts = casts + 1 });
                    message = failed ? "Dimension Door failed!" : "You step through a door in space.";
                    break;
                }
                case "visions":
                    GameService.SetEffects(hero, effects with { VisionsTurn = game.CurrentTurn, VisionsMastery = Spells.Mastery(spell, skills) });
                    message = "Visions reveals nearby creatures until tomorrow.";
                    break;
                default:
                    return Results.BadRequest(new { error = "That spell is not available in this peaceful game." });
            }

            hero.Mana -= manaCost;
            if (key is "townPortal" or "dimensionDoor")
            {
                var map = await games.LoadMapAsync(game);
                var fog = GameService.ParseFog(player.FogJson, map.Width * map.Height);
                GameService.RecomputeFog(fog, map.Width, map.Height,
                    await db.Heroes.Where(h => h.PlayerId == player.Id).ToListAsync(),
                    await db.Towns.Where(t => t.PlayerId == player.Id).ToListAsync());
                player.FogJson = GameService.SerializeFog(fog);
            }
            await db.SaveChangesAsync();
            return Results.Ok(new { message, mana = hero.Mana, x = hero.X, y = hero.Y, view });
        });

        // End the current turn: advance the day and restore every hero's movement.
        app.MapPost("/api/games/{id:guid}/endturn", async (Guid id, AppDb db, GameService games, WsHub ws, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });

            var game = await db.Games.FirstOrDefaultAsync(g => g.Id == id);
            if (game == null) return Results.NotFound();
            if (game.State != GameState.InProgress) return Results.BadRequest(new { error = "The game is not in progress." });

            var player = await db.Players.FirstOrDefaultAsync(p => p.GameId == id && p.Faction == sp.Faction);
            if (player == null) return Results.NotFound();

            // force this game's turn to expire, then let the ticker do the day rollover
            game.TurnStartedAt = DateTime.UtcNow.AddSeconds(-(game.TurnSeconds + 1));
            await db.SaveChangesAsync();
            await games.TickAsync(false);

            var fresh = await db.Games.FirstAsync(g => g.Id == id);
            await ws.BroadcastAsync(id, "turnEnded", new { day = fresh.Day, week = fresh.Week, month = fresh.Month });
            return Results.Ok(new { day = fresh.Day, week = fresh.Week, month = fresh.Month, turn = fresh.CurrentTurn });
        });

        // ------------------------------------------------------------ obelisk
        // The obelisk board: the player's HoMM3 puzzle map (a terrain-only view centred on the Grail,
        // uncovered piece by piece as obelisks are read) and the notes players left on the stone.
        app.MapGet("/api/games/{id:guid}/obelisk", async (Guid id, AppDb db, GameService games, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });

            var player = await db.Players.FirstOrDefaultAsync(p => p.GameId == id && p.Faction == sp.Faction);
            if (player == null) return Results.NotFound();

            var game = await db.Games.FirstAsync(g => g.Id == id);
            var map = await games.LoadMapAsync(game);
            var visited = GameService.ParseVisitedObelisks(player.ObelisksVisitedJson).Count;
            var total = GameService.ObeliskTotal(map);
            var players = await db.Players.Where(p => p.GameId == id).ToListAsync();
            var msgs = await db.ObeliskMessages.Where(m => m.GameId == id).OrderBy(m => m.CreatedAt).ToListAsync();

            const int w = 21, h = 19;
            var gx = game.GrailX >= 0 ? game.GrailX : map.Width / 2;
            var gy = game.GrailY >= 0 ? game.GrailY : map.Height / 2;
            int ox = gx - w / 2, oy = gy - h / 2;
            var tiles = new List<object>(w * h);
            for (var y = oy; y < oy + h; y++)
            for (var x = ox; x < ox + w; x++)
            {
                if (!map.InBounds(x, y))
                {
                    tiles.Add(new { terrain = (string?)null, variant = 0, objectKind = (string?)null, objectData = (string?)null, owner = -1, passable = false, fog = 0 });
                    continue;
                }
                var t = map.At(x, y);
                var natural = t.ObjectKind is "tree" or "rock" or "mountain";
                tiles.Add(new
                {
                    terrain = t.Terrain,
                    variant = t.Variant,
                    objectKind = natural ? t.ObjectKind : null,
                    objectData = natural ? t.ObjectData : null,
                    sprite = natural ? t.Sprite : null,
                    owner = -1,
                    passable = natural ? t.Passable : Terrain.Passable(t.Terrain),
                    fog = 2
                });
            }

            return Results.Ok(new
            {
                visited,
                total,
                complete = total > 0 && visited >= total,
                faction = player.Faction,
                grailFound = game.GrailFound,
                puzzle = new { width = w, height = h, grailX = gx - ox, grailY = gy - oy, tiles },
                messages = msgs.Select(m =>
                {
                    var author = players.FirstOrDefault(p => p.Id == m.AuthorUserId);
                    var who = author == null ? null : Seeded.Players.FirstOrDefault(s => s.Faction == author.Faction);
                    return new
                    {
                        id = m.Id,
                        author = who == null ? "A traveller" : $"{who.Name} of {author!.CityName}",
                        mine = author?.Id == player.Id,
                        body = m.Body,
                        createdAt = m.CreatedAt
                    };
                })
            });
        });
        app.MapPost("/api/games/{id:guid}/obelisk/message", async (Guid id, BodyReq req, AppDb db, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });
            if (string.IsNullOrWhiteSpace(req.Body)) return Results.BadRequest(new { error = "Message cannot be empty." });

            var player = await db.Players.FirstOrDefaultAsync(p => p.GameId == id && p.Faction == sp.Faction);
            if (player == null) return Results.NotFound();

            var m = new ObeliskMessage { GameId = id, AuthorUserId = player.Id, Body = req.Body.Trim() };
            db.ObeliskMessages.Add(m);
            await db.SaveChangesAsync();
            return Results.Ok(new { messageId = m.Id });
        });

        // ------------------------------------------------------------ catalogue
        app.MapGet("/api/buildings", (string? faction) => Results.Ok(TownBuildings.For(faction ?? "castle").Values.Select(b => new
        {
            key = b.Key, name = b.Name, category = b.Category,
            cost = new { gold = b.Gold, wood = b.Wood, ore = b.Ore, mercury = b.Mercury, sulfur = b.Sulfur, crystal = b.Crystal, gems = b.Gems },
            requires = b.Requires, description = b.Description, tier = b.Tier
        })));

        app.MapGet("/api/creatures", () => Results.Ok(Creatures.All.Values.Select(c => new
        {
            key = c.Key, name = c.Name, faction = c.Faction, tier = c.Tier,
            attack = c.Attack, defense = c.Defense, damageMin = c.DamageMin, damageMax = c.DamageMax,
            health = c.Health, speed = c.Speed, growth = c.Growth, gold = c.Gold, shots = c.Shots,
            cost = TownRules.CostOf(c),
            aiValue = c.AiValue, abilities = c.Abilities, upgradeOf = c.UpgradeOf
        })));

        app.MapGet("/api/heroes", () => Results.Ok(Heroes.All.Values.Select(h => new
        {
            key = h.Key, name = h.Name, faction = h.Faction, might = h.Might,
            attack = h.Attack, defense = h.Defense, power = h.Power, knowledge = h.Knowledge,
            skills = h.Skills
        })));

        app.MapGet("/api/spells", () => Results.Ok(Spells.All.Values.Select(s => new
        {
            key = s.Key, name = s.Name, level = s.Level, school = s.School,
            cost = s.Cost, description = s.Description, id = s.Id, manaCosts = s.ManaCosts, adventure = s.Adventure
        })));

// ------------------------------------------------------------ encounters
        app.MapPost("/api/heroes/{heroId:guid}/encounter", async (Guid heroId, EncounterReq req, SocialService social, AppDb db, WsHub ws, HttpContext ctx) =>
        {
            var playerId = ctx.Request.Headers["X-Player-Id"].ToString();
            var (ok, error, message) = await social.ResolveEncounterAsync(heroId, playerId, req.Action ?? "");
            if (!ok) return Results.BadRequest(new { error });
            var hero = await db.Heroes.FirstOrDefaultAsync(h => h.Id == heroId);
            if (hero != null)
                await ws.BroadcastAsync(hero.GameId, "encounter", new { heroId, action = req.Action });
            return Results.Ok(new { message });
        });

        // ------------------------------------------------------------ hero messages
        app.MapGet("/api/games/{id:guid}/messages", async (Guid id, SocialService social, HttpContext ctx) =>
        {
            var playerId = ctx.Request.Headers["X-Player-Id"].ToString();
            return Results.Ok(await social.GetHeroMessagesAsync(id, playerId));
        });

        app.MapPost("/api/games/{id:guid}/messages", async (Guid id, MessageReq req, SocialService social, WsHub ws, HttpContext ctx) =>
        {
            var playerId = ctx.Request.Headers["X-Player-Id"].ToString();
            if (!Guid.TryParse(req.ToPlayerId, out var to)) return Results.BadRequest(new { error = "Recipient required." });
            var (ok, error) = await social.SendHeroMessageAsync(id, playerId, to, req.Body ?? "");
            if (!ok) return Results.BadRequest(new { error });
            await ws.BroadcastAsync(id, "message", new { toPlayerId = to });
            return Results.NoContent();
        });

        // ------------------------------------------------------------ trades
        app.MapGet("/api/games/{id:guid}/trades", async (Guid id, SocialService social, HttpContext ctx) =>
        {
            var playerId = ctx.Request.Headers["X-Player-Id"].ToString();
            return Results.Ok(await social.GetTradesAsync(id, playerId));
        });

        app.MapPost("/api/games/{id:guid}/trades", async (Guid id, TradeOfferReq req, SocialService social, WsHub ws, HttpContext ctx) =>
        {
            var playerId = ctx.Request.Headers["X-Player-Id"].ToString();
            if (!Guid.TryParse(req.ToPlayerId, out var to)) return Results.BadRequest(new { error = "Recipient required." });
            var (ok, error) = await social.OfferTradeAsync(id, playerId, to,
                req.Give ?? "", req.GiveAmount, req.Want ?? "", req.WantAmount);
            if (!ok) return Results.BadRequest(new { error });
            await ws.BroadcastAsync(id, "trade", new { toPlayerId = to });
            return Results.NoContent();
        });

        app.MapPost("/api/trades/{offerId:guid}/respond", async (Guid offerId, RespondReq req, SocialService social, AppDb db, WsHub ws, HttpContext ctx) =>
        {
            var playerId = ctx.Request.Headers["X-Player-Id"].ToString();
            var (ok, error) = await social.RespondTradeAsync(offerId, playerId, req.Accept);
            if (!ok) return Results.BadRequest(new { error });
            var offer = await db.TradeOffers.FirstOrDefaultAsync(o => o.Id == offerId);
            if (offer != null) await ws.BroadcastAsync(offer.GameId, "trade", new { offerId });
            return Results.NoContent();
        });

        // ------------------------------------------------------------ shared projects
        app.MapGet("/api/games/{id:guid}/projects", async (Guid id, SocialService social) =>
            Results.Ok(await social.GetProjectsAsync(id)));

        app.MapPost("/api/projects/{projectId:guid}/contribute", async (Guid projectId, ContributeReq req, SocialService social, AppDb db, WsHub ws, HttpContext ctx) =>
        {
            var playerId = ctx.Request.Headers["X-Player-Id"].ToString();
            var (ok, error) = await social.ContributeAsync(projectId, playerId, req.Gold, req.Wood, req.Ore);
            if (!ok) return Results.BadRequest(new { error });
            var project = await db.SharedProjects.FirstAsync(p => p.Id == projectId);
            await ws.BroadcastAsync(project.GameId, "project", new { projectId, completed = project.Completed });
            return Results.NoContent();
        });

        // ------------------------------------------------------------ annotations
        app.MapGet("/api/games/{id:guid}/annotations", async (Guid id, SocialService social) =>
            Results.Ok(await social.GetAnnotationsAsync(id)));

        app.MapPost("/api/games/{id:guid}/annotations", async (Guid id, AnnotationReq req, SocialService social, WsHub ws, HttpContext ctx) =>
        {
            var playerId = ctx.Request.Headers["X-Player-Id"].ToString();
            var (ok, error) = await social.AddAnnotationAsync(id, playerId, req.X, req.Y, req.Text ?? "");
            if (!ok) return Results.BadRequest(new { error });
            await ws.BroadcastAsync(id, "annotation", new { x = req.X, y = req.Y });
            return Results.NoContent();
        });

        // ------------------------------------------------------------ tower content
        app.MapGet("/api/towns/{townId:guid}/posts", async (Guid townId, AppDb db) =>
        {
            var posts = await db.Posts.Where(p => p.TownId == townId).OrderBy(p => p.CreatedAt).ToListAsync();
            return Results.Ok(posts.Select(p => new
            {
                id = p.Id, title = p.Title, body = p.Body, imageUrl = p.ImageUrl,
                createdAt = p.CreatedAt, updatedAt = p.UpdatedAt
            }));
        });

        app.MapPost("/api/towns/{townId:guid}/posts", async (Guid townId, PostReq req, SocialService social, HttpContext ctx) =>
        {
            var playerId = ctx.Request.Headers["X-Player-Id"].ToString();
            var (id, error) = await social.CreatePostAsync(townId, playerId, req.Title ?? "", req.Body ?? "", req.ImageUrl);
            if (error != null) return Results.BadRequest(new { error });
            return Results.Ok(new { postId = id });
        });

        app.MapGet("/api/posts/{postId:guid}/comments", async (Guid postId, AppDb db) =>
        {
            var comments = await db.Comments.Where(c => c.PostId == postId).OrderBy(c => c.CreatedAt).ToListAsync();
            var players = await db.Players.ToListAsync();
            string NameOf(Guid id) =>
                Seeded.Players.FirstOrDefault(s => s.Faction == players.FirstOrDefault(p => p.Id == id)?.Faction)?.Name ?? "?";
            return Results.Ok(comments.Select(c => new
            {
                id = c.Id, author = NameOf(c.AuthorUserId), body = c.Body, createdAt = c.CreatedAt
            }));
        });

        app.MapPost("/api/posts/{postId:guid}/comments", async (Guid postId, BodyReq req, SocialService social, HttpContext ctx) =>
        {
            var playerId = ctx.Request.Headers["X-Player-Id"].ToString();
            var (id, error) = await social.AddCommentAsync(postId, playerId, req.Body ?? "");
            if (error != null) return Results.BadRequest(new { error });
            return Results.Ok(new { commentId = id });
        });

        app.MapGet("/api/towns/{townId:guid}/wall", async (Guid townId, AppDb db) =>
        {
            var msgs = await db.WallMessages.Where(m => m.TownId == townId).OrderBy(m => m.CreatedAt).ToListAsync();
            var players = await db.Players.ToListAsync();
            string NameOf(Guid id) =>
                Seeded.Players.FirstOrDefault(s => s.Faction == players.FirstOrDefault(p => p.Id == id)?.Faction)?.Name ?? "?";
            return Results.Ok(msgs.Select(m => new
            {
                id = m.Id, author = NameOf(m.AuthorUserId), body = m.Body,
                parentId = m.ParentId, createdAt = m.CreatedAt
            }));
        });

        app.MapPost("/api/towns/{townId:guid}/wall", async (Guid townId, WallReq req, SocialService social, AppDb db, WsHub ws, HttpContext ctx) =>
        {
            var playerId = ctx.Request.Headers["X-Player-Id"].ToString();
            if (await TownHiddenAsync(db, townId, playerId)) return Results.BadRequest(new { error = "You have not discovered that town yet." });
            var (id, error) = await social.AddWallMessageAsync(townId, playerId, req.Body ?? "", req.ParentId);
            if (error != null) return Results.BadRequest(new { error });
            await NotifyTownAsync(db, ws, townId, "wall");
            return Results.Ok(new { messageId = id });
        });

        app.MapGet("/api/towns/{townId:guid}/tower", async (Guid townId, AppDb db) =>
        {
            var msgs = await db.TowerMessages.Where(m => m.TownId == townId).OrderBy(m => m.CreatedAt).ToListAsync();
            var players = await db.Players.ToListAsync();
            string NameOf(Guid id) =>
                Seeded.Players.FirstOrDefault(s => s.Faction == players.FirstOrDefault(p => p.Id == id)?.Faction)?.Name ?? "?";
            return Results.Ok(msgs.Select(m => new
            {
                id = m.Id, author = NameOf(m.AuthorUserId), body = m.Body, createdAt = m.CreatedAt
            }));
        });

        app.MapPost("/api/towns/{townId:guid}/tower", async (Guid townId, BodyReq req, SocialService social, AppDb db, WsHub ws, HttpContext ctx) =>
        {
            var playerId = ctx.Request.Headers["X-Player-Id"].ToString();
            if (await TownHiddenAsync(db, townId, playerId)) return Results.BadRequest(new { error = "You have not discovered that town yet." });
            var (id, error) = await social.AddTowerMessageAsync(townId, playerId, req.Body ?? "");
            if (error != null) return Results.BadRequest(new { error });
            await NotifyTownAsync(db, ws, townId, "tower");
            return Results.Ok(new { messageId = id });
        });

        // ------------------------------------------------------------ test-only
        // reveals the whole map for one player so API tests can reach distant towns
        app.MapPost("/api/games/{id:guid}/_test/explore", async (Guid id, AppDb db, IHostEnvironment env, HttpContext ctx) =>
        {
            if (!env.IsDevelopment()) return Results.NotFound();
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            var game = await db.Games.FirstOrDefaultAsync(g => g.Id == id);
            if (sp == null || game == null) return Results.NotFound();
            var player = await db.Players.FirstOrDefaultAsync(p => p.GameId == id && p.Faction == sp.Faction);
            if (player == null) return Results.NotFound();
            var fog = GameService.ParseFog(player.FogJson, game.MapWidth * game.MapHeight);
            for (var i = 0; i < fog.Length; i++) if (fog[i] == 0) fog[i] = 1;
            player.FogJson = GameService.SerializeFog(fog);
            await db.SaveChangesAsync();
            return Results.NoContent();
        });

        // gives a hero map artifacts by key (artifact{n}) so API tests can check the paper-doll rules
        app.MapPost("/api/heroes/{id:guid}/_test/artifacts", async (Guid id, string[] keys, AppDb db, IHostEnvironment env) =>
        {
            if (!env.IsDevelopment()) return Results.NotFound();
            var hero = await db.Heroes.FirstOrDefaultAsync(h => h.Id == id);
            if (hero == null) return Results.NotFound();
            var gear = GameService.ParseStringArray(hero.ArtifactsJson).Where(TownRules.IsGear);
            hero.ArtifactsJson = System.Text.Json.JsonSerializer.Serialize(gear.Concat(keys).ToList());
            hero.ArtifactLayoutJson = "{}";
            await db.SaveChangesAsync();
            return Results.Ok(HeroDto(hero));
        });

        // Deterministic adventure states for browser regressions.
        app.MapPost("/api/heroes/{id:guid}/_test/adventure", async (Guid id, AdventureFixtureReq req, AppDb db,
            GameService games, IHostEnvironment env, HttpContext ctx) =>
        {
            if (!env.IsDevelopment()) return Results.NotFound();
            var hero = await db.Heroes.FirstOrDefaultAsync(h => h.Id == id);
            if (hero == null) return Results.NotFound();
            var player = await db.Players.FirstAsync(p => p.Id == hero.PlayerId);
            if (Seeded.Find(ctx.Request.Headers["X-Player-Id"])?.Faction != player.Faction) return Results.NotFound();
            var game = await db.Games.FirstAsync(g => g.Id == hero.GameId);
            var map = await games.LoadMapAsync(game);
            if (req.Army != null) hero.ArmyJson = GameService.SerializeArmy(req.Army);
            if (req.Skills != null) hero.SkillsJson = System.Text.Json.JsonSerializer.Serialize(req.Skills);
            if (req.Gold is { } gold) player.Gold = Math.Max(0, gold);
            if (req.Spells != null)
            {
                TownRules.AddGear(hero, TownRules.Spellbook);
                hero.SpellsJson = System.Text.Json.JsonSerializer.Serialize(req.Spells);
                hero.Knowledge = 10;
                hero.Mana = hero.MaxMana = 100;
            }
            if (req.MovementPenalty > 0)
                GameService.SetEffects(hero, new AdventureEffects(MovementPenalty: req.MovementPenalty));
            hero.Movement = hero.MaxMovement = GameService.MaxMovement(hero);
            if (req.MovementPenalty > 0) hero.Movement = 500;
            if (req.Treasure || req.FlightLane)
            {
                hero.Y++;
                var start = map.At(hero.X, hero.Y);
                start.Terrain = "grass"; start.ObjectKind = null; start.ObjectData = null; start.Passable = true;
                var tile = map.At(hero.X, hero.Y + 1);
                tile.Terrain = "grass";
                tile.ObjectKind = req.Treasure ? "treasure" : "tree";
                tile.ObjectData = req.Treasure ? "1500:1000" : null;
                tile.Passable = req.Treasure;
                var end = map.At(hero.X, hero.Y + 2);
                end.Terrain = "grass"; end.ObjectKind = null; end.ObjectData = null; end.Passable = true;
            }
            if (req.SurveyTargets)
            {
                map.At(0, 0).ObjectKind = "artifact"; map.At(0, 0).ObjectData = "artifact1";
                map.At(1, 0).ObjectKind = "resource"; map.At(1, 0).ObjectData = "wood:5";
            }
            if (req.ObjectKind != null)
            {
                var town = await db.Towns.FirstAsync(t => t.PlayerId == player.Id);
                hero.X = town.X; hero.Y = town.Y + 5;
                for (var y = town.Y + 1; y <= town.Y + 6; y++)
                for (var x = town.X - 4; x <= town.X + 4; x++)
                {
                    var tile = map.At(x, y);
                    tile.Terrain = "grass"; tile.ObjectKind = null; tile.ObjectData = null; tile.Sprite = null; tile.Passable = true;
                }
                if (req.ObjectKind == "obelisk")
                    foreach (var old in map.Tiles.Where(t => t.ObjectKind == "obelisk"))
                    { old.ObjectKind = null; old.ObjectData = null; old.Passable = true; }
                if (req.ObjectKind == "town")
                {
                    // an unowned town whose entrance is the tile above the hero
                    var ax = hero.X; var ay = hero.Y - 1;
                    var neutral = new Town
                    {
                        GameId = game.Id, PlayerId = Guid.Empty, Name = "Castellatus", Faction = req.ObjectData ?? "castle",
                        X = ax, Y = ay, BuildingsJson = "[\"villageHall\",\"fort\",\"dwelling1\"]", AvailableJson = "{\"1\":14}",
                        GarrisonJson = GameService.SerializeArmy(Enumerable.Range(0, 7).Select(_ => new GameService.ArmySlot(null, 0)).ToArray())
                    };
                    db.Towns.Add(neutral);
                    var anchor = map.At(ax, ay);
                    anchor.ObjectKind = "town"; anchor.ObjectData = neutral.Id.ToString(); anchor.Owner = -1; anchor.Passable = false;
                    Footprint.Stamp(map, ax, ay, "town", null);
                }
                var target = map.At(hero.X, hero.Y - 1);
                if (req.ObjectKind != "town")
                {
                target.ObjectKind = req.ObjectKind;
                target.ObjectData = req.ObjectData;
                target.Owner = 0;
                target.Passable = req.ObjectKind is "resource" or "artifact" or "treasure" or "grail";
                }
                if (req.ObjectKind == "grail")
                {
                    game.GrailX = hero.X; game.GrailY = hero.Y - 1;
                    player.ObeliskVisits = 10;
                }
                if (req.Wisdom > 0)
                {
                    var skills = GameService.ParseSkills(hero.SkillsJson);
                    skills["wisdom"] = req.Wisdom;
                    hero.SkillsJson = System.Text.Json.JsonSerializer.Serialize(skills);
                }
            }
            if (req.Movement is { } movement) hero.Movement = Math.Clamp(movement, 0, hero.MaxMovement);
            if (req.Meeting != null)
            {
                var town = await db.Towns.FirstAsync(t => t.PlayerId == player.Id);
                hero.X = town.X; hero.Y = town.Y + 3;
                for (var y = hero.Y - 1; y <= hero.Y + 2; y++)
                for (var x = hero.X - 1; x <= hero.X + 2; x++)
                {
                    var tile = map.At(x, y);
                    tile.Terrain = "grass"; tile.ObjectKind = null; tile.ObjectData = null; tile.Passable = true;
                }
                var owner = req.Meeting == "own" ? player : await db.Players.FirstAsync(p => p.GameId == game.Id && p.Faction == "rampart");
                if (owner != player) owner.IsAi = false;
                var friend = GameService.CreateHero(game.Id, owner.Id, Seeded.Players.First(p => p.Faction == owner.Faction), hero.X + 1, hero.Y);
                friend.Name = req.Meeting == "own" ? "Companion" : "Visitor";
                db.Heroes.Add(friend);
            }
            if (req.PortalTown)
            {
                var tx = hero.X + 5; var ty = hero.Y + 5;
                var town = new Town { GameId = game.Id, PlayerId = player.Id, Faction = player.Faction, Name = "Portal destination", X = tx, Y = ty };
                db.Towns.Add(town);
                var tile = map.At(tx, ty);
                tile.Terrain = "grass"; tile.ObjectKind = "town"; tile.ObjectData = town.Id.ToString(); tile.Passable = false;
            }
            if (req.Experience > 0) GameService.GainExperience(hero, req.Experience);
            if (req.Buildings != null)
            {
                var town = await db.Towns.FirstAsync(t => t.PlayerId == player.Id);
                town.BuildingsJson = JsonSerializer.Serialize(req.Buildings, Json);
                town.AvailableJson = JsonSerializer.Serialize(new Dictionary<string, int> { ["1"] = 20, ["2"] = 20, ["7"] = 3 }, Json);
                foreach (var key in req.Buildings.Where(k => k.StartsWith("mageGuild"))) TownRules.StockGuild(town, key);
                player.Wood = player.Ore = player.Mercury = player.Sulfur = player.Crystal = player.Gems = 50;
            }
            var fog = GameService.ParseFog(player.FogJson, map.Width * map.Height);
            GameService.RecomputeFog(fog, map.Width, map.Height,
                await db.Heroes.Where(h => h.PlayerId == player.Id).ToListAsync(),
                await db.Towns.Where(t => t.PlayerId == player.Id).ToListAsync());
            player.FogJson = GameService.SerializeFog(fog);
            await games.SaveMapAsync(game, map);
            return Results.Ok(HeroDto(hero));
        });

        // advances this game one day so API tests can walk long distances
        app.MapPost("/api/games/{id:guid}/_test/nextday", async (Guid id, AppDb db, GameService games, WsHub ws, IHostEnvironment env) =>
        {
            if (!env.IsDevelopment()) return Results.NotFound();
            // force the turn timer to expire so TickAsync advances this game
            var game = await db.Games.FirstOrDefaultAsync(g => g.Id == id);
            if (game == null) return Results.NotFound();
            game.TurnStartedAt = DateTime.UtcNow.AddSeconds(-(game.TurnSeconds + 1));
            await db.SaveChangesAsync();
            foreach (var advanced in await games.TickAsync(false))
                await ws.BroadcastAsync(advanced, "turnAdvanced", new { gameId = advanced });
            return Results.NoContent();
        });
    }

    private static async Task NotifyTownAsync(AppDb db, WsHub ws, Guid townId, string type)
    {
        var town = await db.Towns.FirstOrDefaultAsync(t => t.Id == townId);
        if (town != null) await ws.BroadcastAsync(town.GameId, type, new { townId });
    }

    /// <summary>Marketplaces the player owns across all their towns (HoMM3 trade rates improve with each).</summary>
    private static async Task<int> MarketCountAsync(AppDb db, Guid gameId, Guid playerId)
    {
        var towns = await db.Towns.Where(t => t.GameId == gameId && t.PlayerId == playerId).Select(t => t.BuildingsJson).ToListAsync();
        return towns.Count(b => GameService.ParseStringArray(b).Contains("marketplace"));
    }

    /// <summary>True when the caller is a player in the town's game who has not yet explored its tile.</summary>
    private static async Task<bool> TownHiddenAsync(AppDb db, Guid townId, string? playerId)
    {
        var sp = Seeded.Find(playerId);
        if (sp == null) return false;
        var town = await db.Towns.FirstOrDefaultAsync(t => t.Id == townId);
        if (town == null) return false;
        var viewer = await db.Players.FirstOrDefaultAsync(p => p.GameId == town.GameId && p.Faction == sp.Faction);
        if (viewer == null || viewer.Id == town.PlayerId) return false;
        var game = await db.Games.FirstAsync(g => g.Id == town.GameId);
        var fog = GameService.ParseFog(viewer.FogJson, game.MapWidth * game.MapHeight);
        var i = town.Y * game.MapWidth + town.X;
        return i < 0 || i >= fog.Length || fog[i] == 0;
    }

    /// <summary>Loads a town and checks the caller owns it.</summary>
    private static async Task<(Town? town, Player? player, IResult? error)> OwnTownAsync(AppDb db, HttpContext ctx, Guid townId)
    {
        var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
        if (sp == null) return (null, null, Results.BadRequest(new { error = "Unknown player." }));
        var town = await db.Towns.FirstOrDefaultAsync(t => t.Id == townId);
        if (town == null) return (null, null, Results.NotFound());
        var player = await db.Players.FirstOrDefaultAsync(p => p.Id == town.PlayerId);
        if (player == null || player.Faction != sp.Faction) return (null, null, Results.BadRequest(new { error = "Not your town." }));
        if (await InactiveGameAsync(db, town.GameId) is { } inactive) return (null, null, inactive);
        return (town, player, null);
    }

    private static async Task<IResult?> InactiveGameAsync(AppDb db, Guid gameId) =>
        await db.Games.AnyAsync(g => g.Id == gameId && g.State == GameState.InProgress)
            ? null : Results.BadRequest(new { error = "The game is not in progress." });

    private static async Task<object?> HeroInTownAsync(AppDb db, Guid playerId, int x, int y)
    {
        // the hero steps onto its own town's tile, so "in town" means standing on it
        var hero = await db.Heroes.FirstOrDefaultAsync(h =>
            h.PlayerId == playerId && h.X == x && h.Y == y);
        if (hero == null) return null;
        return new
        {
            id = hero.Id, name = hero.Name, heroKey = hero.HeroKey, army = GameService.ParseArmy(hero.ArmyJson),
            spellbook = TownRules.HasGear(hero, TownRules.Spellbook),
            warMachines = TownRules.WarMachines(hero),
            spells = GameService.ParseStringArray(hero.SpellsJson),
            wisdom = GameService.ParseSkills(hero.SkillsJson).TryGetValue("wisdom", out var w) ? w : 0
        };
    }

    private static object ArtifactDto(string key)
    {
        var d = Artifacts.Of(key);
        return new { key, id = d.Id, name = d.Name, slot = d.Slot, attack = d.Attack, defense = d.Defense, power = d.Power, knowledge = d.Knowledge };
    }

    private static object HeroDto(CityOfTowers.Domain.Hero h)
    {
        var arts = TownRules.Artifacts(h);
        var (worn, backpack) = Artifacts.Layout(h);
        var bonus = Artifacts.Bonus(h);
        var effects = GameService.Effects(h);
        // HoMM3 shows the totals including worn artifacts; attack/defense floor at 0, power/knowledge at 1
        return new
        {
            id = h.Id,
            name = h.Name,
            heroKey = h.HeroKey,
            heroClass = NativeRules.ClassOf(h).Name,
            faction = h.Faction,
            level = h.Level,
            experience = h.Experience,
            nextLevel = Experience.NextThreshold(h.Level),
            attack = Math.Max(0, h.Attack + bonus.Attack),
            defense = Math.Max(0, h.Defense + bonus.Defense),
            power = Math.Max(1, h.Power + bonus.Power),
            knowledge = Math.Max(1, h.Knowledge + bonus.Knowledge),
            baseStats = new { attack = h.Attack, defense = h.Defense, power = h.Power, knowledge = h.Knowledge },
            mana = h.Mana,
            maxMana = h.MaxMana,
            movement = h.Movement,
            maxMovement = h.MaxMovement,
            nativeTerrain = GameService.NativeTerrain(h),
            x = h.X,
            y = h.Y,
            army = GameService.ParseArmy(h.ArmyJson),
            strength = GameService.ArmyStrength(GameService.ParseArmy(h.ArmyJson)),
            skills = GameService.ParseSkills(h.SkillsJson).Where(kv => !kv.Key.StartsWith('_')).ToDictionary(kv => kv.Key, kv => kv.Value),
            spells = GameService.ParseStringArray(h.SpellsJson).Select(Spells.CanonicalKey).Distinct(),
            artifacts = arts,
            worn = worn.ToDictionary(kv => kv.Key, kv => ArtifactDto(kv.Value)),
            backpack = backpack.Select(ArtifactDto),
            spellbook = TownRules.HasGear(h, TownRules.Spellbook),
            warMachines = TownRules.WarMachines(h),
            pendingEncounter = h.PendingEncounterJson,
            encounterOffer = SocialService.GetEncounterOffer(h),
            pendingTreasure = GameService.Treasure(h),
            pendingLevel = HeroProgression.Pending(h),
            flying = effects.FlightTurn > 0,
            flightPenalty = effects.FlightPenalty,
            hasteBonus = effects.HasteBonus,
            movementPenalty = effects.MovementPenalty,
            visions = effects.VisionsTurn > 0,
            visionRadius = effects.VisionsTurn > 0 ? Math.Max(3, Math.Max(1, h.Power + bonus.Power) * Math.Max(1, effects.VisionsMastery)) : 0,
            doorCasts = effects.DoorCasts
        };
    }

    private static int GetRes(Player p, string r) => r switch
    {
        "wood" => p.Wood, "ore" => p.Ore, "mercury" => p.Mercury,
        "sulfur" => p.Sulfur, "crystal" => p.Crystal, "gems" => p.Gems,
        "gold" => p.Gold, _ => 0
    };

    private static void SetRes(Player p, string r, int v)
    {
        switch (r)
        {
            case "wood": p.Wood = v; break;
            case "ore": p.Ore = v; break;
            case "mercury": p.Mercury = v; break;
            case "sulfur": p.Sulfur = v; break;
            case "crystal": p.Crystal = v; break;
            case "gems": p.Gems = v; break;
            case "gold": p.Gold = v; break;
        }
    }
}

public record CreateGameReq(string? Name, int Capacity, int TurnSeconds, int TurnLimit, int MapSize, int AiPlayers = 0);
public record MoveReq(List<Point>? Path);
public record Point(int X, int Y);
public record BuildReq(string? BuildingKey);
public record RecruitReq(int Tier, int Count, string? Creature = null);
public record ArmyMoveReq(string? FromArmy, int FromSlot, string? ToArmy, int ToSlot, int Count);
public record StackReq(string? Army, int Slot);
public record TradeReq(string? From, string? To, int Amount);
public record HireReq(string? HeroKey);
public record GarrisonReq(string? Direction, int Slot, int Count);
public record CastReq(string? Spell, int? X, int? Y, Guid? TownId = null);
public record BodyReq(string? Body);
public record EncounterReq(string? Action);
public record TreasureReq(string? Choice);
public record AdventureFixtureReq(string[]? Spells, int MovementPenalty = 0, bool Treasure = false, bool FlightLane = false,
    bool SurveyTargets = false, string? ObjectKind = null, string? ObjectData = null, int Wisdom = 0, int? Movement = null,
    GameService.ArmySlot[]? Army = null, int? Gold = null, Dictionary<string, int>? Skills = null,
    string? Meeting = null, bool PortalTown = false, int Experience = 0, string[]? Buildings = null);
public record MessageReq(string? ToPlayerId, string? Body);
public record TradeOfferReq(string? ToPlayerId, string? Give, int GiveAmount, string? Want, int WantAmount);
public record RespondReq(bool Accept);
public record ContributeReq(int Gold, int Wood, int Ore);
public record AnnotationReq(int X, int Y, string? Text);
public record PostReq(string? Title, string? Body, string? ImageUrl);
public record WallReq(string? Body, Guid? ParentId);
