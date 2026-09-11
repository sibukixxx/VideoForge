use std::{fs, hint::black_box, time::Instant};

use videoforge_timeline_wasm::calculate_timeline_json;

fn main() {
    let input = fs::read_to_string("fixtures/micro-wasm/multi-clip.input.json")
        .expect("run from the repository root");
    for iterations in [1_u32, 100, 1_000, 10_000] {
        let started = Instant::now();
        for _ in 0..iterations {
            black_box(calculate_timeline_json(black_box(&input)).unwrap());
        }
        let elapsed = started.elapsed();
        println!(
            "Native {iterations:>5} calls: {:>9.3} ms ({:>8} ns/call)",
            elapsed.as_secs_f64() * 1_000.0,
            elapsed.as_nanos() / u128::from(iterations)
        );
    }
}
