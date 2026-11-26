//! JavaScript bindings for an application's optional diagnostics source.

use std::collections::BTreeMap;
use std::sync::Arc;

use impasto::abi::PluginValue;
use impasto::{
    DiagnosticsSource, FromPluginValue, ImpastoError, ImpastoType, IntoPluginValue, PluginContext,
    PluginInitError, PluginRegistrar, TsTypes,
};

mod types;
use types::{
    DiagError, Flags, FlagsPatch, FrameSample, GpuFrameTiming, LogLine, LogSource, NetEntry,
    Platform, Versions,
};

struct Page<T> {
    revision: f64,
    entries: Vec<T>,
}

impl<T: ImpastoType> FromPluginValue for Page<T> {
    fn from_value(value: PluginValue) -> Result<Self, String> {
        let PluginValue::Map(fields) = value else {
            return Err("expected a diagnostics page".into());
        };
        let mut fields: BTreeMap<_, _> = fields.into_iter().collect();
        Ok(Self {
            revision: impasto::macro_support::decode_field(&mut fields, "revision")?,
            entries: impasto::macro_support::decode_field(&mut fields, "entries")?,
        })
    }
}

impl<T: ImpastoType> IntoPluginValue for Page<T> {
    fn into_value(self) -> Result<PluginValue, String> {
        Ok(PluginValue::Map(vec![
            ("revision".into(), self.revision.into_value()?),
            ("entries".into(), self.entries.into_value()?),
        ]))
    }
}

impl<T: ImpastoType> ImpastoType for Page<T> {
    fn typescript(types: &mut TsTypes) -> String {
        struct PageDeclaration;
        types.define::<PageDeclaration>("Page", |_| {
            "export type Page<T> = { revision: number; entries: T[] };".into()
        });
        format!("Page<{}>", T::typescript(types))
    }
}

#[derive(Debug)]
struct DevtoolsError(String);

impl ImpastoError for DevtoolsError {
    fn scope(&self) -> &'static str {
        "devtools"
    }
    fn code(&self) -> &'static str {
        "diagnostics"
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

#[impasto::module(name = "Devtools", specifier = "@impasto/devtools/native", init = init)]
struct DevtoolsPlugin {
    source: Option<Arc<DiagnosticsSource>>,
}

impl DevtoolsPlugin {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        Ok(Self {
            source: ctx.extension(),
        })
    }

    fn query<T: FromPluginValue>(
        &self,
        operation: &str,
        args: &[PluginValue],
    ) -> Result<T, DevtoolsError> {
        let source = self
            .source
            .as_ref()
            .ok_or_else(|| DevtoolsError("diagnostics are unavailable".into()))?;
        let result = source.query(operation, args).map_err(DevtoolsError)?;
        T::from_value(result).map_err(DevtoolsError)
    }

    fn read_page<T: ImpastoType>(
        &self,
        operation: &str,
        since: Option<f64>,
    ) -> Result<Page<T>, DevtoolsError> {
        if self.source.is_some() {
            self.query(operation, &[PluginValue::F64(since.unwrap_or_default())])
        } else {
            Ok(Page {
                revision: 0.0,
                entries: Vec::new(),
            })
        }
    }
}

#[impasto::export]
impl DevtoolsPlugin {
    pub fn enabled(&self) -> bool {
        self.source.is_some()
    }
    pub fn source_map(&self) -> Result<Option<String>, DevtoolsError> {
        if self.source.is_some() {
            self.query("sourceMap", &[])
        } else {
            Ok(None)
        }
    }
    pub fn open_source(&self, path: String, line: u32, column: u32) -> Result<bool, DevtoolsError> {
        if self.source.is_none() {
            return Ok(false);
        }
        self.query(
            "openSource",
            &[
                PluginValue::Str(path),
                PluginValue::F64(f64::from(line)),
                PluginValue::F64(f64::from(column)),
            ],
        )
    }
    pub fn revision(&self) -> Result<f64, DevtoolsError> {
        if self.source.is_some() {
            self.query("revision", &[])
        } else {
            Ok(0.0)
        }
    }
    pub fn logs(&self, since: Option<f64>) -> Result<Page<LogLine>, DevtoolsError> {
        self.read_page("logs", since)
    }
    pub fn errors(&self, since: Option<f64>) -> Result<Page<DiagError>, DevtoolsError> {
        self.read_page("errors", since)
    }
    pub fn network(&self, since: Option<f64>) -> Result<Page<NetEntry>, DevtoolsError> {
        self.read_page("network", since)
    }
    pub fn frames(&self, since: Option<f64>) -> Result<Page<FrameSample>, DevtoolsError> {
        self.read_page("frames", since)
    }
    pub fn gpu_timing(&self, frame: f64) -> Result<Option<GpuFrameTiming>, DevtoolsError> {
        self.query("gpuTiming", &[PluginValue::F64(frame)])
    }
    pub fn start_trace(&self) -> Result<f64, DevtoolsError> {
        self.query("startTrace", &[])
    }
    #[impasto::method(blocking)]
    pub fn finish_trace(&self, recording: f64) -> Result<String, DevtoolsError> {
        self.query("finishTrace", &[PluginValue::F64(recording)])
    }
    pub fn flags(&self) -> Result<Flags, DevtoolsError> {
        self.query("flags", &[])
    }
    pub fn set_flags(&self, flags: Option<FlagsPatch>) -> Result<(), DevtoolsError> {
        self.query("setFlags", &[flags.into_value().map_err(DevtoolsError)?])
    }
    pub fn clear(&self) -> Result<(), DevtoolsError> {
        self.query("clear", &[])
    }
    pub fn platform(&self) -> Result<Platform, DevtoolsError> {
        self.query("platform", &[])
    }
    pub fn versions(&self) -> Result<Versions, DevtoolsError> {
        self.query("versions", &[])
    }
    pub fn log_source(&self) -> Vec<LogSource> {
        vec![
            LogSource::Script,
            LogSource::Layout,
            LogSource::Host,
            LogSource::Net,
        ]
    }
}

/// Registers the diagnostics API used by the DevTools panel.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(DevtoolsPlugin::plugin());
}
