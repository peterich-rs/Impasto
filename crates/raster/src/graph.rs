//! Render target dependencies across offscreen producers and surface consumers.

use std::{collections::HashSet, hash::Hash};

/// Main-target consumers can be recorded before their input textures. Pin
/// those inputs through the consumer pass, then return them to the alias pool.
pub(crate) struct AttachmentLifetimes<Key, Image> {
    reads: HashSet<Key>,
    retired: Vec<Image>,
    bytes: u64,
}

impl<Key: Eq + Hash, Image> AttachmentLifetimes<Key, Image> {
    pub(crate) fn new() -> Self {
        Self {
            reads: HashSet::new(),
            retired: Vec::new(),
            bytes: 0,
        }
    }

    pub(crate) fn read(&mut self, key: Key) {
        self.reads.insert(key);
    }

    pub(crate) fn contains(&self, key: &Key) -> bool {
        self.reads.contains(key)
    }

    pub(crate) fn retire(&mut self, image: Image, bytes: u64) {
        self.retired.push(image);
        self.bytes += bytes;
    }

    pub(crate) fn exceeds(&self, images: usize, bytes: u64) -> bool {
        self.retired.len() >= images || self.bytes >= bytes
    }

    pub(crate) fn finish(&mut self) -> impl Iterator<Item = Image> + '_ {
        self.reads.clear();
        self.bytes = 0;
        self.retired.drain(..)
    }
}

/// GPU commands between surface reads are already in dependency order. Each
/// segment's producers precede its surface consumer, which must precede any
/// later producer that reads the surface or reuses one of these attachments.
pub(crate) struct SurfaceGraph<Pass> {
    segments: Vec<(wgpu::CommandEncoder, Pass)>,
    last: Option<Pass>,
}

impl<Pass> Default for SurfaceGraph<Pass> {
    fn default() -> Self {
        Self {
            segments: Vec::new(),
            last: None,
        }
    }
}

impl<Pass> SurfaceGraph<Pass> {
    pub(crate) fn segment(&mut self, producers: wgpu::CommandEncoder, consumer: Pass) {
        self.segments.push((producers, consumer));
    }

    pub(crate) fn finish(&mut self, consumer: Pass) {
        self.last = Some(consumer);
    }

    pub(crate) fn encode(
        self,
        tail: &mut wgpu::CommandEncoder,
        mut encode: impl FnMut(Pass, &mut wgpu::CommandEncoder),
    ) -> Vec<wgpu::CommandBuffer> {
        let mut commands = Vec::with_capacity(self.segments.len());
        for (mut producers, consumer) in self.segments {
            encode(consumer, &mut producers);
            commands.push(producers.finish());
        }
        if let Some(consumer) = self.last {
            encode(consumer, tail);
        }
        commands
    }
}

/// Frame-local identity assigned before a transient texture is allocated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct VirtualTexture(pub(crate) usize);

/// A render attachment, either imported or owned by the current graph.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Attachment {
    Physical(wgpu::TextureView),
    Virtual(VirtualTexture),
}

impl Attachment {
    pub(crate) fn physical(&self) -> Option<&wgpu::TextureView> {
        match self {
            Self::Physical(view) => Some(view),
            Self::Virtual(_) => None,
        }
    }

    pub(crate) fn virtual_id(&self) -> Option<VirtualTexture> {
        match self {
            Self::Virtual(id) => Some(*id),
            Self::Physical(_) => None,
        }
    }
}

/// A dependency graph over virtual and imported attachments. Each virtual
/// attachment has an independent lifetime until scheduling fixes execution order.
pub(crate) struct RenderGraph<Pass> {
    nodes: Vec<Node<Pass>>,
    textures: std::collections::HashMap<Attachment, Access>,
}

pub(crate) struct ScheduledPass<Pass> {
    pub(crate) pass: Pass,
    pub(crate) attachments: Vec<VirtualTexture>,
}

struct Node<Pass> {
    pass: Pass,
    dependencies: Vec<usize>,
    main: bool,
    attachments: Vec<VirtualTexture>,
}

#[derive(Default)]
struct Access {
    writer: Option<usize>,
    readers: Vec<usize>,
}

impl<Pass> Default for RenderGraph<Pass> {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            textures: Default::default(),
        }
    }
}

impl<Pass> RenderGraph<Pass> {
    pub(crate) fn record(
        &mut self,
        pass: Pass,
        main: bool,
        reads: impl IntoIterator<Item = Attachment>,
        writes: impl IntoIterator<Item = Attachment>,
    ) {
        let index = self.nodes.len();
        let mut dependencies = Vec::new();
        let mut attachments = Vec::new();
        for texture in reads {
            attachments.extend(texture.virtual_id());
            let access = self.textures.entry(texture).or_default();
            dependencies.extend(access.writer);
            access.readers.push(index);
        }
        for texture in writes {
            let virtual_texture = texture.virtual_id();
            attachments.extend(virtual_texture);
            let target = self.textures.entry(texture).or_default();
            let previous = target.writer.replace(index);
            if virtual_texture.is_some() && previous.is_none() {
                // A virtual image has no imported contents. A consumer
                // declared before this first producer must run after it.
                for reader in target.readers.drain(..).filter(|reader| *reader != index) {
                    self.nodes[reader].dependencies.push(index);
                }
            } else {
                dependencies.extend(previous);
                dependencies.append(&mut target.readers);
            }
        }
        dependencies.retain(|&dependency| dependency != index);
        dependencies.sort_unstable();
        dependencies.dedup();
        self.nodes.push(Node {
            pass,
            dependencies,
            main,
            attachments,
        });
    }

    pub(crate) fn schedule(&mut self) -> Option<Vec<ScheduledPass<Pass>>> {
        use std::{cmp::Reverse, collections::BinaryHeap};

        let mut nodes: Vec<_> = self.nodes.drain(..).map(Some).collect();
        if self
            .textures
            .drain()
            .any(|(texture, access)| texture.virtual_id().is_some() && access.writer.is_none())
        {
            return None;
        }
        let mut dependents = vec![Vec::<usize>::new(); nodes.len()];
        let mut remaining = vec![0; nodes.len()];
        let mut ready = BinaryHeap::new();
        for (index, node) in nodes.iter().enumerate() {
            if let Some(node) = node {
                remaining[index] = node.dependencies.len();
                if node.dependencies.is_empty() {
                    ready.push(Reverse((node.main, index)));
                }
                for &dependency in &node.dependencies {
                    dependents[dependency].push(index);
                }
            }
        }
        let mut ordered = Vec::with_capacity(nodes.len());
        while let Some(Reverse((_, index))) = ready.pop() {
            let node = nodes[index].take()?;
            for &dependent in &dependents[index] {
                remaining[dependent] -= 1;
                if remaining[dependent] == 0
                    && let Some(node) = &nodes[dependent]
                {
                    ready.push(Reverse((node.main, dependent)));
                }
            }
            ordered.push(ScheduledPass {
                pass: node.pass,
                attachments: node.attachments,
            });
        }
        (ordered.len() == nodes.len()).then_some(ordered)
    }
}

/// Assigns matching specifications to the same slot only after the previous
/// image's last scheduled reader. Two resources in the same pass never alias.
pub(crate) fn allocate_intervals<Spec: Copy + Eq + Hash>(
    specifications: &[Spec],
    uses: impl IntoIterator<Item = (usize, VirtualTexture)>,
) -> (Vec<Spec>, Vec<Option<usize>>, u64) {
    use std::{
        cmp::Reverse,
        collections::{BinaryHeap, HashMap},
    };

    let mut intervals = vec![None; specifications.len()];
    for (pass, VirtualTexture(id)) in uses {
        let interval = intervals[id].get_or_insert((pass, pass));
        interval.1 = pass;
    }
    let mut ordered: Vec<_> = intervals
        .into_iter()
        .enumerate()
        .filter_map(|(id, interval)| interval.map(|(first, last)| (first, last, id)))
        .collect();
    ordered.sort_unstable();

    let mut slots = Vec::new();
    let mut assigned = vec![None; specifications.len()];
    let mut free: HashMap<Spec, Vec<usize>> = HashMap::new();
    let mut live = BinaryHeap::new();
    let mut count = 0;
    for (first, last, id) in ordered {
        while let Some(&Reverse((end, slot))) = live.peek() {
            if end >= first {
                break;
            }
            live.pop();
            free.entry(slots[slot]).or_default().push(slot);
        }
        let spec = specifications[id];
        let slot = free.entry(spec).or_default().pop().unwrap_or_else(|| {
            let slot = slots.len();
            slots.push(spec);
            slot
        });
        assigned[id] = Some(slot);
        live.push(Reverse((last, slot)));
        count += 1;
    }
    (slots, assigned, count)
}

#[cfg(test)]
mod tests {
    use super::{Attachment, RenderGraph, VirtualTexture, allocate_intervals};

    #[test]
    fn aliases_follow_scheduled_reads_and_never_cross_the_same_pass() {
        let [a, b, c, d] = [0, 1, 2, 3].map(|id| Attachment::Virtual(VirtualTexture(id)));
        let mut graph = RenderGraph::default();
        graph.record("first", false, [], [a.clone()]);
        graph.record("main", true, [a.clone(), d.clone()], []);
        graph.record("pair", false, [a], [b.clone()]);
        graph.record("local", false, [b], [c.clone()]);
        graph.record("final", false, [c], [d]);
        let schedule = graph.schedule().expect("valid pass dependencies");
        let order: Vec<_> = schedule.iter().map(|node| node.pass).collect();
        assert_eq!(order, ["first", "pair", "local", "final", "main"]);
        let (_, slots, used) = allocate_intervals(
            &[1, 1, 1, 2],
            schedule
                .iter()
                .enumerate()
                .flat_map(|(index, node)| node.attachments.iter().map(move |&id| (index, id))),
        );
        assert_eq!(used, 4);
        assert_ne!(slots[0], slots[1]);
        assert_ne!(slots[1], slots[2]);
        assert_ne!(slots[0], slots[2]);
        assert_ne!(slots[2], slots[3]);

        let (specs, reuse, _) = allocate_intervals(
            &[1, 1, 2],
            [
                (0, VirtualTexture(0)),
                (3, VirtualTexture(1)),
                (4, VirtualTexture(2)),
            ],
        );
        assert_eq!(specs, [1, 2]);
        assert_eq!(reuse[0], reuse[1]);
        assert_ne!(reuse[1], reuse[2]);
    }
}
