//! Version 1 scene bytes. The four-byte magic is `UIFW`.

mod decode;
mod draw_list;
mod encode;
mod error;
mod fixture;

pub use decode::decode;
pub use draw_list::{
    Command, DrawList, TextAlign, TextStyle, V1_FONT_SIZE, V1_PARAGRAPH_WIDTH, scene_to_draw_list,
};
pub use encode::{Node, Scene};
pub use error::Error;
pub use fixture::counter;

/// Node kinds stored in the version 1 byte stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Layer = 1,
    Rect = 2,
    Text = 3,
    Image = 4,
}

impl Kind {
    fn from_byte(byte: u8) -> Result<Self, Error> {
        match byte {
            1 => Ok(Self::Layer),
            2 => Ok(Self::Rect),
            3 => Ok(Self::Text),
            4 => Ok(Self::Image),
            _ => Err(Error::BadKind),
        }
    }
}

pub const VERSION: u32 = 1;
const HEADER_LEN: usize = 12;
const NODE_PREFIX: usize = 36;
const MAGIC: &[u8; 4] = b"UIFW";

const OFF_ID: usize = 0;
const OFF_PARENT: usize = 4;
const OFF_KIND: usize = 8;
const OFF_X: usize = 12;
const OFF_Y: usize = 16;
const OFF_W: usize = 20;
const OFF_H: usize = 24;
const OFF_COLOR: usize = 28;
const OFF_STR_LEN: usize = 32;

const _: () = assert!(OFF_STR_LEN + 4 == NODE_PREFIX);
