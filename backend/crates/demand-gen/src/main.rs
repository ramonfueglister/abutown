//! CLI wrapper around [`demand_gen::bake`]:
//! `demand-gen --net data/winterthur/trafficnet.json --demand-dir scratch/demand \
//!   --authored data/winterthur/demand-authored.json --out data/winterthur/trips.bin \
//!   [--landuse scratch/geo/osm-landuse.json]`

use demand_gen::{BakeConfig, bake};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut net = None;
    let mut demand_dir = None;
    let mut authored = None;
    let mut out = None;
    let mut landuse: Option<PathBuf> = None;

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let Some(value) = args.next() else {
            eprintln!("missing value for {flag}");
            return ExitCode::FAILURE;
        };
        let value = PathBuf::from(value);
        match flag.as_str() {
            "--net" => net = Some(value),
            "--demand-dir" => demand_dir = Some(value),
            "--authored" => authored = Some(value),
            "--out" => out = Some(value),
            "--landuse" => landuse = Some(value),
            other => {
                eprintln!("unknown flag {other}");
                return ExitCode::FAILURE;
            }
        }
    }
    let (Some(net_path), Some(demand_dir), Some(authored_path), Some(out_path)) =
        (net, demand_dir, authored, out)
    else {
        eprintln!(
            "usage: demand-gen --net <trafficnet.json> --demand-dir <dir> \
             --authored <demand-authored.json> --out <trips.bin> [--landuse <osm-landuse.json>]"
        );
        return ExitCode::FAILURE;
    };
    let cfg = BakeConfig {
        net_path,
        demand_dir,
        authored_path,
        landuse_path: landuse.unwrap_or_else(|| PathBuf::from("scratch/geo/osm-landuse.json")),
        out_path: out_path.clone(),
    };

    match bake(&cfg) {
        Ok(stats) => {
            let [int, inb, outb, thr] = stats.weekday_by_segment;
            let wd = stats.weekday_total as f64;
            println!("net_hash            {}", stats.net_hash_hex);
            println!(
                "furness             {} iters, max rel err {:.5}",
                stats.furness_iters, stats.furness_max_rel_err
            );
            println!("weekday trips       {}", stats.weekday_total);
            println!(
                "  internal          {int} ({:.1} %)",
                100.0 * int as f64 / wd
            );
            println!(
                "  inbound           {inb} ({:.1} %)",
                100.0 * inb as f64 / wd
            );
            println!(
                "  outbound          {outb} ({:.1} %)",
                100.0 * outb as f64 / wd
            );
            println!(
                "  through           {thr} ({:.1} %)",
                100.0 * thr as f64 / wd
            );
            println!("weekend trips       {}", stats.weekend_total);
            println!("skipped communes    {}", stats.skipped_communes);
            println!("top gateways (weekday volume):");
            for (node, vol) in stats.gateway_volumes.iter().take(5) {
                println!("  node {node:>6}  {vol}");
            }
            let bytes = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
            println!(
                "wrote {} ({:.2} MB)",
                out_path.display(),
                bytes as f64 / 1e6
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("demand-gen failed: {e}");
            ExitCode::FAILURE
        }
    }
}
