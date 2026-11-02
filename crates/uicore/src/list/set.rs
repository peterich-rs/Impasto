//! Session-owned list and scroll drivers. Handles are an index plus a generation.

use crate::tree::{NodeId, Tree};

use super::controller::{DragStep, ParentLink, PullMode, ScrollController};
use super::diff::ListOp;
use super::driver::{Chrome, ListDriver, ListItem, ListNodes};

/// Pull threshold, rubber band, and content floor for one scroll driver.
#[derive(Clone, Copy, Debug)]
pub struct ScrollConfig {
    pub content_min: f32,
    pub pull_mode: PullMode,
    pub threshold: f32,
    pub rubber: bool,
    pub factor: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriverId {
    pub index: u32,
    pub generation: u32,
}

#[derive(Clone, Debug)]
pub enum DriverEvent {
    ListOp { id: DriverId, op: ListOp },
    ListChrome { id: DriverId, chrome: Chrome },
    EndReached { id: DriverId },
    Refresh { id: DriverId },
}

struct Entry<T> {
    generation: u32,
    value: Option<T>,
}

#[derive(Default)]
pub struct DriverSet {
    lists: Vec<Entry<ListDriver>>,
    scrolls: Vec<Entry<ScrollController>>,
    free_lists: Vec<u32>,
    free_scrolls: Vec<u32>,
    events: Vec<DriverEvent>,
    pending: bool,
}

impl DriverSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn wants_frame(&self) -> bool {
        self.pending
    }

    pub fn take_events(&mut self) -> Vec<DriverEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn list_create(&mut self, overscan: i32, estimate: f32) -> DriverId {
        alloc(
            &mut self.lists,
            &mut self.free_lists,
            ListDriver::new(overscan, estimate),
        )
    }

    pub fn list_release(&mut self, id: DriverId) -> bool {
        release(&mut self.lists, &mut self.free_lists, id)
    }

    pub fn list_bind(&mut self, id: DriverId, nodes: ListNodes) -> bool {
        let Some(driver) = get_mut(&mut self.lists, id) else {
            return false;
        };
        driver.bind(nodes);
        true
    }

    pub fn list_configure(
        &mut self,
        id: DriverId,
        estimate: f32,
        overscan: i32,
        blocked: bool,
    ) -> bool {
        let Some(driver) = get_mut(&mut self.lists, id) else {
            return false;
        };
        driver.configure(estimate, overscan, blocked);
        true
    }

    pub fn list_begin(&mut self, id: DriverId) -> bool {
        let Some(driver) = get_mut(&mut self.lists, id) else {
            return false;
        };
        driver.begin_items();
        true
    }

    pub fn list_item(&mut self, id: DriverId, item: ListItem) -> bool {
        let Some(driver) = get_mut(&mut self.lists, id) else {
            return false;
        };
        driver.push_item(item);
        true
    }

    pub fn list_end(&mut self, id: DriverId) -> bool {
        let Some(driver) = get_mut(&mut self.lists, id) else {
            return false;
        };
        driver.end_items();
        true
    }

    pub fn list_note_row(&mut self, id: DriverId, key: &str, node: Option<NodeId>) -> bool {
        let Some(driver) = get_mut(&mut self.lists, id) else {
            return false;
        };
        driver.note_row(key, node);
        true
    }

    pub fn scroll_create(&mut self) -> DriverId {
        alloc(
            &mut self.scrolls,
            &mut self.free_scrolls,
            ScrollController::default(),
        )
    }

    pub fn scroll_release(&mut self, id: DriverId) -> bool {
        release(&mut self.scrolls, &mut self.free_scrolls, id)
    }

    pub fn scroll_bind(
        &mut self,
        id: DriverId,
        viewport: Option<NodeId>,
        content: Option<NodeId>,
        tree: &Tree,
    ) -> bool {
        let Some(controller) = get_mut(&mut self.scrolls, id) else {
            return false;
        };
        controller.bind(viewport, content, tree);
        true
    }

    pub fn scroll_configure(
        &mut self,
        id: DriverId,
        config: ScrollConfig,
        parent: Option<DriverId>,
    ) -> bool {
        let link = parent.filter(|parent| {
            self.scrolls
                .get(parent.index as usize)
                .is_some_and(|entry| entry.generation == parent.generation && entry.value.is_some())
        });
        let Some(controller) = get_mut(&mut self.scrolls, id) else {
            return false;
        };
        controller.configure(
            config.content_min,
            config.pull_mode,
            config.threshold,
            config.rubber,
            config.factor,
            link.map(|parent| ParentLink {
                index: parent.index,
                generation: parent.generation,
            }),
        );
        true
    }

    pub fn scroll_drag_by(
        &mut self,
        id: DriverId,
        tree: &mut Tree,
        dx: f32,
        dy: f32,
    ) -> Option<DragStep> {
        let parent = get_mut(&mut self.scrolls, id)?.parent;
        let step = get_mut(&mut self.scrolls, id)?.drag_by(tree, dx, dy);
        if !step.pending {
            return Some(step);
        }
        if let Some(parent) = parent
            && self.consume_at(parent, tree, step.overflow_x, step.overflow_y)
        {
            let _ = get_mut(&mut self.scrolls, id).is_some_and(|controller| {
                controller.finish_drag(tree, true);
                true
            });
            return Some(DragStep {
                pending: false,
                ..step
            });
        }
        Some(step)
    }

    pub fn scroll_finish(&mut self, id: DriverId, tree: &mut Tree, taken: bool) -> Option<bool> {
        let refresh = get_mut(&mut self.scrolls, id)?.finish_drag(tree, taken);
        if refresh {
            self.events.push(DriverEvent::Refresh { id });
        }
        Some(refresh)
    }

    pub fn scroll_drag_end(&mut self, id: DriverId) -> Option<bool> {
        let refresh = get_mut(&mut self.scrolls, id)?.drag_end();
        if refresh {
            self.events.push(DriverEvent::Refresh { id });
        }
        Some(refresh)
    }

    pub fn scroll_wheel(&mut self, id: DriverId, tree: &mut Tree, dx: f32, dy: f32) -> bool {
        let Some(controller) = get_mut(&mut self.scrolls, id) else {
            return false;
        };
        controller.wheel(tree, dx, dy);
        true
    }

    pub fn scroll_query(&self, id: DriverId, tree: &Tree, cmd: i32) -> Option<(f32, f32)> {
        let entry = self.scrolls.get(id.index as usize)?;
        if entry.generation != id.generation {
            return None;
        }
        entry
            .value
            .as_ref()
            .map(|controller| controller.query(tree, cmd))
    }

    pub fn scroll_consume(
        &mut self,
        id: DriverId,
        tree: &mut Tree,
        ox: f32,
        oy: f32,
    ) -> Option<bool> {
        Some(get_mut(&mut self.scrolls, id)?.consume(tree, ox, oy))
    }

    pub fn tick(&mut self, tree: &mut Tree) {
        let mut refresh = Vec::new();
        for index in 0..self.scrolls.len() {
            let Some(entry) = self.scrolls.get_mut(index) else {
                continue;
            };
            let generation = entry.generation;
            let Some(controller) = entry.value.as_mut() else {
                continue;
            };
            if controller.tick(tree) {
                refresh.push(DriverId {
                    index: crate::cast::u32_from_usize(index),
                    generation,
                });
            }
        }
        for id in refresh {
            self.events.push(DriverEvent::Refresh { id });
        }

        self.pending = false;
        let mut listed = Vec::new();
        for index in 0..self.lists.len() {
            let Some(entry) = self.lists.get_mut(index) else {
                continue;
            };
            let generation = entry.generation;
            let Some(driver) = entry.value.as_mut() else {
                continue;
            };
            let (ops, chrome, end, pending) = driver.tick(tree);
            let id = DriverId {
                index: crate::cast::u32_from_usize(index),
                generation,
            };
            listed.push((id, ops, chrome, end, pending));
        }
        for (id, ops, chrome, end, pending) in listed {
            if pending {
                self.pending = true;
            }
            for op in ops {
                self.events.push(DriverEvent::ListOp { id, op });
            }
            if let Some(chrome) = chrome {
                self.events.push(DriverEvent::ListChrome { id, chrome });
            }
            if end {
                self.events.push(DriverEvent::EndReached { id });
            }
        }
    }

    fn consume_at(&mut self, parent: ParentLink, tree: &mut Tree, ox: f32, oy: f32) -> bool {
        let id = DriverId {
            index: parent.index,
            generation: parent.generation,
        };
        get_mut(&mut self.scrolls, id).is_some_and(|controller| controller.consume(tree, ox, oy))
    }
}

fn alloc<T>(entries: &mut Vec<Entry<T>>, free: &mut Vec<u32>, value: T) -> DriverId {
    if let Some(index) = free.pop() {
        let entry = &mut entries[index as usize];
        entry.value = Some(value);
        return DriverId {
            index,
            generation: entry.generation,
        };
    }
    let index = crate::cast::u32_from_usize(entries.len());
    entries.push(Entry {
        generation: 1,
        value: Some(value),
    });
    DriverId {
        index,
        generation: 1,
    }
}

fn release<T>(entries: &mut [Entry<T>], free: &mut Vec<u32>, id: DriverId) -> bool {
    let Some(entry) = entries.get_mut(id.index as usize) else {
        return false;
    };
    if entry.generation != id.generation || entry.value.is_none() {
        return false;
    }
    entry.value = None;
    entry.generation = entry.generation.wrapping_add(1).max(1);
    free.push(id.index);
    true
}

fn get_mut<T>(entries: &mut [Entry<T>], id: DriverId) -> Option<&mut T> {
    let entry = entries.get_mut(id.index as usize)?;
    if entry.generation != id.generation {
        return None;
    }
    entry.value.as_mut()
}
