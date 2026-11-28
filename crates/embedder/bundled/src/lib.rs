//! Capability plugins bundled by framework development and SDK hosts.

/// Registers the complete development-host catalog.
pub fn register(registrar: &mut impasto::PluginRegistrar) {
    impasto_app_state::register(registrar);
    impasto_async_storage::register(registrar);
    impasto_clipboard::register(registrar);
    impasto_connectivity::register(registrar);
    impasto_devtools::register(registrar);
    impasto_file_system::register(registrar);
    impasto_haptics::register(registrar);
    impasto_http::register(registrar);
    impasto_linking::register(registrar);
    impasto_permissions::register(registrar);
    impasto_share::register(registrar);
    impasto_system_info::register(registrar);
    impasto_crypto::register(registrar);
    impasto_sqlite::register(registrar);
    impasto_intl::register(registrar);
    impasto_biometrics::register(registrar);
    impasto_image_picker::register(registrar);
    impasto_geolocator::register(registrar);
    impasto_notifications::register(registrar);
    impasto_websocket::register(registrar);
}
