//! Picture replay through nested effect groups and clipped draw scopes.

use super::{Frame, Image, ImageSource, PictureReplay, Status, intersect, merge_clip};
use crate::batch;
use crate::compiler::{self, Affine, Clip, Draw, Effect, Group, IDENTITY, PictureBounds, Scope};
use crate::pipelines::BlendKey;

struct ScopeSpan<'a> {
    scopes: &'a [Scope],
    first: usize,
    depth: usize,
}

impl Frame<'_> {
    pub(super) fn replay_picture(
        &mut self,
        replay: &PictureReplay<'_>,
        target: &Image,
        clip: &Clip,
    ) -> Result<(), Status> {
        let picture = replay.picture;
        let origin = replay.origin;
        if picture.scope_blocks.is_empty()
            || origin[..4] != IDENTITY[..4]
            || self.scene.scale < picture.scale
        {
            return self.scopes(replay, &picture.scopes, 0, target, clip, 0);
        }
        let block_size = compiler::SCOPES_PER_BLOCK;
        for (block, (&bounds, scopes)) in picture
            .scope_blocks
            .iter()
            .zip(picture.scopes.chunks(block_size))
            .enumerate()
        {
            let bounds = bounds.transform(origin).expand(1.0 / self.scene.scale);
            if self.area(bounds, target).is_some() {
                self.scopes(replay, scopes, 0, target, clip, block * block_size)?;
            }
        }
        Ok(())
    }

    fn scopes(
        &mut self,
        replay: &PictureReplay<'_>,
        mut scopes: &[Scope],
        depth: usize,
        target: &Image,
        parent_clip: &Clip,
        first_scope: usize,
    ) -> Result<(), Status> {
        let scope_end = first_scope + scopes.len();
        while let Some((scope, rest)) = scopes.split_first() {
            let scope_index = scope_end - scopes.len();
            let state = &scope.state;
            if let Some(id) = state.groups.get(depth) {
                let length = scopes
                    .iter()
                    .take_while(|scope| scope.state.groups.get(depth) == Some(id))
                    .count();
                let (group_scopes, remaining) = scopes.split_at(length);
                self.replay_group(
                    replay,
                    ScopeSpan {
                        scopes: group_scopes,
                        first: scope_index,
                        depth,
                    },
                    *id,
                    target,
                    parent_clip,
                )?;
                scopes = remaining;
            } else {
                self.replay_scope(replay, scope, target, parent_clip)?;
                scopes = rest;
            }
        }
        Ok(())
    }

    fn replay_group(
        &mut self,
        replay: &PictureReplay<'_>,
        span: ScopeSpan<'_>,
        id: usize,
        target: &Image,
        parent_clip: &Clip,
    ) -> Result<(), Status> {
        let ScopeSpan {
            scopes: group_scopes,
            first: scope_index,
            depth,
        } = span;
        let origin = replay.origin;
        let group = replay.group(id)?;
        let transform = compiler::compose(origin, group.transform);
        let clip = merge_clip(parent_clip, &group_scopes[0].state.clip, origin);
        if group.effect == Effect::Opacity
            && group.opacity == 1.0
            && group_scopes
                .iter()
                .filter(|scope| !scope.draws.is_empty())
                .all(|scope| {
                    scope.state.groups.len() == depth + 1
                        && scope.draws.iter().all(Draw::source_over)
                })
        {
            return self.scopes(
                replay,
                group_scopes,
                depth + 1,
                target,
                parent_clip,
                scope_index,
            );
        }
        if self.fold_opacity(group, group_scopes, depth + 1, target, origin, parent_clip)? {
            return Ok(());
        }
        let render = |frame: &mut Self, target: &Image| {
            frame.group(target, transform, &clip, group, |frame, target| {
                frame.scopes(
                    replay,
                    group_scopes,
                    depth + 1,
                    target,
                    parent_clip,
                    scope_index,
                )
            })
        };
        render(self, target)
    }

    fn replay_scope(
        &mut self,
        replay: &PictureReplay<'_>,
        scope: &Scope,
        target: &Image,
        parent_clip: &Clip,
    ) -> Result<(), Status> {
        let origin = replay.origin;
        let state = &scope.state;
        if scope.draws.is_empty()
            || self
                .area(
                    scope
                        .bounds_in(origin, self.scene.scale)
                        .expand(1.0 / self.scene.scale),
                    target,
                )
                .is_none()
        {
            return Ok(());
        }
        let transform = compiler::compose(origin, state.transform);
        let clip = merge_clip(parent_clip, &state.clip, origin);
        let coverage = self.coverage(target, &clip)?;
        if coverage.empty {
            return Ok(());
        }
        self.draws(replay.layer, target, transform, &coverage, &scope.draws)
    }

    pub(super) fn fold_opacity(
        &mut self,
        group: Group,
        scopes: &[Scope],
        expected_groups: usize,
        target: &Image,
        origin: Affine,
        parent_clip: &Clip,
    ) -> Result<bool, Status> {
        if group.effect != Effect::Opacity || group.opacity <= 0.0 {
            return Ok(false);
        }
        if !can_fold_opacity(scopes, expected_groups, origin, self.scene.scale) {
            return Ok(false);
        }
        for scope in scopes.iter().filter(|scope| !scope.draws.is_empty()) {
            let transform = compiler::compose(origin, scope.state.transform);
            let clip = merge_clip(parent_clip, &scope.state.clip, origin);
            let coverage = self.coverage(target, &clip)?;
            if coverage.empty {
                continue;
            }
            for draw in &scope.draws {
                match draw {
                    Draw::Rects(run) => {
                        let mut rects = run.rects.clone();
                        for rect in &mut rects {
                            rect.gradient_b[3] *= group.opacity;
                        }
                        self.rects(target, transform, &coverage, &rects, run.stops())?;
                    }
                    Draw::Images {
                        source, instances, ..
                    } => {
                        if let Some(image) = self.scene.textures.get(source) {
                            let mut quads = instances.clone();
                            for quad in &mut quads {
                                quad.color = quad.color.map(|channel| channel * group.opacity);
                            }
                            self.images(
                                target,
                                transform,
                                &coverage,
                                &quads,
                                ImageSource::External(image),
                                BlendKey::SrcOver,
                            )?;
                        }
                    }
                    Draw::Glyphs(glyphs) => {
                        let mut quads = glyphs.clone();
                        for glyph in &mut quads {
                            glyph.color = glyph.color.map(|bits| {
                                half::f16::from_f32(
                                    half::f16::from_bits(bits).to_f32() * group.opacity,
                                )
                                .to_bits()
                            });
                        }
                        self.glyphs(target, transform, &coverage, &quads)?;
                    }
                    _ => return Err(Status::BadScene),
                }
            }
        }
        Ok(true)
    }
}

fn can_fold_opacity(scopes: &[Scope], expected_groups: usize, origin: Affine, scale: f32) -> bool {
    let mut bounds = Vec::new();
    for scope in scopes.iter().filter(|scope| !scope.draws.is_empty()) {
        if scope.state.groups.len() != expected_groups {
            return false;
        }
        let transform = compiler::compose(origin, scope.state.transform);
        let margin = 1.0 / scale;
        let mut add = |rect: [f32; 4], stroke: f32| {
            if bounds.len() >= 32 {
                return false;
            }
            let PictureBounds::Rect(rect) = PictureBounds::rect(rect)
                .expand(stroke)
                .transform(transform)
                .expand(margin)
            else {
                return false;
            };
            if bounds.iter().any(|&previous| {
                intersect(previous, rect)[2..]
                    .iter()
                    .all(|&size| size > 0.0)
            }) {
                return false;
            }
            bounds.push(rect);
            true
        };
        for draw in &scope.draws {
            let compatible = match draw {
                Draw::Rects(run) if run.blend == BlendKey::SrcOver => {
                    run.rects.iter().all(|rect| {
                        matches!(rect.kind, batch::RECT_FILL | batch::RECT_STROKE)
                            && add(rect.rect, rect.stroke_width * 0.5)
                    })
                }
                Draw::Images {
                    instances,
                    blend: BlendKey::SrcOver,
                    ..
                } => instances.iter().all(|instance| add(instance.rect, 0.0)),
                Draw::Glyphs(glyphs) => glyphs.iter().all(|glyph| {
                    let pad = glyph.size * 2.0 + 1.0;
                    glyph.gradient.is_none()
                        && add(
                            glyph.decoration.unwrap_or([
                                glyph.x - pad,
                                glyph.y - pad,
                                2.0 * pad,
                                2.0 * pad,
                            ]),
                            0.0,
                        )
                }),
                _ => false,
            };
            if !compatible {
                return false;
            }
        }
    }
    true
}
