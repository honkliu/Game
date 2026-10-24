using System.Text.Json;
using Microsoft.EntityFrameworkCore;
using CityOfTowers.Data;
using CityOfTowers.Domain;
using CityOfTowers.Services;

namespace CityOfTowers.Api;

public static class Endpoints
{
    private static readonly JsonSerializerOptions Json = new() { PropertyNamingPolicy = JsonNamingPolicy.CamelCase };

    public static void MapApi(this WebApplication app)
    {
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
                req.MapSize <= 0 ? 72 : req.MapSize);
            return Results.Ok(new { gameId = game.Id });
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
                grailRevealed = game.GrailRevealed,
                grailX = game.GrailRevealed ? game.GrailX : (int?)null,
                grailY = game.GrailRevealed ? game.GrailY : (int?)null,
                players = players.Select(p => new
                {
                    playerId = p.Id,
                    name = Seeded.Players.FirstOrDefault(s => s.Faction == p.Faction)?.Name ?? "?",
                    faction = p.Faction,
                    cityName = p.CityName,
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
                    owner = t.Owner,
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
                    isMine = h.PlayerId == player.Id
                }),
                towns = towns.Where(t => t.PlayerId == player.Id || Explored(t.X, t.Y)).Select(t => new
                {
                    id = t.Id,
                    name = t.Name,
                    faction = t.Faction,
                    x = t.X,
                    y = t.Y,
                    isMine = t.PlayerId == player.Id
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
            return Results.Ok(HeroDto(hero!));
        });

        // ------------------------------------------------------------ towns
        app.MapGet("/api/towns/{townId:guid}", async (Guid townId, AppDb db, HttpContext ctx) =>
        {
            var town = await db.Towns.FirstOrDefaultAsync(t => t.Id == townId);
            if (town == null) return Results.NotFound();
            if (await TownHiddenAsync(db, townId, ctx.Request.Headers["X-Player-Id"]))
                return Results.BadRequest(new { error = "You have not discovered that town yet." });

            var player = await db.Players.FirstAsync(p => p.Id == town.PlayerId);
            var built = GameService.ParseStringArray(town.BuildingsJson);
            var avail = JsonSerializer.Deserialize<Dictionary<string, int>>(town.AvailableJson, Json) ?? [];
            var tiers = Creatures.BaseTiers(town.Faction);
            var owner = Seeded.Players.FirstOrDefault(p => p.Faction == player.Faction);
            var selected = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);

            return Results.Ok(new
            {
                id = town.Id,
                gameId = town.GameId,
                name = town.Name,
                ownerName = owner?.Name ?? "?",
                isOwner = selected?.Faction == player.Faction,
                faction = town.Faction,
                x = town.X,
                y = town.Y,
                buildings = built,
                builtThisTurn = player.BuiltThisTurn,
                postSlots = SocialService.TowerSlots(built),
                goldIncome = TownBuildings.GoldIncome(built),
                growthMultiplier = TownBuildings.GrowthMultiplier(built),
                available = Enumerable.Range(1, 7).Select(tier => new
                {
                    tier,
                    creature = tiers.Length >= tier ? tiers[tier - 1] : null,
                    count = avail.TryGetValue(tier.ToString(), out var c) ? c : 0,
                    dwellingBuilt = TownBuildings.HasDwelling(built, tier)
                }),
                garrison = GameService.ParseArmy(town.GarrisonJson),
                tavern = GameService.ParseStringArray(town.TavernJson),
                mageGuild = GameService.ParseStringArray(town.MageGuildJson),
                resources = new
                {
                    wood = player.Wood, ore = player.Ore, mercury = player.Mercury, sulfur = player.Sulfur,
                    crystal = player.Crystal, gems = player.Gems, gold = player.Gold
                }
            });
        });

        app.MapPost("/api/towns/{townId:guid}/build", async (Guid townId, BuildReq req, AppDb db, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });

            var town = await db.Towns.FirstOrDefaultAsync(t => t.Id == townId);
            if (town == null) return Results.NotFound();

            var player = await db.Players.FirstAsync(p => p.Id == town.PlayerId);
            if (player.Faction != sp.Faction) return Results.BadRequest(new { error = "You can only build in your own town." });

            var game = await db.Games.FirstAsync(g => g.Id == town.GameId);
            if (game.State != GameState.InProgress) return Results.BadRequest(new { error = "The game is not in progress." });
            if (player.BuiltThisTurn) return Results.BadRequest(new { error = "You have already built this turn." });

            var key = req.BuildingKey ?? "";
            if (!TownBuildings.All.TryGetValue(key, out var def)) return Results.BadRequest(new { error = "Unknown building." });
            if (key == "grail" && !player.Precious)
                return Results.BadRequest(new { error = "Requires the Grail. Dig it up first." });

            var built = GameService.ParseStringArray(town.BuildingsJson).ToList();
            if (built.Contains(key)) return Results.BadRequest(new { error = "You have already built that." });

            foreach (var r in def.Requires)
                if (!built.Contains(r))
                    return Results.BadRequest(new { error = "Requires: " + (TownBuildings.All.TryGetValue(r, out var rd) ? rd.Name : r) + "." });

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

            if (def.Tier > 0 && key.StartsWith("dwelling"))
            {
                var avail = JsonSerializer.Deserialize<Dictionary<string, int>>(town.AvailableJson, Json) ?? [];
                var k = def.Tier.ToString();
                if (!avail.ContainsKey(k)) avail[k] = 0;
                town.AvailableJson = JsonSerializer.Serialize(avail, Json);
            }

            await db.SaveChangesAsync();
            return Results.NoContent();
        });

        app.MapPost("/api/towns/{townId:guid}/recruit", async (Guid townId, RecruitReq req, AppDb db, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });

            var town = await db.Towns.FirstOrDefaultAsync(t => t.Id == townId);
            if (town == null) return Results.NotFound();

            var player = await db.Players.FirstAsync(p => p.Id == town.PlayerId);
            if (player.Faction != sp.Faction) return Results.BadRequest(new { error = "You can only recruit in your own town." });

            var built = GameService.ParseStringArray(town.BuildingsJson);
            if (!TownBuildings.HasDwelling(built, req.Tier))
                return Results.BadRequest(new { error = "That dwelling is not built." });

            var avail = JsonSerializer.Deserialize<Dictionary<string, int>>(town.AvailableJson, Json) ?? [];
            var k = req.Tier.ToString();
            var have = avail.TryGetValue(k, out var c) ? c : 0;
            var count = Math.Min(req.Count, have);
            if (count <= 0) return Results.BadRequest(new { error = "No creatures available." });

            var tiers = Creatures.BaseTiers(town.Faction);
            var creatureKey = tiers[req.Tier - 1];
            var def = Creatures.All[creatureKey];
            var cost = def.Gold * count;
            if (player.Gold < cost) return Results.BadRequest(new { error = "Not enough gold." });

            player.Gold -= cost;
            avail[k] = have - count;
            town.AvailableJson = JsonSerializer.Serialize(avail, Json);

            var hero = await db.Heroes.FirstOrDefaultAsync(h => h.PlayerId == player.Id && h.X == town.X && h.Y == town.Y);
            if (hero != null)
            {
                var army = GameService.ParseArmy(hero.ArmyJson);
                if (!GameService.AddToArmy(army, creatureKey, count))
                    return Results.BadRequest(new { error = "The hero's army is full." });
                hero.ArmyJson = GameService.SerializeArmy(army);
            }
            else
            {
                var garrison = GameService.ParseArmy(town.GarrisonJson);
                if (!GameService.AddToArmy(garrison, creatureKey, count))
                    return Results.BadRequest(new { error = "The garrison is full." });
                town.GarrisonJson = GameService.SerializeArmy(garrison);
            }

            await db.SaveChangesAsync();
            return Results.Ok(new { recruited = count, creature = creatureKey });
        });

        app.MapPost("/api/towns/{townId:guid}/trade", async (Guid townId, TradeReq req, AppDb db, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });

            var town = await db.Towns.FirstOrDefaultAsync(t => t.Id == townId);
            if (town == null) return Results.NotFound();
            var player = await db.Players.FirstAsync(p => p.Id == town.PlayerId);
            if (player.Faction != sp.Faction) return Results.BadRequest(new { error = "Not your town." });

            var built = GameService.ParseStringArray(town.BuildingsJson);
            if (!built.Contains("marketplace")) return Results.BadRequest(new { error = "No Marketplace built." });

            var from = req.From ?? "";
            var to = req.To ?? "";
            var amount = Math.Max(1, req.Amount);
            string[] valid = ["gold", "wood", "ore", "mercury", "sulfur", "crystal", "gems"];
            if (!valid.Contains(from) || !valid.Contains(to)) return Results.BadRequest(new { error = "Unknown resource." });
            if (from == to) return Results.BadRequest(new { error = "Cannot trade a resource for itself." });

            // amount is always the number of non-gold units bought or sold
            if (from == "gold")
            {
                var cost = 500 * amount;
                if (player.Gold < cost) return Results.BadRequest(new { error = "Not enough gold." });
                player.Gold -= cost;
                SetRes(player, to, GetRes(player, to) + amount);
            }
            else if (to == "gold")
            {
                var have = GetRes(player, from);
                if (have < amount) return Results.BadRequest(new { error = "Not enough " + from + "." });
                SetRes(player, from, have - amount);
                player.Gold += 250 * amount;
            }
            else
            {
                var give = 2 * amount;
                var have = GetRes(player, from);
                if (have < give) return Results.BadRequest(new { error = "Not enough " + from + "." });
                SetRes(player, from, have - give);
                SetRes(player, to, GetRes(player, to) + amount);
            }

            await db.SaveChangesAsync();
            return Results.NoContent();
        });

        app.MapPost("/api/towns/{townId:guid}/hire", async (Guid townId, HireReq req, AppDb db, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });

            var town = await db.Towns.FirstOrDefaultAsync(t => t.Id == townId);
            if (town == null) return Results.NotFound();
            var player = await db.Players.FirstAsync(p => p.Id == town.PlayerId);
            if (player.Faction != sp.Faction) return Results.BadRequest(new { error = "Not your town." });

            var built = GameService.ParseStringArray(town.BuildingsJson);
            if (!built.Contains("tavern")) return Results.BadRequest(new { error = "No Tavern built." });
            if (player.Gold < 2000) return Results.BadRequest(new { error = "Not enough gold (2000)." });

            var pool = GameService.ParseStringArray(town.TavernJson).ToList();
            var key = req.HeroKey ?? "";
            if (!pool.Contains(key)) return Results.BadRequest(new { error = "That hero is not available." });

            var count = await db.Heroes.CountAsync(h => h.PlayerId == player.Id);
            if (count >= 8) return Results.BadRequest(new { error = "You already have 8 heroes." });

            player.Gold -= 2000;
            pool.Remove(key);
            town.TavernJson = JsonSerializer.Serialize(pool, Json);

            var hero = GameService.CreateHero(town.GameId, player.Id, sp with { HeroKey = key }, town.X, town.Y);
            db.Heroes.Add(hero);
            await db.SaveChangesAsync();
            return Results.Ok(new { heroId = hero.Id });
        });

        // ------------------------------------------------------------ obelisk
        app.MapGet("/api/games/{id:guid}/obelisk", async (Guid id, AppDb db, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });

            var player = await db.Players.FirstOrDefaultAsync(p => p.GameId == id && p.Faction == sp.Faction);
            if (player == null) return Results.NotFound();

            var game = await db.Games.FirstAsync(g => g.Id == id);
            var all = GameService.ParseStringArray(game.FragmentsJson);
            var msgs = await db.ObeliskMessages.Where(m => m.GameId == id).OrderBy(m => m.CreatedAt).ToListAsync();

            return Results.Ok(new
            {
                visits = player.ObeliskVisits,
                total = all.Length,
                fragments = all.Take(player.ObeliskVisits).ToArray(),
                wrongAnswers = player.WrongAnswers,
                grailRevealed = game.GrailRevealed,
                messages = msgs.Select(m => new { id = m.Id, author = m.AuthorUserId.ToString(), body = m.Body, createdAt = m.CreatedAt })
            });
        });

        app.MapPost("/api/games/{id:guid}/obelisk/visit", async (Guid id, GameService games, HttpContext ctx) =>
        {
            var playerId = ctx.Request.Headers["X-Player-Id"].ToString();
            var (visits, fragments, error) = await games.VisitObeliskAsync(id, playerId);
            if (error != null) return Results.BadRequest(new { error });
            return Results.Ok(new { visits, fragments });
        });

        app.MapPost("/api/games/{id:guid}/obelisk/solve", async (Guid id, BodyReq req, GameService games, HttpContext ctx) =>
        {
            var playerId = ctx.Request.Headers["X-Player-Id"].ToString();
            var (correct, error) = await games.SolveAsync(id, playerId, req.Body ?? "");
            if (error != null) return Results.BadRequest(new { error, correct = false });
            return Results.Ok(new { correct });
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
        app.MapGet("/api/buildings", () => Results.Ok(TownBuildings.All.Values.Select(b => new
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
            abilities = c.Abilities, upgradeOf = c.UpgradeOf
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
        app.MapGet("/api/games/{id:guid}/_test/answer", async (Guid id, AppDb db, IHostEnvironment env) =>
        {
            if (!env.IsDevelopment()) return Results.NotFound();
            var game = await db.Games.FirstOrDefaultAsync(g => g.Id == id);
            if (game == null) return Results.NotFound();
            return Results.Ok(new { answer = Riddles.All[game.RiddleId].Answer });
        });

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
    }

    private static async Task NotifyTownAsync(AppDb db, WsHub ws, Guid townId, string type)
    {
        var town = await db.Towns.FirstOrDefaultAsync(t => t.Id == townId);
        if (town != null) await ws.BroadcastAsync(town.GameId, type, new { townId });
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

    private static object HeroDto(CityOfTowers.Domain.Hero h) => new
    {
        id = h.Id,
        name = h.Name,
        heroKey = h.HeroKey,
        faction = h.Faction,
        level = h.Level,
        experience = h.Experience,
        nextLevel = Experience.NextThreshold(h.Level),
        attack = h.Attack,
        defense = h.Defense,
        power = h.Power,
        knowledge = h.Knowledge,
        mana = h.Mana,
        maxMana = h.MaxMana,
        movement = h.Movement,
        maxMovement = h.MaxMovement,
        x = h.X,
        y = h.Y,
        army = GameService.ParseArmy(h.ArmyJson),
        skills = GameService.ParseSkills(h.SkillsJson),
        spells = GameService.ParseStringArray(h.SpellsJson),
        artifacts = GameService.ParseStringArray(h.ArtifactsJson),
        pendingEncounter = h.PendingEncounterJson
    };

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

public record CreateGameReq(string? Name, int Capacity, int TurnSeconds, int TurnLimit, int MapSize);
public record MoveReq(List<Point>? Path);
public record Point(int X, int Y);
public record BuildReq(string? BuildingKey);
public record RecruitReq(int Tier, int Count);
public record TradeReq(string? From, string? To, int Amount);
public record HireReq(string? HeroKey);
public record BodyReq(string? Body);
public record EncounterReq(string? Action);
public record MessageReq(string? ToPlayerId, string? Body);
public record TradeOfferReq(string? ToPlayerId, string? Give, int GiveAmount, string? Want, int WantAmount);
public record RespondReq(bool Accept);
public record ContributeReq(int Gold, int Wood, int Ore);
public record AnnotationReq(int X, int Y, string? Text);
public record PostReq(string? Title, string? Body, string? ImageUrl);
public record WallReq(string? Body, Guid? ParentId);