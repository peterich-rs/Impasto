//! QuickJS entry points for [`uicore::DriverSet`]. The interpreter owns no
//! driver state: each class carries an index and a generation.

use std::ffi::{c_char, c_void};

use uicore::{
    Chrome, DriverEvent, DriverId, ListItem, ListNodes, ListOp, NodeId, PullMode, ScrollConfig,
};

use super::ffi::qjs_driver;
use super::{Inner, c_message, inner, utf8_of};

fn driver_id(index: u32, generation: u32) -> DriverId {
    DriverId { index, generation }
}

fn node_of(raw: u64) -> Option<NodeId> {
    (raw != 0).then(|| NodeId::from_raw(raw))
}

pub(super) fn collect_driver_tick(inner: &Inner) -> Vec<DriverEvent> {
    {
        let mut drivers = inner.drivers.borrow_mut();
        let mut tree = inner.tree.borrow_mut();
        drivers.tick(&mut tree);
    }
    let pending = inner.drivers.borrow().wants_frame();
    inner.driver_frame.set(pending);
    inner.drivers.borrow_mut().take_events()
}

pub(super) fn emit_driver(inner: &Inner, event: &DriverEvent) -> Result<(), String> {
    let mut error = [0u8; 512];
    let failed = match event {
        DriverEvent::ListOp { id, op } => emit_op(inner, *id, op, &mut error),
        DriverEvent::ListChrome { id, chrome } => emit_chrome(inner, *id, chrome, &mut error),
        DriverEvent::EndReached { id } => call(
            inner, 1, *id, 3, 0, 0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, "", "", &mut error,
        ),
        DriverEvent::Refresh { id } => call(
            inner, 2, *id, 5, 0, 0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, "", "", &mut error,
        ),
    };
    if failed == 0 {
        Ok(())
    } else {
        Err(c_message(&error))
    }
}

fn emit_op(inner: &Inner, id: DriverId, op: &ListOp, error: &mut [u8]) -> i32 {
    match op {
        ListOp::Detach { key } => call(
            inner, 1, id, 1, 1, 0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, key, "", error,
        ),
        ListOp::Adopt { key, ty, index } => call(
            inner,
            1,
            id,
            1,
            2,
            i32::try_from(*index).unwrap_or(0),
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            key,
            ty,
            error,
        ),
        ListOp::Set { key, index } => call(
            inner,
            1,
            id,
            1,
            3,
            i32::try_from(*index).unwrap_or(0),
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            key,
            "",
            error,
        ),
        ListOp::Place { key, before } => call(
            inner, 1, id, 1, 4, 0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, key, before, error,
        ),
    }
}

fn emit_chrome(inner: &Inner, id: DriverId, chrome: &Chrome, error: &mut [u8]) -> i32 {
    let mut flags = 0;
    if chrome.thumb_on {
        flags |= 1;
    }
    if chrome.h_on {
        flags |= 2;
    }
    call(
        inner,
        1,
        id,
        2,
        flags,
        chrome.pin_index,
        chrome.thumb_len,
        chrome.thumb_off,
        chrome.h_len,
        chrome.h_off,
        chrome.pin_y,
        chrome.extent,
        &chrome.pin_key,
        "",
        error,
    )
}

fn call(
    inner: &Inner,
    kind: i32,
    id: DriverId,
    tag: i32,
    i0: i32,
    i1: i32,
    f0: f32,
    f1: f32,
    f2: f32,
    f3: f32,
    f4: f32,
    f5: f32,
    s0: &str,
    s1: &str,
    error: &mut [u8],
) -> i32 {
    // SAFETY: `inner.host` is the open interpreter. The strings live for this call.
    unsafe {
        qjs_driver(
            inner.host,
            kind,
            id.index,
            id.generation,
            tag,
            i0,
            i1,
            f0,
            f1,
            f2,
            f3,
            f4,
            f5,
            s0.as_ptr().cast(),
            s0.len(),
            s1.as_ptr().cast(),
            s1.len(),
            error.as_mut_ptr().cast(),
            error.len(),
        )
    }
}

/// # Safety
/// `session` is the live `Inner`. `index` and `generation` are writable.
pub unsafe extern "C" fn list_create(
    session: *mut c_void,
    overscan: i32,
    estimate: f32,
    index: *mut u32,
    generation: *mut u32,
) -> i32 {
    if index.is_null() || generation.is_null() {
        return -1;
    }
    let inner = unsafe { inner(session) };
    let id = inner.drivers.borrow_mut().list_create(overscan, estimate);
    unsafe {
        *index = id.index;
        *generation = id.generation;
    }
    0
}

/// # Safety
/// `session` is the live `Inner`.
pub unsafe extern "C" fn list_release(session: *mut c_void, index: u32, generation: u32) -> i32 {
    let inner = unsafe { inner(session) };
    i32::from(
        !inner
            .drivers
            .borrow_mut()
            .list_release(driver_id(index, generation)),
    )
}

/// # Safety
/// `nodes` points at `count` ids when `count > 0`.
pub unsafe extern "C" fn list_bind(
    session: *mut c_void,
    index: u32,
    generation: u32,
    nodes: *const u64,
    count: i32,
) -> i32 {
    let inner = unsafe { inner(session) };
    let bound = unsafe { read_nodes(nodes, count) };
    i32::from(
        !inner
            .drivers
            .borrow_mut()
            .list_bind(driver_id(index, generation), bound),
    )
}

/// # Safety
/// `session` is the live `Inner`.
pub unsafe extern "C" fn list_configure(
    session: *mut c_void,
    index: u32,
    generation: u32,
    estimate: f32,
    overscan: i32,
    blocked: i32,
) -> i32 {
    let inner = unsafe { inner(session) };
    i32::from(!inner.drivers.borrow_mut().list_configure(
        driver_id(index, generation),
        estimate,
        overscan,
        blocked != 0,
    ))
}

/// # Safety
/// `session` is the live `Inner`.
pub unsafe extern "C" fn list_begin(session: *mut c_void, index: u32, generation: u32) -> i32 {
    let inner = unsafe { inner(session) };
    i32::from(
        !inner
            .drivers
            .borrow_mut()
            .list_begin(driver_id(index, generation)),
    )
}

/// # Safety
/// `key` and `ty` address `key_len` and `ty_len` bytes for this call.
pub unsafe extern "C" fn list_item(
    session: *mut c_void,
    index: u32,
    generation: u32,
    key: *const c_char,
    key_len: usize,
    ty: *const c_char,
    ty_len: usize,
    sticky: i32,
) -> i32 {
    let Some(key) = (unsafe { utf8_of(key, key_len) }) else {
        return -1;
    };
    let Some(ty) = (unsafe { utf8_of(ty, ty_len) }) else {
        return -1;
    };
    let inner = unsafe { inner(session) };
    i32::from(!inner.drivers.borrow_mut().list_item(
        driver_id(index, generation),
        ListItem {
            key: key.to_owned(),
            ty: ty.to_owned(),
            sticky: sticky != 0,
        },
    ))
}

/// # Safety
/// `session` is the live `Inner`.
pub unsafe extern "C" fn list_end(session: *mut c_void, index: u32, generation: u32) -> i32 {
    let inner = unsafe { inner(session) };
    i32::from(
        !inner
            .drivers
            .borrow_mut()
            .list_end(driver_id(index, generation)),
    )
}

/// # Safety
/// `key` addresses `key_len` bytes for this call.
pub unsafe extern "C" fn list_note_row(
    session: *mut c_void,
    index: u32,
    generation: u32,
    key: *const c_char,
    key_len: usize,
    node: u64,
) -> i32 {
    let Some(key) = (unsafe { utf8_of(key, key_len) }) else {
        return -1;
    };
    let inner = unsafe { inner(session) };
    i32::from(!inner.drivers.borrow_mut().list_note_row(
        driver_id(index, generation),
        key,
        node_of(node),
    ))
}

/// # Safety
/// `index` and `generation` are writable.
pub unsafe extern "C" fn scroll_create(
    session: *mut c_void,
    index: *mut u32,
    generation: *mut u32,
) -> i32 {
    if index.is_null() || generation.is_null() {
        return -1;
    }
    let inner = unsafe { inner(session) };
    let id = inner.drivers.borrow_mut().scroll_create();
    unsafe {
        *index = id.index;
        *generation = id.generation;
    }
    0
}

/// # Safety
/// `session` is the live `Inner`.
pub unsafe extern "C" fn scroll_release(session: *mut c_void, index: u32, generation: u32) -> i32 {
    let inner = unsafe { inner(session) };
    i32::from(
        !inner
            .drivers
            .borrow_mut()
            .scroll_release(driver_id(index, generation)),
    )
}

/// # Safety
/// `session` is the live `Inner`.
pub unsafe extern "C" fn scroll_bind(
    session: *mut c_void,
    index: u32,
    generation: u32,
    viewport: u64,
    content: u64,
) -> i32 {
    let inner = unsafe { inner(session) };
    let mut drivers = inner.drivers.borrow_mut();
    let tree = inner.tree.borrow();
    i32::from(!drivers.scroll_bind(
        driver_id(index, generation),
        node_of(viewport),
        node_of(content),
        &tree,
    ))
}

/// # Safety
/// `session` is the live `Inner`.
pub unsafe extern "C" fn scroll_configure(
    session: *mut c_void,
    index: u32,
    generation: u32,
    content_min: f32,
    pull_mode: i32,
    threshold: f32,
    rubber: i32,
    factor: f32,
    parent_index: u32,
    parent_generation: u32,
    has_parent: i32,
) -> i32 {
    let inner = unsafe { inner(session) };
    let parent = (has_parent != 0).then_some(driver_id(parent_index, parent_generation));
    i32::from(!inner.drivers.borrow_mut().scroll_configure(
        driver_id(index, generation),
        ScrollConfig {
            content_min,
            pull_mode: PullMode::from_code(pull_mode),
            threshold,
            rubber: rubber != 0,
            factor,
        },
        parent,
    ))
}

/// # Safety
/// The four float pointers are writable.
pub unsafe extern "C" fn scroll_drag_by(
    session: *mut c_void,
    index: u32,
    generation: u32,
    dx: f32,
    dy: f32,
    overflow_x: *mut f32,
    overflow_y: *mut f32,
    over_x: *mut f32,
    over_y: *mut f32,
) -> i32 {
    if overflow_x.is_null() || overflow_y.is_null() || over_x.is_null() || over_y.is_null() {
        return -1;
    }
    let inner = unsafe { inner(session) };
    let mut drivers = inner.drivers.borrow_mut();
    let mut tree = inner.tree.borrow_mut();
    let Some(step) = drivers.scroll_drag_by(driver_id(index, generation), &mut tree, dx, dy) else {
        return -1;
    };
    unsafe {
        *overflow_x = step.overflow_x;
        *overflow_y = step.overflow_y;
        *over_x = step.over_x;
        *over_y = step.over_y;
    }
    i32::from(step.pending)
}

/// # Safety
/// `session` is the live `Inner`.
pub unsafe extern "C" fn scroll_finish(
    session: *mut c_void,
    index: u32,
    generation: u32,
    taken: i32,
) -> i32 {
    let inner = unsafe { inner(session) };
    let mut drivers = inner.drivers.borrow_mut();
    let mut tree = inner.tree.borrow_mut();
    if drivers
        .scroll_finish(driver_id(index, generation), &mut tree, taken != 0)
        .is_none()
    {
        return -1;
    }
    0
}

/// # Safety
/// `session` is the live `Inner`.
pub unsafe extern "C" fn scroll_drag_end(session: *mut c_void, index: u32, generation: u32) -> i32 {
    let inner = unsafe { inner(session) };
    if inner
        .drivers
        .borrow_mut()
        .scroll_drag_end(driver_id(index, generation))
        .is_none()
    {
        return -1;
    }
    0
}

/// # Safety
/// `session` is the live `Inner`.
pub unsafe extern "C" fn scroll_wheel(
    session: *mut c_void,
    index: u32,
    generation: u32,
    dx: f32,
    dy: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    let mut drivers = inner.drivers.borrow_mut();
    let mut tree = inner.tree.borrow_mut();
    i32::from(!drivers.scroll_wheel(driver_id(index, generation), &mut tree, dx, dy))
}

/// # Safety
/// `x` and `y` are writable.
pub unsafe extern "C" fn scroll_query(
    session: *mut c_void,
    index: u32,
    generation: u32,
    cmd: i32,
    x: *mut f32,
    y: *mut f32,
) -> i32 {
    if x.is_null() || y.is_null() {
        return -1;
    }
    let inner = unsafe { inner(session) };
    let drivers = inner.drivers.borrow();
    let tree = inner.tree.borrow();
    let Some((qx, qy)) = drivers.scroll_query(driver_id(index, generation), &tree, cmd) else {
        return -1;
    };
    unsafe {
        *x = qx;
        *y = qy;
    }
    0
}

/// # Safety
/// `session` is the live `Inner`.
pub unsafe extern "C" fn scroll_consume(
    session: *mut c_void,
    index: u32,
    generation: u32,
    ox: f32,
    oy: f32,
) -> i32 {
    let inner = unsafe { inner(session) };
    let mut drivers = inner.drivers.borrow_mut();
    let mut tree = inner.tree.borrow_mut();
    match drivers.scroll_consume(driver_id(index, generation), &mut tree, ox, oy) {
        Some(true) => 1,
        Some(false) => 0,
        None => -1,
    }
}

/// # Safety
/// `nodes` is readable for `count` elements when non-null.
unsafe fn read_nodes(nodes: *const u64, count: i32) -> ListNodes {
    let mut raw = [0u64; 10];
    let len = usize::try_from(count).unwrap_or(0).min(raw.len());
    if !nodes.is_null() && len > 0 {
        raw[..len].copy_from_slice(unsafe { std::slice::from_raw_parts(nodes, len) });
    }
    ListNodes {
        viewport: node_of(raw[0]),
        content: node_of(raw[1]),
        top: node_of(raw[2]),
        bottom: node_of(raw[3]),
        rows: node_of(raw[4]),
        v_thumb: node_of(raw[5]),
        h_thumb: node_of(raw[6]),
        pin: node_of(raw[7]),
        v_size: node_of(raw[8]),
        h_size: node_of(raw[9]),
    }
}
