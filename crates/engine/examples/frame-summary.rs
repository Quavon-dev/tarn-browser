use std::io::Read;

fn main() -> std::io::Result<()> {
    let mut log = String::new();
    std::io::stdin().read_to_string(&mut log)?;
    println!("{:<16} {:>6} {:>10} {:>10}", "section", "count", "p50 µs", "p95 µs");
    for (section, count, p50, p95) in engine::startup::summarise(&log) {
        println!("{section:<16} {count:>6} {p50:>10} {p95:>10}");
    }
    Ok(())
}
