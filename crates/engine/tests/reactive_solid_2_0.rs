//! Observable behavior of the synchronous Solid 2.0.0 APIs Impasto supports.
//!
//! The fixture is a small Node script, `fixtures/order.mjs`, run once against
//! the installed `solid-js`. It prints the order of memo, render-effect, and
//! tracked-effect updates. This file asserts the same order. It does not copy
//! Solid's source.

use std::cell::Cell;
use std::rc::Rc;

use engine::reactive::{CallResult, Eval, Graph, Id};

type GraphRun = Rc<dyn Fn(&Graph<RustEval>) -> i32>;
type Cleanup = Rc<dyn Fn()>;
type EffectRun = Rc<dyn Fn(i32) -> Option<Cleanup>>;
type TrackedRun = Rc<dyn Fn(&Graph<RustEval>) -> Option<Cleanup>>;

enum Func {
    Memo(GraphRun),
    /// Render effect, first phase. Subscribes and returns the value the effect
    /// phase receives.
    Compute(GraphRun),
    /// Render effect, second phase. Untracked. Receives the compute value.
    Effect(EffectRun),
    /// Tracked effect. Subscribes and returns its own cleanup.
    Tracked(TrackedRun),
}

#[derive(Clone, Copy)]
struct RustEval;

impl Eval for RustEval {
    type Value = i32;
    type Func = Func;
    type Cleanup = Rc<dyn Fn()>;

    fn dup(self, value: &i32) -> i32 {
        *value
    }

    fn free(self, _value: i32) {}

    fn call(
        self,
        graph: &Graph<Self>,
        func: &Func,
        prev: Option<i32>,
    ) -> CallResult<i32, Rc<dyn Fn()>> {
        match func {
            Func::Memo(run) => {
                let produced = run(graph);
                CallResult::Value {
                    value: Some(produced),
                    cleanup: None,
                }
            }
            Func::Compute(run) => {
                let produced = run(graph);
                CallResult::Value {
                    value: Some(produced),
                    cleanup: None,
                }
            }
            Func::Effect(run) => CallResult::Value {
                value: None,
                cleanup: run(prev.unwrap_or(0)),
            },
            Func::Tracked(run) => {
                let cleanup = run(graph);
                CallResult::Value {
                    value: None,
                    cleanup,
                }
            }
        }
    }

    fn same(self, a: &i32, b: &i32) -> bool {
        a == b
    }

    fn run_cleanup(self, _graph: &Graph<Self>, cleanup: Rc<dyn Fn()>) {
        cleanup();
    }
}

thread_local! {
    static LOG: Cell<Vec<String>> = const { Cell::new(Vec::new()) };
}

fn log(line: impl Into<String>) {
    LOG.with(|log| {
        let mut lines = log.take();
        lines.push(line.into());
        log.set(lines);
    });
}

fn take_log() -> Vec<String> {
    LOG.with(|log| log.take())
}

#[test]
fn signal_read_and_write() {
    let graph = Graph::<RustEval>::new();
    let id = graph.create_signal(1);
    assert_eq!(graph.get(RustEval, id), Some(1));
    assert!(graph.set(RustEval, id, 2));
    assert_eq!(graph.get(RustEval, id), Some(2));
    assert!(!graph.set(RustEval, id, 2));
}

#[test]
fn owner_is_the_root_while_it_runs() {
    let graph = Graph::<RustEval>::new();
    let mut seen = None;
    let root = graph.create_root(RustEval, |graph| {
        seen = graph.owner();
    });
    assert_eq!(seen, Some(root));
    assert_eq!(graph.owner(), None);
}

#[test]
fn cleanup_runs_on_dispose_latest_first() {
    let graph = Graph::<RustEval>::new();
    let root = graph.create_root(RustEval, |graph| {
        graph.on_cleanup(Rc::new(|| log("first")) as Rc<dyn Fn()>);
        graph.on_cleanup(Rc::new(|| log("second")) as Rc<dyn Fn()>);
    });
    assert!(take_log().is_empty());
    graph.dispose(RustEval, root);
    assert_eq!(take_log(), ["second", "first"]);
}

#[test]
fn child_cleanup_runs_before_the_owner_cleanup() {
    let graph = Graph::<RustEval>::new();
    let root = graph.create_root(RustEval, |graph| {
        graph.on_cleanup(Rc::new(|| log("owner")) as Rc<dyn Fn()>);
        let _child = graph.create_root(RustEval, |graph| {
            graph.on_cleanup(Rc::new(|| log("child")) as Rc<dyn Fn()>);
        });
    });
    graph.dispose(RustEval, root);
    assert_eq!(take_log(), ["child", "owner"]);
}

#[test]
fn run_with_owner_attaches_cleanup_to_that_owner() {
    let graph = Graph::<RustEval>::new();
    let root = graph.create_root(RustEval, |_| {});
    graph.run_with_owner(Some(root), |graph| {
        graph.on_cleanup(Rc::new(|| log("owned")) as Rc<dyn Fn()>);
    });
    graph.on_cleanup(Rc::new(|| log("outside")) as Rc<dyn Fn()>);
    graph.dispose(RustEval, root);
    assert_eq!(take_log(), ["owned"]);
}

#[test]
fn disposed_id_cannot_be_read() {
    let graph = Graph::<RustEval>::new();
    let root = graph.create_root(RustEval, |_| {});
    let id = {
        let mut created = Id::from_raw(0);
        graph.run_with_owner(Some(root), |graph| {
            created = graph.create_signal(1);
        });
        created
    };
    graph.dispose(RustEval, root);
    assert_eq!(graph.get(RustEval, id), None);
    assert!(!graph.set(RustEval, id, 9));
}

#[test]
fn untrack_reads_without_a_subscription() {
    let graph = Graph::<RustEval>::new();
    let source = graph.create_signal(1);
    let read = graph.untrack(|graph| graph.get(RustEval, source));
    assert_eq!(read, Some(1));
    assert!(graph.set(RustEval, source, 4));
    assert_eq!(graph.get(RustEval, source), Some(4));
}

fn memo(run: impl Fn(&Graph<RustEval>) -> i32 + 'static) -> Func {
    Func::Memo(Rc::new(run))
}

fn render(
    compute: impl Fn(&Graph<RustEval>) -> i32 + 'static,
    effect: impl Fn(i32) -> Option<Rc<dyn Fn()>> + 'static,
) -> (Func, Func) {
    (
        Func::Compute(Rc::new(compute)),
        Func::Effect(Rc::new(effect)),
    )
}

fn tracked(run: impl Fn(&Graph<RustEval>) -> Option<Rc<dyn Fn()>> + 'static) -> Func {
    Func::Tracked(Rc::new(run))
}

#[test]
fn memo_caches_until_its_source_changes() {
    let graph = Graph::<RustEval>::new();
    let source = graph.create_signal(1);
    let derived = graph.create_memo(memo(move |graph| {
        log("memo");
        graph.get(RustEval, source).unwrap_or(0) + 1
    }));
    assert_eq!(graph.get(RustEval, derived), Some(2));
    assert_eq!(graph.get(RustEval, derived), Some(2));
    assert_eq!(take_log(), ["memo"]);
    graph.set(RustEval, source, 4);
    assert_eq!(graph.get(RustEval, derived), Some(5));
    assert_eq!(take_log(), ["memo"]);
}

#[test]
fn render_effect_runs_before_tracked_effect() {
    let graph = Graph::<RustEval>::new();
    let source = graph.create_signal(1);
    graph.create_tracked_effect(tracked(move |graph| {
        log(format!(
            "tracked {}",
            graph.get(RustEval, source).unwrap_or(0)
        ));
        None
    }));
    // Compute subscribes. The effect phase sees the value and does not subscribe.
    let (compute, effect) = render(
        move |graph| graph.get(RustEval, source).unwrap_or(0),
        |value| {
            log(format!("render {value}"));
            None
        },
    );
    graph.create_render_effect(RustEval, compute, effect);
    // The render effect ran both phases during creation. The tracked effect waits.
    assert_eq!(take_log(), ["render 1"]);
    graph.run_updates(RustEval);
    assert_eq!(take_log(), ["tracked 1"]);
    // A write outside a flush updates on its own, render effect first.
    graph.set(RustEval, source, 2);
    assert!(take_log().is_empty());
    graph.run_updates(RustEval);
    assert_eq!(take_log(), ["render 2", "tracked 2"]);
}

#[test]
fn untrack_inside_a_memo_does_not_subscribe() {
    let graph = Graph::<RustEval>::new();
    let tracked = graph.create_signal(1);
    let ignored = graph.create_signal(10);
    let derived = graph.create_memo(memo(move |graph| {
        let extra = graph.untrack(|graph| graph.get(RustEval, ignored).unwrap_or(0));
        graph.get(RustEval, tracked).unwrap_or(0) + extra
    }));
    assert_eq!(graph.get(RustEval, derived), Some(11));
    graph.set(RustEval, ignored, 100);
    graph.run_updates(RustEval);
    assert_eq!(graph.get(RustEval, derived), Some(11));
    graph.set(RustEval, tracked, 2);
    assert_eq!(graph.get(RustEval, derived), Some(102));
}

#[test]
fn batch_notifies_once() {
    let graph = Graph::<RustEval>::new();
    let source = graph.create_signal(0);
    let (compute, effect) = render(
        move |graph| graph.get(RustEval, source).unwrap_or(0),
        |value| {
            log(format!("{value}"));
            None
        },
    );
    graph.create_render_effect(RustEval, compute, effect);
    take_log();
    graph.batch(RustEval, |_| {});
    // Nothing was written, so the effect does not rerun.
    assert!(take_log().is_empty());
    // Two writes inside one batch notify once, with the last value.
    graph.batch(RustEval, |graph| {
        graph.set(RustEval, source, 1);
        graph.set(RustEval, source, 2);
    });
    assert_eq!(take_log(), ["2"]);
}

#[test]
fn effect_cleanup_runs_before_the_next_run() {
    let graph = Graph::<RustEval>::new();
    let source = graph.create_signal(1);
    let (compute, effect) = render(
        move |graph| graph.get(RustEval, source).unwrap_or(0),
        |value| {
            log(format!("run {value}"));
            Some(Rc::new(move || log(format!("cleanup {value}"))) as Rc<dyn Fn()>)
        },
    );
    graph.create_render_effect(RustEval, compute, effect);
    assert_eq!(take_log(), ["run 1"]);
    graph.set(RustEval, source, 2);
    assert!(take_log().is_empty());
    graph.run_updates(RustEval);
    assert_eq!(take_log(), ["cleanup 1", "run 2"]);
}

#[test]
fn pinned_node_reference_matches_the_fixture() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if !root
        .join("packages/reactive/node_modules/solid-js")
        .exists()
    {
        eprintln!("run pnpm install to compare the saved fixture with Solid 2.0.0");
        return;
    }
    let output = std::process::Command::new("node")
        .args([
            "--conditions=browser",
            "crates/engine/tests/fixtures/order.mjs",
        ])
        .current_dir(root)
        .output()
        .expect("Node runs the pinned Solid reference");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        include_str!("fixtures/order.txt")
    );
}

#[test]
fn memo_equality_stops_downstream_notifications() {
    let graph = Graph::<RustEval>::new();
    let source = graph.create_signal(1);
    let parity = graph.create_memo(memo(move |graph| {
        graph.get(RustEval, source).unwrap_or(0) % 2
    }));
    let (compute, effect) = render(
        move |graph| graph.get(RustEval, parity).unwrap_or(0),
        |value| {
            log(value.to_string());
            None
        },
    );
    graph.create_render_effect(RustEval, compute, effect);
    take_log();
    graph.set(RustEval, source, 3);
    graph.run_updates(RustEval);
    assert!(take_log().is_empty());
}

#[test]
fn rerun_disposes_owned_children_and_registered_cleanup() {
    let graph = Graph::<RustEval>::new();
    let source = graph.create_signal(0);
    graph.create_tracked_effect(tracked(move |graph| {
        graph.get(RustEval, source);
        graph.create_root(RustEval, |graph| {
            graph.on_cleanup(Rc::new(|| log("child")) as Rc<dyn Fn()>)
        });
        graph.on_cleanup(Rc::new(|| log("cleanup")) as Rc<dyn Fn()>);
        None
    }));
    graph.run_updates(RustEval);
    graph.set(RustEval, source, 1);
    graph.run_updates(RustEval);
    assert_eq!(take_log(), ["child", "cleanup"]);
}
