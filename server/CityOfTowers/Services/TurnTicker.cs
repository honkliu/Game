using Microsoft.EntityFrameworkCore;
using CityOfTowers.Data;

namespace CityOfTowers.Services;

/// <summary>
/// Advances turns for every in-progress game once per second.
/// Skips the database entirely when no game is running.
/// </summary>
public class TurnTicker : BackgroundService
{
    private readonly IServiceScopeFactory _scopes;
    private readonly WsHub _hub;
    private readonly ILogger<TurnTicker> _log;

    public TurnTicker(IServiceScopeFactory scopes, WsHub hub, ILogger<TurnTicker> log)
    {
        _scopes = scopes;
        _hub = hub;
        _log = log;
    }

    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        while (!stoppingToken.IsCancellationRequested)
        {
            try
            {
                using var scope = _scopes.CreateScope();
                var db = scope.ServiceProvider.GetRequiredService<AppDb>();

                // Cheap existence check: no in-progress game means no work at all.
                var anyRunning = await db.Games.AnyAsync(g => g.State == Domain.GameState.InProgress, stoppingToken);
                if (anyRunning)
                {
                    var games = scope.ServiceProvider.GetRequiredService<GameService>();
                    var advanced = await games.TickAsync();
                    foreach (var gameId in advanced)
                    {
                        await _hub.BroadcastAsync(gameId, "turnAdvanced", new { gameId });
                    }
                }
            }
            catch (OperationCanceledException)
            {
                break;
            }
            catch (Exception ex)
            {
                _log.LogError(ex, "Turn tick failed");
            }
            await Task.Delay(1000, stoppingToken);
        }
    }
}