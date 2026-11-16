use rusqlite::Connection;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base = std::env::var("LOCALAPPDATA")?;
    let path = std::path::Path::new(&base).join("native-browser").join("data.db");
    let conn = Connection::open(&path)?;
    {
        let mut s = conn.prepare("SELECT name, sql FROM sqlite_master WHERE type='table'")?;
        let mut rows = s.query([])?;
        while let Some(r) = rows.next()? {
            let name: String = r.get(0)?;
            let sql: Option<String> = r.get(1)?;
            println!("== {name}\n{}\n", sql.unwrap_or_default());
        }
    }
    let mut s2 = conn.prepare("SELECT * FROM history")?;
    let cols: Vec<String> = s2.column_names().iter().map(|c| c.to_string()).collect();
    println!("history columns: {}", cols.join(", "));
    let mut r2 = s2.query([])?;
    let mut n = 0;
    while let Some(row) = r2.next()? {
        n += 1;
        let mut parts = Vec::new();
        for (i, c) in cols.iter().enumerate() {
            let v = row.get_ref(i).map(|v| format!("{v:?}")).unwrap_or_default();
            parts.push(format!("{c}={v}"));
        }
        println!("{}", parts.join("  "));
    }
    println!("--- {n} history rows ---");
    Ok(())
}