using Microsoft.AspNetCore.SignalR;
using CityOfTowers.Services;

namespace CityOfTowers.Hubs;

/// <summary>
/// Real-time game events. Clients connect with ?gameId=... and join the
/// "game:{id}" group; <see cref="WsHub"/> sends a single "event" message
/// shaped { type, gameId, payload }.
/// </summary>
public class GameHub : Hub
{
    public override async Task OnConnectedAsync()
    {
        var gameId = Context.GetHttpContext()?.Request.Query["gameId"].ToString();
        if (Guid.TryParse(gameId, out var id))
            await Groups.AddToGroupAsync(Context.ConnectionId, WsHub.Group(id));
        await base.OnConnectedAsync();
    }
}
