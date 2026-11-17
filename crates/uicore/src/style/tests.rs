use super::codec::{Value, from_value};
use super::environment::{Environment, Interaction};
use super::property::{Impact, Key, expand_flex, expand_padding};
use super::values::{BasisValue, FlexDir, FlexShorthand, Length, SizeValue};
use super::{Declaration, Property, Style, StyleError, flatten, merge_layers, parse_class};

fn set(property: Property) -> Declaration {
    Declaration::set(property)
}

#[test]
fn padding_longhand_beats_shorthand_in_one_object() {
    let style = from_value(&Value::Object(vec![
        ("padding".into(), Value::Number(16.0)),
        ("paddingLeft".into(), Value::Number(8.0)),
    ]))
    .expect("padding");
    let map = flatten(&style);
    assert_eq!(
        map.get(&Key::PaddingTop),
        Some(&set(Property::PaddingTop(Length::points(16.0))))
    );
    assert_eq!(
        map.get(&Key::PaddingLeft),
        Some(&set(Property::PaddingLeft(Length::points(8.0))))
    );
}

#[test]
fn longhand_before_shorthand_still_wins() {
    let style = from_value(&Value::Object(vec![
        ("paddingLeft".into(), Value::Number(8.0)),
        ("padding".into(), Value::Number(16.0)),
    ]))
    .expect("padding");
    let map = flatten(&style);
    assert_eq!(
        map.get(&Key::PaddingLeft),
        Some(&set(Property::PaddingLeft(Length::points(8.0))))
    );
    assert_eq!(
        map.get(&Key::PaddingTop),
        Some(&set(Property::PaddingTop(Length::points(16.0))))
    );
}

#[test]
fn flex_number_is_percent_basis() {
    let decls = expand_flex(FlexShorthand::Number(1.0)).expect("flex");
    let style = Style::fragment(decls).expect("fragment");
    let map = flatten(&style);
    assert_eq!(map.get(&Key::FlexGrow), Some(&set(Property::FlexGrow(1.0))));
    assert_eq!(
        map.get(&Key::FlexShrink),
        Some(&set(Property::FlexShrink(1.0)))
    );
    assert_eq!(
        map.get(&Key::FlexBasis),
        Some(&set(Property::FlexBasis(BasisValue::Size(
            SizeValue::fraction(0.0)
        ))))
    );
}

#[test]
fn flex_col_then_flex_keeps_column() {
    let parsed = parse_class("flex-col flex").expect("class");
    let style = parsed
        .select(&Environment::light(), Interaction::default())
        .expect("select");
    let map = flatten(&style);
    assert_eq!(
        map.get(&Key::FlexDirection),
        Some(&set(Property::FlexDirection(FlexDir::Column)))
    );
}

#[test]
fn null_reset_hides_the_lower_layer() {
    let mut class = Style::new();
    class
        .push_fragment(expand_padding(Length::points(16.0)).expect("pad"))
        .expect("class");
    let mut inline = Style::new();
    inline
        .push_fragment(vec![
            Declaration::reset(Key::PaddingTop),
            Declaration::reset(Key::PaddingRight),
            Declaration::reset(Key::PaddingBottom),
            Declaration::reset(Key::PaddingLeft),
        ])
        .expect("inline");
    let resolved = merge_layers(&[&class, &inline]);
    assert!(matches!(
        resolved.values.get(&Key::PaddingTop),
        Some(super::WinningDeclaration::Initial)
    ));
}

#[test]
fn class_padding_then_horizontal_override() {
    let parsed = parse_class("p-4 px-2").expect("class");
    let style = parsed
        .select(&Environment::default(), Interaction::default())
        .expect("select");
    let map = flatten(&style);
    assert_eq!(
        map.get(&Key::PaddingTop),
        Some(&set(Property::PaddingTop(Length::steps(4.0))))
    );
    assert_eq!(
        map.get(&Key::PaddingLeft),
        Some(&set(Property::PaddingLeft(Length::steps(2.0))))
    );
    assert_eq!(
        map.get(&Key::PaddingRight),
        Some(&set(Property::PaddingRight(Length::steps(2.0))))
    );
}

#[test]
fn unknown_class_token_fails() {
    let error = parse_class("not-a-token").expect_err("token");
    assert!(matches!(error, StyleError::UnknownClass { .. }));
}

#[test]
fn invalid_opacity_does_not_return_a_style() {
    let error = from_value(&Value::Object(vec![("opacity".into(), Value::Number(2.0))]))
        .expect_err("opacity");
    assert!(matches!(error, StyleError::Invalid { .. }));
}

#[test]
fn color_does_not_share_a_layout_impact() {
    assert!(Key::Color.impact().contains(Impact::PAINT));
    assert!(!Key::Color.impact().contains(Impact::LAYOUT));
    assert!(!Key::Color.impact().contains(Impact::MEASURE));
    assert!(Key::FontSize.impact().contains(Impact::MEASURE));
}

#[test]
fn empty_snapshot_clears() {
    assert!(from_value(&Value::Null).expect("empty").is_empty());
    assert!(
        from_value(&Value::Object(vec![]))
            .expect("object")
            .is_empty()
    );
}

#[test]
fn grid_source_accepts_zero_and_rejects_fr_min() {
    use super::property::Property;
    use super::values::TrackList;
    let legal = Property::GridTemplateColumns(TrackList::source("minmax(0, 1fr)"));
    assert!(legal.validate().is_ok());
    let illegal = Property::GridTemplateColumns(TrackList::source("minmax(1fr, 2fr)"));
    assert!(illegal.validate().is_err());
}

#[test]
fn bracket_spaces_stay_inside_one_class_token() {
    let error = parse_class("gap-[4 px]").expect_err("spaced arbitrary");
    match error {
        StyleError::UnknownClass { token, .. } => assert_eq!(token, "gap-[4 px]"),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn class_cache_promotes_a_hit_before_eviction() {
    use super::classes::ClassCache;
    let mut cache = ClassCache::new(2, 1024 * 1024);
    cache.get("flex").expect("flex");
    cache.get("grid").expect("grid");
    cache.get("flex").expect("promote");
    cache.get("hidden").expect("hidden");
    assert!(cache.contains("flex"));
    assert!(cache.contains("hidden"));
    assert!(!cache.contains("grid"));
}
