#![allow(clippy::expect_used)]

use protocol::{Command, Error, Kind, decode, scene_to_draw_list};

fn golden() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/counter.bin"
    ))
    .expect("counter.bin is checked in")
}

#[test]
fn counter_matches_golden_bytes() {
    let bytes = protocol::counter().encode();
    assert_eq!(bytes, golden());
}

#[test]
fn short_buffer_does_not_write() {
    let scene = protocol::counter();
    let mut buffer = [0xabu8; 12];
    assert_eq!(scene.encode_into(&mut buffer), Err(Error::ShortBuffer));
    assert_eq!(buffer, [0xab; 12]);
}

#[test]
fn empty_scene_is_a_header() {
    let bytes = protocol::Scene::new().encode();
    assert_eq!(bytes.len(), 12);
    assert_eq!(&bytes[..4], b"UIFW");
    assert_eq!(bytes[4], 1);
    assert_eq!(&bytes[8..12], &0u32.to_le_bytes());
}

#[test]
fn unknown_parent_is_rejected() {
    let mut scene = protocol::Scene::new();
    assert_eq!(
        scene.rect(99, 0.0, 0.0, 1.0, 1.0, [0, 0, 0, 0]),
        Err(Error::UnknownParent)
    );
}

#[test]
fn golden_text_and_image_decode() {
    let scene = decode(&golden()).expect("golden decodes");
    let nodes = scene.nodes();
    assert_eq!(nodes.len(), 5);
    let count = &nodes[2];
    assert_eq!(count.kind, Kind::Text);
    assert_eq!(count.text, "Count");
    assert_eq!(count.x, 24.0);
    assert_eq!(count.color, [255, 255, 255, 255]);
    let image = &nodes[4];
    assert_eq!(image.kind, Kind::Image);
    assert_eq!(image.text, "asset:logo");
    assert_eq!(image.w, 56.0);
}

#[test]
fn decode_rejects_damaged_input() {
    let bytes = golden();
    assert_eq!(decode(&bytes[..bytes.len() - 1]), Err(Error::Truncated));

    let mut magic = bytes.clone();
    magic[0] = b'X';
    assert_eq!(decode(&magic), Err(Error::BadMagic));

    let mut version = bytes.clone();
    version[4] = 2;
    assert_eq!(decode(&version), Err(Error::BadVersion(2)));

    let mut kind = bytes.clone();
    // First node kind sits at header + 8.
    kind[12 + 8] = 9;
    assert_eq!(decode(&kind), Err(Error::BadKind));

    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(decode(&trailing), Err(Error::Trailing));
}

#[test]
fn exact_buffer_round_trips_and_leaves_the_tail() {
    let scene = protocol::counter();
    let encoded = scene.encode();
    let mut exact = vec![0u8; encoded.len()];
    assert_eq!(scene.encode_into(&mut exact), Ok(encoded.len()));
    assert_eq!(exact, encoded);

    let mut longer = vec![0x5au8; encoded.len() + 4];
    assert_eq!(scene.encode_into(&mut longer), Ok(encoded.len()));
    assert_eq!(&longer[..encoded.len()], encoded.as_slice());
    assert_eq!(&longer[encoded.len()..], &[0x5a; 4]);
}

#[test]
fn decode_rejects_bad_utf8_parent_and_id() {
    let bytes = golden();
    let mut utf8 = bytes.clone();
    // "Count" starts after the 12-byte header and two 36-byte nodes.
    utf8[12 + 36 + 36 + 36] = 0xff;
    assert_eq!(decode(&utf8), Err(Error::BadUtf8));

    let mut parent = bytes.clone();
    parent[12 + 36 + 4..12 + 36 + 8].copy_from_slice(&99u32.to_le_bytes());
    assert_eq!(decode(&parent), Err(Error::UnknownParent));

    let mut id = bytes.clone();
    id[12 + 36..12 + 36 + 4].copy_from_slice(&9u32.to_le_bytes());
    assert_eq!(decode(&id), Err(Error::BadId));
}

#[test]
fn declared_node_count_cannot_outrun_the_buffer() {
    let mut header = [0u8; 12];
    header[..4].copy_from_slice(b"UIFW");
    header[4..8].copy_from_slice(&1u32.to_le_bytes());
    header[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(decode(&header), Err(Error::Truncated));
}

#[test]
fn version_one_draw_list_skips_the_layer() {
    let list = scene_to_draw_list(&protocol::counter());
    assert_eq!(
        list.commands,
        vec![
            Command::Fill {
                color: [17, 17, 17, 255]
            },
            Command::Rect {
                x: 0.0,
                y: 0.0,
                w: 320.0,
                h: 180.0,
                corner_radius: 0.0,
                color: [17, 17, 17, 255]
            },
            Command::Text {
                x: 24.0,
                y: 36.0,
                max_width: 280.0,
                font_size: 28.0,
                color: [255, 255, 255, 255],
                utf8: "Count".to_owned(),
                style: protocol::TextStyle::default(),
            },
            Command::Text {
                x: 24.0,
                y: 72.0,
                max_width: 280.0,
                font_size: 28.0,
                color: [124, 255, 107, 255],
                utf8: "0".to_owned(),
                style: protocol::TextStyle::default(),
            },
            Command::Image {
                x: 240.0,
                y: 24.0,
                w: 56.0,
                h: 56.0,
                source: "asset:logo".to_owned()
            },
        ]
    );
}

#[test]
fn draw_list_stores_an_unbalanced_stack_as_data() {
    let list = protocol::DrawList {
        commands: vec![Command::Pop],
    };
    assert_eq!(list.commands, vec![Command::Pop]);
}
