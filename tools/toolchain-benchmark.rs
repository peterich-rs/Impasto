    #[test]
    #[ignore = "run explicitly to record toolchain-baseline.md"]
    fn toolchain_baseline_counts() {
        for scenario in ["click", "scroll"] {
            let Some(mut session) = open_gallery() else { return; };
            session.metrics(&phone());
            let first=pump_first(session.as_mut());
            if scenario=="click" {
                let (x,y)=text_spot(&first,"市场");click(session.as_mut(),x,y);
            } else {session.pointer(PointerEvent::wheel(180.0,300.0,16.0,0.0,48.0));}
            let frame=session.frame(session::FrameTime::at(16.0));
            assert!(session.take_script_error().is_none());
            assert!(session.take_faults().is_empty());
            let phase=frame.phases.expect("timed frame");
            println!("BENCH {scenario} {} {} {:.3}",phase.trace.updates,phase.trace.sets,(phase.script_ms+phase.layout_cb_ms)*1000.0);
        }
    }
