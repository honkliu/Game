using Microsoft.EntityFrameworkCore;
using CityOfTowers.Domain;

namespace CityOfTowers.Data;

public class AppDb : DbContext
{
    public AppDb(DbContextOptions<AppDb> options) : base(options) { }

    public DbSet<Game> Games => Set<Game>();
    public DbSet<Player> Players => Set<Player>();
    public DbSet<Hero> Heroes => Set<Hero>();
    public DbSet<Town> Towns => Set<Town>();
    public DbSet<Post> Posts => Set<Post>();
    public DbSet<Comment> Comments => Set<Comment>();
    public DbSet<WallMessage> WallMessages => Set<WallMessage>();
    public DbSet<TowerMessage> TowerMessages => Set<TowerMessage>();
    public DbSet<ObeliskMessage> ObeliskMessages => Set<ObeliskMessage>();
    public DbSet<SharedProject> SharedProjects => Set<SharedProject>();
    public DbSet<MapAnnotation> MapAnnotations => Set<MapAnnotation>();
    public DbSet<HeroMessage> HeroMessages => Set<HeroMessage>();
    public DbSet<TradeOffer> TradeOffers => Set<TradeOffer>();

    protected override void OnModelCreating(ModelBuilder b)
    {
        b.Entity<Player>().HasIndex(p => new { p.GameId, p.Faction }).IsUnique();
        b.Entity<Hero>().HasIndex(h => new { h.GameId, h.PlayerId });
        b.Entity<Hero>().Property(h => h.PendingTreasureJson).IsConcurrencyToken();
        b.Entity<Town>().HasIndex(t => new { t.GameId, t.PlayerId });
        b.Entity<WallMessage>().HasIndex(x => new { x.TownId, x.CreatedAt });
        b.Entity<Comment>().HasIndex(x => new { x.PostId, x.CreatedAt });
        b.Entity<Post>().HasIndex(x => new { x.TownId, x.CreatedAt });
        b.Entity<ObeliskMessage>().HasIndex(x => new { x.GameId, x.CreatedAt });
        b.Entity<TowerMessage>().HasIndex(x => new { x.TownId, x.CreatedAt });
        b.Entity<MapAnnotation>().HasIndex(x => new { x.GameId, x.CreatedAt });
        b.Entity<HeroMessage>().HasIndex(x => new { x.GameId, x.ToPlayerId, x.CreatedAt });
        b.Entity<TradeOffer>().HasIndex(x => new { x.GameId, x.ToPlayerId, x.Status });
    }
}