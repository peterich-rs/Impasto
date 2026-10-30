use std::ffi::CString;

pub(super) struct Face {
    pub(super) bytes: Vec<u8>,
    pub(super) family: CString,
}

pub(crate) struct FontRegistry {
    pub(super) faces: Vec<Face>,
    pub(super) default_family: Option<CString>,
}

impl FontRegistry {
    pub(super) fn new() -> Self {
        Self {
            faces: Vec::new(),
            default_family: None,
        }
    }
}
