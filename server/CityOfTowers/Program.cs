using Microsoft.EntityFrameworkCore;
using CityOfTowers.Api;
using CityOfTowers.Data;
using CityOfTowers.Hubs;
using CityOfTowers.Services;

var builder = WebApplication.CreateBuilder(args);

builder.Services.AddDbContext<AppDb>(o =>
    o.UseSqlite(builder.Configuration.GetConnectionString("Default") ?? "Data Source=cityoftowers.db"));

builder.Services.AddScoped<GameService>();
builder.Services.AddScoped<AiService>();
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

    // EnsureCreated does not alter an existing database, so add columns introduced later.
    // Check the schema first: a blind ALTER would fail on every start once the column exists,
    // and swallowing that error would hide a genuinely missing column.
    var hasIsAi = db.Database
        .SqlQueryRaw<string>("SELECT name AS Value FROM pragma_table_info('Players') WHERE name = 'IsAi'")
        .ToList()
        .Count > 0;
    if (!hasIsAi)
        db.Database.ExecuteSqlRaw("ALTER TABLE Players ADD COLUMN IsAi INTEGER NOT NULL DEFAULT 0;");

    // Abandoned games accumulate: each holds a full map plus heroes, towns and messages.
    // Drop anything untouched for a day, then cap the total so the file cannot grow without bound.
    var cutoff = DateTime.UtcNow.AddDays(-1);
    var stale = db.Games.Where(g => g.TurnStartedAt < cutoff).Select(g => g.Id).ToList();
    var keep = db.Games.OrderByDescending(g => g.TurnStartedAt).Take(20).Select(g => g.Id).ToList();
    stale.AddRange(db.Games.Where(g => !keep.Contains(g.Id)).Select(g => g.Id));
    stale = stale.Distinct().ToList();
    if (stale.Count > 0)
    {
        db.Players.RemoveRange(db.Players.Where(p => stale.Contains(p.GameId)));
        db.Heroes.RemoveRange(db.Heroes.Where(h => stale.Contains(h.GameId)));
        db.Towns.RemoveRange(db.Towns.Where(t => stale.Contains(t.GameId)));
        db.HeroMessages.RemoveRange(db.HeroMessages.Where(m => stale.Contains(m.GameId)));
        db.TradeOffers.RemoveRange(db.TradeOffers.Where(t => stale.Contains(t.GameId)));
        db.MapAnnotations.RemoveRange(db.MapAnnotations.Where(a => stale.Contains(a.GameId)));
        db.ObeliskMessages.RemoveRange(db.ObeliskMessages.Where(m => stale.Contains(m.GameId)));
        db.SharedProjects.RemoveRange(db.SharedProjects.Where(s => stale.Contains(s.GameId)));
        db.Games.RemoveRange(db.Games.Where(g => stale.Contains(g.Id)));
        db.SaveChanges();
        db.Database.ExecuteSqlRaw("VACUUM;");
    }
}

app.UseDefaultFiles();
app.UseStaticFiles();

app.MapApi();
H3Assets.Map(app);
app.MapHub<GameHub>("/hubs/game");

app.Run();