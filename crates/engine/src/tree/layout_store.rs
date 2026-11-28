//! Storage and dispatch for Taffy's layout algorithms.

use std::collections::HashSet;

use taffy::prelude::*;
use taffy::{
    BlockContext, Cache, CacheTree, DetailedGridInfo, DetailedLayoutInfo, Layout,
    LayoutBlockContainer, LayoutContainingBlock, LayoutFlexboxContainer, LayoutGridContainer,
    LayoutInput, LayoutOutput, LayoutPartialTree, Point, RoundTree, RunMode, TaffyError,
    TraversePartialTree, TraverseTree, compute_block_layout, compute_cached_layout,
    compute_flexbox_layout, compute_grid_layout, compute_hidden_layout, compute_oof_layout,
    compute_root_layout, round_layout,
};

use super::layout::RootLayoutTrigger;

type Result<T> = std::result::Result<T, TaffyError>;
type Query = (LayoutInput, LayoutOutput);

const QUERY_LIMIT: usize = 8;
const ROUND_PARENT: NodeId = NodeId::new(u64::MAX);

#[derive(Default)]
struct Queries {
    entries: Vec<Query>,
    complete: bool,
}

impl Queries {
    fn push(&mut self, input: LayoutInput, output: &LayoutOutput) {
        if self.entries.is_empty() {
            self.complete = true;
        }
        if let Some((_, previous)) = self.entries.iter_mut().find(|(key, _)| *key == input) {
            previous.clone_from(output);
        } else if self.entries.len() < QUERY_LIMIT {
            self.entries.push((input, output.clone()));
        } else {
            self.complete = false;
        }
    }
}

#[derive(Default)]
struct LayoutNode {
    style: Style,
    owner: Option<super::NodeId>,
    measure: bool,
    parent: Option<NodeId>,
    containing: Option<NodeId>,
    children: Vec<NodeId>,
    hoisted: Vec<NodeId>,
    details: Option<DetailedLayoutInfo>,
    cache: Cache,
    unrounded: Layout,
    rounded: Layout,
    latest_layout: Option<LayoutInput>,
    queries: Queries,
}

impl LayoutNode {
    fn measure_index(&self) -> Option<usize> {
        self.owner
            .filter(|_| self.measure)
            .map(|id| id.index as usize)
    }
}

#[derive(Default)]
pub(super) struct LayoutStore {
    nodes: Vec<Option<LayoutNode>>,
    free: Vec<NodeId>,
    // Count in reverse depth order during replay so a boundary sees the
    // results of any changed boundaries inside it.
    pending: HashSet<NodeId>,
    root_dirty: Option<RootLayoutTrigger>,
    rounded_changes: HashSet<NodeId>,
    cache_hits: u32,
    cache_misses: u32,
}

impl LayoutStore {
    fn get(&self, id: NodeId) -> Result<&LayoutNode> {
        self.nodes
            .get(usize::from(id))
            .and_then(Option::as_ref)
            .ok_or(TaffyError::InvalidInputNode(id))
    }

    fn get_mut(&mut self, id: NodeId) -> Result<&mut LayoutNode> {
        self.nodes
            .get_mut(usize::from(id))
            .and_then(Option::as_mut)
            .ok_or(TaffyError::InvalidInputNode(id))
    }

    #[inline]
    #[allow(
        clippy::expect_used,
        reason = "Taffy only traverses IDs in the live child and hoisted lists"
    )]
    fn at(&self, id: NodeId) -> &LayoutNode {
        self.nodes[usize::from(id)]
            .as_ref()
            .expect("live layout node")
    }

    #[inline]
    #[allow(
        clippy::expect_used,
        reason = "Taffy only mutates IDs in the live storage graph"
    )]
    fn at_mut(&mut self, id: NodeId) -> &mut LayoutNode {
        self.nodes[usize::from(id)]
            .as_mut()
            .expect("live layout node")
    }

    pub(super) fn new_node(&mut self, style: Style, owner: super::NodeId, measure: bool) -> NodeId {
        let node = LayoutNode {
            style,
            owner: Some(owner),
            measure,
            ..LayoutNode::default()
        };
        if let Some(id) = self.free.pop() {
            self.nodes[usize::from(id)] = Some(node);
            id
        } else {
            let id = NodeId::from(self.nodes.len());
            self.nodes.push(Some(node));
            id
        }
    }

    pub(super) fn layout(&self, id: NodeId) -> Result<&Layout> {
        Ok(&self.get(id)?.rounded)
    }

    pub(super) fn insert_child_at_index(
        &mut self,
        parent: NodeId,
        index: usize,
        child: NodeId,
    ) -> Result<()> {
        let node = self.get_mut(child)?;
        node.parent = Some(parent);
        node.latest_layout = None;
        node.queries = Queries::default();
        self.get_mut(parent)?.children.insert(index, child);
        self.invalidate(parent, parent, "structure");
        Ok(())
    }

    pub(super) fn set_children(&mut self, parent: NodeId, children: &[NodeId]) -> Result<()> {
        for (index, child) in children.iter().enumerate() {
            self.insert_child_at_index(parent, index, *child)?;
        }
        Ok(())
    }

    fn detach_from_containing(&mut self, child: NodeId) -> Option<NodeId> {
        let containing = self.get_mut(child).ok()?.containing.take()?;
        if let Ok(owner) = self.get_mut(containing) {
            owner.hoisted.retain(|&id| id != child);
            Some(containing)
        } else {
            None
        }
    }

    fn detach_subtree_from_ancestors(&mut self, root: NodeId, origin: NodeId) {
        let mut subtree = HashSet::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if !subtree.insert(id) {
                continue;
            }
            if let Ok(node) = self.get(id) {
                stack.extend_from_slice(&node.children);
            }
        }
        let mut to_invalidate = HashSet::new();
        for &id in &subtree {
            let Some(containing) = self.get(id).ok().and_then(|node| node.containing) else {
                continue;
            };
            if subtree.contains(&containing) {
                continue;
            }
            if let Some(owner) = self.detach_from_containing(id)
                && owner != origin
            {
                to_invalidate.insert(owner);
            }
        }
        for owner in to_invalidate {
            if self.get(owner).is_ok() {
                self.invalidate(owner, origin, "structure");
            }
        }
    }

    #[cfg(feature = "js")]
    pub(super) fn reconcile_children(
        &mut self,
        parent: NodeId,
        children: Vec<NodeId>,
        removed: &[NodeId],
    ) -> Result<()> {
        for &child in &children {
            let node = self.get_mut(child)?;
            if node.parent != Some(parent) {
                node.parent = Some(parent);
                node.latest_layout = None;
                node.queries = Queries::default();
            }
        }
        for &child in removed {
            {
                let node = self.get_mut(child)?;
                node.parent = None;
                node.latest_layout = None;
            }
            self.pending.remove(&child);
            self.detach_subtree_from_ancestors(child, parent);
        }
        self.get_mut(parent)?.children = children;
        self.invalidate(parent, parent, "structure");
        Ok(())
    }

    pub(super) fn remove_child(&mut self, parent: NodeId, child: NodeId) -> Result<()> {
        self.get_mut(parent)?.children.retain(|&id| id != child);
        {
            let removed = self.get_mut(child)?;
            removed.parent = None;
            removed.latest_layout = None;
        }
        self.pending.remove(&child);
        self.detach_subtree_from_ancestors(child, parent);
        self.invalidate(parent, parent, "structure");
        Ok(())
    }

    pub(super) fn remove(&mut self, id: NodeId) -> Result<()> {
        if let Some(parent) = self.get(id)?.parent {
            self.remove_child(parent, id)?;
        }
        self.detach_from_containing(id);
        let (children, hoisted) = {
            let node = self.get_mut(id)?;
            (
                std::mem::take(&mut node.children),
                std::mem::take(&mut node.hoisted),
            )
        };
        for child in children {
            if let Ok(node) = self.get_mut(child)
                && node.parent == Some(id)
            {
                node.parent = None;
            }
        }
        for hoisted_child in hoisted {
            if let Ok(node) = self.get_mut(hoisted_child)
                && node.containing == Some(id)
            {
                node.containing = None;
            }
        }
        self.pending.remove(&id);
        self.rounded_changes.remove(&id);
        self.nodes[usize::from(id)] = None;
        self.free.push(id);
        Ok(())
    }

    pub(super) fn set_style(&mut self, id: NodeId, style: Style) -> Result<()> {
        let node = self.get_mut(id)?;
        node.style = style;
        node.cache.clear();
        node.latest_layout = None;
        node.queries = Queries::default();
        self.pending.remove(&id);
        // Position and flex inputs are read by the parent.
        if let Some(parent) = self.at(id).parent {
            self.invalidate(parent, id, "style");
        }
        Ok(())
    }

    pub(super) fn mark_dirty(&mut self, id: NodeId) -> Result<()> {
        self.get(id)?;
        self.invalidate(id, id, "text");
        Ok(())
    }

    fn note_root(&mut self, id: NodeId, cause: &'static str) {
        if self.root_dirty.is_none() {
            self.root_dirty = Some(RootLayoutTrigger {
                node: self.at(id).owner,
                cause,
            });
        }
    }

    fn clear_to_root(&mut self, mut id: NodeId, cause: &'static str) {
        self.note_root(id, cause);
        loop {
            let node = self.at_mut(id);
            node.cache.clear();
            if let Some(parent) = node.parent {
                id = parent;
            } else {
                break;
            }
        }
    }

    fn invalidate(&mut self, mut id: NodeId, origin: NodeId, cause: &'static str) {
        loop {
            self.at_mut(id).cache.clear();
            if self.can_relayout(id) {
                self.pending.insert(id);
                return;
            }
            self.at_mut(id).queries = Queries::default();
            if let Some(parent) = self.at(id).parent {
                id = parent;
            } else {
                self.note_root(origin, cause);
                return;
            }
        }
    }

    fn eligible(&self, id: NodeId) -> bool {
        let node = self.at(id);
        let Some(parent) = node.parent else {
            return false;
        };
        let style = &node.style;
        let parent_style = &self.at(parent).style;
        let points = |dimension: Dimension| dimension.into_raw().tag() == CompactLength::LENGTH_TAG;
        if style.display == Display::None
            || parent_style.display == Display::None
            || !points(style.size.width)
            || !points(style.size.height)
        {
            return false;
        }
        if style.position == Position::Absolute {
            return true;
        }
        let baseline = |align: AlignItems| align.keyword() == AlignItemsKeyword::Baseline;
        parent_style.display == Display::Flex
            && style.position == Position::Relative
            && style.flex_shrink == 0.0
            && style.flex_basis == Dimension::auto()
            && !baseline(parent_style.align_items)
            && !style.align_self.is_some_and(baseline)
    }

    fn can_relayout(&self, id: NodeId) -> bool {
        let node = self.at(id);
        self.eligible(id)
            && node.latest_layout.is_some()
            && node.queries.complete
            && (node.style.position != Position::Absolute || node.containing == node.parent)
            && node
                .queries
                .entries
                .iter()
                .all(|(_, output)| output.oof_candidates.is_empty())
    }

    fn depth(&self, mut id: NodeId) -> usize {
        let mut depth = 0;
        while let Some(parent) = self.at(id).parent {
            id = parent;
            depth += 1;
        }
        depth
    }

    pub(super) fn begin_layout(&mut self) {
        self.rounded_changes.clear();
    }

    pub(super) fn subtree_changed(&self, id: NodeId) -> bool {
        self.rounded_changes.contains(&id)
    }

    pub(super) fn can_probe_text(&self, id: NodeId) -> bool {
        let node = self.at(id);
        self.root_dirty.is_none()
            && !node.cache.is_empty()
            && node.children.is_empty()
            && node.latest_layout.is_some()
            && node.queries.complete
    }

    pub(super) fn text_outputs_match(
        &self,
        id: NodeId,
        mut measure: impl FnMut(LayoutInput, Option<usize>, &Style) -> LayoutOutput,
    ) -> bool {
        let node = self.at(id);
        node.queries.entries.iter().all(|(input, previous)| {
            let mut current = measure(*input, node.measure_index(), &node.style);
            if input.run_mode == RunMode::PerformLayout
                && overflow_inside_box(&current)
                && overflow_inside_box(previous)
            {
                // Taffy always contributes the child's border box to its parent's
                // scrollable overflow. Text changes inside it preserve that
                // contribution. Size, baselines and other outputs still compare.
                current.scrollable_overflow_rect = previous.scrollable_overflow_rect;
            }
            same_output(*input, &current, previous)
        })
    }

    pub(super) fn relayout(
        &mut self,
        force_root: bool,
        mut measure: impl FnMut(LayoutInput, Option<usize>, &Style) -> LayoutOutput,
    ) -> (Option<RootLayoutTrigger>, Vec<super::NodeId>) {
        let mut candidates: Vec<_> = self.pending.drain().collect();
        candidates.sort_unstable_by_key(|&id| std::cmp::Reverse(self.depth(id)));
        let mut local = Vec::new();
        for id in candidates {
            if force_root || self.root_dirty.is_some() || !self.can_relayout(id) {
                self.clear_to_root(id, "boundary_ineligible");
                continue;
            }
            let node = self.at_mut(id);
            let Some(layout_input) = node.latest_layout else {
                self.clear_to_root(id, "boundary_ineligible");
                continue;
            };
            let previous = std::mem::take(&mut node.queries);
            let mut view = View {
                store: self,
                measure: &mut measure,
                rounding: None,
            };
            let same = previous.entries.iter().all(|(input, output)| {
                *input == layout_input
                    || same_output(*input, &view.compute_child_layout(id, *input), output)
            }) && previous.entries.iter().any(|(input, output)| {
                *input == layout_input && view.compute_child_layout(id, layout_input) == *output
            });
            if same {
                let parent = view.store.rounding_parent(id);
                view.rounding = Some((id, parent));
                round_layout(&mut view, ROUND_PARENT);
                local.extend(view.store.at(id).owner);
            } else {
                self.clear_to_root(id, "boundary_output_changed");
            }
        }
        let root = self
            .root_dirty
            .take()
            .or(force_root.then_some(RootLayoutTrigger {
                node: None,
                cause: "initial",
            }));
        (root, local)
    }

    fn rounding_parent(&self, id: NodeId) -> Point<f32> {
        let mut ancestors = Vec::new();
        let mut current = id;
        while let Some(parent) = self.at(current).containing.or(self.at(current).parent) {
            ancestors.push(self.at(parent).unrounded.location);
            current = parent;
        }
        ancestors
            .into_iter()
            .rev()
            .fold(Point::ZERO, |origin, point| Point {
                x: origin.x + point.x,
                y: origin.y + point.y,
            })
    }

    pub(super) fn compute_layout_with_measure(
        &mut self,
        id: NodeId,
        available: Size<AvailableSpace>,
        mut measure: impl FnMut(LayoutInput, Option<usize>, &Style) -> LayoutOutput,
    ) {
        let mut view = View {
            store: self,
            measure: &mut measure,
            rounding: None,
        };
        compute_root_layout(&mut view, id, available);
        round_layout(&mut view, id);
    }

    pub(super) fn dirty_root(&mut self, id: NodeId, cause: &'static str) {
        self.clear_to_root(id, cause);
    }

    pub(super) fn take_cache_counts(&mut self) -> (u32, u32) {
        (
            std::mem::take(&mut self.cache_hits),
            std::mem::take(&mut self.cache_misses),
        )
    }

    pub(super) fn forget_parent_input(&mut self, id: NodeId) {
        self.at_mut(id).latest_layout = None;
        self.at_mut(id).queries = Queries::default();
    }
}

fn overflow_inside_box(output: &LayoutOutput) -> bool {
    let rect = output.scrollable_overflow_rect;
    rect.left >= 0.0
        && rect.top >= 0.0
        && rect.right <= output.size.width
        && rect.bottom <= output.size.height
}

fn same_output(input: LayoutInput, current: &LayoutOutput, previous: &LayoutOutput) -> bool {
    if input.run_mode == RunMode::ComputeSize {
        // Taffy's size cache and measure_child_size retain only this field.
        current.size == previous.size
    } else {
        current == previous
    }
}

struct View<'a, M> {
    store: &'a mut LayoutStore,
    measure: &'a mut M,
    // Taffy's rounding starts at (0, 0). A synthetic parent supplies the
    // same cumulative origin as a full pass, including fractional offsets.
    rounding: Option<(NodeId, Point<f32>)>,
}

impl<M> TraversePartialTree for View<'_, M> {
    type ChildIter<'a>
        = std::iter::Copied<std::slice::Iter<'a, NodeId>>
    where
        Self: 'a;

    fn child_ids(&self, id: NodeId) -> Self::ChildIter<'_> {
        if id == ROUND_PARENT {
            self.rounding
                .as_ref()
                .map_or(&[][..], |(child, _)| std::slice::from_ref(child))
                .iter()
                .copied()
        } else {
            self.store.at(id).children.iter().copied()
        }
    }

    fn child_count(&self, id: NodeId) -> usize {
        if id == ROUND_PARENT {
            1
        } else {
            self.store.at(id).children.len()
        }
    }

    fn get_child_id(&self, id: NodeId, index: usize) -> NodeId {
        if id == ROUND_PARENT
            && let Some((child, _)) = self.rounding
        {
            child
        } else {
            self.store.at(id).children[index]
        }
    }
}

impl<M> TraverseTree for View<'_, M> {}

impl<M> View<'_, M>
where
    M: FnMut(LayoutInput, Option<usize>, &Style) -> LayoutOutput,
{
    fn compute(
        &mut self,
        id: NodeId,
        input: LayoutInput,
        block: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        if input.run_mode == RunMode::PerformHiddenLayout {
            self.store.at_mut(id).latest_layout = None;
            self.set_final_layout(id, &Layout::with_order(0));
            return compute_hidden_layout(self, id);
        }
        if block.is_some() && input.run_mode == RunMode::PerformLayout {
            self.store.forget_parent_input(id);
        }
        let display = self.store.at(id).style.display;
        let track = block.is_none() && (self.store.at(id).measure || self.store.eligible(id));
        let output = compute_cached_layout(self, id, input, |tree, id, input| {
            let mut output = match (display, tree.child_count(id)) {
                (Display::None, _) => {
                    if input.run_mode == RunMode::PerformLayout {
                        tree.clear_hoisted_children(id);
                    }
                    compute_hidden_layout(tree, id)
                }
                (_, 0) => {
                    if input.run_mode == RunMode::PerformLayout {
                        tree.clear_hoisted_children(id);
                    }
                    let node = tree.store.at(id);
                    (tree.measure)(input, node.measure_index(), &node.style)
                }
                (Display::Block, _) => compute_block_layout(tree, id, input, block),
                (Display::FlowRoot, _) => compute_block_layout(tree, id, input, None),
                (Display::Flex, _) => compute_flexbox_layout(tree, id, input),
                (Display::Grid, _) => compute_grid_layout(tree, id, input),
            };
            if input.run_mode == RunMode::PerformLayout {
                compute_oof_layout(tree, id, &mut output);
            }
            output
        });
        if track && display != Display::None {
            let node = self.store.at_mut(id);
            node.queries.push(input, &output);
            if input.run_mode == RunMode::PerformLayout {
                node.latest_layout = Some(input);
            }
        }
        output
    }
}

impl<M> LayoutPartialTree for View<'_, M>
where
    M: FnMut(LayoutInput, Option<usize>, &Style) -> LayoutOutput,
{
    type CoreContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type CustomIdent = String;

    fn get_core_container_style(&self, id: NodeId) -> &Style {
        &self.store.at(id).style
    }

    fn set_unrounded_layout(&mut self, id: NodeId, layout: &Layout) {
        self.store.at_mut(id).unrounded = *layout;
    }

    fn compute_child_layout(&mut self, id: NodeId, input: LayoutInput) -> LayoutOutput {
        self.compute(id, input, None)
    }
}

impl<M> CacheTree for View<'_, M> {
    fn cache_get(&mut self, id: NodeId, input: &LayoutInput) -> Option<LayoutOutput> {
        let result = self.store.at_mut(id).cache.get(input);
        let count = if result.is_some() {
            &mut self.store.cache_hits
        } else {
            &mut self.store.cache_misses
        };
        *count = count.saturating_add(1);
        result
    }

    fn cache_store(&mut self, id: NodeId, input: &LayoutInput, output: LayoutOutput) {
        self.store.at_mut(id).cache.store(input, output);
    }

    fn cache_clear(&mut self, id: NodeId) {
        let node = self.store.at_mut(id);
        node.cache.clear();
        node.queries = Queries::default();
    }
}

impl<M> LayoutContainingBlock for View<'_, M>
where
    M: FnMut(LayoutInput, Option<usize>, &Style) -> LayoutOutput,
{
    type OofItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_oof_item_style(&self, id: NodeId) -> &Style {
        self.get_core_container_style(id)
    }

    fn clear_hoisted_children(&mut self, id: NodeId) {
        let previous = std::mem::take(&mut self.store.at_mut(id).hoisted);
        for child in previous {
            if self
                .store
                .get(child)
                .is_ok_and(|node| node.containing == Some(id))
            {
                self.store.at_mut(child).containing = None;
            }
        }
    }

    fn add_hoisted_children(&mut self, id: NodeId, children: &[NodeId]) {
        self.store.at_mut(id).hoisted.extend_from_slice(children);
        for &child in children {
            self.store.at_mut(child).containing = Some(id);
        }
    }

    fn get_detailed_layout_info(&self, id: NodeId) -> &DetailedLayoutInfo {
        self.store
            .at(id)
            .details
            .as_ref()
            .unwrap_or(&DetailedLayoutInfo::None)
    }
}

impl<M> LayoutBlockContainer for View<'_, M>
where
    M: FnMut(LayoutInput, Option<usize>, &Style) -> LayoutOutput,
{
    type BlockContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_block_container_style(&self, id: NodeId) -> &Style {
        self.get_core_container_style(id)
    }
    fn get_block_child_style(&self, id: NodeId) -> &Style {
        self.get_core_container_style(id)
    }

    fn compute_block_child_layout(
        &mut self,
        id: NodeId,
        input: LayoutInput,
        block: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        self.compute(id, input, block)
    }
}

impl<M> LayoutFlexboxContainer for View<'_, M>
where
    M: FnMut(LayoutInput, Option<usize>, &Style) -> LayoutOutput,
{
    type FlexboxContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_flexbox_container_style(&self, id: NodeId) -> &Style {
        self.get_core_container_style(id)
    }
    fn get_flexbox_child_style(&self, id: NodeId) -> &Style {
        self.get_core_container_style(id)
    }
}

impl<M> LayoutGridContainer for View<'_, M>
where
    M: FnMut(LayoutInput, Option<usize>, &Style) -> LayoutOutput,
{
    type GridContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type GridItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_grid_container_style(&self, id: NodeId) -> &Style {
        self.get_core_container_style(id)
    }
    fn get_grid_child_style(&self, id: NodeId) -> &Style {
        self.get_core_container_style(id)
    }

    fn set_detailed_grid_info(&mut self, id: NodeId, grid: DetailedGridInfo) {
        self.store.at_mut(id).details = Some(DetailedLayoutInfo::Grid(Box::new(grid)));
    }
}

impl<M> RoundTree for View<'_, M> {
    fn get_unrounded_layout(&self, id: NodeId) -> Layout {
        if id == ROUND_PARENT
            && let Some((_, location)) = self.rounding
        {
            Layout {
                location,
                ..Layout::with_order(0)
            }
        } else {
            self.store.at(id).unrounded
        }
    }

    fn set_final_layout(&mut self, id: NodeId, layout: &Layout) {
        if id == ROUND_PARENT || self.store.at(id).rounded == *layout {
            return;
        }
        self.store.at_mut(id).rounded = *layout;
        let mut current = self.store.at(id).parent;
        while let Some(id) = current {
            if !self.store.rounded_changes.insert(id) {
                break;
            }
            current = self.store.at(id).parent;
        }
    }

    fn is_out_of_flow(&self, id: NodeId) -> bool {
        let node = self.store.at(id);
        self.rounding.is_none_or(|(root, _)| root != id)
            && node.style.position.is_out_of_flow()
            && node.style.display != Display::None
    }

    fn hoisted_child_count(&self, id: NodeId) -> usize {
        if id == ROUND_PARENT {
            0
        } else {
            self.store.at(id).hoisted.len()
        }
    }

    fn get_hoisted_child_id(&self, id: NodeId, index: usize) -> NodeId {
        self.store.at(id).hoisted[index]
    }
}
