use crate::{Error, HEADER_LEN, Kind, MAGIC, NODE_PREFIX, VERSION};

/// One version 1 node. `text` is the text body or the image source.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: u32,
    pub parent: u32,
    pub kind: Kind,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: [u8; 4],
    pub text: String,
}

/// A scene built in creation order. Ids start at 1. Parent 0 hangs off the root.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub(crate) nodes: Vec<Node>,
}

impl Scene {
    pub fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn layer(&mut self, parent: u32) -> Result<u32, Error> {
        self.add(
            parent,
            Kind::Layer,
            0.0,
            0.0,
            0.0,
            0.0,
            [0, 0, 0, 0],
            String::new(),
        )
    }

    pub fn rect(
        &mut self,
        parent: u32,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [u8; 4],
    ) -> Result<u32, Error> {
        self.add(parent, Kind::Rect, x, y, w, h, color, String::new())
    }

    pub fn text(
        &mut self,
        parent: u32,
        x: f32,
        y: f32,
        utf8: impl Into<String>,
        color: [u8; 4],
    ) -> Result<u32, Error> {
        self.add(parent, Kind::Text, x, y, 0.0, 0.0, color, utf8.into())
    }

    pub fn image(
        &mut self,
        parent: u32,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        source: impl Into<String>,
    ) -> Result<u32, Error> {
        self.add(parent, Kind::Image, x, y, w, h, [0, 0, 0, 0], source.into())
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![0; self.encoded_len()];
        self.write_at(&mut out);
        out
    }

    /// Writes the scene when `out` is large enough. A short buffer is left unchanged.
    pub fn encode_into(&self, out: &mut [u8]) -> Result<usize, Error> {
        let len = self.encoded_len();
        if out.len() < len {
            return Err(Error::ShortBuffer);
        }
        self.write_at(&mut out[..len]);
        Ok(len)
    }

    pub(crate) fn from_nodes(nodes: Vec<Node>) -> Result<Self, Error> {
        let count = u32::try_from(nodes.len()).map_err(|_| Error::TooManyNodes)?;
        for (index, node) in nodes.iter().enumerate() {
            if node.id != next_id(index)? {
                return Err(Error::BadId);
            }
            text_len(node.text.len())?;
        }
        for node in &nodes {
            let known = node.parent == 0 || (node.parent != node.id && node.parent <= count);
            if !known {
                return Err(Error::UnknownParent);
            }
        }
        Ok(Self { nodes })
    }

    fn write_at(&self, out: &mut [u8]) {
        let mut cursor = 0;
        put(out, &mut cursor, MAGIC);
        put(out, &mut cursor, &VERSION.to_le_bytes());
        let count = u32::try_from(self.nodes.len()).unwrap_or(0);
        put(out, &mut cursor, &count.to_le_bytes());
        for node in &self.nodes {
            put(out, &mut cursor, &node.id.to_le_bytes());
            put(out, &mut cursor, &node.parent.to_le_bytes());
            put(out, &mut cursor, &[node.kind as u8, 0, 0, 0]);
            put(out, &mut cursor, &node.x.to_le_bytes());
            put(out, &mut cursor, &node.y.to_le_bytes());
            put(out, &mut cursor, &node.w.to_le_bytes());
            put(out, &mut cursor, &node.h.to_le_bytes());
            put(out, &mut cursor, &node.color);
            let len = u32::try_from(node.text.len()).unwrap_or(0);
            put(out, &mut cursor, &len.to_le_bytes());
            put(out, &mut cursor, node.text.as_bytes());
        }
    }

    fn encoded_len(&self) -> usize {
        HEADER_LEN
            + self
                .nodes
                .iter()
                .map(|node| NODE_PREFIX + node.text.len())
                .sum::<usize>()
    }

    fn add(
        &mut self,
        parent: u32,
        kind: Kind,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [u8; 4],
        text: String,
    ) -> Result<u32, Error> {
        if parent != 0 && !self.nodes.iter().any(|node| node.id == parent) {
            return Err(Error::UnknownParent);
        }
        text_len(text.len())?;
        let id = next_id(self.nodes.len())?;
        self.nodes.push(Node {
            id,
            parent,
            kind,
            x,
            y,
            w,
            h,
            color,
            text,
        });
        Ok(id)
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

fn text_len(len: usize) -> Result<u32, Error> {
    u32::try_from(len).map_err(|_| Error::TextTooLong)
}

fn next_id(len: usize) -> Result<u32, Error> {
    u32::try_from(len)
        .map_err(|_| Error::TooManyNodes)?
        .checked_add(1)
        .ok_or(Error::TooManyNodes)
}

fn put(out: &mut [u8], cursor: &mut usize, bytes: &[u8]) {
    let end = *cursor + bytes.len();
    out[*cursor..end].copy_from_slice(bytes);
    *cursor = end;
}

#[cfg(test)]
mod tests {
    use super::{Kind, Node, Scene, next_id, text_len};
    use crate::Error;

    #[test]
    fn text_longer_than_u32_is_rejected() {
        assert_eq!(text_len(u32::MAX as usize + 1), Err(Error::TextTooLong));
    }

    #[test]
    fn next_id_rejects_a_full_table() {
        assert_eq!(next_id(u32::MAX as usize), Err(Error::TooManyNodes));
    }

    #[test]
    fn parent_may_follow_its_child() {
        let scene = Scene::from_nodes(vec![sample(1, 2, Kind::Rect), sample(2, 0, Kind::Layer)]);
        assert!(scene.is_ok());
    }

    #[test]
    fn self_parent_is_rejected() {
        assert!(matches!(
            Scene::from_nodes(vec![sample(1, 1, Kind::Layer)]),
            Err(Error::UnknownParent)
        ));
    }

    fn sample(id: u32, parent: u32, kind: Kind) -> Node {
        Node {
            id,
            parent,
            kind,
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            color: [0, 0, 0, 0],
            text: String::new(),
        }
    }
}
