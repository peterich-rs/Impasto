use icu_provider::buf::BufferMarker;
use icu_provider::prelude::*;
use icu_provider_blob::BlobDataProvider;

#[cfg(feature = "embedded-data")]
use std::sync::OnceLock;

#[cfg(feature = "embedded-data")]
pub(crate) static CONFIGURED: OnceLock<&'static [u8]> = OnceLock::new();

pub(crate) struct Data(BlobDataProvider);

#[cfg(feature = "embedded-data")]
pub(crate) fn configured() -> Result<Option<&'static Data>, String> {
    static DECODED: OnceLock<Result<Data, String>> = OnceLock::new();
    CONFIGURED
        .get()
        .map(|compressed| {
            DECODED
                .get_or_init(|| {
                    let bytes = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(
                        compressed,
                        64 * 1024 * 1024,
                    )
                    .map_err(|_| "Invalid compressed ICU data".to_owned())?;
                    BlobDataProvider::try_new_from_blob(bytes.into_boxed_slice())
                        .map(Data)
                        .map_err(|error| error.to_string())
                })
                .as_ref()
                .map_err(Clone::clone)
        })
        .transpose()
}

impl DynamicDataProvider<BufferMarker> for Data {
    fn load_data(
        &self,
        marker: DataMarkerInfo,
        request: DataRequest<'_>,
    ) -> Result<DataResponse<BufferMarker>, DataError> {
        use icu_locale_fallback::LocaleFallbacker;

        if marker.is_singleton {
            return self.0.load_data(marker, request);
        }
        let fallback = LocaleFallbacker::new().for_config(marker.fallback_config);
        let mut locales = fallback.fallback_for(*request.id.locale);
        loop {
            let candidate = locales.get();
            let id = DataIdentifierBorrowed::for_marker_attributes_and_locale(
                request.id.marker_attributes,
                candidate,
            );
            match self.0.load_data(
                marker,
                DataRequest {
                    id,
                    metadata: request.metadata,
                },
            ) {
                Ok(mut result) => {
                    result.metadata.locale = Some(*candidate);
                    return Ok(result);
                }
                Err(error)
                    if error.kind == DataErrorKind::IdentifierNotFound
                        && !candidate.is_unknown() =>
                {
                    locales.step();
                }
                Err(error) => return Err(error),
            }
        }
    }
}

macro_rules! construct {
    ($data:expr, $type:ident :: $built_in:ident, $buffer:ident $(, $arg:expr)* $(,)?) => {{
        #[cfg(feature = "embedded-data")]
        {
            $type::$buffer($data.ok_or_else(crate::IntlError::data)?, $($arg),*)
        }
        #[cfg(not(feature = "embedded-data"))]
        {
            let _ = $data;
            $type::$built_in($($arg),*)
        }
    }};
}

pub(crate) use construct;
