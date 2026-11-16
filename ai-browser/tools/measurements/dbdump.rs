// Read the history table from the app's SQLite db using the crate's own
// rusqlite so no external sqlite3 binary is needed.
use rusqlite::Connection;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base = std::env::var("LOCALAPPDATA")?;
    let path = std::path::Path::new(&base).join("native-browser").join("data.db");
    let conn = Connection::open(&path)?;
    let mut stmt = conn.prepare("SELECT url, title, visit_count, typed FROM history ORDER BY id")?;
    let mut rows = stmt.query([])?;
    let mut n = 0;
    let mut typed_n = 0;
    while let Some(r) = rows.next()? {
        let url: String = r.get(0)?;
        let title: String = r.get(1)?;
        let visits: i64 = r.get(2)?;
        let typed: i64 = r.get(3)?;
        n += 1;
        if typed != 0 { typed_n += 1; }
        println!("{visits}x typed={typed}  {url}  [{title}]");
    }
    println!("--- {n} rows, {typed_n} typed ---");
    Ok(())
}