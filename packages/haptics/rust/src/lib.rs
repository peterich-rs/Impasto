//! Tactile feedback and validated vibration patterns.

use impasto::os::{NativePlugin, NativePluginError};
use impasto::{ImpastoError, ImpastoType, PluginContext, PluginInitError, PluginRegistrar};
use serde::Serialize;

const MAX_DURATION_MS: u32 = 10_000;
const MAX_PATTERN_MS: u32 = 60_000;

#[derive(Debug, Default, ImpastoType, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ImpactStyle {
    Light,
    #[default]
    Medium,
    Heavy,
    Soft,
    Rigid,
}

#[derive(Debug, ImpastoType, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NotificationFeedbackType {
    Success,
    Warning,
    Error,
}

#[derive(Debug)]
struct HapticsError {
    code: &'static str,
    message: String,
}

impl HapticsError {
    fn invalid(message: &str) -> Self {
        Self {
            code: "invalid-argument",
            message: message.into(),
        }
    }
}

impl From<NativePluginError> for HapticsError {
    fn from(value: NativePluginError) -> Self {
        Self {
            code: if value.code == "host-unavailable" {
                "host-unavailable"
            } else {
                "platform"
            },
            message: value.message,
        }
    }
}

impl ImpastoError for HapticsError {
    fn scope(&self) -> &'static str {
        "haptics"
    }
    fn code(&self) -> &'static str {
        self.code
    }
    fn message(&self) -> String {
        self.message.clone()
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "accepts only integral u32 values within explicit bounds"
)]
fn integer(value: f64, min: u32, max: u32) -> Option<u32> {
    (value.is_finite()
        && value.fract() == 0.0
        && (f64::from(min)..=f64::from(max)).contains(&value))
    .then_some(value as u32)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Pattern {
    timings_ms: Vec<u32>,
    amplitudes: Option<Vec<u32>>,
}

fn pattern(timings: Vec<f64>, amplitudes: Option<Vec<f64>>) -> Result<Pattern, HapticsError> {
    if timings.is_empty() || timings.len() > 512 {
        return Err(HapticsError::invalid(
            "vibration patterns require 1 to 512 entries",
        ));
    }
    let timings_ms = timings
        .into_iter()
        .map(|value| {
            integer(value, 0, MAX_DURATION_MS).ok_or_else(|| {
                HapticsError::invalid(
                    "vibration timings require whole milliseconds from 0 to 10000",
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !(1..=MAX_PATTERN_MS).contains(&timings_ms.iter().sum()) {
        return Err(HapticsError::invalid(
            "total vibration pattern duration must be between 1 and 60000 ms",
        ));
    }
    let amplitudes = amplitudes
        .map(|amplitudes| {
            if amplitudes.len() != timings_ms.len() {
                return Err(HapticsError::invalid(
                    "timings and amplitudes must have the same length",
                ));
            }
            amplitudes
                .into_iter()
                .map(|value| {
                    integer(value, 0, 255).ok_or_else(|| {
                        HapticsError::invalid("vibration amplitudes must be integers from 0 to 255")
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    Ok(Pattern {
        timings_ms,
        amplitudes,
    })
}

#[impasto::module(name = "Haptics", specifier = "@impasto/haptics/native", init = init)]
struct HapticsPlugin(NativePlugin);

impl HapticsPlugin {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        Ok(Self(NativePlugin::new(ctx, "@impasto/haptics")))
    }

    async fn play(&self, action: &str, payload: &impl Serialize) -> Result<(), HapticsError> {
        if self.0.is_hosted() {
            self.0.request::<()>(action, payload).await?;
        }
        Ok(())
    }
}

#[impasto::export]
impl HapticsPlugin {
    pub fn is_haptics_host_available(&self) -> bool {
        self.0.is_hosted()
    }
    pub async fn is_supported(&self) -> Result<bool, HapticsError> {
        if self.0.is_hosted() {
            return Ok(self.0.request("isSupported", &()).await?);
        }
        Ok(false)
    }
    pub async fn impact(&self, style: Option<ImpactStyle>) -> Result<(), HapticsError> {
        self.play("impact", &style.unwrap_or_default()).await
    }
    pub async fn selection(&self) -> Result<(), HapticsError> {
        self.play("selection", &()).await
    }
    pub async fn notification(&self, kind: NotificationFeedbackType) -> Result<(), HapticsError> {
        self.play("notification", &kind).await
    }
    pub async fn vibrate(&self, duration_ms: Option<f64>) -> Result<(), HapticsError> {
        let duration =
            integer(duration_ms.unwrap_or(400.0), 1, MAX_DURATION_MS).ok_or_else(|| {
                HapticsError::invalid(
                    "vibration duration must be a whole number between 1 and 10000 ms",
                )
            })?;
        self.play("vibrate", &duration).await
    }
    pub async fn vibrate_pattern(
        &self,
        timings_ms: Vec<f64>,
        amplitudes: Option<Vec<f64>>,
    ) -> Result<(), HapticsError> {
        let pattern = pattern(timings_ms, amplitudes)?;
        self.play("vibratePattern", &pattern).await
    }
    pub async fn cancel(&self) -> Result<(), HapticsError> {
        self.play("cancel", &()).await
    }
}

/// Registers tactile feedback bindings.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(HapticsPlugin::plugin());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vibration_patterns_reject_nonfinite_times_and_mismatched_amplitudes() {
        assert!(pattern(vec![0.0, 20.0], Some(vec![0.0, 255.0])).is_ok());
        assert!(pattern(vec![f64::INFINITY], None).is_err());
        assert!(pattern(vec![2.0, 3.0], Some(vec![255.0])).is_err());
        assert!(pattern(vec![5.0], Some(vec![256.0])).is_err());
    }
}
