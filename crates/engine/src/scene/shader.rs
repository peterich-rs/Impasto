//! GPU-independent scene data for application fragment effects.

use std::sync::Arc;

/// The reserved texture input containing the pixels already drawn into the target.
pub const BACKDROP_TEXTURE: &str = "backdrop:";
/// Maximum WGSL source size for one program.
pub const MAX_SHADER_SOURCE_BYTES: usize = 64 * 1024;
/// Maximum byte size of the packed, little-endian f32 uniform array.
pub const MAX_SHADER_UNIFORM_BYTES: usize = 32 * size_of::<f32>();
/// Maximum number of sampled textures for one effect.
pub const MAX_SHADER_TEXTURES: usize = 4;
const MAX_TEXTURE_SOURCE_BYTES: usize = 4096;

/// Identity of a registered program, unique across session reloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShaderProgramId(pub u64);

/// A program source transferred to Raster without exposing device resources to UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderProgram {
    pub id: ShaderProgramId,
    pub source: Arc<str>,
    /// The effect reads the compositor clock and needs a frame while visible.
    pub animated: bool,
}

/// How an effect requests frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShaderClock {
    /// Render when the scene changes.
    #[default]
    Static,
    /// Render on each vsync and expose the target timestamp to the effect.
    Frame,
}

/// Parameters attached to a layout node and recorded in its content box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderEffect {
    pub shader: ShaderProgramId,
    pub uniforms: Arc<[u8]>,
    pub textures: Arc<[Arc<str>]>,
}

impl ShaderEffect {
    /// Checks the input layout and resource limits before assigning it to a node.
    ///
    /// # Errors
    /// Returns a [`ShaderError`] when the bytes or texture sources exceed the contract.
    pub fn validate(&self) -> Result<(), ShaderError> {
        if self.uniforms.len() > MAX_SHADER_UNIFORM_BYTES
            || !self.uniforms.len().is_multiple_of(size_of::<f32>())
        {
            return Err(ShaderError::Uniforms);
        }
        if self.textures.len() > MAX_SHADER_TEXTURES
            || self
                .textures
                .iter()
                .any(|source| source.is_empty() || source.len() > MAX_TEXTURE_SOURCE_BYTES)
        {
            return Err(ShaderError::Textures);
        }
        Ok(())
    }
}

/// Failures at the scene boundary. WGSL compilation diagnostics come from Raster.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ShaderError {
    #[error("this session does not support shader effects")]
    Unsupported,
    #[error("shader source must contain between 1 and {MAX_SHADER_SOURCE_BYTES} bytes")]
    Source,
    #[error("shader program registration capacity reached")]
    Capacity,
    #[error("shader program is not registered in this session")]
    UnknownProgram,
    #[error("shader uniforms must be packed f32 values, up to {MAX_SHADER_UNIFORM_BYTES} bytes")]
    Uniforms,
    #[error("shader effects support up to {MAX_SHADER_TEXTURES} nonempty texture sources")]
    Textures,
}

pub(crate) fn image_textures(textures: &[Arc<str>]) -> impl Iterator<Item = &Arc<str>> {
    textures
        .iter()
        .filter(|source| source.as_ref() != BACKDROP_TEXTURE)
}
