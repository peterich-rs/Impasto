fn sample_gallery_frames() {
    let source = std::env::var_os("IMPASTO_BENCH_GALLERY")
        .map(std::fs::read_to_string)
        .transpose()
        .expect("read the requested Gallery ESM")
        .unwrap_or_else(gallery_source);
    let started = std::time::Instant::now();
    let empty =
        open("", "empty.js", crate::text::MonoShaper::default(), no_log()).expect("empty UI");
    println!(
        "SAMPLE ui-start {:.3}",
        started.elapsed().as_secs_f64() * 1_000_000.0
    );
    drop(empty);

    for scenario in ["click", "scroll"] {
        let started = std::time::Instant::now();
        let Some(mut session) = open_gallery_source(&source) else {
            return;
        };
        let startup_us = started.elapsed().as_secs_f64() * 1_000_000.0;
        session.metrics(&phone());
        let first = pump_first(session.as_mut());
        let first_phase = first.phases.expect("timed first frame");
        println!(
            "FIRST {scenario} {} {} {} {:.3}",
            first_phase.trace.nodes_created,
            first_phase.trace.ffi_calls,
            first_phase.trace.ffi_batches,
            (first_phase.script_ms + first_phase.layout_cb_ms) * 1000.0
        );
        let target = if scenario == "click" {
            let shown = texts(&first);
            let (navigation, label, next_frame) = if shown.iter().any(|text| text == "市场") {
                (first, "市场", 16.0)
            } else if shown.iter().any(|text| text == "账户") {
                (first, "账户", 16.0)
            } else {
                let (x, y) = text_spot(&first, "⋯");
                click(session.as_mut(), x, y);
                (session.frame(session::FrameTime::at(16.0)), "市场", 32.0)
            };
            let (x, y) = text_spot(&navigation, label);
            click(session.as_mut(), x, y);
            next_frame
        } else {
            session.pointer(PointerEvent::wheel(180.0, 300.0, 16.0, 0.0, 48.0));
            16.0
        };
        let frame = session.frame(session::FrameTime::at(target));
        assert!(session.take_script_error().is_none());
        assert!(session.take_faults().is_empty());
        let phase = frame.phases.expect("timed frame");
        println!("STARTUP {scenario} {startup_us:.3}");
        println!(
            "BENCH {scenario} {} {} {} {} {:.3}",
            phase.trace.updates,
            phase.trace.sets,
            phase.trace.ffi_calls,
            phase.trace.ffi_batches,
            (phase.script_ms + phase.layout_cb_ms) * 1000.0
        );
    }
    let registry = std::sync::Arc::unwrap_or_clone(gallery_host().registry);
    let bytecode = super::compile_with_registry(&source, "app.js", true, registry)
        .expect("compile Gallery to IQJS");
    println!("IQJS {}", bytecode.as_bytes().len());
}
