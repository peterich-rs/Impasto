//! Per-session shader registrations and paint-node bindings.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::motion::{AnimProp, Easing, Tween};
use crate::scene::{
    MAX_SHADER_SOURCE_BYTES, MAX_SHADER_UNIFORM_BYTES, ShaderClock, ShaderEffect, ShaderError,
    ShaderProgram, ShaderProgramId, shader,
};

use super::node::NodeFlags;
use super::{NodeId, Tree};

const MAX_PROGRAMS: usize = 64;
const MAX_SOURCE_BYTES: usize = 1024 * 1024;
// Numeric program handles cross QuickJS without rounding. Unique process-wide
// IDs prevent either hot reload or another session reusing a cached pipeline.
const MAX_SHADER_ID: u64 = (1 << 53) - 1;
static NEXT_SHADER_ID: AtomicU64 = AtomicU64::new(1);

impl Tree {
    /// Registers WGSL for this session and reuses an ID for identical registrations.
    ///
    /// # Errors
    /// Returns a [`ShaderError`] for invalid sizes or exhausted registration capacity.
    pub fn register_shader_program(
        &mut self,
        source: &str,
        clock: ShaderClock,
    ) -> Result<ShaderProgramId, ShaderError> {
        if source.len() > MAX_SHADER_SOURCE_BYTES || source.trim().is_empty() {
            return Err(ShaderError::Source);
        }
        let animated = clock == ShaderClock::Frame;
        let programs = &self.layers.shader_programs;
        if let Some(program) = programs
            .iter()
            .find(|program| program.animated == animated && program.source.as_ref() == source)
        {
            return Ok(program.id);
        }
        let bytes = programs
            .iter()
            .map(|program| program.source.len())
            .sum::<usize>();
        if programs.len() >= MAX_PROGRAMS || bytes + source.len() > MAX_SOURCE_BYTES {
            return Err(ShaderError::Capacity);
        }
        let raw = NEXT_SHADER_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
                (id <= MAX_SHADER_ID).then_some(id + 1)
            })
            .map_err(|_| ShaderError::Capacity)?;
        let id = ShaderProgramId(raw);
        let mut registered = programs.to_vec();
        registered.push(ShaderProgram {
            id,
            source: Arc::from(source),
            animated,
        });
        self.layers.shader_programs = Arc::from(registered);
        Ok(id)
    }

    /// Assigns an effect to the content box of a node without invalidating layout.
    ///
    /// # Errors
    /// Returns an error for a stale node, an unknown program, or invalid inputs.
    pub fn set_shader_effect(
        &mut self,
        id: NodeId,
        effect: Option<ShaderEffect>,
    ) -> Result<(), crate::Error> {
        self.node(id)?;
        if let Some(next) = &effect {
            next.validate()?;
            if self.layers.shader_program(next.shader).is_none() {
                return Err(ShaderError::UnknownProgram.into());
            }
        }
        if self.shader_effects.get(&id) == effect.as_ref() {
            return Ok(());
        }
        let was_owner = self.is_picture_owner(id);
        self.node_mut(id)?
            .flags
            .set(NodeFlags::PICTURE_FROM_SHADER, effect.is_some());
        self.motions.retain(|motion| {
            !(motion.node == id && matches!(motion.prop, AnimProp::ShaderUniform(_)))
        });
        let previous = match effect {
            Some(next) => self.shader_effects.insert(id, next),
            None => self.shader_effects.remove(&id),
        };
        let previous_textures = previous
            .as_ref()
            .map_or(&[][..], |effect| effect.textures.as_ref());
        let textures = self
            .shader_effects
            .get(&id)
            .map(|effect| Arc::clone(&effect.textures));
        let next_textures = textures.as_deref().unwrap_or_default();
        if previous_textures != next_textures {
            for source in shader::image_textures(previous_textures) {
                self.note_image(source, -1);
            }
            for source in shader::image_textures(next_textures) {
                self.note_image(source, 1);
            }
        }
        self.repaint_picture_owner(id, was_owner);
        Ok(())
    }

    /// Animates one shader parameter from its current value using the scene clock.
    ///
    /// # Errors
    /// Returns an error when the effect or parameter is missing or the motion is invalid.
    pub fn animate_shader_uniform(
        &mut self,
        id: NodeId,
        index: u8,
        to: f32,
        duration_ms: f64,
        easing: Easing,
    ) -> Result<(), crate::Error> {
        let slot = shader_slot(index).ok_or(ShaderError::Uniforms)?;
        if !to.is_finite() || !duration_ms.is_finite() || duration_ms <= 0.0 {
            return Err(crate::Error::BadNumber);
        }
        let effect = self
            .shader_effects
            .get(&id)
            .ok_or(crate::Error::WrongKind)?;
        let from = effect
            .uniforms
            .get(slot..slot + size_of::<f32>())
            .and_then(|bytes| bytes.try_into().ok())
            .map(f32::from_le_bytes)
            .filter(|value| value.is_finite())
            .unwrap_or(0.0);
        self.animate(
            id,
            AnimProp::ShaderUniform(index),
            Box::new(Tween {
                from,
                to,
                duration_ms,
                easing,
            }),
        );
        self.mark_repaint(id);
        Ok(())
    }

    pub(super) fn apply_shader_uniform(&mut self, id: NodeId, index: u8, value: f32) -> bool {
        let Some(slot) = shader_slot(index).filter(|_| value.is_finite()) else {
            return true;
        };
        let Some(effect) = self.shader_effects.get_mut(&id) else {
            return true;
        };
        let data = value.to_le_bytes();
        let end = slot + data.len();
        if effect.uniforms.get(slot..end) == Some(&data) {
            return false;
        }
        if end > effect.uniforms.len() {
            let mut bytes = effect.uniforms.to_vec();
            bytes.resize(end, 0);
            effect.uniforms = Arc::from(bytes);
        }
        Arc::make_mut(&mut effect.uniforms)[slot..end].copy_from_slice(&data);
        self.mark_repaint(id);
        false
    }

    pub(super) fn remove_shader_effect(&mut self, id: NodeId) {
        if let Some(effect) = self.shader_effects.remove(&id) {
            for source in shader::image_textures(&effect.textures) {
                self.note_image(source, -1);
            }
        }
    }
}

fn shader_slot(index: u8) -> Option<usize> {
    let slot = usize::from(index) * size_of::<f32>();
    (slot < MAX_SHADER_UNIFORM_BYTES).then_some(slot)
}

#[cfg(test)]
mod tests {
    use super::{MAX_PROGRAMS, Tree};
    use crate::motion::Easing;
    use crate::scene::{
        BACKDROP_TEXTURE, Command, CompositorProp, CompositorSnapshot, LayerKind,
        MAX_SHADER_UNIFORM_BYTES, ShaderClock, ShaderEffect, ShaderError,
    };
    use crate::text::MonoShaper;
    use std::sync::Arc;

    #[test]
    fn shader_ids_deduplicate_within_but_not_between_sessions() {
        let mut tree = Tree::new(MonoShaper::default());
        let source = "fn example() {}";
        let first = tree
            .register_shader_program(source, ShaderClock::Static)
            .unwrap();
        assert_eq!(
            tree.register_shader_program(source, ShaderClock::Static),
            Ok(first)
        );
        assert_ne!(
            tree.register_shader_program(source, ShaderClock::Frame),
            Ok(first)
        );
        let mut another = Tree::new(MonoShaper::default());
        assert_ne!(
            another.register_shader_program(source, ShaderClock::Static),
            Ok(first)
        );
        assert_eq!(
            tree.register_shader_program("   ", ShaderClock::Static),
            Err(ShaderError::Source)
        );
        for index in 2..MAX_PROGRAMS {
            tree.register_shader_program(&format!("// {index}"), ShaderClock::Static)
                .unwrap();
        }
        assert_eq!(
            tree.register_shader_program("// other", ShaderClock::Static),
            Err(ShaderError::Capacity)
        );
        assert_eq!(
            tree.register_shader_program(source, ShaderClock::Static),
            Ok(first)
        );
    }

    #[test]
    fn shader_inputs_update_census_and_unregister_on_node_retirement() -> Result<(), crate::Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let root = tree.view()?;
        let node = tree.view()?;
        tree.add_child(root, node)?;
        let shader = tree.register_shader_program("fn f() {}", ShaderClock::Frame)?;
        let image: Arc<str> = Arc::from("asset:photo");
        let effect = ShaderEffect {
            shader,
            uniforms: Arc::from(0.5_f32.to_le_bytes()),
            textures: Arc::from([Arc::from(BACKDROP_TEXTURE), Arc::clone(&image)]),
        };
        tree.set_shader_effect(node, Some(effect.clone()))?;
        assert_eq!(tree.image_source_counts().get(&image), Some(&1));
        assert!(!tree.image_source_counts().contains_key(BACKDROP_TEXTURE));
        let generation = tree.image_generation();
        assert!(matches!(
            tree.set_shader_effect(
                node,
                Some(ShaderEffect {
                    uniforms: Arc::from(vec![0; MAX_SHADER_UNIFORM_BYTES + 4]),
                    ..effect.clone()
                })
            ),
            Err(crate::Error::Shader(ShaderError::Uniforms))
        ));
        tree.set_shader_effect(
            node,
            Some(ShaderEffect {
                uniforms: Arc::from(0.75_f32.to_le_bytes()),
                ..effect
            }),
        )?;
        assert_eq!(tree.image_generation(), generation);
        tree.remove(root, node)?;
        tree.discard(node)?;
        assert!(tree.image_source_counts().is_empty());
        Ok(())
    }

    #[test]
    fn a_picture_owned_shader_stays_inside_its_parent_clip() -> Result<(), crate::Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let root = tree.view()?;
        let clip = tree.clip(8.0)?;
        let node = tree.sized(32.0, 24.0)?;
        tree.add_child(root, clip)?;
        tree.add_child(clip, node)?;
        let shader = tree.register_shader_program("fn f() {}", ShaderClock::Static)?;
        tree.set_shader_effect(
            node,
            Some(ShaderEffect {
                shader,
                uniforms: Arc::default(),
                textures: Arc::default(),
            }),
        )?;
        tree.layout()?;
        tree.record_layers()?;
        let layers = tree.layer_tree();
        let effect = layers
            .layers
            .iter()
            .find(|layer| {
                matches!(layer.kind, LayerKind::Picture { picture }
                    if layers.picture(picture).is_some_and(|picture| picture.commands().iter()
                        .any(|command| matches!(command, Command::ShaderEffect { .. }))))
            })
            .expect("a shader has a recorded picture");
        let offset = &layers.layers[effect.parent.expect("picture offset").0 as usize];
        let clip = &layers.layers[offset.parent.expect("shader clips at its parent").0 as usize];
        assert!(matches!(
            clip.kind,
            LayerKind::Clip {
                corner_radius: 8.0,
                ..
            }
        ));
        assert!(layers.pictures.iter().all(|picture| {
            picture
                .commands()
                .iter()
                .all(|command| !matches!(command, Command::PushClip { .. } | Command::Pop))
        }));
        Ok(())
    }

    #[test]
    fn scalar_tracks_target_one_effect_picture_and_commit_packed_values() -> Result<(), crate::Error>
    {
        let mut tree = Tree::new(MonoShaper::default());
        let root = tree.view()?;
        let node = tree.sized(32.0, 24.0)?;
        tree.add_child(root, node)?;
        let shader = tree.register_shader_program("fn f() {}", ShaderClock::Static)?;
        tree.set_shader_effect(
            node,
            Some(ShaderEffect {
                shader,
                uniforms: Arc::from(0.25_f32.to_le_bytes()),
                textures: Arc::from([]),
            }),
        )?;
        tree.layout()?;
        tree.tick_motions(100.0);
        tree.animate_shader_uniform(node, 0, 0.75, 100.0, Easing::Linear)?;
        tree.record_layers()?;
        let layers = tree.layer_tree();
        let [track] = layers.tracks.as_ref() else {
            panic!("expected one scalar track")
        };
        assert_eq!(track.prop, CompositorProp::ShaderUniform(0));
        let LayerKind::Picture { picture } = layers.layers[track.layer.0 as usize].kind else {
            panic!("a scalar track must target its own picture")
        };
        assert_eq!(
            layers
                .picture(picture)
                .unwrap()
                .commands()
                .iter()
                .filter(|command| matches!(command, Command::ShaderEffect { .. }))
                .count(),
            1
        );
        let mid = track.sample(150.0).expect("still active");
        assert_eq!(mid.value, 0.5);
        tree.commit_compositor_state(&CompositorSnapshot {
            target_ms: 150.0,
            samples: vec![mid],
        });
        assert_eq!(
            tree.shader_effects[&node].uniforms.as_ref(),
            0.5_f32.to_le_bytes()
        );
        assert!(!tree.tick_motions(200.0));
        assert_eq!(
            tree.shader_effects[&node].uniforms.as_ref(),
            0.75_f32.to_le_bytes()
        );
        assert_eq!(tree.take_motion_ends().len(), 1);
        Ok(())
    }
}
