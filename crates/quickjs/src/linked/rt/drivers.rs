//! Generation-checked native driver classes. Their Drop runs before Inner dies.
use super::super::driver;
use super::bindings::{boolean, handle, num, string, value};
use super::*;
use rquickjs::{Class, JsLifetime, class::Trace, function::Rest};

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct ListDriver {
    #[qjs(skip_trace)]
    host: *mut Host,
    index: u32,
    generation: u32,
    #[qjs(skip_trace)]
    released: Cell<bool>,
}
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct ScrollController {
    #[qjs(skip_trace)]
    host: *mut Host,
    index: u32,
    generation: u32,
    #[qjs(skip_trace)]
    released: Cell<bool>,
}

macro_rules! driver_common {
    ($ty:ident,$release:path) => {
        impl $ty {
            fn host(&self) -> &Host {
                // SAFETY: Engine drops the VM and all its classes before Host and Inner.
                unsafe { &*self.host }
            }
            fn check(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
                canvas::side_effect(ctx)?;
                if self.released.get() {
                    Err(type_error(ctx, "driver is released"))
                } else {
                    Ok(())
                }
            }
            fn release_inner(&self) {
                if !self.released.replace(true) {
                    // SAFETY: Engine keeps the session alive through all class finalizers.
                    unsafe {
                        $release(self.host().session(), self.index, self.generation);
                    }
                }
            }
            fn id_object<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
                self.check(ctx)?;
                let obj = Object::new(ctx.clone())?;
                obj.set("index", self.index)?;
                obj.set("generation", self.generation)?;
                Ok(obj)
            }
        }
        impl Drop for $ty {
            fn drop(&mut self) {
                self.release_inner();
            }
        }
    };
}
driver_common!(ListDriver, driver::list_release);
driver_common!(ScrollController, driver::scroll_release);
fn done(ctx: &Ctx<'_>, status: i32, name: &str) -> rquickjs::Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(type_error(ctx, name))
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "JavaScript f64 values convert to the existing fixed-width callback record and f32 geometry"
)]
pub(super) fn list<'js>(
    host: &Host,
    ctx: &Ctx<'js>,
    args: &[Value<'js>],
) -> rquickjs::Result<Value<'js>> {
    let overscan = if args.is_empty() {
        4
    } else {
        num(ctx, args, 0)? as i32
    };
    let estimate = if args.len() < 2 {
        1.0
    } else {
        num(ctx, args, 1)? as f32
    };
    let (mut index, mut generation) = (0, 0);
    // SAFETY: output ids are local, and session is the live Inner.
    let status = unsafe {
        driver::list_create(
            host.session(),
            overscan,
            estimate,
            &mut index,
            &mut generation,
        )
    };
    done(ctx, status, "listDriver")?;
    Ok(Class::instance(
        ctx.clone(),
        ListDriver {
            host: ptr::from_ref(host).cast_mut(),
            index,
            generation,
            released: Cell::new(false),
        },
    )?
    .into_value())
}
pub(super) fn scroll<'js>(host: &Host, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
    let (mut index, mut generation) = (0, 0);
    // SAFETY: output ids are local, and session is the live Inner.
    let status = unsafe { driver::scroll_create(host.session(), &mut index, &mut generation) };
    done(ctx, status, "scrollController")?;
    Ok(Class::instance(
        ctx.clone(),
        ScrollController {
            host: ptr::from_ref(host).cast_mut(),
            index,
            generation,
            released: Cell::new(false),
        },
    )?
    .into_value())
}

#[rquickjs::methods]
impl ListDriver {
    pub fn id<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Object<'js>> {
        self.id_object(&ctx)
    }
    pub fn release(&self) {
        self.release_inner();
    }
    pub fn bind<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        self.check(&ctx)?;
        let host = self.host();
        host.with_current(&ctx, || {
            let mut nodes = [0; 10];
            for (i, node) in nodes.iter_mut().enumerate() {
                *node = handle(&ctx, &args.0, i)?;
            }
            // SAFETY: the ten initialized ids and the session live through this call.
            done(
                &ctx,
                unsafe {
                    driver::list_bind(
                        host.session(),
                        self.index,
                        self.generation,
                        nodes.as_ptr(),
                        10,
                    )
                },
                "listDriver.bind",
            )
        })
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "JavaScript f64 values convert to the existing fixed-width callback record and f32 geometry"
    )]
    pub fn configure<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        if args.0.len() < 3 {
            return Err(type_error(&ctx, "driver arguments"));
        }
        self.check(&ctx)?;
        let host = self.host();
        host.with_current(&ctx, || {
            let estimate = num(&ctx, &args.0, 0)? as f32;
            let overscan = num(&ctx, &args.0, 1)? as i32;
            let blocked = boolean(&ctx, &args.0, 2)?;
            // SAFETY: only numeric parameters and the live session cross the callback.
            done(
                &ctx,
                unsafe {
                    driver::list_configure(
                        host.session(),
                        self.index,
                        self.generation,
                        estimate,
                        overscan,
                        i32::from(blocked),
                    )
                },
                "listDriver.configure",
            )
        })
    }
    #[qjs(rename = "setItems")]
    pub fn set_items<'js>(
        &self,
        ctx: Ctx<'js>,
        items: rquickjs::Array<'js>,
    ) -> rquickjs::Result<()> {
        self.check(&ctx)?;
        let host = self.host();
        host.with_current(&ctx, || {
            // SAFETY: the live driver is addressed by its index and generation.
            done(
                &ctx,
                // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
                unsafe { driver::list_begin(host.session(), self.index, self.generation) },
                "listDriver.setItems",
            )?;
            for item in items.iter::<Object>() {
                let item = item?;
                let key = item.get("key")?;
                let ty = item.get("type")?;
                let sticky = item.get("sticky")?;
                let key = string(&ctx, &[key], 0)?;
                let ty = string(&ctx, &[ty], 0)?;
                let sticky = boolean(&ctx, &[sticky], 0)?;
                // SAFETY: both strings are borrowed until the callback returns.
                done(
                    &ctx,
                    // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
                    unsafe {
                        driver::list_item(
                            host.session(),
                            self.index,
                            self.generation,
                            key.as_ptr().cast(),
                            key.len(),
                            ty.as_ptr().cast(),
                            ty.len(),
                            i32::from(sticky),
                        )
                    },
                    "listDriver.setItems",
                )?;
            }
            // SAFETY: the session and generation-checked driver remain alive.
            done(
                &ctx,
                unsafe { driver::list_end(host.session(), self.index, self.generation) },
                "listDriver.setItems",
            )
        })
    }
    #[qjs(rename = "noteRow")]
    pub fn note_row<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        if args.0.len() < 2 {
            return Err(type_error(&ctx, "driver arguments"));
        }
        self.check(&ctx)?;
        let host = self.host();
        host.with_current(&ctx, || {
            let key = string(&ctx, &args.0, 0)?;
            let node = handle(&ctx, &args.0, 1)?;
            // SAFETY: the key's bytes live through this call.
            done(
                &ctx,
                unsafe {
                    driver::list_note_row(
                        host.session(),
                        self.index,
                        self.generation,
                        key.as_ptr().cast(),
                        key.len(),
                        node,
                    )
                },
                "listDriver.noteRow",
            )
        })
    }
}

#[rquickjs::methods]
impl ScrollController {
    pub fn id<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Object<'js>> {
        self.id_object(&ctx)
    }
    pub fn release(&self) {
        self.release_inner();
    }
    pub fn bind<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        if args.0.len() < 2 {
            return Err(type_error(&ctx, "driver arguments"));
        }
        self.check(&ctx)?;
        let host = self.host();
        host.with_current(&ctx, || {
            let view = handle(&ctx, &args.0, 0)?;
            let content = handle(&ctx, &args.0, 1)?;
            // SAFETY: the callback uses live session state and validated numeric handles.
            done(
                &ctx,
                unsafe {
                    driver::scroll_bind(host.session(), self.index, self.generation, view, content)
                },
                "scrollController.bind",
            )
        })
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "JavaScript f64 values convert to the existing fixed-width callback record and f32 geometry"
    )]
    pub fn configure<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        if args.0.len() < 5 {
            return Err(type_error(&ctx, "driver arguments"));
        }
        self.check(&ctx)?;
        let host = self.host();
        host.with_current(&ctx, || {
            let content_min = num(&ctx, &args.0, 0)? as f32;
            let mode = num(&ctx, &args.0, 1)? as i32;
            let threshold = num(&ctx, &args.0, 2)? as f32;
            let rubber = boolean(&ctx, &args.0, 3)?;
            let factor = num(&ctx, &args.0, 4)? as f32;
            let parent = value(&ctx, &args.0, 5);
            let (pi, pg, has) = if parent.is_null() || parent.is_undefined() {
                (0, 0, 0)
            } else {
                let parent = Class::<ScrollController>::from_value(&parent)?;
                let parent = parent.borrow();
                parent.check(&ctx)?;
                (parent.index, parent.generation, 1)
            };
            // SAFETY: both drivers are generation-checked by DriverSet.
            done(
                &ctx,
                unsafe {
                    driver::scroll_configure(
                        host.session(),
                        self.index,
                        self.generation,
                        content_min,
                        mode,
                        threshold,
                        i32::from(rubber),
                        factor,
                        pi,
                        pg,
                        has,
                    )
                },
                "scrollController.configure",
            )
        })
    }
    #[qjs(rename = "dragBy")]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "JavaScript f64 values convert to the existing fixed-width callback record and f32 geometry"
    )]
    pub fn drag_by<'js>(
        &self,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        if args.0.len() < 2 {
            return Err(type_error(&ctx, "driver arguments"));
        }
        self.check(&ctx)?;
        let host = self.host();
        host.with_current(&ctx, || {
            let dx = num(&ctx, &args.0, 0)? as f32;
            let dy = num(&ctx, &args.0, 1)? as f32;
            let (mut ox, mut oy, mut over_x, mut over_y) = (0.0, 0.0, 0.0, 0.0);
            // SAFETY: all out parameters are writable local floats.
            let status = unsafe {
                driver::scroll_drag_by(
                    host.session(),
                    self.index,
                    self.generation,
                    dx,
                    dy,
                    &mut ox,
                    &mut oy,
                    &mut over_x,
                    &mut over_y,
                )
            };
            if status < 0 {
                return Err(type_error(&ctx, "scrollController.dragBy"));
            }
            let object = Object::new(ctx.clone())?;
            object.set("ox", ox)?;
            object.set("oy", oy)?;
            object.set("overX", over_x)?;
            object.set("overY", over_y)?;
            object.set("pending", status > 0)?;
            Ok(object)
        })
    }
    #[qjs(rename = "finishDrag")]
    pub fn finish_drag<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        if args.0.is_empty() {
            return Err(type_error(&ctx, "driver arguments"));
        }
        self.check(&ctx)?;
        let host = self.host();
        host.with_current(&ctx, || {
            let taken = boolean(&ctx, &args.0, 0)?;
            // SAFETY: only the live session and scalar values are passed.
            done(
                &ctx,
                unsafe {
                    driver::scroll_finish(
                        host.session(),
                        self.index,
                        self.generation,
                        i32::from(taken),
                    )
                },
                "scrollController.finishDrag",
            )
        })
    }
    #[qjs(rename = "dragEnd")]
    pub fn drag_end(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        self.check(&ctx)?;
        let host = self.host();
        host.with_current(&ctx, || {
            // SAFETY: the callback addresses a generation-checked driver in the live session.
            done(
                &ctx,
                unsafe { driver::scroll_drag_end(host.session(), self.index, self.generation) },
                "scrollController.dragEnd",
            )
        })
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "JavaScript f64 values convert to the existing fixed-width callback record and f32 geometry"
    )]
    pub fn wheel<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        if args.0.len() < 2 {
            return Err(type_error(&ctx, "driver arguments"));
        }
        self.check(&ctx)?;
        let host = self.host();
        host.with_current(&ctx, || {
            let dx = num(&ctx, &args.0, 0)? as f32;
            let dy = num(&ctx, &args.0, 1)? as f32;
            // SAFETY: the callback receives live session state and scalars only.
            done(
                &ctx,
                unsafe {
                    driver::scroll_wheel(host.session(), self.index, self.generation, dx, dy)
                },
                "scrollController.wheel",
            )
        })
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "JavaScript f64 values convert to the existing fixed-width callback record and f32 geometry"
    )]
    pub fn query<'js>(
        &self,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        if args.0.is_empty() {
            return Err(type_error(&ctx, "driver arguments"));
        }
        self.check(&ctx)?;
        let host = self.host();
        host.with_current(&ctx, || {
            let cmd = num(&ctx, &args.0, 0)? as i32;
            let (mut x, mut y) = (0.0, 0.0);
            // SAFETY: x and y are local writable floats.
            done(
                &ctx,
                // SAFETY: the Rust caller keeps the session and input slices alive and supplies writable output slots.
                unsafe {
                    driver::scroll_query(
                        host.session(),
                        self.index,
                        self.generation,
                        cmd,
                        &mut x,
                        &mut y,
                    )
                },
                "scrollController.query",
            )?;
            let object = Object::new(ctx.clone())?;
            object.set("x", x)?;
            object.set("y", y)?;
            Ok(object)
        })
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "JavaScript f64 values convert to the existing fixed-width callback record and f32 geometry"
    )]
    pub fn consume<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<bool> {
        if args.0.len() < 2 {
            return Err(type_error(&ctx, "driver arguments"));
        }
        self.check(&ctx)?;
        let host = self.host();
        host.with_current(&ctx, || {
            let ox = num(&ctx, &args.0, 0)? as f32;
            let oy = num(&ctx, &args.0, 1)? as f32;
            // SAFETY: only the live session and scalar values cross this callback.
            let status = unsafe {
                driver::scroll_consume(host.session(), self.index, self.generation, ox, oy)
            };
            if status < 0 {
                Err(type_error(&ctx, "scrollController.consume"))
            } else {
                Ok(status == 1)
            }
        })
    }
}
