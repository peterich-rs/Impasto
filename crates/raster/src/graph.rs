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

/// A DAG of render passes over the physical attachment leases. A recycled
/// attachment has the same identity, so its next writer depends on every
/// reader of the previous lease as well as the previous writer.
pub(crate) struct RenderGraph<Pass> {
    nodes: Vec<Node<Pass>>,
    textures: std::collections::HashMap<wgpu::Texture, Access>,
}

struct Node<Pass> {
    pass: Pass,
    dependencies: Vec<usize>,
    main: bool,
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
        reads: impl IntoIterator<Item = wgpu::Texture>,
        writes: impl IntoIterator<Item = wgpu::Texture>,
    ) {
        let index = self.nodes.len();
        let mut dependencies = Vec::new();
        for texture in reads {
            let access = self.textures.entry(texture).or_default();
            dependencies.extend(access.writer);
            access.readers.push(index);
        }
        for texture in writes {
            let target = self.textures.entry(texture).or_default();
            dependencies.extend(target.writer.replace(index));
            dependencies.append(&mut target.readers);
        }
        dependencies.retain(|&dependency| dependency != index);
        dependencies.sort_unstable();
        dependencies.dedup();
        self.nodes.push(Node {
            pass,
            dependencies,
            main,
        });
    }

    pub(crate) fn schedule(&mut self) -> impl Iterator<Item = Pass> + use<Pass> {
        use std::{cmp::Reverse, collections::BinaryHeap};

        let mut nodes: Vec<_> = self.nodes.drain(..).map(Some).collect();
        self.textures.clear();
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
        std::iter::from_fn(move || {
            let Reverse((_, index)) = ready.pop()?;
            let node = nodes[index].take()?;
            for &dependent in &dependents[index] {
                remaining[dependent] -= 1;
                if remaining[dependent] == 0
                    && let Some(node) = &nodes[dependent]
                {
                    ready.push(Reverse((node.main, dependent)));
                }
            }
            Some(node.pass)
        })
    }
}
