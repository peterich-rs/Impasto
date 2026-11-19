//! A small repeatable measurement. It is not a leak or speedup claim.
//!
//! The bound is only a guard against a hung runtime. The printed median is the
//! observation for this process, profile, and machine.

use js::RuntimeBuilder;

#[test]
fn eval_drive_shutdown_median() {
    let mut samples = Vec::new();
    for _ in 0..21 {
        let started = std::time::Instant::now();
        let mut runtime = RuntimeBuilder::new().build().expect("runtime");
        runtime
            .eval_script(
                "var n = 0; for (var i = 0; i < 20; i++) Promise.resolve().then(function () { n += 1; }); setTimeout(function () { globalThis.done = n; }, 0);",
                "bench.js",
            )
            .expect("eval");
        runtime.drive(1).expect("drive");
        runtime.shutdown();
        samples.push(started.elapsed().as_micros());
    }
    samples.sort_unstable();
    let median = samples[samples.len() / 2];
    eprintln!(
        "eval-drive-shutdown n={} min_us={} median_us={} max_us={}",
        samples.len(),
        samples[0],
        median,
        samples[samples.len() - 1]
    );
    assert!(
        median < 5_000_000,
        "median {median} us exceeded the 5s guard"
    );
}
