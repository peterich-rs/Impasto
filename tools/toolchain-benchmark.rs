    #[test]
    #[ignore = "run explicitly to collect release frame samples"]
    fn toolchain_baseline_counts() {
        let started = std::time::Instant::now();
        let mut runtime = js::RuntimeBuilder::new().build().expect("empty runtime");
        println!("SAMPLE runtime-start {:.3}", started.elapsed().as_secs_f64() * 1_000_000.0);
        runtime
            .eval_script(
                r#"
                    globalThis.finished = 0;
                    for (let i = 0; i < 16; ++i) {
                        setTimeout(() => {
                            Promise.resolve().then(() => { finished += 1; });
                        }, 0);
                    }
                "#,
                "bench-timers.js",
            )
            .expect("schedule timer and promise fixtures");
        let started = std::time::Instant::now();
        let report = runtime.drive(100).expect("drive timer and promise fixtures");
        println!("SAMPLE timer-jobs {:.3}", started.elapsed().as_secs_f64() * 1_000_000.0);
        assert_eq!(report.timers_ran, 16);
        runtime
            .eval_script("if (finished !== 16) throw new Error('incomplete job fixture')", "check.js")
            .expect("verify promise checkpoint");
        drop(runtime);

        let started = std::time::Instant::now();
        let empty = open("", "empty.js", uicore::MonoShaper::default(), no_log())
            .expect("empty UI");
        println!("SAMPLE ui-start {:.3}", started.elapsed().as_secs_f64() * 1_000_000.0);
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
                let (x, y) = text_spot(&first, "⋯");
                click(session.as_mut(), x, y);
                let menu = session.frame(session::FrameTime::at(16.0));
                let (x, y) = text_spot(&menu, "市场");
                click(session.as_mut(), x, y);
                32.0
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
