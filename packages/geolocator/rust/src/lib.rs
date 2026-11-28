//! Permission-aware location samples and geographic calculations.

use futures_core::Stream as _;
use impasto::os::{NativeEventChannel, NativePlugin, NativePluginError};
use impasto::{
    CancelToken, ImpastoError, ImpastoType, IntoPluginValue, PluginContext, PluginInitError,
    PluginRegistrar, TypedSink,
};
use impasto_permissions::{Permission, PermissionStatus, ServiceStatus};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeMap,
    future::{Future, poll_fn},
    pin::pin,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    task::Poll,
};

#[derive(Clone, Debug, ImpastoType, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: f64,
    #[impasto(nullable)]
    pub altitude: Option<f64>,
    #[impasto(nullable)]
    pub altitude_accuracy: Option<f64>,
    #[impasto(nullable)]
    pub heading: Option<f64>,
    #[impasto(nullable)]
    pub heading_accuracy: Option<f64>,
    #[impasto(nullable)]
    pub speed: Option<f64>,
    #[impasto(nullable)]
    pub speed_accuracy: Option<f64>,
    pub timestamp_ms: f64,
    pub is_mocked: bool,
}

#[derive(Clone, Copy, Debug, ImpastoType)]
pub struct Coordinates {
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Clone, Copy, Debug, Serialize, ImpastoType)]
#[serde(rename_all = "camelCase")]
enum Accuracy {
    Lowest,
    Low,
    Medium,
    High,
    Best,
    BestForNavigation,
}

#[derive(Default, Serialize, ImpastoType)]
#[serde(rename_all = "camelCase")]
struct CurrentOptions {
    accuracy: Option<Accuracy>,
    timeout_ms: Option<u32>,
    maximum_age_ms: Option<u32>,
    ensure_permission: Option<bool>,
}

#[derive(Default, Serialize, ImpastoType)]
#[serde(rename_all = "camelCase")]
struct WatchOptions {
    accuracy: Option<Accuracy>,
    distance_filter_meters: Option<f64>,
    interval_ms: Option<u32>,
    allow_background: Option<bool>,
    ensure_permission: Option<bool>,
}

#[derive(Deserialize, ImpastoType)]
#[serde(rename_all = "camelCase")]
enum LocationService {
    Enabled,
    Disabled,
}
#[derive(ImpastoType)]
enum PermissionScope {
    WhenInUse,
    Always,
}

#[derive(Debug)]
struct LocationError(&'static str, String);
impl LocationError {
    fn new(code: &'static str, message: &str) -> Self {
        Self(code, message.to_owned())
    }
}
impl ImpastoError for LocationError {
    fn scope(&self) -> &'static str {
        "geolocator"
    }
    fn code(&self) -> &'static str {
        self.0
    }
    fn message(&self) -> String {
        self.1.clone()
    }
}
impl From<NativePluginError> for LocationError {
    fn from(error: NativePluginError) -> Self {
        let code = match error.code.as_str() {
            "timeout" => "timeout",
            "cancelled" => "cancelled",
            "service-disabled" => "service-disabled",
            "permission-denied" => "permission-denied",
            "host-unavailable" => "host-unavailable",
            "position-unavailable" => "position-unavailable",
            _ => "platform",
        };
        Self(code, error.message)
    }
}

fn permission_error(error: impasto_permissions::PermissionError) -> LocationError {
    LocationError("platform", error.to_string())
}

async fn authorize(ctx: &PluginContext, background: bool, ask: bool) -> Result<(), LocationError> {
    if !ctx.is_hosted() {
        return Err(LocationError::new(
            "host-unavailable",
            "Location requires a native window",
        ));
    }
    if impasto_permissions::service_status(ctx, Permission::Location)
        .await
        .map_err(permission_error)?
        == ServiceStatus::Disabled
    {
        return Err(LocationError::new(
            "service-disabled",
            "Location services are off",
        ));
    }
    let permission = if background {
        Permission::LocationAlways
    } else {
        Permission::LocationWhenInUse
    };
    let mut status = impasto_permissions::check(ctx, permission)
        .await
        .map_err(permission_error)?;
    if status == PermissionStatus::Denied && ask {
        status = impasto_permissions::ensure(ctx, permission)
            .await
            .map_err(permission_error)?;
    }
    match status {
        PermissionStatus::Granted | PermissionStatus::Limited => Ok(()),
        PermissionStatus::PermanentlyDenied => Err(LocationError::new(
            "permission-permanently-denied",
            "Location access is permanently denied",
        )),
        _ => Err(LocationError::new(
            "permission-denied",
            "Location access is denied",
        )),
    }
}

fn request_cancelled() -> LocationError {
    LocationError::new("cancelled", "Location request cancelled")
}

const EARTH_RADIUS: f64 = 6_371_000.0;

fn radians(point: Coordinates) -> Result<(f64, f64), LocationError> {
    if !(-90.0..=90.0).contains(&point.latitude) || !(-180.0..=180.0).contains(&point.longitude) {
        return Err(LocationError::new(
            "platform",
            "Coordinates are outside their geographic bounds",
        ));
    }
    Ok((point.latitude.to_radians(), point.longitude.to_radians()))
}
fn distance(a: Coordinates, b: Coordinates) -> Result<f64, LocationError> {
    let (lat_a, lon_a) = radians(a)?;
    let (lat_b, lon_b) = radians(b)?;
    let a = ((lat_b - lat_a) / 2.0).sin().powi(2)
        + lat_a.cos() * lat_b.cos() * ((lon_b - lon_a) / 2.0).sin().powi(2);
    Ok(2.0 * EARTH_RADIUS * a.clamp(0.0, 1.0).sqrt().asin())
}
fn bearing(a: Coordinates, b: Coordinates) -> Result<f64, LocationError> {
    let (lat_a, lon_a) = radians(a)?;
    let (lat_b, lon_b) = radians(b)?;
    let dx = lon_b - lon_a;
    let y = dx.sin() * lat_b.cos();
    let x = lat_a.cos() * lat_b.sin() - lat_a.sin() * lat_b.cos() * dx.cos();
    Ok(y.atan2(x).to_degrees().rem_euclid(360.0))
}
fn destination(
    origin: Coordinates,
    distance: f64,
    bearing: f64,
) -> Result<Coordinates, LocationError> {
    let (lat, lon) = radians(origin)?;
    if !distance.is_finite() || distance < 0.0 || !bearing.is_finite() {
        return Err(LocationError::new(
            "platform",
            "Distance and bearing must be finite and distance must be nonnegative",
        ));
    }
    let delta = distance / EARTH_RADIUS;
    let bearing = bearing.rem_euclid(360.0).to_radians();
    let lat_b = (lat.sin() * delta.cos() + lat.cos() * delta.sin() * bearing.cos())
        .clamp(-1.0, 1.0)
        .asin();
    let lon_b = lon
        + (bearing.sin() * delta.sin() * lat.cos()).atan2(delta.cos() - lat.sin() * lat_b.sin());
    Ok(Coordinates {
        latitude: lat_b.to_degrees(),
        longitude: (lon_b.to_degrees() + 180.0).rem_euclid(360.0) - 180.0,
    })
}

#[impasto::module(name = "Geolocator", specifier = "@impasto/geolocator/native", init = init)]
struct Geolocator {
    ctx: PluginContext,
    platform: NativePlugin,
    requests: Mutex<BTreeMap<u64, CancelToken>>,
}

impl Geolocator {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        Ok(Self {
            ctx: ctx.clone(),
            platform: NativePlugin::new(ctx, "@impasto/geolocator"),
            requests: Mutex::default(),
        })
    }
    async fn watch<T: DeserializeOwned + IntoPluginValue>(
        &self,
        topic: &str,
        parameters: &str,
        mut sink: TypedSink<T>,
        cancel: CancelToken,
        prepare: impl Future<Output = Result<(), LocationError>>,
    ) -> Result<(), LocationError> {
        let mut watch = pin!(async {
            prepare.await?;
            let channel = self.ctx.extension::<NativeEventChannel>().ok_or_else(|| {
                LocationError::new(
                    "host-unavailable",
                    "Location requires a native event channel",
                )
            })?;
            let mut events = pin!(
                channel
                    .subscribe_latest(format!("@impasto/geolocator/{topic}"), parameters, 8)
                    .await
                    .map_err(|message| LocationError("platform", message))?
            );
            while let Some(item) = poll_fn(|context| events.as_mut().poll_next(context)).await {
                let result = serde_json::from_str(&item).map_err(|_| {
                    LocationError::new("position-unavailable", "Invalid location sample")
                })?;
                if sink.send(result).await.is_err() {
                    break;
                }
            }
            Ok(())
        });
        let mut cancelled = pin!(cancel.cancelled());
        poll_fn(|ctx| {
            if cancelled.as_mut().poll(ctx).is_ready() {
                Poll::Ready(Ok(()))
            } else {
                watch.as_mut().poll(ctx)
            }
        })
        .await
    }
}

#[impasto::export]
impl Geolocator {
    pub fn is_geolocator_host_available(&self) -> bool {
        self.platform.is_hosted()
    }
    pub async fn check_permission(&self) -> Result<PermissionStatus, LocationError> {
        impasto_permissions::check(&self.ctx, Permission::LocationWhenInUse)
            .await
            .map_err(permission_error)
    }
    #[impasto::method(awaits_user)]
    pub async fn request_permission(
        &self,
        scope: Option<PermissionScope>,
    ) -> Result<PermissionStatus, LocationError> {
        let permission = if matches!(scope, Some(PermissionScope::Always)) {
            Permission::LocationAlways
        } else {
            Permission::LocationWhenInUse
        };
        impasto_permissions::ensure(&self.ctx, permission)
            .await
            .map_err(permission_error)
    }
    pub async fn is_location_service_enabled(&self) -> Result<bool, LocationError> {
        Ok(self.platform.request("service", &()).await?)
    }
    pub fn next_request_id(&self) -> Result<u64, LocationError> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let mut pending = self.requests.lock().map_err(|_| request_cancelled())?;
        if pending.len() >= 64 {
            return Err(LocationError::new(
                "platform",
                "Too many active location requests",
            ));
        }
        pending.insert(id, CancelToken::new());
        Ok(id)
    }

    #[impasto::method(awaits_user)]
    pub async fn get_current_position(
        &self,
        id: u64,
        options: Option<CurrentOptions>,
    ) -> Result<Position, LocationError> {
        let cancel = self
            .requests
            .lock()
            .map_err(|_| request_cancelled())?
            .get(&id)
            .cloned()
            .ok_or_else(request_cancelled)?;
        let mut operation = pin!(async {
            let options = options.unwrap_or_default();
            authorize(&self.ctx, false, options.ensure_permission.unwrap_or(true)).await?;
            if cancel.is_cancelled() {
                return Err(request_cancelled());
            }
            let mut payload = serde_json::to_value(options)
                .map_err(|_| LocationError::new("platform", "Invalid location options"))?;
            payload["id"] = id.into();
            Ok(self.platform.request("current", &payload).await?)
        });
        let mut cancelled = pin!(cancel.cancelled());
        let result = poll_fn(|context| {
            if cancelled.as_mut().poll(context).is_ready() {
                Poll::Ready(Err(request_cancelled()))
            } else {
                operation.as_mut().poll(context)
            }
        })
        .await;
        if let Ok(mut pending) = self.requests.lock() {
            pending.remove(&id);
        }
        result
    }

    pub async fn cancel(&self, id: u64) -> Result<(), LocationError> {
        let pending = self
            .requests
            .lock()
            .map_err(|_| request_cancelled())?
            .remove(&id);
        if let Some(token) = pending {
            token.cancel();
            self.platform.request::<()>("cancel", &id).await?;
        }
        Ok(())
    }
    pub async fn get_last_known_position(&self) -> Result<Option<Position>, LocationError> {
        authorize(&self.ctx, false, false).await?;
        Ok(self.platform.request("last", &()).await?)
    }
    pub async fn watch_position(
        &self,
        options: Option<WatchOptions>,
        sink: TypedSink<Position>,
        cancel: CancelToken,
    ) -> Result<(), LocationError> {
        let options = options.unwrap_or_default();
        if options
            .distance_filter_meters
            .is_some_and(|distance| !(0.0..=100_000.0).contains(&distance))
            || options.interval_ms == Some(0)
        {
            return Err(LocationError::new(
                "platform",
                "Location filter and interval must be in range",
            ));
        }
        let parameters = serde_json::to_string(&options)
            .map_err(|_| LocationError::new("platform", "Invalid location options"))?;
        self.watch(
            "position",
            &parameters,
            sink,
            cancel,
            authorize(
                &self.ctx,
                options.allow_background.unwrap_or(false),
                options.ensure_permission.unwrap_or(true),
            ),
        )
        .await
    }
    pub async fn watch_service_status(
        &self,
        sink: TypedSink<LocationService>,
        cancel: CancelToken,
    ) -> Result<(), LocationError> {
        self.watch("service", "", sink, cancel, async { Ok(()) })
            .await
    }
    pub fn distance_between(&self, a: Coordinates, b: Coordinates) -> Result<f64, LocationError> {
        distance(a, b)
    }
    pub fn bearing_between(&self, a: Coordinates, b: Coordinates) -> Result<f64, LocationError> {
        bearing(a, b)
    }
    pub fn destination_point(
        &self,
        origin: Coordinates,
        distance_meters: f64,
        bearing_degrees: f64,
    ) -> Result<Coordinates, LocationError> {
        destination(origin, distance_meters, bearing_degrees)
    }
}

/// Registers the geolocation bindings.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(Geolocator::plugin());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eastbound_destination_round_trips_and_wraps_longitude() {
        let a = Coordinates {
            latitude: 0.0,
            longitude: 179.9,
        };
        let b = destination(a, 30_000.0, 90.0).unwrap();
        assert!(b.longitude < -179.0);
        assert!((distance(a, b).unwrap() - 30_000.0).abs() < 0.001);
        assert!((bearing(a, b).unwrap() - 90.0).abs() < 0.001);
    }
}
