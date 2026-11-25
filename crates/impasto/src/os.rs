//! Access to window-specific services on their platform queue.

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
            runtime.spawn_main(move || {
                let view = (platform.view)().ok_or("the Android view is detached")?;
                platform
                    .vm
                    .attach_current_thread_for_scope(|env| action(env, view.as_obj()))
                    .map_err(|error| error.to_string())
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
