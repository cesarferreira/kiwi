use std::{hint::black_box, time::Instant};

use kiwi_keymapper::{
    config::Config,
    engine::{Engine, EventKind, Input},
    macos::keycode_to_key,
};

const SAMPLES: usize = 11;
const CONFIG: &str = r#"
[bindings]
"hyper+a" = { keys = "control+a" }
"hyper+c" = { app = "ChatGPT" }
"hyper+d" = { app = "Dia" }
"hyper+m" = { app = "Spotify" }
"hyper+n" = { app = "Notion" }
"hyper+p" = { command = "bluepods connect AirPods" }
"hyper+s" = { app = "Slack" }
"hyper+t" = { app = "Ghostty" }
"hyper+w" = { app = "Whatsapp" }
"#;

fn main() {
    let ordinary = benchmark_engine(2_000_000, |engine| {
        black_box(engine.handle(key(12, EventKind::Down)));
        black_box(engine.handle(key(12, EventKind::Up)));
    });
    let mapped = benchmark_engine(1_000_000, |engine| {
        black_box(engine.handle(key(57, EventKind::Down)));
        black_box(engine.handle(key(0, EventKind::Down)));
        black_box(engine.handle(key(0, EventKind::Up)));
        black_box(engine.handle(key(57, EventKind::Up)));
    });
    let unmapped = benchmark_engine(1_000_000, |engine| {
        black_box(engine.handle(key(57, EventKind::Down)));
        black_box(engine.handle(key(6, EventKind::Down)));
        black_box(engine.handle(key(6, EventKind::Up)));
        black_box(engine.handle(key(57, EventKind::Up)));
    });
    let config = benchmark_config();

    println!("Kiwi engine benchmark (median of {SAMPLES} samples)");
    println!("config parse + compile  {config:>6.0} ns");
    println!("ordinary key down/up    {ordinary:>6.0} ns/cycle");
    println!("mapped Hyper shortcut   {mapped:>6.0} ns/cycle");
    println!("unmapped Hyper shortcut {unmapped:>6.0} ns/cycle");
}

fn key(code: u16, kind: EventKind) -> Input {
    Input::Key {
        key: keycode_to_key(code).unwrap(),
        kind,
        repeat: false,
    }
}

fn benchmark_engine(iterations: usize, mut cycle: impl FnMut(&mut Engine)) -> f64 {
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let config = Config::from_toml(CONFIG).unwrap().compile().unwrap();
        let mut engine = Engine::new(config);
        let start = Instant::now();
        for _ in 0..iterations {
            cycle(&mut engine);
        }
        samples.push(start.elapsed().as_nanos() as f64 / iterations as f64);
    }
    median(samples)
}

fn benchmark_config() -> f64 {
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        for _ in 0..1_000 {
            black_box(
                Config::from_toml(black_box(CONFIG))
                    .unwrap()
                    .compile()
                    .unwrap(),
            );
        }
        samples.push(start.elapsed().as_nanos() as f64 / 1_000.0);
    }
    median(samples)
}

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}
