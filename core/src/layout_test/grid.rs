use super::*;

#[test]
fn grid_puts_two_children_in_two_columns() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let grid = tree.grid(2)?;
    let left = tree.sized(30.0, 10.0)?;
    let right = tree.sized(14.0, 10.0)?;
    tree.add_child(grid, left)?;
    tree.add_child(grid, right)?;
    tree.layout()?;
    let left_rect = tree.rect(left)?;
    let right_rect = tree.rect(right)?;
    assert_eq!(left_rect.x, 0.0);
    assert_eq!(right_rect.x, left_rect.w);
    assert_eq!(right_rect.y, left_rect.y);
    Ok(())
}

#[test]
fn grid_tracks_assign_fixed_and_fraction_columns() -> Result<(), crate::Error> {
    let mut tree = Tree::new(MonoShaper::default());
    let grid = tree.grid_tracks(
        &[TrackSize::Points(40.0), TrackSize::Fraction(1.0)],
        &[TrackSize::MinContent],
        0.0,
    )?;
    let left = tree.column(0.0)?;
    let right = tree.column(0.0)?;
    tree.add_child(grid, left)?;
    tree.add_child(grid, right)?;
    tree.set_fill(grid)?;
    tree.set_viewport(0.0, 0.0, 100.0, 40.0)?;
    tree.layout()?;
    let second = tree.rect(right)?;
    assert_eq!(second.x, 40.0);
    assert_eq!(second.w, 60.0);
    Ok(())
}
