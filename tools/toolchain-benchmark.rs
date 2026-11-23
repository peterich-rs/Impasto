fn sample_gallery_frames() {
    let started = std::time::Instant::now();
    let empty = open("", "empty.js", crate::text::MonoShaper::default(), no_log()).expect("empty UI");
    println!(
        "SAMPLE ui-start {:.3}",
        started.elapsed().as_secs_f64() * 1_000_000.0
    );
    drop(empty);

    for scenario in ["click", "scroll"] {
        let started = std::time::Instant::now();
        let Some(mut session) = open_gallery() else {
            return;
        };
        let startup_us = started.elapsed().as_secs_f64() * 1_000_000.0;
        session.metrics(&phone());
        let first = pump_first(session.as_mut());
        let target = if scenario == "click" {
            let (navigation, next_frame) = if texts(&first).iter().any(|text| text == "市场") {
                (first, 16.0)
            } else {
                let (x, y) = text_spot(&first, "⋯");
                click(session.as_mut(), x, y);
                (session.frame(session::FrameTime::at(16.0)), 32.0)
            };
            let (x, y) = text_spot(&navigation, "市场");
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
            "BENCH {scenario} {} {} {:.3}",
            phase.trace.updates,
            phase.trace.sets,
            (phase.script_ms + phase.layout_cb_ms) * 1000.0
        );
    }
}
