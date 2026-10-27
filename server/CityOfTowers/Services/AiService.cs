using System.Text.Json;
using Microsoft.EntityFrameworkCore;
using CityOfTowers.Data;
using CityOfTowers.Domain;

namespace CityOfTowers.Services;

/// <summary>
/// A simple computer opponent. Each day it builds the cheapest affordable structure,
/// recruits what it can afford, and walks its hero toward the nearest unclaimed mine.
/// It never fights (there is no combat) and never touches another player's town.
/// </summary>
public class AiService
{
    private readonly AppDb _db;
    private readonly GameService _games;

    public AiService(AppDb db, GameService games)
    {
        _db = db;
        _games = games;
    }

    /// <summary>Play one day for every computer-controlled seat in the game.</summary>
    public async Task PlayAsync(Game game)
    {
        var players = await _db.Players.Where(p => p.GameId == game.Id && p.IsAi).ToListAsync();
        foreach (var player in players)
        {
            try
            {
                await BuildAsync(game, player);
                await RecruitAsync(game, player);
                await ExploreAsync(game, player);
            }
            catch
            {
                // a single failed action must not stall the other seats
            }
        }
        await _db.SaveChangesAsync();
    }

    private async Task BuildAsync(Game game, Player player)
    {
        if (player.BuiltThisTurn) return;
        var town = await _db.Towns.FirstOrDefaultAsync(t => t.PlayerId == player.Id);
        if (town == null) return;

        var built = GameService.ParseStringArray(town.BuildingsJson).ToHashSet();
        var candidates = TownBuildings.All.Values
            .Where(b => !built.Contains(b.Key))
            .Where(b => b.Requires.All(built.Contains))
            .Where(b => CanAfford(player, b))
            .OrderBy(b => b.Gold + b.Wood * 50 + b.Ore * 50)
            .ToList();

        var pick = candidates.FirstOrDefault();
        if (pick == null) return;

        player.Gold -= pick.Gold;
        player.Wood -= pick.Wood;
        player.Ore -= pick.Ore;
        player.Mercury -= pick.Mercury;
        player.Sulfur -= pick.Sulfur;
        player.Crystal -= pick.Crystal;
        player.Gems -= pick.Gems;

        built.Add(pick.Key);
        town.BuildingsJson = JsonSerializer.Serialize(built.ToArray());
        player.BuiltThisTurn = true;

        if (pick.Key.StartsWith("mageGuild"))
        {
            var level = int.TryParse(pick.Key.AsSpan("mageGuild".Length), out var lv) ? lv : 1;
            var known = GameService.ParseStringArray(town.MageGuildJson).ToList();
            foreach (var spell in Spells.ForGuild(level, Random.Shared))
                if (!known.Contains(spell)) known.Add(spell);
            town.MageGuildJson = JsonSerializer.Serialize(known);
        }
    }

    private async Task RecruitAsync(Game game, Player player)
    {
        var town = await _db.Towns.FirstOrDefaultAsync(t => t.PlayerId == player.Id);
        if (town == null) return;
        var hero = await _db.Heroes.FirstOrDefaultAsync(h => h.PlayerId == player.Id);
        if (hero == null) return;

        var built = GameService.ParseStringArray(town.BuildingsJson).ToHashSet();
        var avail = JsonSerializer.Deserialize<Dictionary<string, int>>(town.AvailableJson) ?? [];
        var tiers = Creatures.BaseTiers(town.Faction);

        for (var tier = 7; tier >= 1; tier--)
        {
            if (!TownBuildings.HasDwelling(built, tier)) continue;
            var key = tiers[tier - 1];
            if (!avail.TryGetValue(tier.ToString(), out var count) || count <= 0) continue;

            var cost = Creatures.All[key].Gold;
            if (cost <= 0) continue;
            var affordable = Math.Min(count, player.Gold / cost);
            if (affordable <= 0) continue;

            var army = GameService.ParseArmy(hero.ArmyJson);
            if (!GameService.AddToArmy(army, key, affordable)) continue;

            hero.ArmyJson = GameService.SerializeArmy(army);
            player.Gold -= affordable * cost;
            avail[tier.ToString()] = count - affordable;
            town.AvailableJson = JsonSerializer.Serialize(avail);
            return;
        }
    }

    private async Task ExploreAsync(Game game, Player player)
    {
        var hero = await _db.Heroes.FirstOrDefaultAsync(h => h.PlayerId == player.Id);
        if (hero == null || hero.Movement <= 0) return;

        var map = await _games.LoadMapAsync(game);
        var target = NearestMine(map, player, hero);
        if (target == null) return;

        // step one tile toward the target; the move endpoint handles the rest
        var step = StepToward(map, hero.X, hero.Y, target.Value.x, target.Value.y);
        if (step == null) return;
        await _games.MoveHeroAsync(hero.Id, player.Faction, [step.Value]);
    }

    private static (int x, int y)? NearestMine(AdventureMap map, Player player, Hero hero)
    {
        var seat = Array.FindIndex(Seeded.Players, p => p.Faction == player.Faction);
        if (seat < 0) return null;
        var bit = 1 << seat;

        (int x, int y)? best = null;
        var bestDist = int.MaxValue;
        for (var y = 0; y < map.Height; y++)
        {
            for (var x = 0; x < map.Width; x++)
            {
                var t = map.At(x, y);
                if (t.ObjectKind != "mine" || (t.Owner & bit) != 0) continue;
                var d = Math.Abs(x - hero.X) + Math.Abs(y - hero.Y);
                if (d < bestDist) { bestDist = d; best = (x, y); }
            }
        }
        return best;
    }

    private static (int x, int y)? StepToward(AdventureMap map, int sx, int sy, int tx, int ty)
    {
        var dx = Math.Sign(tx - sx);
        var dy = Math.Sign(ty - sy);
        foreach (var (nx, ny) in new[] { (sx + dx, sy + dy), (sx + dx, sy), (sx, sy + dy) })
        {
            if (nx == sx && ny == sy) continue;
            if (!map.InBounds(nx, ny)) continue;
            var t = map.At(nx, ny);
            if (t.Passable || t.ObjectKind is "mine" or "monster" or "town" or "obelisk") return (nx, ny);
        }
        return null;
    }

    private static bool CanAfford(Player p, TownBuildingDef b) =>
        p.Gold >= b.Gold && p.Wood >= b.Wood && p.Ore >= b.Ore &&
        p.Mercury >= b.Mercury && p.Sulfur >= b.Sulfur && p.Crystal >= b.Crystal && p.Gems >= b.Gems;
}