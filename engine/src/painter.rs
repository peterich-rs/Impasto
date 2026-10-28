//! Per-submit display-list builder. Save stack and pending paragraphs end with the submit.

use std::ptr;

use protocol::Picture;

use crate::pictures::PictureCache;
use crate::player::ffi;
use crate::player::{RetainedPicture, Save};
use crate::textures::TextureStore;
use crate::typography::Typography;
use crate::{ImagePixels, Status};

pub(crate) struct Painter<'a> {
    pub(crate) typography: &'a mut Typography,
    pub(crate) textures: &'a mut TextureStore,
    pub(crate) picture_cache: Option<&'a PictureCache>,
    pub(crate) pending: &'a mut Vec<ffi::ImpellerParagraph>,
    pub(crate) saves: Vec<Save>,
    pub(crate) impeller: ffi::ImpellerContext,
}

pub(crate) fn build_picture<'pixels>(
    picture: &Picture,
    point_scale: f32,
    resolve: &mut impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    typography: &mut Typography,
    textures: &mut TextureStore,
    pending: &mut Vec<ffi::ImpellerParagraph>,
    impeller: ffi::ImpellerContext,
) -> Result<RetainedPicture, Status> {
    let saved = std::mem::take(pending);
    let builder = unsafe { ffi::ImpellerDisplayListBuilderNew(ptr::null()) };
    let paint = unsafe { ffi::ImpellerPaintNew() };
    if builder.is_null() || paint.is_null() {
        unsafe {
            ffi::ImpellerDisplayListBuilderRelease(builder);
            ffi::ImpellerPaintRelease(paint);
        }
        *pending = saved;
        return Err(Status::BadScene);
    }
    let mut painter = Painter {
        typography,
        textures,
        picture_cache: None,
        pending,
        saves: Vec::new(),
        impeller,
    };
    let drawn = painter.play_commands(
        builder,
        paint,
        &picture.commands,
        point_scale,
        false,
        resolve,
    );
    let failed = !drawn || !painter.saves.is_empty();
    let list = if failed {
        ptr::null_mut()
    } else {
        unsafe { ffi::ImpellerDisplayListBuilderCreateDisplayListNew(builder) }
    };
    painter.drain_saves(builder);
    unsafe {
        ffi::ImpellerDisplayListBuilderRelease(builder);
        ffi::ImpellerPaintRelease(paint);
    }
    let held = std::mem::take(painter.pending);
    *painter.pending = saved;
    if list.is_null() {
        for paragraph in held {
            unsafe { ffi::ImpellerParagraphRelease(paragraph) };
        }
        return Err(Status::BadScene);
    }
    Ok(RetainedPicture {
        generation: picture.generation,
        list,
        held,
    })
}

pub(crate) fn release_pending(pending: &mut Vec<ffi::ImpellerParagraph>) {
    for paragraph in pending.drain(..) {
        // SAFETY: the failed builder never installed a display list for these.
        unsafe { ffi::ImpellerParagraphRelease(paragraph) };
    }
}
