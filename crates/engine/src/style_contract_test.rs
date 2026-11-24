#[cfg(test)]
mod tests {
    use crate::style::{
        AbsoluteLength, Declaration, Environment, Length, PaintColor, Patch, Property, SizeValue,
        Style, StyleOrigin,
    };
    use crate::{text::MonoShaper, tree::Tree};
    fn style(properties: Vec<Property>) -> Style {
        Style::fragment(properties.into_iter().map(Declaration::set).collect()).unwrap()
    }
    fn width(points: f32) -> Style {
        style(vec![Property::Width(SizeValue::Length(Length::points(
            points,
        )))])
    }
    #[test]
    fn source_handles_must_identify_the_owner() {
        let mut tree = Tree::new(MonoShaper::default());
        let a = tree.view().unwrap();
        let b = tree.view().unwrap();
        let sa = tree.create_style_source(a, StyleOrigin::Inline).unwrap();
        let sb = tree.create_style_source(b, StyleOrigin::Inline).unwrap();
        tree.replace_style_source(sa, width(11.0)).unwrap();
        tree.replace_style_source(sb, width(22.0)).unwrap();
        tree.add_child(a, b).unwrap();
        tree.layout().unwrap();
        assert_eq!(
            (tree.rect(a).unwrap().w, tree.rect(b).unwrap().w),
            (11.0, 22.0),
            "handles: {sa:?}, {sb:?}"
        );
    }
    #[test]
    fn clear_background_restores_initial_paint() {
        let mut tree = Tree::new(MonoShaper::default());
        let view = tree.view().unwrap();
        tree.set_size(view, 40.0, 40.0).unwrap();
        tree.set_style(
            view,
            style(vec![Property::BackgroundColor(PaintColor::rgba([
                255, 0, 0, 255,
            ]))]),
        )
        .unwrap();
        tree.layout().unwrap();
        assert!(!tree.record().unwrap().commands.is_empty());
        tree.clear_style(view, None).unwrap();
        tree.layout().unwrap();
        assert!(
            tree.record().unwrap().commands.is_empty(),
            "background survives clear_style"
        );
    }
    #[test]
    fn clear_font_size_restores_initial_metrics() {
        let mut tree = Tree::new(MonoShaper::default());
        let text = tree.text("hello", 14.0, [0, 0, 0, 255]).unwrap();
        tree.set_style(
            text,
            style(vec![Property::FontSize(AbsoluteLength::points(30.0))]),
        )
        .unwrap();
        tree.clear_style(text, None).unwrap();
        tree.layout().unwrap();
        let commands = tree.record().unwrap().commands;
        let size = commands.iter().find_map(|c| match c {
            crate::scene::Command::Text { shaped, .. } => shaped
                .as_ref()
                .and_then(|shaped| shaped.runs.first())
                .map(|run| run.font_size),
            _ => None,
        });
        assert_eq!(size, Some(14.0));
    }
    #[test]
    fn failed_style_must_not_poison_later_patches() {
        let mut tree = Tree::new(MonoShaper::default());
        let view = tree.view().unwrap();
        tree.set_style(view, width(20.0)).unwrap();
        let bad = style(vec![
            Property::Width(SizeValue::Length(Length::points(99.0))),
            Property::BackgroundColor(PaintColor::token("does-not-exist")),
        ]);
        assert!(tree.set_style(view, bad).is_err());
        tree.layout().unwrap();
        assert_eq!(
            tree.rect(view).unwrap().w,
            20.0,
            "failed write committed layout"
        );
        let mut patch = Patch::new();
        patch
            .set(Property::Height(SizeValue::Length(Length::points(10.0))))
            .unwrap();
        assert!(
            tree.patch_style(view, patch).is_ok(),
            "failed write poisoned source"
        );
    }
    #[test]
    fn inline_rem_tracks_environment_changes() {
        let mut tree = Tree::new(MonoShaper::default());
        let view = tree.view().unwrap();
        tree.set_style(
            view,
            style(vec![Property::Width(SizeValue::Length(Length::rem(1.0)))]),
        )
        .unwrap();
        tree.layout().unwrap();
        assert_eq!(tree.rect(view).unwrap().w, 16.0);
        let mut env = Environment::light();
        env.root_rem = 32.0;
        tree.set_style_environment(env).unwrap();
        tree.flush_styles().unwrap();
        tree.layout().unwrap();
        assert_eq!(tree.rect(view).unwrap().w, 32.0);
    }
    #[test]
    fn style_blur_must_reach_draw_list() {
        let mut tree = Tree::new(MonoShaper::default());
        let view = tree.view().unwrap();
        tree.set_size(view, 40.0, 40.0).unwrap();
        tree.set_style(
            view,
            style(vec![
                Property::BackgroundColor(PaintColor::rgba([255, 0, 0, 255])),
                Property::Blur(4.0),
            ]),
        )
        .unwrap();
        tree.layout().unwrap();
        let commands = tree.record().unwrap().commands;
        assert!(
            commands.iter().any(|c| format!("{c:?}").contains("Blur")),
            "blur discarded: {commands:?}"
        );
    }
}

#[cfg(test)]
mod geometry {
    use crate::style::{
        Declaration, InsetValue, Length, PaintColor, PositionMode, Property, SizeValue, Style,
    };
    use crate::{input::HitPath, text::MonoShaper, tree::Tree};
    fn style(p: Vec<Property>) -> Style {
        Style::fragment(p.into_iter().map(Declaration::set).collect()).unwrap()
    }
    #[test]
    fn overflow_visible_child_is_hittable_outside_parent() {
        let mut tree = Tree::new(MonoShaper::default());
        let parent = tree.view().unwrap();
        tree.set_size(parent, 40.0, 40.0).unwrap();
        let child = tree.pointer_region().unwrap();
        tree.add_child(parent, child).unwrap();
        tree.set_style(
            child,
            style(vec![
                Property::Position(PositionMode::Absolute),
                Property::Left(InsetValue::Length(Length::points(60.0))),
                Property::Top(InsetValue::Length(Length::points(0.0))),
                Property::Width(SizeValue::Length(Length::points(20.0))),
                Property::Height(SizeValue::Length(Length::points(20.0))),
                Property::BackgroundColor(PaintColor::rgba([255, 0, 0, 255])),
            ]),
        )
        .unwrap();
        tree.layout().unwrap();
        assert_eq!(tree.rect(child).unwrap().x, 60.0);
        let commands = tree.record().unwrap().commands;
        assert!(
            commands
                .iter()
                .any(|c| matches!(c,crate::scene::Command::Rect{x,..} if *x==60.0))
        );
        let mut path = HitPath::default();
        tree.hit_path(65.0, 5.0, &mut path);
        assert_eq!(path.leaf(), Some(child));
    }
    #[test]
    fn layer_opacity_covers_same_node_background() {
        let mut tree = Tree::new(MonoShaper::default());
        let root = tree.view().unwrap();
        tree.set_size(root, 40.0, 40.0).unwrap();
        let child = tree.view().unwrap();
        tree.add_child(root, child).unwrap();
        tree.set_style(
            root,
            style(vec![
                Property::BackgroundColor(PaintColor::rgba([255, 0, 0, 255])),
                Property::Opacity(0.5),
            ]),
        )
        .unwrap();
        tree.set_style(child, style(vec![Property::Opacity(0.5)]))
            .unwrap();
        tree.layout().unwrap();
        tree.record().unwrap();
        let layers = tree.layer_tree();
        let red_picture = layers
            .pictures
            .iter()
            .find(|p| {
                p.commands().iter().any(
                    |c| matches!(c,crate::scene::Command::Rect{color,..} if *color==[255,0,0,255]),
                )
            })
            .unwrap();
        let mut at = layers.layers.iter().position(
            |l| matches!(l.kind,crate::scene::LayerKind::Picture{picture} if picture==red_picture.id),
        );
        let mut opacity = false;
        while let Some(i) = at {
            let l = &layers.layers[i];
            opacity |= matches!(l.kind, crate::scene::LayerKind::Opacity { .. });
            at = l.parent.map(|p| p.0 as usize);
        }
        assert!(
            opacity
                || red_picture
                    .commands()
                    .iter()
                    .any(|c| matches!(c, crate::scene::Command::PushOpacity { .. })),
            "background picture is outside opacity: {layers:?}"
        );
    }
}
#[cfg(test)]
mod states {
    #[test]
    fn focus_variant_restyles_when_focus_moves() {
        let mut tree = crate::tree::Tree::new(crate::text::MonoShaper::default());
        let node = tree.pointer_region().unwrap();
        tree.set_size(node, 40.0, 40.0).unwrap();
        tree.set_focusable(node, true).unwrap();
        tree.set_class(node, "focus:bg-[#ff0000]").unwrap();
        tree.layout().unwrap();
        tree.focus(node).unwrap();
        tree.flush_styles().unwrap();
        tree.layout().unwrap();
        assert!(
            tree.record().unwrap().commands.iter().any(
                |c| matches!(c,crate::scene::Command::Rect{color,..} if *color==[255,0,0,255])
            ),
            "focus did not invalidate focus variant"
        );
    }
}

#[cfg(test)]
mod lifecycle {
    use crate::style::{
        Declaration, Environment, Key, Length, OverflowMode, PaintColor, Patch, Property,
        SizeValue, Style, StyleOrigin,
    };
    use crate::{
        input::{Phase, PointerEvent},
        text::MonoShaper,
        tree::Tree,
    };
    fn style(properties: Vec<Property>) -> Style {
        Style::fragment(properties.into_iter().map(Declaration::set).collect()).unwrap()
    }
    #[test]
    fn cleanup_is_independent_and_stale_sources_never_target_reused_nodes() {
        let mut tree = Tree::new(MonoShaper::default());
        let root = tree.view().unwrap();
        let node = tree.view().unwrap();
        tree.add_child(root, node).unwrap();
        let a = tree.create_style_source(node, StyleOrigin::Inline).unwrap();
        let b = tree.create_style_source(node, StyleOrigin::Inline).unwrap();
        tree.replace_style_source(a, style(vec![Property::Width(SizeValue::points(11.0))]))
            .unwrap();
        tree.replace_style_source(b, style(vec![Property::Height(SizeValue::points(7.0))]))
            .unwrap();
        tree.remove_style_source(a).unwrap();
        tree.layout().unwrap();
        assert_eq!(tree.rect(node).unwrap().h, 7.0);
        tree.remove(root, node).unwrap();
        tree.discard(node).unwrap();
        let reused = tree.view().unwrap();
        tree.add_child(root, reused).unwrap();
        assert!(
            tree.replace_style_source(b, style(vec![Property::Width(SizeValue::points(99.0))]))
                .is_err()
        );
        tree.remove_style_source(b).unwrap();
        tree.layout().unwrap();
        assert_ne!(tree.rect(reused).unwrap().w, 99.0);
    }
    #[test]
    fn reset_and_clear_restore_transform_opacity_and_clip() {
        let mut tree = Tree::new(MonoShaper::default());
        let root = tree.view().unwrap();
        tree.set_size(root, 40.0, 40.0).unwrap();
        tree.set_style(
            root,
            style(vec![
                Property::BackgroundColor(PaintColor::rgba([255, 0, 0, 255])),
                Property::Opacity(0.5),
                Property::TranslateX(Length::points(7.0)),
                Property::Overflow(OverflowMode::Clip),
            ]),
        )
        .unwrap();
        let mut reset = Patch::new();
        for key in [
            Key::BackgroundColor,
            Key::Opacity,
            Key::TranslateX,
            Key::Overflow,
        ] {
            reset.reset(key);
        }
        tree.patch_style(root, reset).unwrap();
        tree.layout().unwrap();
        assert!(tree.record().unwrap().commands.is_empty());
        tree.set_class(root, "bg-[#0000ff]").unwrap();
        tree.clear_style(root, None).unwrap();
        tree.layout().unwrap();
        let list = tree.record().unwrap();
        assert!(
            list.commands.iter().any(
                |c| matches!(c,crate::scene::Command::Rect{color,..} if *color==[0,0,255,255])
            )
        );
        assert!(!list.commands.iter().any(|c| matches!(
            c,
            crate::scene::Command::PushTransform { .. }
                | crate::scene::Command::PushOpacity { .. }
                | crate::scene::Command::PushClip { .. }
        )));
    }
    #[test]
    fn theme_and_spacing_restyle_every_origin_without_poisoning_environment() {
        let mut tree = Tree::new(MonoShaper::default());
        let node = tree.view().unwrap();
        tree.set_class(node, "p-4").unwrap();
        tree.set_inline_style(
            node,
            style(vec![Property::BackgroundColor(PaintColor::token(
                "surface",
            ))]),
        )
        .unwrap();
        let mut env = Environment::dark();
        env.spacing = 8.0;
        tree.set_style_environment(env).unwrap();
        tree.layout().unwrap();
        assert_eq!(tree.rect(node).unwrap().w, 64.0);
        let before = tree.record().unwrap();
        let mut invalid = Environment::light();
        invalid.install_palette(&[]);
        assert!(tree.set_style_environment(invalid).is_err());
        tree.layout().unwrap();
        assert_eq!(tree.record().unwrap(), before);
    }
    #[test]
    fn hover_active_and_focus_are_live_and_paint_only() {
        let mut tree = Tree::new(MonoShaper::default());
        let node = tree.pointer_region().unwrap();
        tree.set_size(node, 40.0, 40.0).unwrap();
        tree.set_class(node, "bg-[#000000] hover:bg-[#ff0000] active:bg-[#0000ff]")
            .unwrap();
        tree.layout().unwrap();
        let passes = tree.layout_passes();
        tree.pointer(PointerEvent::new(Phase::Move, 5.0, 5.0, 0.0));
        tree.layout().unwrap();
        assert!(
            tree.record().unwrap().commands.iter().any(
                |c| matches!(c,crate::scene::Command::Rect{color,..} if *color==[255,0,0,255])
            )
        );
        tree.pointer(PointerEvent::new(Phase::Down, 5.0, 5.0, 1.0));
        tree.layout().unwrap();
        assert!(
            tree.record().unwrap().commands.iter().any(
                |c| matches!(c,crate::scene::Command::Rect{color,..} if *color==[0,0,255,255])
            )
        );
        assert_eq!(tree.layout_passes(), passes);
    }
    #[test]
    fn opacity_updates_reuse_pictures_and_match_flat_layers() {
        let mut tree = Tree::new(MonoShaper::default());
        let node = tree.view().unwrap();
        tree.set_size(node, 40.0, 40.0).unwrap();
        tree.set_style(
            node,
            style(vec![
                Property::BackgroundColor(PaintColor::rgba([255, 0, 0, 255])),
                Property::Opacity(0.5),
                Property::Overflow(OverflowMode::Clip),
            ]),
        )
        .unwrap();
        tree.layout().unwrap();
        let list = tree.record().unwrap();
        crate::layout_test::agree_list(&tree, &list).unwrap();
        let mut patch = Patch::new();
        patch.set(Property::Opacity(0.75)).unwrap();
        tree.patch_style(node, patch).unwrap();
        let list = tree.record().unwrap();
        crate::layout_test::agree_list(&tree, &list).unwrap();
        assert_eq!(tree.picture_writes(), 0);
    }

    #[test]
    fn surfaces_keep_structure_and_image_source_skips_author_size() {
        use crate::style::{DisplayMode, OverflowMode, PaintColor, StyleSurface, TrackList};
        let mut tree = Tree::new(MonoShaper::default());
        let stack = tree.stack().unwrap();
        let tracks = style(vec![Property::GridTemplateColumns(TrackList::source(
            "minmax(0, 1fr)",
        ))]);
        assert!(matches!(
            tree.set_inline_style(stack, tracks.clone()),
            Err(crate::Error::Style(crate::style::StyleError::Forbidden { key, .. }))
                if key == "gridTemplateColumns"
        ));
        tree.set_inline_style(stack, style(vec![Property::Display(DisplayMode::None)]))
            .unwrap();
        tree.set_component_style(stack, tracks).unwrap();

        let scroll = tree.scroll().unwrap();
        assert!(
            tree.set_inline_style(
                scroll,
                style(vec![Property::Overflow(OverflowMode::Visible)])
            )
            .is_err()
        );

        let blocked = tree.view().unwrap();
        tree.set_inline_style(
            blocked,
            style(vec![Property::Width(SizeValue::Length(Length::points(
                5.0,
            )))]),
        )
        .unwrap();
        assert!(
            tree.set_style_surface(blocked, StyleSurface::ListContent)
                .is_err()
        );
        let rows = tree.view().unwrap();
        tree.set_style_surface(rows, StyleSurface::ListContent)
            .unwrap();
        tree.set_inline_style(
            rows,
            style(vec![Property::Color(PaintColor::rgba([1, 2, 3, 255]))]),
        )
        .unwrap();
        assert!(matches!(
            tree.set_inline_style(
                rows,
                style(vec![Property::Width(SizeValue::Length(Length::points(12.0)))]),
            ),
            Err(crate::Error::Style(crate::style::StyleError::Forbidden { key, .. }))
                if key == "width"
        ));
        assert!(tree.set_class(rows, "p-4").is_err());

        let mut pictures = Tree::new(MonoShaper::default());
        let image = pictures.image("asset:a", 10.0, 12.0).unwrap();
        pictures.set_image(image, "asset:b", None).unwrap();
        pictures.layout().unwrap();
        assert_eq!(
            (
                pictures.rect(image).unwrap().w,
                pictures.rect(image).unwrap().h
            ),
            (10.0, 12.0)
        );
        pictures
            .set_inline_style(
                image,
                style(vec![Property::Width(SizeValue::Length(Length::points(
                    40.0,
                )))]),
            )
            .unwrap();
        pictures
            .set_image(image, "asset:c", Some((8.0, 9.0)))
            .unwrap();
        pictures.layout().unwrap();
        assert_eq!(pictures.rect(image).unwrap().w, 40.0);
    }
}
