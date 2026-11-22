//! Splits Impeller picture runs at UIKit placement markers.

use std::collections::HashSet;
use std::ffi::c_void;
use std::ptr;

use engine::scene::{LayerId, LayerKind, LayerTree};

use super::Rasterizer;
use crate::Status;
use crate::painter::Painter;

#[derive(Default)]
pub(super) struct Slices {
    views: Vec<u64>,
    lists: Vec<impeller::ImpellerDisplayList>,
}

impl Drop for Slices {
    fn drop(&mut self) {
        for list in self.lists.drain(..) {
            // SAFETY: each list is owned exclusively by this slice set.
            unsafe { impeller::ImpellerDisplayListRelease(list) };
        }
    }
}

impl Slices {
    pub(super) fn build(
        tree: &LayerTree,
        painter: &mut Painter<'_>,
        point_scale: f32,
    ) -> Result<Self, Status> {
        if !tree
            .layers
            .iter()
            .any(|layer| matches!(layer.kind, LayerKind::PlatformView { .. }))
        {
            return Ok(Self::default());
        }
        let mut slices = Self::default();
        let mut sections = vec![HashSet::<LayerId>::new()];
        let mut stack = vec![tree.root];
        while let Some(id) = stack.pop() {
            let layer = tree.layers.get(id.0 as usize).ok_or(Status::BadScene)?;
            stack.extend(layer.next);
            stack.extend(layer.first_child);
            match layer.kind {
                LayerKind::PlatformView { node } => {
                    slices.views.push(node);
                    sections.push(HashSet::new());
                }
                LayerKind::Picture { .. } => {
                    let subset = sections.last_mut().ok_or(Status::BadScene)?;
                    let mut parent = Some(id);
                    while let Some(id) = parent {
                        if !subset.insert(id) {
                            break;
                        }
                        parent = tree
                            .layers
                            .get(id.0 as usize)
                            .ok_or(Status::BadScene)?
                            .parent;
                    }
                }
                _ => {}
            }
        }
        for included in sections {
            slices
                .lists
                .push(build_list(tree, painter, point_scale, &included)?);
        }
        Ok(slices)
    }
}

fn build_list(
    tree: &LayerTree,
    painter: &mut Painter<'_>,
    point_scale: f32,
    included: &HashSet<LayerId>,
) -> Result<impeller::ImpellerDisplayList, Status> {
    // SAFETY: constructors accept the default bounds and no retained inputs.
    let (builder, paint) = unsafe {
        (
            impeller::ImpellerDisplayListBuilderNew(ptr::null()),
            impeller::ImpellerPaintNew(),
        )
    };
    let mut list = ptr::null_mut();
    if !builder.is_null() && !paint.is_null() {
        super::paint_color(paint, [0; 4]);
        // SAFETY: both objects were created above. Source replaces the full background with transparent pixels.
        unsafe {
            impeller::ImpellerPaintSetBlendMode(
                paint,
                impeller::ImpellerBlendMode::kImpellerBlendModeSource,
            );
            impeller::ImpellerDisplayListBuilderDrawPaint(builder, paint);
        }
        painter.scale_once(builder, point_scale, false);
        if painter.draw_layers_filtered(tree, builder, Some(tree.root), Some(included))
            && painter.saves.is_empty()
        {
            // SAFETY: builder and its recorded inputs are alive.
            list = unsafe { impeller::ImpellerDisplayListBuilderCreateDisplayListNew(builder) };
        }
        painter.drain_saves(builder);
    }
    // SAFETY: release accepts null; the built display list retains its recorded contents.
    unsafe {
        impeller::ImpellerPaintRelease(paint);
        impeller::ImpellerDisplayListBuilderRelease(builder);
    }
    if list.is_null() {
        Err(Status::BadScene)
    } else {
        Ok(list)
    }
}

impl Rasterizer {
    pub fn metal_platform_views(&self) -> &[u64] {
        &self.slices.views
    }

    /// Records one drawable with no present. The host transfers it to UIKit
    /// after all parts of the frame are successfully encoded.
    ///
    /// # Safety
    /// drawable is a retained CAMetalDrawable belonging to this Metal device.
    pub unsafe fn encode_metal(
        &self,
        slice: Option<usize>,
        drawable: *mut c_void,
    ) -> Result<(), Status> {
        let list = match slice {
            Some(index) => *self.slices.lists.get(index).ok_or(Status::BadScene)?,
            None => self.list,
        };
        if self.context.is_null() || list.is_null() {
            return Err(Status::NotReady);
        }
        // SAFETY: the caller retains the drawable and this engine owns its context.
        let surface = unsafe {
            impeller::ImpellerSurfaceCreateWrappedMetalDrawableNew(self.context, drawable)
        };
        if surface.is_null() {
            return Err(Status::PresentFailed);
        }
        // SAFETY: surface and list are both live through encoding.
        let drew = unsafe {
            let drew = impeller::ImpellerSurfaceDrawDisplayList(surface, list);
            impeller::ImpellerSurfaceRelease(surface);
            drew
        };
        if drew {
            Ok(())
        } else {
            Err(Status::PresentFailed)
        }
    }

    pub fn wait_for_metal_schedule(&self) -> Result<(), Status> {
        // SAFETY: the engine exclusively owns its Impeller context.
        if unsafe { impeller::ImpastoMetalWaitUntilScheduled(self.context) } {
            Ok(())
        } else {
            Err(Status::PresentFailed)
        }
    }
}
