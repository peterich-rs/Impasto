//! Generation-checked native driver classes. Their Drop runs before Inner dies.
use super::bindings::{boolean, handle, num, string, value};
use super::*;
use crate::motion::PullMode;
use crate::tree::NodeId;
use crate::tree::list::{DriverId, ListItem, ListNodes, ScrollConfig};
use rquickjs::{Class, JsLifetime, class::Trace, function::Rest};

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct ListDriver {
    #[qjs(skip_trace)]
    inner: Weak<Inner>,
    #[qjs(skip_trace)]
    id: DriverId,
    #[qjs(skip_trace)]
    released: Cell<bool>,
}
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct ScrollController {
    #[qjs(skip_trace)]
    inner: Weak<Inner>,
    #[qjs(skip_trace)]
    id: DriverId,
    #[qjs(skip_trace)]
    released: Cell<bool>,
}

macro_rules! driver_common {
    ($ty:ident, $release:ident) => {
        impl $ty {
            fn check(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
                canvas::side_effect(ctx)?;
                if self.released.get() {
                    Err(type_error(ctx, "driver is released"))
                } else {
                    Ok(())
                }
            }
            fn release_inner(&self) {
                if !self.released.replace(true)
                    && let Some(inner) = self.inner.upgrade()
                {
                    inner.drivers.borrow_mut().$release(self.id);
                }
            }
            fn id_object<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
                self.check(ctx)?;
                let obj = Object::new(ctx.clone())?;
                obj.set("index", self.id.index)?;
                obj.set("generation", self.id.generation)?;
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
driver_common!(ListDriver, list_release);
driver_common!(ScrollController, scroll_release);
fn node_handle<'js>(
    ctx: &Ctx<'js>,
    args: &[Value<'js>],
    index: usize,
) -> rquickjs::Result<Option<NodeId>> {
    let raw = handle(ctx, args, index)?;
    Ok((raw != 0).then(|| NodeId::from_raw(raw)))
}

fn done(ctx: &Ctx<'_>, ok: bool, name: &str) -> rquickjs::Result<()> {
    if ok {
        Ok(())
    } else {
        Err(type_error(ctx, name))
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "JavaScript numeric arguments use f64"
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
    let id = host
        .inner()
        .drivers
        .borrow_mut()
        .list_create(overscan, estimate);
    Ok(Class::instance(
        ctx.clone(),
        ListDriver {
            inner: Rc::downgrade(&host.inner),
            id,
            released: Cell::new(false),
        },
    )?
    .into_value())
}
pub(super) fn scroll<'js>(host: &Host, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
    let id = host.inner().drivers.borrow_mut().scroll_create();
    Ok(Class::instance(
        ctx.clone(),
        ScrollController {
            inner: Rc::downgrade(&host.inner),
            id,
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
        let host = host_from_ctx(&ctx)?;
        host.with_current(&ctx, || {
            let mut nodes = [None; 10];
            for (i, node) in nodes.iter_mut().enumerate() {
                *node = node_handle(&ctx, &args.0, i)?;
            }
            let [
                viewport,
                content,
                top,
                bottom,
                rows,
                v_thumb,
                h_thumb,
                pin,
                v_size,
                h_size,
            ] = nodes;
            let nodes = ListNodes {
                viewport,
                content,
                top,
                bottom,
                rows,
                v_thumb,
                h_thumb,
                pin,
                v_size,
                h_size,
            };
            let ok = host.inner().drivers.borrow_mut().list_bind(self.id, nodes);
            done(&ctx, ok, "listDriver.bind")
        })
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "JavaScript numeric arguments use f64"
    )]
    pub fn configure<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        if args.0.len() < 3 {
            return Err(type_error(&ctx, "driver arguments"));
        }
        self.check(&ctx)?;
        let host = host_from_ctx(&ctx)?;
        host.with_current(&ctx, || {
            let estimate = num(&ctx, &args.0, 0)? as f32;
            let overscan = num(&ctx, &args.0, 1)? as i32;
            let blocked = boolean(&ctx, &args.0, 2)?;
            let ok = host
                .inner()
                .drivers
                .borrow_mut()
                .list_configure(self.id, estimate, overscan, blocked);
            done(&ctx, ok, "listDriver.configure")
        })
    }
    #[qjs(rename = "setItems")]
    pub fn set_items<'js>(
        &self,
        ctx: Ctx<'js>,
        items: rquickjs::Array<'js>,
    ) -> rquickjs::Result<()> {
        self.check(&ctx)?;
        let host = host_from_ctx(&ctx)?;
        host.with_current(&ctx, || {
            let drivers = &host.inner().drivers;
            done(
                &ctx,
                drivers.borrow_mut().list_begin(self.id),
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
                let item = ListItem { key, ty, sticky };
                done(
                    &ctx,
                    drivers.borrow_mut().list_item(self.id, item),
                    "listDriver.setItems",
                )?;
            }
            done(
                &ctx,
                drivers.borrow_mut().list_end(self.id),
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
        let host = host_from_ctx(&ctx)?;
        host.with_current(&ctx, || {
            let key = string(&ctx, &args.0, 0)?;
            let node = node_handle(&ctx, &args.0, 1)?;
            let ok = host
                .inner()
                .drivers
                .borrow_mut()
                .list_note_row(self.id, &key, node);
            done(&ctx, ok, "listDriver.noteRow")
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
        let host = host_from_ctx(&ctx)?;
        host.with_current(&ctx, || {
            let view = node_handle(&ctx, &args.0, 0)?;
            let content = node_handle(&ctx, &args.0, 1)?;
            let inner = host.inner();
            let ok = inner.drivers.borrow_mut().scroll_bind(
                self.id,
                view,
                content,
                &inner.tree.borrow(),
            );
            done(&ctx, ok, "scrollController.bind")
        })
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "JavaScript numeric arguments use f64"
    )]
    pub fn configure<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        if args.0.len() < 5 {
            return Err(type_error(&ctx, "driver arguments"));
        }
        self.check(&ctx)?;
        let host = host_from_ctx(&ctx)?;
        host.with_current(&ctx, || {
            let content_min = num(&ctx, &args.0, 0)? as f32;
            let mode = num(&ctx, &args.0, 1)? as i32;
            let threshold = num(&ctx, &args.0, 2)? as f32;
            let rubber = boolean(&ctx, &args.0, 3)?;
            let factor = num(&ctx, &args.0, 4)? as f32;
            let parent = value(&ctx, &args.0, 5);
            let parent = if parent.is_null() || parent.is_undefined() {
                None
            } else {
                let parent = Class::<ScrollController>::from_value(&parent)?;
                let parent = parent.borrow();
                parent.check(&ctx)?;
                Some(parent.id)
            };
            let config = ScrollConfig {
                content_min,
                pull_mode: PullMode::from_code(mode),
                threshold,
                rubber,
                factor,
            };
            let inner = host.inner();
            let ok = inner.drivers.borrow_mut().scroll_configure(
                self.id,
                config,
                parent,
                &mut inner.tree.borrow_mut(),
            );
            done(&ctx, ok, "scrollController.configure")
        })
    }
    #[qjs(rename = "dragBy")]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "JavaScript numeric arguments use f64"
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
        let host = host_from_ctx(&ctx)?;
        host.with_current(&ctx, || {
            let dx = num(&ctx, &args.0, 0)? as f32;
            let dy = num(&ctx, &args.0, 1)? as f32;
            let inner = host.inner();
            let step = inner
                .drivers
                .borrow_mut()
                .scroll_drag_by(self.id, &mut inner.tree.borrow_mut(), dx, dy)
                .ok_or_else(|| type_error(&ctx, "scrollController.dragBy"))?;
            let object = Object::new(ctx.clone())?;
            object.set("ox", step.overflow_x)?;
            object.set("oy", step.overflow_y)?;
            object.set("overX", step.over_x)?;
            object.set("overY", step.over_y)?;
            object.set("pending", step.pending)?;
            Ok(object)
        })
    }
    #[qjs(rename = "finishDrag")]
    pub fn finish_drag<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        if args.0.is_empty() {
            return Err(type_error(&ctx, "driver arguments"));
        }
        self.check(&ctx)?;
        let host = host_from_ctx(&ctx)?;
        host.with_current(&ctx, || {
            let taken = boolean(&ctx, &args.0, 0)?;
            let inner = host.inner();
            let step = inner.drivers.borrow_mut().scroll_finish(
                self.id,
                &mut inner.tree.borrow_mut(),
                taken,
            );
            done(&ctx, step.is_some(), "scrollController.finishDrag")
        })
    }
    #[qjs(rename = "dragEnd")]
    pub fn drag_end(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        self.check(&ctx)?;
        let host = host_from_ctx(&ctx)?;
        host.with_current(&ctx, || {
            let step = host.inner().drivers.borrow_mut().scroll_drag_end(self.id);
            done(&ctx, step.is_some(), "scrollController.dragEnd")
        })
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "JavaScript numeric arguments use f64"
    )]
    pub fn wheel<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        if args.0.len() < 2 {
            return Err(type_error(&ctx, "driver arguments"));
        }
        self.check(&ctx)?;
        let host = host_from_ctx(&ctx)?;
        host.with_current(&ctx, || {
            let dx = num(&ctx, &args.0, 0)? as f32;
            let dy = num(&ctx, &args.0, 1)? as f32;
            let inner = host.inner();
            let ok = inner.drivers.borrow_mut().scroll_wheel(
                self.id,
                &mut inner.tree.borrow_mut(),
                dx,
                dy,
            );
            done(&ctx, ok, "scrollController.wheel")
        })
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "JavaScript numeric arguments use f64"
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
        let host = host_from_ctx(&ctx)?;
        host.with_current(&ctx, || {
            let cmd = num(&ctx, &args.0, 0)? as i32;
            let inner = host.inner();
            let (x, y) = inner
                .drivers
                .borrow()
                .scroll_query(self.id, &inner.tree.borrow(), cmd)
                .ok_or_else(|| type_error(&ctx, "scrollController.query"))?;
            let object = Object::new(ctx.clone())?;
            object.set("x", x)?;
            object.set("y", y)?;
            Ok(object)
        })
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "JavaScript numeric arguments use f64"
    )]
    pub fn consume<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<bool> {
        if args.0.len() < 2 {
            return Err(type_error(&ctx, "driver arguments"));
        }
        self.check(&ctx)?;
        let host = host_from_ctx(&ctx)?;
        host.with_current(&ctx, || {
            let ox = num(&ctx, &args.0, 0)? as f32;
            let oy = num(&ctx, &args.0, 1)? as f32;
            let inner = host.inner();
            inner
                .drivers
                .borrow_mut()
                .scroll_consume(self.id, &mut inner.tree.borrow_mut(), ox, oy)
                .ok_or_else(|| type_error(&ctx, "scrollController.consume"))
        })
    }
}
