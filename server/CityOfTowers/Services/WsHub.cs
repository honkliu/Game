using Microsoft.AspNetCore.SignalR;
using CityOfTowers.Hubs;

namespace CityOfTowers.Services;

/// <summary>
/// Broadcasts game events to every SignalR client in the game's group
/// ("game:{id}"). Safe to call from services and background workers.
/// </summary>
public class WsHub
{
    private readonly IHubContext<GameHub> _hub;

    public WsHub(IHubContext<GameHub> hub) => _hub = hub;

    public static string Group(Guid gameId) => "game:" + gameId;

    public async Task BroadcastAsync(Guid gameId, string type, object payload)
    {
        try { await _hub.Clients.Group(Group(gameId)).SendAsync("event", new { type, gameId, payload }); }
        catch { /* realtime is best-effort; clients also refresh on focus */ }
    }
}
