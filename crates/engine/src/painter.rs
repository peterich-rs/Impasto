//! Per-submit display-list builder. Save stack and pending paragraphs end with the submit.

use std::ptr;

use protocol::Picture;

use crate::pictures::PictureCache;

use crate::player::{HeldParagraph, RetainedPicture, Save};
use crate::textures::TextureStore;
use crate::typography::Typography;
use crate::{ImagePixels, Status};

pub(crate) struct Painter<'a> {
    pub(crate) typography: &'a mut Typography,
    pub(crate) textures: &'a mut TextureStore,
    pub(crate) picture_cache: Option<&'a PictureCache>,
    pub(crate) pending: &'a mut Vec<HeldParagraph>,
    pub(crate) saves: Vec<Save>,
    /// Canvas image sources skipped because the texture was not ready.
    pub(crate) waiting: Vec<std::sync::Arc<str>>,
    pub(crate) impeller: impeller::ImpellerContext,
}

pub(crate) fn build_picture<'pixels>(
    picture: &Picture,
    point_scale: f32,
    resolve: &mut impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    typography: &mut Typography,
    textures: &mut TextureStore,
    pending: &mut Vec<HeldParagraph>,
    impeller: impeller::ImpellerContext,
) -> Result<RetainedPicture, Status> {
    let saved = std::mem::take(pending);
    let builder = unsafe { impeller::ImpellerDisplayListBuilderNew(ptr::null()) };
    let paint = unsafe { impeller::ImpellerPaintNew() };
    if builder.is_null() || paint.is_null() {
        unsafe {
            impeller::ImpellerDisplayListBuilderRelease(builder);
            impeller::ImpellerPaintRelease(paint);
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
        waiting: Vec::new(),
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
        unsafe { impeller::ImpellerDisplayListBuilderCreateDisplayListNew(builder) }
    };
    painter.drain_saves(builder);
    unsafe {
        impeller::ImpellerDisplayListBuilderRelease(builder);
        impeller::ImpellerPaintRelease(paint);
    }
    let held = std::mem::take(painter.pending);
    *painter.pending = saved;
    if list.is_null() {
        for item in held {
            painter.typography.release_draw(item.slot);
            // SAFETY: this retain was taken for a display list that was not installed.
            unsafe { impeller::ImpellerParagraphRelease(item.paragraph) };
        }
        return Err(Status::BadScene);
    }
    Ok(RetainedPicture {
        generation: picture.generation,
        has_external_texture: picture.commands.iter().any(|command| match command {
            protocol::Command::Image { source, .. } => {
                source.starts_with(protocol::EXTERNAL_TEXTURE_PREFIX)
            }
            protocol::Command::Canvas { list, .. } => list
                .images()
                .iter()
                .any(|image| image.source.starts_with(protocol::EXTERNAL_TEXTURE_PREFIX)),
            _ => false,
        }),
        list,
        held,
        waiting: painter.waiting,
    })
}

pub(crate) fn release_pending(pending: &mut Vec<HeldParagraph>, typography: &mut Typography) {
    for item in pending.drain(..) {
        typography.release_draw(item.slot);
        // SAFETY: the failed builder never installed a display list for this retain.
        unsafe { impeller::ImpellerParagraphRelease(item.paragraph) };
    }
}
