use impasto::{ImpastoType, PluginRegistrar};
use javascript::runtime::{ModuleCatalog, Runtime};

#[derive(ImpastoType)]
pub struct Order {
    total_cents: u32,
    memo: Option<String>,
}

#[impasto::module(specifier = "@app/order")]
pub struct OrderModule;

#[impasto::export]
impl OrderModule {
    pub fn total(order: Order) -> u32 {
        order.total_cents
    }
}

#[test]
fn exported_rust_values_and_types_share_the_module_descriptor()
-> Result<(), Box<dyn std::error::Error>> {
    let mut plugins = PluginRegistrar::default();
    plugins.add(OrderModule::plugin());
    let declarations = plugins.describe()?;
    assert!(declarations[0].typescript.contains("totalCents: number"));
    assert!(declarations[0].typescript.contains("memo?: string | null"));
    let host = plugins.build(&Default::default())?;
    let catalog = ModuleCatalog::new(javascript::host::ModuleRegistry::try_new(host)?)?;
    let mut runtime = Runtime::builder()
        .modules(catalog)
        .module(
            "app:test",
            "import {total} from '@app/order';
             if (total({totalCents: 17}) !== 17) throw Error('incorrect total');
             try { total({totalCents: -1}); throw Error('accepted an invalid number'); }
             catch (error) { if (!(error instanceof TypeError)) throw error; }",
        )
        .build()?;
    let result = runtime.start_module("app:test")?;
    runtime.drive(0)?;
    assert_eq!(
        runtime.take_module_result(result)?,
        Some(javascript::runtime::ModuleOutcome::Fulfilled)
    );
    Ok(())
}

#[impasto::object]
pub struct Counter(u32);

#[impasto::export]
impl Counter {
    pub async fn current(&self) -> u32 {
        self.0
    }

    pub async fn values(
        &self,
        mut sink: impasto::TypedSink<u32>,
        cancel: impasto::CancelToken,
    ) -> Result<(), impasto::TypedSendError> {
        if !cancel.is_cancelled() {
            sink.send(self.0).await?;
        }
        Ok(())
    }
}

#[impasto::module(specifier = "@app/counter")]
pub struct CounterModule;

#[impasto::export]
impl CounterModule {
    #[impasto::method(resource)]
    pub async fn open_counter(value: u32) -> Counter {
        Counter(value)
    }
}

struct ReadyExecutor;

impl impasto::abi::MethodExec for ReadyExecutor {
    fn spawn(&self, mut task: impasto::abi::HostFuture) {
        let mut ctx = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(task.as_mut().poll(&mut ctx).is_ready());
    }

    fn worker(&self, settle: impasto::abi::Settle, work: impasto::abi::Work) {
        settle.send(work());
    }

    fn main_thread(&self, settle: impasto::abi::Settle, work: impasto::abi::Work) {
        settle.send(work());
    }

    fn now(&self, settle: impasto::abi::Settle, result: impasto::abi::PluginResult) {
        settle.send(result);
    }
}

#[test]
fn async_objects_and_streams_use_the_same_resource_lease() -> Result<(), Box<dyn std::error::Error>>
{
    let mut plugins = PluginRegistrar::default();
    plugins.add(CounterModule::plugin());
    let declarations = plugins.describe()?;
    assert!(
        declarations[0]
            .typescript
            .contains("openCounter(value: number): Promise<Counter>")
    );
    assert!(
        declarations[0]
            .typescript
            .contains("values(signal?: AbortSignal): ReadableStream<number>")
    );
    let host = plugins.build(&Default::default())?;
    let catalog = ModuleCatalog::new(javascript::host::ModuleRegistry::try_new(host)?)?;
    let mut runtime = Runtime::builder()
        .executor(std::sync::Arc::new(ReadyExecutor))
        .modules(catalog)
        .module(
            "app:test",
            "import {openCounter} from '@app/counter';
             const counter = await openCounter(41);
             if (await counter.current() !== 41) throw Error('query failed');
             const reader = counter.values().getReader();
             if ((await reader.read()).value !== 41) throw Error('stream failed');
             if (!(await reader.read()).done) throw Error('stream did not complete');
             counter.close();
             counter.close();
             try { await counter.current(); throw Error('closed object accepted work'); }
             catch (error) { if (!(error instanceof TypeError)) throw error; }",
        )
        .build()?;
    let result = runtime.start_module("app:test")?;
    for time in 0..8 {
        runtime.drive(time)?;
    }
    assert_eq!(
        runtime.take_module_result(result)?,
        Some(javascript::runtime::ModuleOutcome::Fulfilled)
    );
    Ok(())
}

#[impasto::object(local)]
pub struct LocalCounter(std::rc::Rc<u32>);

#[impasto::export(local)]
impl LocalCounter {
    pub fn increment(&mut self) -> u32 {
        let value = std::rc::Rc::make_mut(&mut self.0);
        *value += 1;
        *value
    }
}

#[impasto::module(specifier = "@app/local")]
pub struct LocalModule;

#[impasto::export]
impl LocalModule {
    #[impasto::method(resource)]
    pub fn open() -> LocalCounter {
        LocalCounter(std::rc::Rc::new(10))
    }
}

#[test]
fn synchronous_objects_can_own_thread_affine_state() -> Result<(), Box<dyn std::error::Error>> {
    let mut plugins = PluginRegistrar::default();
    plugins.add(LocalModule::plugin());
    let host = plugins.build(&Default::default())?;
    let catalog = ModuleCatalog::new(javascript::host::ModuleRegistry::try_new(host)?)?;
    let mut runtime = Runtime::builder()
        .modules(catalog)
        .module("app:test", "import { open } from '@app/local';
            const counter = open();
            if (counter.increment() !== 11 || counter.increment() !== 12) throw Error('state was not preserved');
            counter.close();")
        .build()?;
    let result = runtime.start_module("app:test")?;
    runtime.drive(0)?;
    assert_eq!(
        runtime.take_module_result(result)?,
        Some(javascript::runtime::ModuleOutcome::Fulfilled)
    );
    Ok(())
}

#[derive(impasto::ImpastoEvents)]
enum Changes {
    Updated { fields: String },
}

#[derive(impasto::ImpastoEvents)]
enum WrongChanges {
    Updated { fields: u32 },
}

#[test]
fn event_payload_has_the_bound_schema_even_with_output_field_names() -> Result<(), String> {
    let published = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let events = std::sync::Arc::clone(&published);
    #[impasto::module(specifier = "@app/events", events = Changes, init = init)]
    struct EventModule;

    impl EventModule {
        fn init(ctx: &impasto::PluginContext) -> Result<Self, impasto::PluginInitError> {
            ctx.emitter::<Changes>()
                .emit(Changes::Updated {
                    fields: "updated".to_owned(),
                })
                .map_err(impasto::PluginInitError::new)?;
            assert!(
                ctx.emitter::<WrongChanges>()
                    .emit(WrongChanges::Updated { fields: 3 })
                    .is_err()
            );
            Ok(Self)
        }
    }

    #[impasto::export]
    impl EventModule {
        pub fn enabled() -> bool {
            true
        }
    }

    let context =
        impasto::PluginContext::new(Default::default(), Default::default(), move |event| {
            let mut events = events.lock().unwrap_or_else(|error| error.into_inner());
            events.push(event);
            true
        });
    let mut registrar = PluginRegistrar::default();
    registrar.add(EventModule::plugin());
    registrar
        .build(&context)
        .map_err(|error| error.to_string())?;
    let events = published.lock().unwrap_or_else(|error| error.into_inner());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event, "updated");
    assert_eq!(
        events[0].payload,
        impasto::abi::PluginValue::Map(vec![(
            "fields".into(),
            impasto::abi::PluginValue::Str("updated".into())
        ),])
    );
    Ok(())
}
