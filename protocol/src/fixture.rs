use crate::Scene;

/// The hand-placed counter scene. Its encoded bytes are `tests/fixtures/counter.bin`.
pub fn counter() -> Scene {
    build().unwrap_or_default()
}

fn build() -> Result<Scene, crate::Error> {
    let mut scene = Scene::new();
    let layer = scene.layer(0)?;
    scene.rect(layer, 0.0, 0.0, 320.0, 180.0, [17, 17, 17, 255])?;
    scene.text(layer, 24.0, 36.0, "Count", [255, 255, 255, 255])?;
    scene.text(layer, 24.0, 72.0, "0", [124, 255, 107, 255])?;
    scene.image(layer, 240.0, 24.0, 56.0, 56.0, "asset:logo")?;
    Ok(scene)
}
