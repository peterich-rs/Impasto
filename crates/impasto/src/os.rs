//! Access to window-specific services on their platform queue.

pub use crate::os_channel::NativeChannel;
pub use crate::os_events::{NativeEventChannel, NativeEventStream};
pub use crate::os_plugin::{NativePlugin, NativePluginError};

#[cfg(target_os = "android")]
pub mod android {
    use std::sync::Arc;

    use jni::{
        Env, JavaVM,
        objects::{Global, JObject},
    };

    use crate::{PluginContext, Runtime, Task};

    type View = Arc<Global<JObject<'static>>>;

    #[derive(Clone)]
    pub struct AndroidContext {
        vm: JavaVM,
        view: Arc<dyn Fn() -> Option<View> + Send + Sync>,
    }

    impl AndroidContext {
        #[doc(hidden)]
        pub fn new(vm: JavaVM, view: impl Fn() -> Option<View> + Send + Sync + 'static) -> Self {
            Self {
                vm,
                view: Arc::new(view),
            }
        }

        pub fn with_view<T: Send + 'static>(
            &self,
            runtime: &Runtime,
            action: impl FnOnce(&mut Env<'_>, &JObject<'_>) -> jni::errors::Result<T> + Send + 'static,
        ) -> Task<Result<T, String>> {
            let platform = self.clone();
            runtime.spawn_main(move || platform.with_view_now(action))
        }

        /// Borrows the current JNI view for operations safe on the calling thread.
        pub fn with_view_now<T>(
            &self,
            action: impl FnOnce(&mut Env<'_>, &JObject<'_>) -> jni::errors::Result<T>,
        ) -> Result<T, String> {
            let view = (self.view)().ok_or("the Android view is detached")?;
            self.vm
                .attach_current_thread_for_scope(|env| action(env, view.as_obj()))
                .map_err(|error| error.to_string())
        }

        /// Runs an action against the Activity attached to this window.
        pub fn with_activity<T: Send + 'static>(
            &self,
            runtime: &Runtime,
            action: impl FnOnce(&mut Env<'_>, &JObject<'_>) -> jni::errors::Result<T> + Send + 'static,
        ) -> Task<Result<T, String>> {
            self.with_view(runtime, move |env, view| {
                let activity = env
                    .call_method(
                        view,
                        jni::jni_str!("getActivity"),
                        jni::jni_sig!("()Landroid/app/Activity;"),
                        &[],
                    )?
                    .l()?;
                action(env, &activity)
            })
        }
    }

    pub trait ContextExt {
        fn android(&self) -> Option<&AndroidContext>;
    }

    impl ContextExt for PluginContext {
        fn android(&self) -> Option<&AndroidContext> {
            self.android.as_ref()
        }
    }
}

#[cfg(any(target_os = "ios", target_os = "macos"))]
pub mod apple {
    use crate::{PluginContext, Task};

    pub trait ContextExt {
        fn with_main<T: Send + 'static>(
            &self,
            action: impl FnOnce(objc2::MainThreadMarker) -> T + Send + 'static,
        ) -> Task<Option<T>>;
    }

    impl ContextExt for PluginContext {
        fn with_main<T: Send + 'static>(
            &self,
            action: impl FnOnce(objc2::MainThreadMarker) -> T + Send + 'static,
        ) -> Task<Option<T>> {
            self.runtime()
                .spawn_main(move || objc2::MainThreadMarker::new().map(action))
        }
    }
}

#[cfg(target_os = "ios")]
pub use apple as ios;

#[cfg(target_os = "macos")]
pub use apple as macos;
