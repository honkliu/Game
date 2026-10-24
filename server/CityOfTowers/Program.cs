using Microsoft.EntityFrameworkCore;
using CityOfTowers.Api;
using CityOfTowers.Data;
using CityOfTowers.Hubs;
using CityOfTowers.Services;

var builder = WebApplication.CreateBuilder(args);

builder.Services.AddDbContext<AppDb>(o =>
    o.UseSqlite(builder.Configuration.GetConnectionString("Default") ?? "Data Source=cityoftowers.db"));

builder.Services.AddScoped<GameService>();
builder.Services.AddScoped<SocialService>();
builder.Services.AddSingleton<WsHub>();
builder.Services.AddSingleton<RateLimiter>();
builder.Services.AddSingleton<H3Assets>();
builder.Services.AddHostedService<TurnTicker>();
builder.Services.AddSignalR();

var app = builder.Build();

using (var scope = app.Services.CreateScope())
{
    var db = scope.ServiceProvider.GetRequiredService<AppDb>();
    db.Database.EnsureCreated();
}

app.UseDefaultFiles();
app.UseStaticFiles();

app.MapApi();
H3Assets.Map(app);
app.MapHub<GameHub>("/hubs/game");

app.Run();