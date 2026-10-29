using System.Text.Json;
using CityOfTowers.Data;
using CityOfTowers.Domain;
using CityOfTowers.Services;
using Microsoft.EntityFrameworkCore;

namespace CityOfTowers.Api;

public static partial class Endpoints
{
    private static async Task<(Hero? Hero, IResult? Error)> OwnHeroAsync(AppDb db, HttpContext ctx, Guid id)
    {
        var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
        if (sp == null) return (null, Results.BadRequest(new { error = "Unknown player." }));
        var hero = await db.Heroes.FirstOrDefaultAsync(h => h.Id == id);
        if (hero == null) return (null, Results.NotFound());
        var player = await db.Players.FirstAsync(p => p.Id == hero.PlayerId);
        if (player.Faction != sp.Faction) return (null, Results.BadRequest(new { error = "Not your hero." }));
        return (hero, await InactiveGameAsync(db, hero.GameId));
    }

    private static bool CanExchange(Hero from, Hero to) => from.Id != to.Id
        && from.PlayerId == to.PlayerId && from.GameId == to.GameId
        && Math.Max(Math.Abs(from.X - to.X), Math.Abs(from.Y - to.Y)) <= 1;

    private static async Task RefreshArmyMovementAsync(AppDb db, params Hero[] heroes)
    {
        var game = await db.Games.FirstAsync(g => g.Id == heroes[0].GameId);
        foreach (var hero in heroes)
        {
            var effects = GameService.Effects(hero);
            hero.MaxMovement = GameService.MaxMovement(hero) + (TownRules.StabledThisWeek(hero, game) ? 400 : 0)
                + (effects.HasteTurn == game.CurrentTurn ? effects.HasteBonus : 0);
        }
    }

    private static async Task RefreshHeroSightAsync(AppDb db, GameService games, Hero hero)
    {
        var game = await db.Games.FirstAsync(g => g.Id == hero.GameId);
        var map = await games.LoadMapAsync(game);
        var player = await db.Players.FirstAsync(p => p.Id == hero.PlayerId);
        var fog = GameService.ParseFog(player.FogJson, map.Width * map.Height);
        GameService.RecomputeFog(fog, map.Width, map.Height,
            await db.Heroes.Where(h => h.PlayerId == player.Id).ToListAsync(),
            await db.Towns.Where(t => t.PlayerId == player.Id).ToListAsync());
        player.FogJson = GameService.SerializeFog(fog);
    }

    private static void TeachScholar(Hero first, Hero second)
    {
        var level = Math.Max(GameService.ParseSkills(first.SkillsJson).GetValueOrDefault("scholar"),
            GameService.ParseSkills(second.SkillsJson).GetValueOrDefault("scholar"));
        if (level <= 0 || !TownRules.HasGear(first, TownRules.Spellbook) || !TownRules.HasGear(second, TownRules.Spellbook)) return;
        var shared = GameService.ParseStringArray(first.SpellsJson).Concat(GameService.ParseStringArray(second.SpellsJson))
            .Select(Spells.CanonicalKey).Distinct().ToArray();
        foreach (var hero in new[] { first, second })
        {
            var max = Math.Min(level + 1, Spells.MaxLearnable(GameService.ParseSkills(hero.SkillsJson).GetValueOrDefault("wisdom")));
            var spells = GameService.ParseStringArray(hero.SpellsJson).Select(Spells.CanonicalKey).ToHashSet();
            spells.UnionWith(shared.Where(key => Spells.All.TryGetValue(key, out var def) && def.Level <= max));
            hero.SpellsJson = JsonSerializer.Serialize(spells, Json);
        }
    }

    private static void MapHeroActions(this WebApplication app)
    {
        app.MapGet("/api/heroes/{heroId:guid}/visions", async (Guid heroId, AppDb db, GameService games, HttpContext ctx) =>
        {
            var (hero, error) = await OwnHeroAsync(db, ctx, heroId);
            if (error != null) return error;
            var effects = GameService.Effects(hero!);
            var game = await db.Games.FirstAsync(g => g.Id == hero!.GameId);
            var targets = new List<VisionTarget>();
            if (effects.VisionsTurn != game.CurrentTurn) return Results.Ok(targets);
            var map = await games.LoadMapAsync(game);
            var player = await db.Players.FirstAsync(p => p.Id == hero!.PlayerId);
            var fog = GameService.ParseFog(player.FogJson, map.Width * map.Height);
            var radius = Math.Max(3, Math.Max(1, hero!.Power + Artifacts.Bonus(hero).Power) * Math.Max(1, effects.VisionsMastery));
            bool Visible(int x, int y) => map.InBounds(x, y) && fog[y * map.Width + x] == 2
                && (long)(x - hero.X) * (x - hero.X) + (long)(y - hero.Y) * (y - hero.Y) < (long)radius * radius;
            for (var y = 0; y < map.Height; y++)
            for (var x = 0; x < map.Width; x++)
            {
                var tile = map.At(x, y);
                if (tile.ObjectKind != "monster" || !Visible(x, y)) continue;
                var parts = (tile.ObjectData ?? "").Split(':');
                if (parts.Length < 2 || !Creatures.All.TryGetValue(parts[0], out var creature)
                    || !int.TryParse(parts[1], out var count) || count <= 0) continue;
                var join = Encounters.CanRecruit(Encounters.Strength(hero.ArmyJson), Encounters.StackStrength(creature.AiValue, count));
                targets.Add(new("monster", creature.Name, x, y, [new(parts[0], count)], CanRecruit: join, Gold: Encounters.RecruitCost(creature.Gold, count)));
            }
            if (effects.VisionsMastery >= 2)
                foreach (var other in await db.Heroes.Where(h => h.GameId == game.Id && h.Id != hero.Id).ToListAsync())
                    if (Visible(other.X, other.Y)) targets.Add(new("hero", other.Name, other.X, other.Y, GameService.ParseArmy(other.ArmyJson), other.Level));
            if (effects.VisionsMastery >= 3)
                foreach (var town in await db.Towns.Where(t => t.GameId == game.Id).ToListAsync())
                    if (Visible(town.X, town.Y)) targets.Add(new("town", town.Name, town.X, town.Y, GameService.ParseArmy(town.GarrisonJson)));
            return Results.Ok(targets);
        });
        app.MapPost("/api/heroes/{heroId:guid}/level-up", async (Guid heroId, LevelUpReq req, AppDb db,
            GameService games, HttpContext ctx) =>
        {
            var (hero, error) = await OwnHeroAsync(db, ctx, heroId);
            if (error != null) return error;
            var choiceError = HeroProgression.Choose(hero!, req.Skill);
            if (choiceError != null) return Results.BadRequest(new { error = choiceError });
            var game = await db.Games.FirstAsync(g => g.Id == hero!.GameId);
            hero!.MaxMovement = GameService.MaxMovement(hero) + (TownRules.StabledThisWeek(hero, game) ? 400 : 0)
                + GameService.Effects(hero).HasteBonus;
            var player = await db.Players.FirstAsync(p => p.Id == hero.PlayerId);
            var map = await games.LoadMapAsync(game);
            var fog = GameService.ParseFog(player.FogJson, map.Width * map.Height);
            GameService.RecomputeFog(fog, map.Width, map.Height,
                await db.Heroes.Where(h => h.PlayerId == player.Id).ToListAsync(),
                await db.Towns.Where(t => t.PlayerId == player.Id).ToListAsync());
            player.FogJson = GameService.SerializeFog(fog);
            await db.SaveChangesAsync();
            return Results.Ok(HeroDto(hero));
        });
        app.MapPost("/api/heroes/{heroId:guid}/exchange", async (Guid heroId, HeroExchangeReq req, AppDb db, HttpContext ctx) =>
        {
            var (hero, error) = await OwnHeroAsync(db, ctx, heroId);
            if (error != null) return error;
            var other = await db.Heroes.FirstOrDefaultAsync(h => h.Id == req.OtherHeroId);
            if (other == null || !CanExchange(hero!, other))
                return Results.BadRequest(new { error = "Your heroes must meet before exchanging belongings." });
            if (req.FromSlot.HasValue || req.ToSlot.HasValue)
            {
                if (!req.FromSlot.HasValue || !req.ToSlot.HasValue) return Results.BadRequest(new { error = "Choose both slots." });
                var from = GameService.ParseArmy(hero!.ArmyJson);
                var to = GameService.ParseArmy(other.ArmyJson);
                var transfer = TownRules.Transfer(from, req.FromSlot.Value, to, req.ToSlot.Value, req.Count, true, true);
                if (transfer != null) return Results.BadRequest(new { error = transfer });
                hero.ArmyJson = GameService.SerializeArmy(from);
                other.ArmyJson = GameService.SerializeArmy(to);
                await RefreshArmyMovementAsync(db, hero, other);
            }
            TeachScholar(hero!, other);
            await db.SaveChangesAsync();
            return Results.Ok(new { hero = HeroDto(hero!), other = HeroDto(other) });
        });

        app.MapPost("/api/heroes/{heroId:guid}/dismiss-stack", async (Guid heroId, DismissStackReq req, AppDb db, HttpContext ctx) =>
        {
            var (hero, error) = await OwnHeroAsync(db, ctx, heroId);
            if (error != null) return error;
            if (!req.Confirmed) return Results.BadRequest(new { error = "Confirm dismissal first." });
            var army = GameService.ParseArmy(hero!.ArmyJson);
            if (req.Slot is < 0 or > 6 || army[req.Slot].Creature == null || army[req.Slot].Count <= 0)
                return Results.BadRequest(new { error = "Choose a creature stack." });
            if (army.Count(s => s.Creature != null && s.Count > 0) <= 1)
                return Results.BadRequest(new { error = "A hero must keep at least one stack." });
            army[req.Slot] = new(null, 0);
            hero.ArmyJson = GameService.SerializeArmy(army);
            await RefreshArmyMovementAsync(db, hero);
            await db.SaveChangesAsync();
            return Results.Ok(HeroDto(hero));
        });

        app.MapPost("/api/heroes/{heroId:guid}/dismiss", async (Guid heroId, ConfirmReq req, AppDb db, GameService games, HttpContext ctx) =>
        {
            var (hero, error) = await OwnHeroAsync(db, ctx, heroId);
            if (error != null) return error;
            if (!req.Confirmed) return Results.BadRequest(new { error = "Confirm dismissal first." });
            if (!await db.Towns.AnyAsync(t => t.PlayerId == hero!.PlayerId)
                && !await db.Heroes.AnyAsync(h => h.PlayerId == hero!.PlayerId && h.Id != hero.Id))
                return Results.BadRequest(new { error = "You cannot dismiss your last hero without a town." });
            db.Heroes.Remove(hero!);
            await db.SaveChangesAsync();
            await RefreshHeroSightAsync(db, games, hero!);
            await db.SaveChangesAsync();
            return Results.NoContent();
        });

        app.MapPost("/api/heroes/{heroId:guid}/artifacts", async (Guid heroId, ArtifactMoveReq req, AppDb db, GameService games, HttpContext ctx) =>
        {
            var (hero, error) = await OwnHeroAsync(db, ctx, heroId);
            if (error != null) return error;
            var other = req.ToHeroId == null || req.ToHeroId == heroId ? hero
                : await db.Heroes.FirstOrDefaultAsync(h => h.Id == req.ToHeroId);
            if (other == null || (other != hero && !CanExchange(hero!, other)))
                return Results.BadRequest(new { error = "Your heroes must meet before exchanging artifacts." });
            var from = Artifacts.Layout(hero!);
            var to = other == hero ? from : Artifacts.Layout(other);
            if (req.From == req.To && other == hero) return Results.BadRequest(new { error = "Choose another slot." });
            if (string.IsNullOrEmpty(req.From) || string.IsNullOrEmpty(req.To))
                return Results.BadRequest(new { error = "Choose both artifact slots." });
            bool Backpack(string place, out int index)
            {
                index = -1;
                return place.StartsWith("backpack:", StringComparison.Ordinal) && int.TryParse(place.AsSpan(9), out index);
            }
            string? At(Artifacts.Equipment e, string place)
            {
                if (Backpack(place, out var n)) return n >= 0 && n < e.Backpack.Count ? e.Backpack[n] : null;
                return e.Worn.GetValueOrDefault(place);
            }
            bool Valid(Artifacts.Equipment e, string place, string? key) => Backpack(place, out var n)
                ? n >= 0 && n <= e.Backpack.Count && (n < e.Backpack.Count || key == null || e.Backpack.Count < 64)
                : Artifacts.Slots.Contains(place) && (key == null || Artifacts.Fits(place, key));
            void Put(Artifacts.Equipment e, string place, string? key)
            {
                if (Backpack(place, out var n))
                {
                    if (key == null) e.Backpack.RemoveAt(n);
                    else if (n == e.Backpack.Count) e.Backpack.Add(key);
                    else e.Backpack[n] = key;
                }
                else if (key == null) e.Worn.Remove(place);
                else e.Worn[place] = key;
            }
            var item = At(from, req.From);
            var displaced = At(to, req.To);
            if (item == null || !Valid(to, req.To, item) || !Valid(from, req.From, displaced))
                return Results.BadRequest(new { error = "The artifact does not fit that slot, or the backpack is full." });
            // Write the destination first: removing a source backpack entry shifts later indices.
            Put(to, req.To, item);
            Put(from, req.From, displaced);
            Artifacts.Save(hero!, from);
            if (other != hero) Artifacts.Save(other, to);
            if (other != hero) TeachScholar(hero!, other);
            await RefreshHeroSightAsync(db, games, hero!);
            await db.SaveChangesAsync();
            return Results.Ok(new { hero = HeroDto(hero!), other = other == hero ? null : HeroDto(other) });
        });
    }
}

public record HeroExchangeReq(Guid OtherHeroId, int? FromSlot = null, int? ToSlot = null, int Count = 0);
public record DismissStackReq(int Slot, bool Confirmed);
public record ConfirmReq(bool Confirmed);
public record ArtifactMoveReq(string From, string To, Guid? ToHeroId = null);
public record LevelUpReq(string? Skill);
public record VisionTarget(string Kind, string Name, int X, int Y, GameService.ArmySlot[] Army, int? Level = null, bool? CanRecruit = null, int? Gold = null);
