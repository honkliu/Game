using System.Text.Json;
using CityOfTowers.Data;
using CityOfTowers.Domain;
using CityOfTowers.Services;
using Microsoft.EntityFrameworkCore;

namespace CityOfTowers.Api;

public static partial class Endpoints
{
    private static void MapPeacefulActions(this WebApplication app)
    {
        app.MapPost("/api/games/{id:guid}/mine-sharing", async (Guid id, MineShareReq req, AppDb db,
            GameService games, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });
            var player = await db.Players.FirstOrDefaultAsync(p => p.GameId == id && p.Faction == sp.Faction);
            if (player == null) return Results.BadRequest(new { error = "Not in this game." });
            if (await InactiveGameAsync(db, id) is { } inactive) return inactive;
            var game = await db.Games.FirstAsync(g => g.Id == id);
            var map = await games.LoadMapAsync(game);
            if (!map.InBounds(req.X, req.Y)) return Results.BadRequest(new { error = "Unknown mine." });
            var mine = map.At(req.X, req.Y);
            var seat = Array.FindIndex(Seeded.Players, p => p.Faction == player.Faction);
            if (mine.ObjectKind != "mine" || mine.Owner != (1 << seat))
                return Results.BadRequest(new { error = "Only the mine's owner may change its sharing." });
            mine.MineShared = req.Shared;
            game.MapJson = JsonSerializer.Serialize(map, Json);
            await db.SaveChangesAsync();
            return Results.NoContent();
        });

        app.MapPost("/api/games/{id:guid}/agreement", async (Guid id, AgreementReq req, AppDb db,
            WsHub ws, HttpContext ctx) =>
        {
            var sp = Seeded.Find(ctx.Request.Headers["X-Player-Id"]);
            if (sp == null) return Results.BadRequest(new { error = "Unknown player." });
            var game = await db.Games.FirstOrDefaultAsync(g => g.Id == id);
            if (game == null) return Results.NotFound();
            if (game.State != GameState.InProgress) return Results.BadRequest(new { error = "The game is not in progress." });
            var humans = await db.Players.Where(p => p.GameId == id && !p.IsAi).ToListAsync();
            var player = humans.FirstOrDefault(p => p.Faction == sp.Faction);
            if (player == null) return Results.BadRequest(new { error = "Not a human player in this game." });
            var votes = GameService.ParseStringArray(game.EndAgreementJson).Where(v => humans.Any(p => p.Id.ToString() == v)).ToHashSet();
            if (req.Agree) votes.Add(player.Id.ToString()); else votes.Remove(player.Id.ToString());
            game.EndAgreementJson = JsonSerializer.Serialize(votes, Json);
            if (humans.Count > 0 && humans.All(p => votes.Contains(p.Id.ToString())))
                game.State = GameState.Finished;
            await db.SaveChangesAsync();
            await ws.BroadcastAsync(id, "agreement", new { state = game.State.ToString(), votes });
            return Results.Ok(new { state = game.State.ToString(), votes });
        });
    }
}

public record MineShareReq(int X, int Y, bool Shared);
public record AgreementReq(bool Agree);
