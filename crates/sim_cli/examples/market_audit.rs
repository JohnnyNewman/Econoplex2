//! Market audit: what is made, what is used, what piles up. Diagnostic only.
//! cargo run --release -p sim_cli --example market_audit -- [years] [seed]
use sim_core::{AssetPaths, Db, Simulation, Towns};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let years: u64 = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(10);
    let seed: Option<u64> = args.get(2).and_then(|v| v.parse().ok());
    let paths = AssetPaths::discover();
    let (db, params, mut scenario) = sim_core::load(&paths).unwrap();
    if let Some(s) = seed {
        scenario.seed = s;
    }
    let dpy = params.life.days_per_year as u64;
    let mut sim = Simulation::new(db, params, &scenario);
    for _ in 0..years * dpy {
        sim.step();
    }
    let db = sim.world.resource::<Db>();
    let towns = sim.world.resource::<Towns>();
    let names: Vec<&str> = db
        .content
        .products
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    let mp = &sim.world.resource::<sim_core::world::Params>().market;
    for t in &towns.0 {
        let np = t.stock.len();
        let n = t.residents.len() as f32;
        let (mut made, mut glut) = (0.0, 0.0);
        let mut rows = Vec::new();
        for (p, &name) in names.iter().enumerate().take(np) {
            let base = db.base_price[p];
            let m = t.produced_last_year.get(p).copied().unwrap_or(0.0);
            let g = t.glut_made_last_year.get(p).copied().unwrap_or(0.0);
            if db.coin[p] > 0.0 || db.category(p as u32) == sim_data::Category::Building {
                continue;
            }
            made += m * base;
            glut += g * base;
            let target = if db.is_food[p] {
                mp.food_target_per_capita
            } else {
                mp.target_per_capita
            } * n;
            if g > 0.0 {
                rows.push((
                    name,
                    m,
                    g,
                    g * base,
                    t.stock[p],
                    target,
                    t.price[p] / base.max(1e-6),
                ));
            }
        }
        println!(
            "
{} (pop {}): made {:.0} at base value, {:.0}% of it while over target",
            t.name,
            t.residents.len(),
            made,
            100.0 * glut / made.max(1.0)
        );
        rows.sort_by(|a, b| b.3.total_cmp(&a.3));
        println!(
            "  {:<18} {:>8} {:>10} {:>9} {:>7} {:>7} {:>6}",
            "good", "made/yr", "over tgt", "value", "stock", "target", "p/base"
        );
        for (nm, m, g, v, s, tg, pr) in rows.iter().take(12) {
            println!("  {nm:<18} {m:8.0} {g:10.0} {v:9.0} {s:7.0} {tg:7.0} {pr:6.2}");
        }
    }
}
