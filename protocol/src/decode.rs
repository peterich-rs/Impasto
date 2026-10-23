use crate::{
    Error, HEADER_LEN, Kind, MAGIC, NODE_PREFIX, OFF_COLOR, OFF_H, OFF_ID, OFF_KIND, OFF_PARENT,
    OFF_STR_LEN, OFF_W, OFF_X, OFF_Y, Scene, VERSION, encode::Node,
};

pub fn decode(bytes: &[u8]) -> Result<Scene, Error> {
    if bytes.len() < MAGIC.len() {
        return Err(Error::Truncated);
    }
    if &bytes[..MAGIC.len()] != MAGIC {
        return Err(Error::BadMagic);
    }
    if bytes.len() < HEADER_LEN {
        return Err(Error::Truncated);
    }
    let version = read_u32(&bytes[4..8])?;
    if version != VERSION {
        return Err(Error::BadVersion);
    }
    let count = read_u32(&bytes[8..12])? as usize;
    let room = bytes.len().saturating_sub(HEADER_LEN) / NODE_PREFIX;
    if count > room {
        return Err(Error::Truncated);
    }
    let mut offset = HEADER_LEN;
    let mut nodes = Vec::with_capacity(count);
    for _ in 0..count {
        if bytes.len().saturating_sub(offset) < NODE_PREFIX {
            return Err(Error::Truncated);
        }
        let kind = Kind::from_byte(bytes[offset + OFF_KIND])?;
        let text_len = read_u32(&bytes[offset + OFF_STR_LEN..offset + NODE_PREFIX])? as usize;
        let text_at = offset + NODE_PREFIX;
        let text_end = text_at.checked_add(text_len).ok_or(Error::Truncated)?;
        if text_end > bytes.len() {
            return Err(Error::Truncated);
        }
        let text = std::str::from_utf8(&bytes[text_at..text_end])
            .map_err(|_| Error::BadUtf8)?
            .to_owned();
        nodes.push(Node {
            id: read_u32(&bytes[offset + OFF_ID..offset + OFF_PARENT])?,
            parent: read_u32(&bytes[offset + OFF_PARENT..offset + OFF_KIND])?,
            kind,
            x: read_f32(&bytes[offset + OFF_X..offset + OFF_Y])?,
            y: read_f32(&bytes[offset + OFF_Y..offset + OFF_W])?,
            w: read_f32(&bytes[offset + OFF_W..offset + OFF_H])?,
            h: read_f32(&bytes[offset + OFF_H..offset + OFF_COLOR])?,
            color: [
                bytes[offset + OFF_COLOR],
                bytes[offset + OFF_COLOR + 1],
                bytes[offset + OFF_COLOR + 2],
                bytes[offset + OFF_COLOR + 3],
            ],
            text,
        });
        offset = text_end;
    }
    if offset != bytes.len() {
        return Err(Error::Trailing);
    }
    Scene::from_nodes(nodes)
}

fn read_u32(bytes: &[u8]) -> Result<u32, Error> {
    let raw: [u8; 4] = bytes
        .get(..4)
        .ok_or(Error::Truncated)?
        .try_into()
        .map_err(|_| Error::Truncated)?;
    Ok(u32::from_le_bytes(raw))
}

fn read_f32(bytes: &[u8]) -> Result<f32, Error> {
    let raw: [u8; 4] = bytes
        .get(..4)
        .ok_or(Error::Truncated)?
        .try_into()
        .map_err(|_| Error::Truncated)?;
    Ok(f32::from_le_bytes(raw))
}
