use std::ffi::c_void;

use impasto::{
    PluginContext, Runtime, Task,
    os::android::{AndroidContext, ContextExt as _},
};
use jni::{
    Env, EnvUnowned, JValue, NativeMethod, jni_sig, jni_str,
    objects::{JByteArray, JClass, JObject},
    sys::{jboolean, jint, jlong},
};

use crate::{CameraError, Facing, Frame};

#[derive(Clone)]
pub struct Camera {
    platform: AndroidContext,
    runtime: Runtime,
    facing: Facing,
}

impl Camera {
    pub fn new(context: &PluginContext, facing: Facing) -> Result<Self, CameraError> {
        Ok(Self {
            platform: context
                .android()
                .cloned()
                .ok_or_else(CameraError::unavailable)?,
            runtime: context.runtime().clone(),
            facing,
        })
    }

    pub async fn start(&self, id: u64, name: &str) -> Result<(), CameraError> {
        self.active(id, Some(name))
            .await
            .map_err(|error| CameraError::new("schedule", error))?
            .map_err(|error| CameraError::new("platform", error))
    }

    pub fn resume(&self, id: u64, name: &str) {
        drop(self.active(id, Some(name)));
    }

    pub fn stop(&self, id: u64) {
        drop(self.active(id, None));
    }

    fn active(&self, id: u64, texture: Option<&str>) -> Task<Result<(), String>> {
        let front = matches!(self.facing, Facing::Front);
        let texture = texture.map(str::to_owned);
        self.platform.with_view(&self.runtime, move |env, view| {
            let class = camera_class(env, view)?;
            let id = jlong::from_ne_bytes(id.to_ne_bytes());
            if let Some(texture) = texture {
                // SAFETY: both functions below match the Kotlin static native declarations.
                unsafe {
                    env.register_native_methods(
                        &class,
                        &[
                            NativeMethod::from_raw_parts(
                                jni_str!("nativeFrame"),
                                jni_str!("(JII[BZ)V"),
                                receive as *mut c_void,
                            ),
                            NativeMethod::from_raw_parts(
                                jni_str!("nativeFailure"),
                                jni_str!("(J)V"),
                                failure as *mut c_void,
                            ),
                        ],
                    )?;
                }
                let texture = env.new_string(texture)?;
                env.call_static_method(
                    &class,
                    jni_str!("start"),
                    jni_sig!("(Landroid/view/View;JZLjava/lang/String;)V"),
                    &[
                        JValue::Object(view),
                        JValue::Long(id),
                        JValue::Bool(front),
                        JValue::Object(texture.as_ref()),
                    ],
                )?;
            } else {
                env.call_static_method(
                    &class,
                    jni_str!("stop"),
                    jni_sig!("(J)V"),
                    &[JValue::Long(id)],
                )?;
            }
            Ok(())
        })
    }
}

fn camera_class<'local>(
    env: &mut Env<'local>,
    view: &JObject<'_>,
) -> jni::errors::Result<JClass<'local>> {
    let context = env
        .call_method(
            view,
            jni_str!("getContext"),
            jni_sig!("()Landroid/content/Context;"),
            &[],
        )?
        .l()?;
    let loader = env
        .call_method(
            &context,
            jni_str!("getClassLoader"),
            jni_sig!("()Ljava/lang/ClassLoader;"),
            &[],
        )?
        .l()?;
    let name = env.new_string("dev.impasto.plugins.camera.CameraBridge")?;
    let class = env
        .call_method(
            &loader,
            jni_str!("loadClass"),
            jni_sig!("(Ljava/lang/String;)Ljava/lang/Class;"),
            &[JValue::Object(name.as_ref())],
        )?
        .l()?;
    env.cast_local::<JClass>(class)
}

extern "system" fn receive<'local>(
    mut env: EnvUnowned<'local>,
    _: JClass<'local>,
    id: jlong,
    width: jint,
    height: jint,
    pixels: JByteArray<'local>,
    native_preview: jboolean,
) {
    env.with_env(|env| {
        let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
            return Ok(());
        };
        let Some((_, _, count)) = crate::dimensions(width, height) else {
            return Ok(());
        };
        if pixels.len(env)? != count * 4 {
            return Ok(());
        }
        let rgba = env.convert_byte_array(&pixels)?;
        crate::frame(
            u64::from_ne_bytes(id.to_ne_bytes()),
            Frame {
                width,
                height,
                rgba,
            },
            |texture, frame| {
                if native_preview {
                    return;
                }
                let _ = texture.publish_rgba(impasto::ExternalFrame {
                    width: frame.width,
                    height: frame.height,
                    pixels: frame.rgba.clone(),
                });
            },
        );
        Ok::<_, jni::errors::Error>(())
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

extern "system" fn failure(_: EnvUnowned<'_>, _: JClass<'_>, id: jlong) {
    crate::failed(u64::from_ne_bytes(id.to_ne_bytes()));
}
