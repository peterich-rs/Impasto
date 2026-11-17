//! Render-thread reactive arena. Values stay in the embedding runtime; edges
//! are paired indices. No arena borrow survives a call into user code.
use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::rc::Rc;

pub const NONE: u32 = u32::MAX;
// Raw identities must stay exact in a JavaScript Number. Exhausted slots retire.
const MAX_GENERATION: u32 = (1 << 21) - 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Signal,
    Memo,
    RenderEffect,
    TrackedEffect,
    Root,
    Free,
}

#[derive(Clone, Copy, Debug)]
pub struct Edge {
    pub node: u32,
    pub index: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Id {
    index: u32,
    generation: u32,
}
impl Id {
    pub fn to_raw(self) -> u64 {
        (u64::from(self.generation) << 32) | u64::from(self.index)
    }
    pub fn from_raw(raw: u64) -> Self {
        let [a, b, c, d, e, f, g, h] = raw.to_le_bytes();
        Self {
            index: u32::from_le_bytes([a, b, c, d]),
            generation: u32::from_le_bytes([e, f, g, h]),
        }
    }
}

pub enum CallResult<V, C> {
    Value {
        value: Option<V>,
        cleanup: Option<C>,
    },
    Failed,
}

/// `prev` is owned by the call. Functions and cleanup registrations are owned
/// by the arena. Implementations can re-enter `graph`, including from cleanup.
pub trait Eval: Copy + 'static {
    type Value;
    type Func;
    type Cleanup;
    fn dup(self, value: &Self::Value) -> Self::Value;
    fn free(self, value: Self::Value);
    fn call(
        self,
        graph: &Graph<Self>,
        func: &Self::Func,
        prev: Option<Self::Value>,
    ) -> CallResult<Self::Value, Self::Cleanup>;
    fn effect(
        self,
        graph: &Graph<Self>,
        func: &Self::Func,
        prev: Option<Self::Value>,
    ) -> CallResult<Self::Value, Self::Cleanup> {
        self.call(graph, func, prev)
    }
    fn same(self, a: &Self::Value, b: &Self::Value) -> bool;
    fn run_cleanup(self, graph: &Graph<Self>, cleanup: Self::Cleanup);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Clean,
    Check,
    Dirty,
}

pub struct Node<V, F, C> {
    pub kind: Kind,
    pub generation: u32,
    pub value: Option<V>,
    pub func: Option<F>,
    pub effect: Option<F>,
    pub owner: u32,
    pub sources: Vec<Edge>,
    pub observers: Vec<Edge>,
    pub cleanups: Vec<C>,
    children: Vec<Id>,
    state: State,
    running: bool,
    effect_pending: bool,
    effect_cleanup: Option<C>,
}
struct Arena<E: Eval> {
    nodes: Vec<Node<E::Value, E::Func, E::Cleanup>>,
    free: Vec<u32>,
}

pub struct Graph<E: Eval> {
    arena: RefCell<Arena<E>>,
    owner: Cell<Option<Id>>,
    reader: Cell<Option<Id>>,
    depth: Cell<u32>,
    batch_depth: Cell<u32>,
    updates: Cell<u32>,
    // Both the graph and the embedding's values are confined to the render thread.
    thread: PhantomData<Rc<()>>,
}
impl<E: Eval> Default for Graph<E> {
    fn default() -> Self {
        Self::new()
    }
}
impl<E: Eval> Graph<E> {
    pub fn new() -> Self {
        Self {
            arena: RefCell::new(Arena {
                nodes: Vec::new(),
                free: Vec::new(),
            }),
            owner: Cell::new(None),
            reader: Cell::new(None),
            depth: Cell::new(0),
            batch_depth: Cell::new(0),
            updates: Cell::new(0),
            thread: PhantomData,
        }
    }
    pub fn node_count(&self) -> u64 {
        self.arena
            .borrow()
            .nodes
            .iter()
            .filter(|n| n.kind != Kind::Free)
            .count() as u64
    }
    pub fn owner(&self) -> Option<Id> {
        self.owner.get().filter(|id| self.live(*id))
    }
    pub fn owner_parent(&self, id: Id) -> Option<Id> {
        let arena = self.arena.borrow();
        let node = arena.nodes.get(id.index as usize)?;
        if node.generation != id.generation || node.kind == Kind::Free {
            return None;
        }
        arena.id(node.owner)
    }
    pub fn is_live(&self, id: Id) -> bool {
        self.live(id)
    }
    pub fn push_owner(&self, owner: Option<Id>) -> Option<Id> {
        self.owner.replace(owner.filter(|id| self.live(*id)))
    }
    pub fn open_root(&self) -> Id {
        self.alloc(Kind::Root)
    }
    pub fn create_root(&self, _eval: E, scope: impl FnOnce(&Self)) -> Id {
        let id = self.open_root();
        self.run_with_owner(Some(id), scope);
        id
    }
    pub fn run_with_owner<T>(&self, owner: Option<Id>, scope: impl FnOnce(&Self) -> T) -> T {
        let previous = self.push_owner(owner);
        let reader = self.push_untrack();
        let _restore = Scope {
            graph: self,
            owner: previous,
            reader,
        };
        scope(self)
    }
    pub fn push_untrack(&self) -> Option<Id> {
        self.reader.replace(None)
    }
    pub fn pop_reader(&self, reader: Option<Id>) {
        self.reader.set(reader.filter(|id| self.live(*id)));
    }
    pub fn untrack<T>(&self, read: impl FnOnce(&Self) -> T) -> T {
        let reader = self.push_untrack();
        let _restore = Scope {
            graph: self,
            owner: self.owner.get(),
            reader,
        };
        read(self)
    }
    pub fn on_cleanup(&self, cleanup: E::Cleanup) {
        if let Some(owner) = self.owner() {
            self.arena.borrow_mut().nodes[owner.index as usize]
                .cleanups
                .push(cleanup);
        }
    }
    pub fn create_signal(&self, value: E::Value) -> Id {
        let id = self.alloc(Kind::Signal);
        self.arena.borrow_mut().nodes[id.index as usize].value = Some(value);
        id
    }
    pub fn create_memo(&self, func: E::Func) -> Id {
        let id = self.alloc(Kind::Memo);
        let mut arena = self.arena.borrow_mut();
        let node = &mut arena.nodes[id.index as usize];
        node.func = Some(func);
        node.state = State::Dirty;
        id
    }
    pub fn create_render_effect(&self, eval: E, compute: E::Func, effect: E::Func) {
        self.render_effect(eval, compute, effect, false);
    }
    pub fn render_effect(&self, eval: E, compute: E::Func, effect: E::Func, scheduled: bool) {
        let id = self.alloc(Kind::RenderEffect);
        {
            let mut arena = self.arena.borrow_mut();
            let node = &mut arena.nodes[id.index as usize];
            node.func = Some(compute);
            node.effect = Some(effect);
            node.state = State::Dirty;
        }
        if !scheduled {
            self.update(eval, id);
            self.effect(eval, id);
        }
    }
    pub fn create_tracked_effect(&self, func: E::Func) {
        let id = self.alloc(Kind::TrackedEffect);
        let mut arena = self.arena.borrow_mut();
        let node = &mut arena.nodes[id.index as usize];
        node.func = Some(func);
        node.state = State::Dirty;
    }
    pub fn get(&self, eval: E, id: Id) -> Option<E::Value> {
        if !self.live(id) {
            return None;
        }
        self.update(eval, id);
        if !self.live(id) {
            return None;
        }
        if let Some(reader) = self.reader.get().filter(|r| self.live(*r) && *r != id) {
            self.arena.borrow_mut().subscribe(id.index, reader.index);
        }
        self.arena.borrow().nodes[id.index as usize]
            .value
            .as_ref()
            .map(|v| eval.dup(v))
    }
    /// A setter only invalidates. Shell owns the two frame flush points.
    pub fn set(&self, eval: E, id: Id, next: E::Value) -> bool {
        if !self.live(id) {
            eval.free(next);
            return false;
        }
        let changed = {
            let arena = self.arena.borrow();
            let node = &arena.nodes[id.index as usize];
            node.kind == Kind::Signal && node.value.as_ref().is_none_or(|v| !eval.same(v, &next))
        };
        if !changed {
            eval.free(next);
            return false;
        }
        let previous = self.arena.borrow_mut().nodes[id.index as usize]
            .value
            .replace(next);
        if let Some(previous) = previous {
            eval.free(previous);
        }
        self.arena.borrow_mut().invalidate(id.index, State::Dirty);
        true
    }
    pub fn push_batch(&self) {
        self.batch_depth.set(self.batch_depth.get() + 1);
    }
    pub fn pop_batch(&self, eval: E) {
        self.batch_depth
            .set(self.batch_depth.get().saturating_sub(1));
        if self.batch_depth.get() == 0 {
            self.run_updates(eval);
        }
    }
    pub fn batch(&self, eval: E, scope: impl FnOnce(&Self)) {
        self.push_batch();
        scope(self);
        self.pop_batch(eval);
    }
    pub fn has_updates(&self) -> bool {
        self.arena.borrow().nodes.iter().any(|n| {
            matches!(n.kind, Kind::RenderEffect | Kind::TrackedEffect)
                && (n.state != State::Clean || n.effect_pending)
        })
    }
    pub fn take_update_count(&self) -> u32 {
        self.updates.replace(0)
    }
    pub fn run_updates(&self, eval: E) {
        if self.depth.get() > 0 || self.batch_depth.get() > 0 {
            return;
        }
        if !self.has_updates() {
            return;
        }
        self.updates.set(self.updates.get().saturating_add(1));
        self.depth.set(1);
        let _depth = Depth(&self.depth);
        while self.has_updates() {
            // Newly created render work preempts tracked work.
            for id in self.ids(Kind::RenderEffect) {
                self.update(eval, id);
            }
            for id in self.ids(Kind::RenderEffect) {
                self.effect(eval, id);
            }
            if self
                .ids(Kind::RenderEffect)
                .iter()
                .any(|id| self.dirty(*id))
            {
                continue;
            }
            for id in self.ids(Kind::TrackedEffect) {
                self.update(eval, id);
                if self
                    .ids(Kind::RenderEffect)
                    .iter()
                    .any(|id| self.dirty(*id))
                {
                    break;
                }
            }
        }
    }
    pub fn dispose(&self, eval: E, id: Id) {
        if !self.live(id) {
            return;
        }
        // Remove edges before user cleanup can re-enter, then retire this generation.
        let (children, cleanups, cleanup, value, func, effect) = {
            let mut arena = self.arena.borrow_mut();
            arena.clear_sources(id.index);
            arena.clear_observers(id.index);
            let node = &mut arena.nodes[id.index as usize];
            let owner = node.owner;
            let parts = (
                std::mem::take(&mut node.children),
                std::mem::take(&mut node.cleanups),
                node.effect_cleanup.take(),
                node.value.take(),
                node.func.take(),
                node.effect.take(),
            );
            node.kind = Kind::Free;
            node.state = State::Clean;
            node.effect_pending = false;
            node.generation += 1;
            if let Some(owner) = arena.nodes.get_mut(owner as usize) {
                owner.children.retain(|child| *child != id);
            }
            parts
        };
        for child in children.into_iter().rev() {
            self.dispose(eval, child);
        }
        for cleanup in cleanups.into_iter().rev() {
            eval.run_cleanup(self, cleanup);
        }
        if let Some(cleanup) = cleanup {
            eval.run_cleanup(self, cleanup);
        }
        if let Some(value) = value {
            eval.free(value);
        }
        drop((func, effect));
        if id.generation < MAX_GENERATION {
            self.arena.borrow_mut().free.push(id.index);
        }
    }
    pub fn clear(&self, eval: E) {
        let ids: Vec<_> = {
            let arena = self.arena.borrow();
            (0..arena.nodes.len())
                .filter_map(|i| arena.id(arena_index(i)))
                .collect()
        };
        for id in ids {
            self.dispose(eval, id);
        }
    }
    fn update(&self, eval: E, id: Id) {
        if !self.live(id) {
            return;
        }
        let (state, running, sources) = {
            let arena = self.arena.borrow();
            let node = &arena.nodes[id.index as usize];
            (
                node.state,
                node.running,
                node.sources
                    .iter()
                    .filter_map(|e| arena.id(e.node))
                    .collect::<Vec<_>>(),
            )
        };
        if state == State::Clean || running {
            return;
        }
        if state == State::Check {
            for source in sources {
                self.update(eval, source);
            }
            let mut arena = self.arena.borrow_mut();
            if !arena.live(id) {
                return;
            }
            let node = &mut arena.nodes[id.index as usize];
            if node.state == State::Check {
                node.state = State::Clean;
                return;
            }
        }
        let (func, prev, kind, children, cleanups, cleanup) = {
            let mut arena = self.arena.borrow_mut();
            let node = &mut arena.nodes[id.index as usize];
            node.state = State::Clean;
            node.running = true;
            let result = (
                node.func.take(),
                node.value.take(),
                node.kind,
                std::mem::take(&mut node.children),
                std::mem::take(&mut node.cleanups),
                node.effect_cleanup.take(),
            );
            arena.clear_sources(id.index);
            result
        };
        for child in children.into_iter().rev() {
            self.dispose(eval, child);
        }
        for cleanup in cleanups.into_iter().rev() {
            eval.run_cleanup(self, cleanup);
        }
        if let Some(cleanup) = cleanup {
            eval.run_cleanup(self, cleanup);
        }
        let Some(func) = func else {
            return;
        };
        let owner = self.owner.replace(Some(id));
        let reader = self.reader.replace(Some(id));
        let argument = prev.as_ref().map(|v| eval.dup(v));
        let result = if kind == Kind::TrackedEffect {
            eval.effect(self, &func, argument)
        } else {
            eval.call(self, &func, argument)
        };
        self.owner.set(owner);
        self.reader.set(reader);
        if !self.live(id) {
            if let Some(prev) = prev {
                eval.free(prev);
            }
            if let CallResult::Value { value, cleanup } = result {
                if let Some(value) = value {
                    eval.free(value);
                }
                if let Some(cleanup) = cleanup {
                    eval.run_cleanup(self, cleanup);
                }
            }
            return;
        }
        let mut next = prev;
        let mut returned_cleanup = None;
        let mut changed = false;
        if let CallResult::Value { value, cleanup } = result {
            returned_cleanup = cleanup;
            if let Some(value) = value {
                changed = next.as_ref().is_none_or(|v| !eval.same(v, &value));
                if let Some(previous) = next.replace(value) {
                    eval.free(previous);
                }
            }
        }
        let mut arena = self.arena.borrow_mut();
        let node = &mut arena.nodes[id.index as usize];
        node.func = Some(func);
        node.value = next;
        node.running = false;
        if kind == Kind::TrackedEffect {
            node.effect_cleanup = returned_cleanup;
        }
        if kind == Kind::RenderEffect && changed {
            node.effect_pending = true;
        }
        if kind == Kind::Memo && changed {
            arena.invalidate(id.index, State::Dirty);
        }
    }
    fn effect(&self, eval: E, id: Id) {
        if !self.live(id) {
            return;
        }
        let (func, value, cleanup) = {
            let mut arena = self.arena.borrow_mut();
            let node = &mut arena.nodes[id.index as usize];
            if !node.effect_pending {
                return;
            }
            node.effect_pending = false;
            (
                node.effect.take(),
                node.value.as_ref().map(|v| eval.dup(v)),
                node.effect_cleanup.take(),
            )
        };
        if let Some(cleanup) = cleanup {
            eval.run_cleanup(self, cleanup);
        }
        let Some(func) = func else {
            return;
        };
        let result = self.run_with_owner(Some(id), |graph| eval.effect(graph, &func, value));
        if let CallResult::Value { value, cleanup } = result {
            if let Some(value) = value {
                eval.free(value);
            }
            if self.live(id) {
                let mut arena = self.arena.borrow_mut();
                let node = &mut arena.nodes[id.index as usize];
                node.effect = Some(func);
                node.effect_cleanup = cleanup;
            } else if let Some(cleanup) = cleanup {
                eval.run_cleanup(self, cleanup);
            }
        }
    }
    fn ids(&self, kind: Kind) -> Vec<Id> {
        let arena = self.arena.borrow();
        arena
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.kind == kind)
            .map(|(i, n)| Id {
                index: arena_index(i),
                generation: n.generation,
            })
            .collect()
    }
    fn dirty(&self, id: Id) -> bool {
        let arena = self.arena.borrow();
        arena.live(id)
            && (arena.nodes[id.index as usize].state != State::Clean
                || arena.nodes[id.index as usize].effect_pending)
    }
    fn live(&self, id: Id) -> bool {
        self.arena.borrow().live(id)
    }
    fn alloc(&self, kind: Kind) -> Id {
        let mut arena = self.arena.borrow_mut();
        let index = arena.free.pop().unwrap_or_else(|| {
            let index = arena_index(arena.nodes.len());
            arena.nodes.push(Node {
                kind: Kind::Free,
                generation: 1,
                value: None,
                func: None,
                effect: None,
                owner: NONE,
                sources: vec![],
                observers: vec![],
                cleanups: vec![],
                children: vec![],
                state: State::Clean,
                running: false,
                effect_pending: false,
                effect_cleanup: None,
            });
            index
        });
        let owner = self.owner.get().filter(|id| arena.live(*id));
        let node = &mut arena.nodes[index as usize];
        node.kind = kind;
        node.owner = owner.map_or(NONE, |id| id.index);
        node.running = false;
        node.state = State::Clean;
        let id = Id {
            index,
            generation: node.generation,
        };
        if let Some(owner) = owner {
            arena.nodes[owner.index as usize].children.push(id);
        }
        id
    }
}
impl<E: Eval> Arena<E> {
    fn live(&self, id: Id) -> bool {
        self.nodes
            .get(id.index as usize)
            .is_some_and(|n| n.kind != Kind::Free && n.generation == id.generation)
    }
    fn id(&self, index: u32) -> Option<Id> {
        self.nodes
            .get(index as usize)
            .filter(|n| n.kind != Kind::Free)
            .map(|n| Id {
                index,
                generation: n.generation,
            })
    }
    fn subscribe(&mut self, source: u32, reader: u32) {
        if self.nodes[reader as usize]
            .sources
            .iter()
            .any(|e| e.node == source)
        {
            return;
        }
        let oi = arena_index(self.nodes[source as usize].observers.len());
        let si = arena_index(self.nodes[reader as usize].sources.len());
        self.nodes[source as usize].observers.push(Edge {
            node: reader,
            index: si,
        });
        self.nodes[reader as usize].sources.push(Edge {
            node: source,
            index: oi,
        });
    }
    fn invalidate(&mut self, source: u32, state: State) {
        let observers: Vec<_> = self.nodes[source as usize]
            .observers
            .iter()
            .map(|e| e.node)
            .collect();
        for index in observers {
            let node = &mut self.nodes[index as usize];
            let old = node.state;
            if old == State::Dirty || (old == State::Check && state == State::Check) {
                continue;
            }
            node.state = state;
            if old == State::Clean {
                self.invalidate(index, State::Check);
            }
        }
    }
    fn clear_sources(&mut self, index: u32) {
        while let Some(edge) = self.nodes[index as usize].sources.pop() {
            let observers = &mut self.nodes[edge.node as usize].observers;
            observers.swap_remove(edge.index as usize);
            if let Some(moved) = observers.get(edge.index as usize).copied() {
                self.nodes[moved.node as usize].sources[moved.index as usize].index = edge.index;
            }
        }
    }
    fn clear_observers(&mut self, index: u32) {
        while let Some(edge) = self.nodes[index as usize].observers.pop() {
            let sources = &mut self.nodes[edge.node as usize].sources;
            sources.swap_remove(edge.index as usize);
            if let Some(moved) = sources.get(edge.index as usize).copied() {
                self.nodes[moved.node as usize].observers[moved.index as usize].index = edge.index;
            }
        }
    }
}
struct Scope<'a, E: Eval> {
    graph: &'a Graph<E>,
    owner: Option<Id>,
    reader: Option<Id>,
}
impl<E: Eval> Drop for Scope<'_, E> {
    fn drop(&mut self) {
        self.graph.owner.set(self.owner);
        self.graph.reader.set(self.reader);
    }
}
struct Depth<'a>(&'a Cell<u32>);
impl Drop for Depth<'_> {
    fn drop(&mut self) {
        self.0.set(0);
    }
}

fn arena_index(value: usize) -> u32 {
    match u32::try_from(value) {
        Ok(index) => index,
        Err(_) => panic!("reactive arena exceeded its u32 address space"),
    }
}
