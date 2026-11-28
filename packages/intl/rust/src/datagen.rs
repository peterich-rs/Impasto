//! Builds the application's ICU blob from the same versioned data as the host.

use std::collections::{BTreeSet, HashSet};

use icu::locale::Locale;
use icu_locale_core::preferences::LocalePreferences;
use icu_locale_fallback::LocaleFallbacker;
use icu_provider::export::{DataExporter, ExportableProvider};
use icu_provider::prelude::*;
use icu_provider_blob::export::BlobExporter;

extern crate alloc;

struct Source;
icu_plurals_data::make_provider!(Source);

macro_rules! declare_markers {
    (
        localized { $($component:ident :: $loader:ident => $marker:path,)* }
        singleton { $($singleton:ident :: $load_singleton:ident => $singleton_marker:path,)* }
    ) => {
        $(use $component::$loader; $loader!(Source, ITER);)*
        $(
            $singleton::$load_singleton!(Source);
            impl IterableDataProvider<$singleton_marker> for Source {
                fn iter_ids(&self) -> Result<BTreeSet<DataIdentifierCow<'static>>, DataError> {
                    Ok(BTreeSet::from([DataIdentifierCow::default()]))
                }
            }
        )*
        icu_provider::export::make_exportable_provider!(Source, [$($marker,)* $($singleton_marker,)*]);
    };
}
// Singleton IDs are always empty. ICU4X 2.3's generated iterator macros
// for these markers use an invalid type name, so only use their load macros.
declare_markers! {
    localized {

    icu_calendar_data::__impl_calendar_preferred_v1 => icu::calendar::provider::CalendarPreferredV1,
    icu_calendar_data::__impl_calendar_week_v1 => icu::calendar::provider::CalendarWeekV1,




    icu_datetime_data::__impl_datetime_names_dayperiod_v1 => icu::datetime::provider::names::DatetimeNamesDayperiodV1,
    icu_datetime_data::__impl_datetime_names_month_buddhist_v1 => icu::datetime::provider::names::DatetimeNamesMonthBuddhistV1,
    icu_datetime_data::__impl_datetime_names_month_chinese_v1 => icu::datetime::provider::names::DatetimeNamesMonthChineseV1,
    icu_datetime_data::__impl_datetime_names_month_coptic_v1 => icu::datetime::provider::names::DatetimeNamesMonthCopticV1,
    icu_datetime_data::__impl_datetime_names_month_dangi_v1 => icu::datetime::provider::names::DatetimeNamesMonthDangiV1,
    icu_datetime_data::__impl_datetime_names_month_ethiopian_v1 => icu::datetime::provider::names::DatetimeNamesMonthEthiopianV1,
    icu_datetime_data::__impl_datetime_names_month_gregorian_v1 => icu::datetime::provider::names::DatetimeNamesMonthGregorianV1,
    icu_datetime_data::__impl_datetime_names_month_hebrew_v1 => icu::datetime::provider::names::DatetimeNamesMonthHebrewV1,
    icu_datetime_data::__impl_datetime_names_month_hijri_v1 => icu::datetime::provider::names::DatetimeNamesMonthHijriV1,
    icu_datetime_data::__impl_datetime_names_month_indian_v1 => icu::datetime::provider::names::DatetimeNamesMonthIndianV1,
    icu_datetime_data::__impl_datetime_names_month_japanese_v1 => icu::datetime::provider::names::DatetimeNamesMonthJapaneseV1,
    icu_datetime_data::__impl_datetime_names_month_persian_v1 => icu::datetime::provider::names::DatetimeNamesMonthPersianV1,
    icu_datetime_data::__impl_datetime_names_month_roc_v1 => icu::datetime::provider::names::DatetimeNamesMonthRocV1,
    icu_datetime_data::__impl_datetime_names_weekday_v1 => icu::datetime::provider::names::DatetimeNamesWeekdayV1,
    icu_datetime_data::__impl_datetime_names_year_buddhist_v1 => icu::datetime::provider::names::DatetimeNamesYearBuddhistV1,
    icu_datetime_data::__impl_datetime_names_year_chinese_v1 => icu::datetime::provider::names::DatetimeNamesYearChineseV1,
    icu_datetime_data::__impl_datetime_names_year_coptic_v1 => icu::datetime::provider::names::DatetimeNamesYearCopticV1,
    icu_datetime_data::__impl_datetime_names_year_dangi_v1 => icu::datetime::provider::names::DatetimeNamesYearDangiV1,
    icu_datetime_data::__impl_datetime_names_year_ethiopian_v1 => icu::datetime::provider::names::DatetimeNamesYearEthiopianV1,
    icu_datetime_data::__impl_datetime_names_year_gregorian_v1 => icu::datetime::provider::names::DatetimeNamesYearGregorianV1,
    icu_datetime_data::__impl_datetime_names_year_hebrew_v1 => icu::datetime::provider::names::DatetimeNamesYearHebrewV1,
    icu_datetime_data::__impl_datetime_names_year_hijri_v1 => icu::datetime::provider::names::DatetimeNamesYearHijriV1,
    icu_datetime_data::__impl_datetime_names_year_indian_v1 => icu::datetime::provider::names::DatetimeNamesYearIndianV1,
    icu_datetime_data::__impl_datetime_names_year_japanese_v1 => icu::datetime::provider::names::DatetimeNamesYearJapaneseV1,
    icu_datetime_data::__impl_datetime_names_year_persian_v1 => icu::datetime::provider::names::DatetimeNamesYearPersianV1,
    icu_datetime_data::__impl_datetime_names_year_roc_v1 => icu::datetime::provider::names::DatetimeNamesYearRocV1,
    icu_datetime_data::__impl_datetime_patterns_date_buddhist_v1 => icu::datetime::provider::semantic_skeletons::DatetimePatternsDateBuddhistV1,
    icu_datetime_data::__impl_datetime_patterns_date_chinese_v1 => icu::datetime::provider::semantic_skeletons::DatetimePatternsDateChineseV1,
    icu_datetime_data::__impl_datetime_patterns_date_coptic_v1 => icu::datetime::provider::semantic_skeletons::DatetimePatternsDateCopticV1,
    icu_datetime_data::__impl_datetime_patterns_date_dangi_v1 => icu::datetime::provider::semantic_skeletons::DatetimePatternsDateDangiV1,
    icu_datetime_data::__impl_datetime_patterns_date_ethiopian_v1 => icu::datetime::provider::semantic_skeletons::DatetimePatternsDateEthiopianV1,
    icu_datetime_data::__impl_datetime_patterns_date_gregorian_v1 => icu::datetime::provider::semantic_skeletons::DatetimePatternsDateGregorianV1,
    icu_datetime_data::__impl_datetime_patterns_date_hebrew_v1 => icu::datetime::provider::semantic_skeletons::DatetimePatternsDateHebrewV1,
    icu_datetime_data::__impl_datetime_patterns_date_hijri_v1 => icu::datetime::provider::semantic_skeletons::DatetimePatternsDateHijriV1,
    icu_datetime_data::__impl_datetime_patterns_date_indian_v1 => icu::datetime::provider::semantic_skeletons::DatetimePatternsDateIndianV1,
    icu_datetime_data::__impl_datetime_patterns_date_japanese_v1 => icu::datetime::provider::semantic_skeletons::DatetimePatternsDateJapaneseV1,
    icu_datetime_data::__impl_datetime_patterns_date_persian_v1 => icu::datetime::provider::semantic_skeletons::DatetimePatternsDatePersianV1,
    icu_datetime_data::__impl_datetime_patterns_date_roc_v1 => icu::datetime::provider::semantic_skeletons::DatetimePatternsDateRocV1,
    icu_datetime_data::__impl_datetime_patterns_glue_v1 => icu::datetime::provider::semantic_skeletons::DatetimePatternsGlueV1,
    icu_datetime_data::__impl_datetime_patterns_time_v1 => icu::datetime::provider::semantic_skeletons::DatetimePatternsTimeV1,
    icu_datetime_data::__impl_timezone_names_cities_override_v1 => icu::datetime::provider::time_zones::TimezoneNamesCitiesOverrideV1,
    icu_datetime_data::__impl_timezone_names_cities_root_v1 => icu::datetime::provider::time_zones::TimezoneNamesCitiesRootV1,
    icu_datetime_data::__impl_timezone_names_essentials_v1 => icu::datetime::provider::time_zones::TimezoneNamesEssentialsV1,
    icu_datetime_data::__impl_timezone_names_generic_long_v1 => icu::datetime::provider::time_zones::TimezoneNamesGenericLongV1,
    icu_datetime_data::__impl_timezone_names_generic_short_v1 => icu::datetime::provider::time_zones::TimezoneNamesGenericShortV1,
    icu_datetime_data::__impl_timezone_names_locations_override_v1 => icu::datetime::provider::time_zones::TimezoneNamesLocationsOverrideV1,
    icu_datetime_data::__impl_timezone_names_locations_root_v1 => icu::datetime::provider::time_zones::TimezoneNamesLocationsRootV1,
    icu_datetime_data::__impl_timezone_names_specific_long_v1 => icu::datetime::provider::time_zones::TimezoneNamesSpecificLongV1,
    icu_datetime_data::__impl_timezone_names_specific_short_v1 => icu::datetime::provider::time_zones::TimezoneNamesSpecificShortV1,
    icu_datetime_data::__impl_timezone_names_standard_long_v1 => icu::datetime::provider::time_zones::TimezoneNamesStandardLongV1,
    icu_decimal_data::__impl_decimal_compact_long_v1 => icu::decimal::provider::DecimalCompactLongV1,
    icu_decimal_data::__impl_decimal_compact_short_v1 => icu::decimal::provider::DecimalCompactShortV1,
    icu_decimal_data::__impl_decimal_digits_v1 => icu::decimal::provider::DecimalDigitsV1,
    icu_decimal_data::__impl_decimal_symbols_v1 => icu::decimal::provider::DecimalSymbolsV1,
    icu_collator_data::__impl_collation_diacritics_v1 => icu::collator::provider::CollationDiacriticsV1,

    icu_collator_data::__impl_collation_metadata_v1 => icu::collator::provider::CollationMetadataV1,
    icu_collator_data::__impl_collation_reordering_v1 => icu::collator::provider::CollationReorderingV1,


    icu_collator_data::__impl_collation_tailoring_v1 => icu::collator::provider::CollationTailoringV1,







    icu_plurals_data::__impl_plurals_cardinal_v1 => icu::plurals::provider::PluralsCardinalV1,
    icu_plurals_data::__impl_plurals_ordinal_v1 => icu::plurals::provider::PluralsOrdinalV1,
    icu_plurals_data::__impl_plurals_ranges_v1 => icu::plurals::provider::PluralsRangesV1,
    icu_experimental_data::__impl_currency_displayname_v1 => icu::experimental::dimension::provider::currency::displayname::CurrencyDisplaynameV1,
    icu_experimental_data::__impl_currency_essentials_v1 => icu::experimental::dimension::provider::currency::essentials::CurrencyEssentialsV1,
    icu_experimental_data::__impl_currency_extended_data_v1 => icu::experimental::dimension::provider::currency::extended::CurrencyExtendedDataV1,

    icu_experimental_data::__impl_currency_patterns_data_v1 => icu::experimental::dimension::provider::currency::patterns::CurrencyPatternsDataV1,
    icu_experimental_data::__impl_currency_patterns_no_currency_v1 => icu::experimental::dimension::provider::currency::no_currency::CurrencyPatternsNoCurrencyV1,
    icu_experimental_data::__impl_currency_symbols_v1 => icu::experimental::dimension::provider::currency::symbols::CurrencySymbolsV1,
    icu_experimental_data::__impl_long_day_relative_v1 => icu::experimental::relativetime::provider::LongDayRelativeV1,
    icu_experimental_data::__impl_long_hour_relative_v1 => icu::experimental::relativetime::provider::LongHourRelativeV1,
    icu_experimental_data::__impl_long_minute_relative_v1 => icu::experimental::relativetime::provider::LongMinuteRelativeV1,
    icu_experimental_data::__impl_long_month_relative_v1 => icu::experimental::relativetime::provider::LongMonthRelativeV1,
    icu_experimental_data::__impl_long_quarter_relative_v1 => icu::experimental::relativetime::provider::LongQuarterRelativeV1,
    icu_experimental_data::__impl_long_second_relative_v1 => icu::experimental::relativetime::provider::LongSecondRelativeV1,
    icu_experimental_data::__impl_long_week_relative_v1 => icu::experimental::relativetime::provider::LongWeekRelativeV1,
    icu_experimental_data::__impl_long_year_relative_v1 => icu::experimental::relativetime::provider::LongYearRelativeV1,
    icu_experimental_data::__impl_narrow_day_relative_v1 => icu::experimental::relativetime::provider::NarrowDayRelativeV1,
    icu_experimental_data::__impl_narrow_hour_relative_v1 => icu::experimental::relativetime::provider::NarrowHourRelativeV1,
    icu_experimental_data::__impl_narrow_minute_relative_v1 => icu::experimental::relativetime::provider::NarrowMinuteRelativeV1,
    icu_experimental_data::__impl_narrow_month_relative_v1 => icu::experimental::relativetime::provider::NarrowMonthRelativeV1,
    icu_experimental_data::__impl_narrow_quarter_relative_v1 => icu::experimental::relativetime::provider::NarrowQuarterRelativeV1,
    icu_experimental_data::__impl_narrow_second_relative_v1 => icu::experimental::relativetime::provider::NarrowSecondRelativeV1,
    icu_experimental_data::__impl_narrow_week_relative_v1 => icu::experimental::relativetime::provider::NarrowWeekRelativeV1,
    icu_experimental_data::__impl_narrow_year_relative_v1 => icu::experimental::relativetime::provider::NarrowYearRelativeV1,
    icu_experimental_data::__impl_percent_essentials_v1 => icu::experimental::dimension::provider::percent::PercentEssentialsV1,
    icu_experimental_data::__impl_short_day_relative_v1 => icu::experimental::relativetime::provider::ShortDayRelativeV1,
    icu_experimental_data::__impl_short_hour_relative_v1 => icu::experimental::relativetime::provider::ShortHourRelativeV1,
    icu_experimental_data::__impl_short_minute_relative_v1 => icu::experimental::relativetime::provider::ShortMinuteRelativeV1,
    icu_experimental_data::__impl_short_month_relative_v1 => icu::experimental::relativetime::provider::ShortMonthRelativeV1,
    icu_experimental_data::__impl_short_quarter_relative_v1 => icu::experimental::relativetime::provider::ShortQuarterRelativeV1,
    icu_experimental_data::__impl_short_second_relative_v1 => icu::experimental::relativetime::provider::ShortSecondRelativeV1,
    icu_experimental_data::__impl_short_week_relative_v1 => icu::experimental::relativetime::provider::ShortWeekRelativeV1,
    icu_experimental_data::__impl_short_year_relative_v1 => icu::experimental::relativetime::provider::ShortYearRelativeV1,


    }
    singleton {
    icu_calendar_data::__impl_calendar_japanese_modern_v1 => icu::calendar::provider::CalendarJapaneseModernV1,
    icu_collator_data::__impl_collation_jamo_v1 => icu::collator::provider::CollationJamoV1,
    icu_collator_data::__impl_collation_root_v1 => icu::collator::provider::CollationRootV1,
    icu_collator_data::__impl_collation_special_primaries_v1 => icu::collator::provider::CollationSpecialPrimariesV1,
    icu_normalizer_data::__impl_normalizer_nfc_v1 => icu::normalizer::provider::NormalizerNfcV1,
    icu_normalizer_data::__impl_normalizer_nfd_data_v1 => icu::normalizer::provider::NormalizerNfdDataV1,
    icu_normalizer_data::__impl_normalizer_nfd_supplement_v1 => icu::normalizer::provider::NormalizerNfdSupplementV1,
    icu_normalizer_data::__impl_normalizer_nfd_tables_v1 => icu::normalizer::provider::NormalizerNfdTablesV1,
    icu_normalizer_data::__impl_normalizer_nfkd_data_v1 => icu::normalizer::provider::NormalizerNfkdDataV1,
    icu_normalizer_data::__impl_normalizer_nfkd_tables_v1 => icu::normalizer::provider::NormalizerNfkdTablesV1,
    icu_normalizer_data::__impl_normalizer_uts46_data_v1 => icu::normalizer::provider::NormalizerUts46DataV1,
    icu_experimental_data::__impl_currency_fractions_v1 => icu::experimental::dimension::provider::currency::fractions::CurrencyFractionsV1,
    icu_locale_fallback_data::__impl_locale_likely_subtags_language_v1 => icu::locale::fallback::provider::LocaleLikelySubtagsLanguageV1,
    icu_locale_fallback_data::__impl_locale_parents_v1 => icu::locale::fallback::provider::LocaleParentsV1,
    }
}

/// Exports a compressed ICU postcard blob for the selected languages and their CLDR parents.
pub fn export_data(locales: &[String]) -> Result<Vec<u8>, String> {
    let languages = locales
        .iter()
        .map(|tag| super::locale(tag).map(|locale: Locale| LocalePreferences::from(&locale)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "Invalid @impasto/intl locale".to_owned())?;
    let mut data = Vec::new();
    {
        let source = Source;
        let mut exporter = BlobExporter::new_with_sink(Box::new(&mut data));
        for marker in source.supported_markers() {
            let fallback = LocaleFallbacker::new().for_config(marker.fallback_config);
            let parents = languages
                .iter()
                .flat_map(|language| {
                    let mut parents = Vec::new();
                    let mut chain = fallback.fallback_for(marker.make_locale(*language));
                    loop {
                        parents.push(*chain.get());
                        if chain.get().is_unknown() {
                            break;
                        }
                        chain.step();
                    }
                    parents
                })
                .collect::<HashSet<_>>();
            for id in source
                .iter_ids_for_marker(marker)
                .map_err(|error| error.to_string())?
            {
                if !marker.is_singleton && !parents.contains(&id.locale) {
                    continue;
                }
                let id = id.as_borrowed();
                let response = source
                    .load_data(
                        marker,
                        DataRequest {
                            id,
                            metadata: Default::default(),
                        },
                    )
                    .map_err(|error| error.to_string())?;
                exporter
                    .put_payload(marker, id, &response.payload)
                    .map_err(|error| error.to_string())?;
            }
            exporter
                .flush(marker, Default::default())
                .map_err(|error| error.to_string())?;
        }
        exporter.close().map_err(|error| error.to_string())?;
    }
    Ok(miniz_oxide::deflate::compress_to_vec_zlib(&data, 10))
}
