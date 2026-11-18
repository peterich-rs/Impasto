use std::cell::{Cell, RefCell};
use std::rc::Rc;

use reactive::{CallResult, Eval, Graph};

type Cleanup = Rc<dyn Fn(&Graph<RustEval>)>;
type Func = Rc<dyn Fn(&Graph<RustEval>, Option<i32>) -> CallResult<i32, Cleanup>>;

#[derive(Clone, Copy)]
struct RustEval;

thread_local! {
    static FREED: Cell<Vec<i32>> = const { Cell::new(Vec::new()) };
    static EFFECT_CALLS: Cell<u32> = const { Cell::new(0) };
}

impl Eval for RustEval {
    type Value = i32;
    type Func = Func;
    type Cleanup = Cleanup;

    fn dup(self, value: &i32) -> i32 {
        *value
    }

    fn free(self, value: i32) {
        FREED.with(|freed| {
            let mut values = freed.take();
            values.push(value);
            freed.set(values);
        });
    }

    fn call(self, graph: &Graph<Self>, func: &Func, prev: Option<i32>) -> CallResult<i32, Cleanup> {
        func(graph, prev)
    }

    fn effect(
        self,
        graph: &Graph<Self>,
        func: &Func,
        prev: Option<i32>,
    ) -> CallResult<i32, Cleanup> {
        EFFECT_CALLS.with(|calls| calls.set(calls.get() + 1));
        self.call(graph, func, prev)
    }

    fn same(self, a: &i32, b: &i32) -> bool {
        a == b
    }

    fn run_cleanup(self, graph: &Graph<Self>, cleanup: Cleanup) {
        cleanup(graph);
    }
}

fn effect(run: impl Fn(&Graph<RustEval>) -> Option<Cleanup> + 'static) -> Func {
    Rc::new(move |graph, prev| {
        if let Some(prev) = prev {
            RustEval.free(prev);
        }
        CallResult::Value {
            value: None,
            cleanup: run(graph),
        }
    })
}

fn compute(run: impl Fn(&Graph<RustEval>) -> i32 + 'static) -> Func {
    Rc::new(move |graph, prev| {
        if let Some(prev) = prev {
            RustEval.free(prev);
        }
        CallResult::Value {
            value: Some(run(graph)),
            cleanup: None,
        }
    })
}

#[test]
fn paint_effect_waits_for_paint_flush() {
    let graph = Graph::<RustEval>::new();
    let calls = Rc::new(Cell::new(0));
    let count = Rc::clone(&calls);
    let paint = graph.create_paint_effect(effect(move |graph| {
        assert!(graph.is_painting());
        count.set(count.get() + 1);
        None
    }));
    assert!(graph.has_paints());
    assert!(!graph.has_updates());
    assert!(!graph.is_painting());
    assert_eq!(graph.get(RustEval, paint), None);
    graph.run_updates(RustEval);
    assert_eq!(calls.get(), 0);
    assert_eq!(graph.take_update_count(), 0);
    graph.run_paints(RustEval);
    graph.run_paints(RustEval);
    assert_eq!(calls.get(), 1);
    assert_eq!(EFFECT_CALLS.with(Cell::get), 1);
    assert!(!graph.has_paints());
    assert!(!graph.has_updates());
    assert!(!graph.is_painting());
    assert_eq!(graph.take_update_count(), 0);
}

#[test]
fn signal_changes_only_schedule_paints() {
    let graph = Graph::<RustEval>::new();
    let source = graph.create_signal(1);
    let values = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&values);
    graph.create_paint_effect(effect(move |graph| {
        seen.borrow_mut().push(graph.get(RustEval, source));
        None
    }));
    graph.run_paints(RustEval);
    assert_eq!(*values.borrow(), [Some(1)]);
    assert!(graph.set(RustEval, source, 2));
    assert!(graph.has_paints());
    assert!(!graph.has_updates());
    graph.run_updates(RustEval);
    assert_eq!(*values.borrow(), [Some(1)]);
    graph.run_paints(RustEval);
    assert_eq!(*values.borrow(), [Some(1), Some(2)]);
    assert!(!graph.has_paints());
}

#[test]
fn paint_owner_runs_before_children_and_disposes_them_on_rerun() {
    let graph = Graph::<RustEval>::new();
    let source = graph.create_signal(1);
    let log = Rc::new(RefCell::new(Vec::new()));
    let runs = Rc::clone(&log);
    let parent = graph.create_paint_effect(effect(move |graph| {
        graph.get(RustEval, source);
        runs.borrow_mut().push("parent");
        let owner = graph.owner();
        let child_runs = Rc::clone(&runs);
        graph.create_paint_effect(effect(move |graph| {
            assert_eq!(graph.owner().and_then(|id| graph.owner_parent(id)), owner);
            graph.get(RustEval, source);
            child_runs.borrow_mut().push("child");
            let cleanup = Rc::clone(&child_runs);
            Some(Rc::new(move |_| cleanup.borrow_mut().push("child cleanup")))
        }));
        None
    }));
    graph.run_paints(RustEval);
    assert_eq!(*log.borrow(), ["parent", "child"]);
    assert!(!graph.has_paints());
    graph.set(RustEval, source, 2);
    graph.run_paints(RustEval);
    assert_eq!(
        *log.borrow(),
        ["parent", "child", "child cleanup", "parent", "child"]
    );
    graph.dispose(RustEval, parent);
    assert_eq!(log.borrow().last(), Some(&"child cleanup"));
    assert!(!graph.has_paints());
    assert!(!graph.has_updates());
}

#[test]
fn paint_flush_leaves_ordinary_effects_pending() {
    let graph = Graph::<RustEval>::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    let tracked = Rc::clone(&log);
    graph.create_tracked_effect(effect(move |_| {
        tracked.borrow_mut().push("tracked");
        None
    }));
    let render_compute = Rc::clone(&log);
    let render_effect = Rc::clone(&log);
    graph.render_effect(
        RustEval,
        compute(move |_| {
            render_compute.borrow_mut().push("compute");
            1
        }),
        effect(move |_| {
            render_effect.borrow_mut().push("render");
            None
        }),
        true,
    );
    let paint = Rc::clone(&log);
    graph.create_paint_effect(effect(move |_| {
        paint.borrow_mut().push("paint");
        None
    }));
    graph.run_paints(RustEval);
    assert_eq!(*log.borrow(), ["paint"]);
    assert!(graph.has_updates());
    assert!(!graph.has_paints());
    graph.run_updates(RustEval);
    assert_eq!(*log.borrow(), ["paint", "compute", "render", "tracked"]);
    assert!(!graph.has_updates());
}

#[test]
fn paint_writes_are_rejected_and_freed() {
    let graph = Graph::<RustEval>::new();
    let source = graph.create_signal(1);
    assert!(!graph.set(RustEval, source, 1));
    assert!(!graph.take_paint_blocked());
    assert_eq!(FREED.with(Cell::take), [1]);
    graph.create_paint_effect(effect(move |graph| {
        assert!(graph.is_painting());
        assert!(!graph.set(RustEval, source, 2));
        assert!(!graph.set(RustEval, source, 1));
        assert_eq!(graph.get(RustEval, source), Some(1));
        None
    }));
    graph.run_paints(RustEval);
    assert_eq!(FREED.with(Cell::take), [2, 1]);
    assert_eq!(graph.get(RustEval, source), Some(1));
    assert!(!graph.has_paints());
    assert!(!graph.has_updates());
    assert!(graph.take_paint_blocked());
    assert!(!graph.take_paint_blocked());
    assert!(graph.set(RustEval, source, 3));
    assert_eq!(graph.get(RustEval, source), Some(3));
    assert!(!graph.take_paint_blocked());
}

#[test]
fn paint_flush_defers_new_ordinary_computations() {
    let graph = Graph::<RustEval>::new();
    let calls = Rc::new(Cell::new(0));
    let count = Rc::clone(&calls);
    graph.create_paint_effect(effect(move |graph| {
        assert!(graph.is_painting());
        let tracked = Rc::clone(&count);
        graph.create_tracked_effect(effect(move |_| {
            tracked.set(tracked.get() + 1);
            None
        }));
        let render_compute = Rc::clone(&count);
        let render_effect = Rc::clone(&count);
        graph.create_render_effect(
            RustEval,
            compute(move |_| {
                render_compute.set(render_compute.get() + 1);
                1
            }),
            effect(move |_| {
                render_effect.set(render_effect.get() + 1);
                None
            }),
        );
        graph.run_updates(RustEval);
        graph.batch(RustEval, |_| {});
        graph.run_paints(RustEval);
        assert!(graph.is_painting());
        assert_eq!(count.get(), 0);
        None
    }));
    graph.run_paints(RustEval);
    assert_eq!(calls.get(), 0);
    assert!(graph.has_updates());
    assert!(!graph.has_paints());
    assert!(!graph.is_painting());
    graph.run_updates(RustEval);
    assert_eq!(calls.get(), 3);
    assert!(!graph.has_updates());
}

#[test]
fn paint_reads_refresh_memos_and_respect_equality() {
    let graph = Graph::<RustEval>::new();
    let source = graph.create_signal(1);
    let calls = Rc::new(Cell::new(0));
    let count = Rc::clone(&calls);
    let parity = graph.create_memo(compute(move |graph| {
        count.set(count.get() + 1);
        graph.get(RustEval, source).unwrap_or(0) % 2
    }));
    let values = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&values);
    graph.create_paint_effect(effect(move |graph| {
        seen.borrow_mut().push(graph.get(RustEval, parity));
        None
    }));
    graph.run_paints(RustEval);
    assert_eq!(*values.borrow(), [Some(1)]);
    assert_eq!(calls.get(), 1);
    graph.set(RustEval, source, 3);
    assert!(graph.has_paints());
    assert!(!graph.has_updates());
    graph.run_updates(RustEval);
    assert_eq!(calls.get(), 1);
    graph.run_paints(RustEval);
    assert_eq!(calls.get(), 2);
    assert_eq!(*values.borrow(), [Some(1)]);
    assert!(!graph.has_paints());
    graph.set(RustEval, source, 4);
    graph.run_paints(RustEval);
    assert_eq!(*values.borrow(), [Some(1), Some(0)]);
    assert!(!graph.has_paints());
}

#[test]
fn owner_disposal_drops_paint_effects_and_clears_dependencies() {
    let graph = Graph::<RustEval>::new();
    let source = graph.create_signal(1);
    let log = Rc::new(RefCell::new(Vec::new()));
    let root = graph.open_root();
    let held = Rc::new(());
    let released = Rc::downgrade(&held);
    let cleanups = Rc::clone(&log);
    let paint = graph.run_with_owner(Some(root), |graph| {
        graph.create_paint_effect(effect(move |graph| {
            let _held = &held;
            graph.get(RustEval, source);
            let registered = Rc::clone(&cleanups);
            graph.on_cleanup(Rc::new(move |_| registered.borrow_mut().push("registered")));
            let child = Rc::clone(&cleanups);
            graph.create_root(RustEval, |graph| {
                graph.on_cleanup(Rc::new(move |_| child.borrow_mut().push("child")));
            });
            let returned = Rc::clone(&cleanups);
            Some(Rc::new(move |_| returned.borrow_mut().push("returned")))
        }))
    });
    assert_eq!(graph.owner_parent(paint), Some(root));
    graph.run_paints(RustEval);
    graph.set(RustEval, source, 2);
    assert!(graph.has_paints());
    graph.dispose(RustEval, root);
    assert_eq!(*log.borrow(), ["child", "registered", "returned"]);
    assert!(released.upgrade().is_none());
    assert!(!graph.is_live(paint));
    assert!(!graph.has_paints());
    assert!(graph.set(RustEval, source, 3));
    assert!(!graph.has_paints());
    graph.run_paints(RustEval);
    assert_eq!(*log.borrow(), ["child", "registered", "returned"]);
}

#[test]
fn disposing_unrun_paint_resets_the_reused_slot_phase() {
    let graph = Graph::<RustEval>::new();
    let paint = graph.create_paint_effect(effect(|_| panic!("disposed paint must not run")));
    let cleaned = Rc::new(Cell::new(false));
    let cleanup = Rc::clone(&cleaned);
    graph.run_with_owner(Some(paint), |graph| {
        graph.on_cleanup(Rc::new(move |_| cleanup.set(true)));
    });
    graph.dispose(RustEval, paint);
    assert!(cleaned.get());
    assert!(!graph.is_live(paint));
    assert!(!graph.has_paints());
    let calls = Rc::new(Cell::new(0));
    let count = Rc::clone(&calls);
    graph.create_tracked_effect(effect(move |_| {
        count.set(count.get() + 1);
        None
    }));
    assert!(graph.has_updates());
    assert!(!graph.has_paints());
    graph.run_paints(RustEval);
    assert_eq!(calls.get(), 0);
    graph.run_updates(RustEval);
    assert_eq!(calls.get(), 1);
}

#[test]
fn paint_rerun_cleanup_cannot_write_signals() {
    let graph = Graph::<RustEval>::new();
    let source = graph.create_signal(1);
    let target = graph.create_signal(10);
    graph.create_paint_effect(effect(move |graph| {
        graph.get(RustEval, source);
        Some(Rc::new(move |graph| {
            assert!(graph.is_painting());
            assert!(!graph.set(RustEval, target, 20));
        }))
    }));
    graph.run_paints(RustEval);
    assert!(!graph.take_paint_blocked());
    graph.set(RustEval, source, 2);
    FREED.with(Cell::take);
    graph.run_paints(RustEval);
    assert_eq!(graph.get(RustEval, target), Some(10));
    assert_eq!(FREED.with(Cell::take), [20]);
    assert!(graph.take_paint_blocked());
    assert!(!graph.take_paint_blocked());
}
