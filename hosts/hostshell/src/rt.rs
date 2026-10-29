//! Process-wide tokio runtime. HTTP and module workers share it.

use std::sync::OnceLock;

pub struct Rt {
    runtime: tokio::runtime::Runtime,
}

impl Rt {
    fn create() -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("uifw-rt")
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self { runtime })
    }

    pub fn handle(&self) -> tokio::runtime::Handle {
        self.runtime.handle().clone()
    }
}

pub fn rt() -> Result<&'static Rt, String> {
    static RT: OnceLock<Result<Rt, String>> = OnceLock::new();
    match RT.get_or_init(Rt::create) {
        Ok(rt) => Ok(rt),
        Err(message) => Err(message.clone()),
    }
}

pub fn handle() -> Result<tokio::runtime::Handle, String> {
    Ok(rt()?.handle())
}
