//! ICU4X internationalization with synchronous resource-local formatters.

mod data;
#[cfg(feature = "datagen")]
pub mod datagen;
mod numbers;
mod temporal;

use std::{cmp::Ordering, num::NonZeroUsize, sync::Mutex};

use fixed_decimal::{Decimal, FloatPrecision};
use icu::{
    collator::{
        Collator, CollatorPreferences,
        options::{CaseLevel, CollatorOptions, Strength},
        preferences::CollationNumericOrdering,
    },
    locale::{
        LanguageIdentifier, Locale, LocaleCanonicalizer, LocaleDirectionality, LocaleExpander,
        subtags::Language,
    },
    plurals::{PluralCategory as IcuCategory, PluralRules},
};
use impasto::{ImpastoError, ImpastoType, PluginContext, PluginInitError, PluginRegistrar};
use lru::LruCache;

#[derive(Debug)]
struct IntlError(&'static str, &'static str);
impl IntlError {
    fn invalid() -> Self {
        Self("invalid-input", "Invalid format options or value")
    }
    fn data() -> Self {
        Self(
            "missing-locale-data",
            "The requested locale data is unavailable",
        )
    }
}
impl ImpastoError for IntlError {
    fn scope(&self) -> &'static str {
        "intl"
    }
    fn code(&self) -> &'static str {
        self.0
    }
    fn message(&self) -> String {
        self.1.to_owned()
    }
}

fn locale(tag: &str) -> Result<Locale, IntlError> {
    let mut locale: Locale = tag
        .parse()
        .map_err(|_| IntlError("invalid-locale", "Invalid BCP 47 locale tag"))?;
    LocaleCanonicalizer::new_extended().canonicalize(&mut locale);
    Ok(locale)
}
fn decimal(value: f64) -> Result<Decimal, IntlError> {
    if !value.is_finite() {
        return Err(IntlError::invalid());
    }
    Decimal::try_from_f64(value, FloatPrecision::RoundTrip).map_err(|_| IntlError::invalid())
}

#[derive(ImpastoType)]
enum Direction {
    Ltr,
    Rtl,
}
#[derive(ImpastoType)]
struct ParsedLocale {
    canonical: String,
    language: String,
    #[impasto(nullable)]
    script: Option<String>,
    #[impasto(nullable)]
    region: Option<String>,
    direction: Direction,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, ImpastoType)]
enum Plural {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}
impl From<IcuCategory> for Plural {
    fn from(value: IcuCategory) -> Self {
        match value {
            IcuCategory::Zero => Self::Zero,
            IcuCategory::One => Self::One,
            IcuCategory::Two => Self::Two,
            IcuCategory::Few => Self::Few,
            IcuCategory::Many => Self::Many,
            IcuCategory::Other => Self::Other,
        }
    }
}
#[derive(ImpastoType)]
enum PluralType {
    Cardinal,
    Ordinal,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq, ImpastoType)]
enum Sensitivity {
    Base,
    Accent,
    Case,
    Variant,
}
#[derive(Clone, Default, Eq, Hash, PartialEq, ImpastoType)]
struct CompareOptions {
    sensitivity: Option<Sensitivity>,
    numeric: Option<bool>,
}

struct Cache<K: Eq + std::hash::Hash, V>(Mutex<LruCache<K, V>>);
impl<K: Eq + std::hash::Hash, V> Cache<K, V> {
    fn new() -> Self {
        Self(Mutex::new(LruCache::new(
            NonZeroUsize::new(64).unwrap_or(NonZeroUsize::MIN),
        )))
    }
    fn get<T>(
        &self,
        key: K,
        create: impl FnOnce() -> Result<V, IntlError>,
        use_item: impl FnOnce(&V) -> T,
    ) -> Result<T, IntlError> {
        let mut cache = self.0.lock().map_err(|_| IntlError::data())?;
        let entry = cache.try_get_or_insert(key, create)?;
        Ok(use_item(entry))
    }
}

#[impasto::module(name = "Intl", specifier = "@impasto/intl/native", init = init)]
struct Intl {
    data: Option<&'static data::Data>,
    hosted: bool,
    plurals: Cache<(String, bool), PluralRules>,
    collators: Cache<(String, CompareOptions), Collator>,
    numbers: Cache<(String, numbers::Options), numbers::Formatter>,
    temporal: temporal::Formatters,
}

impl Intl {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        #[cfg(feature = "embedded-data")]
        let data = data::configured().map_err(PluginInitError::new)?;
        #[cfg(not(feature = "embedded-data"))]
        let data = None;
        Ok(Self {
            data,
            hosted: ctx.is_hosted(),
            plurals: Cache::new(),
            collators: Cache::new(),
            numbers: Cache::new(),
            temporal: temporal::Formatters::new(),
        })
    }
    fn with_collator<T>(
        &self,
        language: String,
        options: Option<CompareOptions>,
        use_item: impl FnOnce(&Collator) -> T,
    ) -> Result<T, IntlError> {
        let lang = locale(&language)?;
        let options = options.unwrap_or_default();
        self.collators.get(
            (lang.to_string(), options.clone()),
            || {
                let mut icu_options = CollatorOptions::default();
                icu_options.strength =
                    Some(match options.sensitivity.unwrap_or(Sensitivity::Variant) {
                        Sensitivity::Base | Sensitivity::Case => Strength::Primary,
                        Sensitivity::Accent => Strength::Secondary,
                        Sensitivity::Variant => Strength::Tertiary,
                    });
                if options.sensitivity == Some(Sensitivity::Case) {
                    icu_options.case_level = Some(CaseLevel::On);
                }
                let mut prefs = CollatorPreferences::from(&lang);
                if let Some(numeric) = options.numeric {
                    prefs.numeric_ordering = Some(if numeric {
                        CollationNumericOrdering::True
                    } else {
                        CollationNumericOrdering::False
                    });
                }
                #[cfg(feature = "embedded-data")]
                {
                    Collator::try_new_with_buffer_provider(
                        self.data.ok_or_else(IntlError::data)?,
                        prefs,
                        icu_options,
                    )
                    .map_err(|_| IntlError::data())
                }
                #[cfg(not(feature = "embedded-data"))]
                {
                    Ok(Collator::try_new(prefs, icu_options)
                        .map_err(|_| IntlError::data())?
                        .static_to_owned())
                }
            },
            use_item,
        )
    }
}

#[impasto::export]
impl Intl {
    pub fn is_intl_host_available(&self) -> bool {
        self.hosted
    }
    pub fn parse_locale(&self, tag: String) -> Result<ParsedLocale, IntlError> {
        let locale = locale(&tag)?;
        let rtl = LocaleDirectionality::new_extended().get(&locale.id)
            == Some(icu::locale::Direction::RightToLeft);
        Ok(ParsedLocale {
            canonical: locale.to_string(),
            language: locale.id.language.to_string(),
            script: locale.id.script.as_ref().map(ToString::to_string),
            region: locale.id.region.as_ref().map(ToString::to_string),
            direction: if rtl { Direction::Rtl } else { Direction::Ltr },
        })
    }
    pub fn match_locale(
        &self,
        requested: Vec<String>,
        available: Vec<String>,
        fallback: String,
    ) -> Result<String, IntlError> {
        let available = available
            .iter()
            .map(|tag| locale(tag).map(|locale| locale.id))
            .collect::<Result<Vec<_>, _>>()?;
        for requested in requested {
            let request = locale(&requested)?.id;
            if let Some(found) = available.iter().find(|id| **id == request) {
                return Ok(found.to_string());
            }
            if let Some(found) = closest(&request, &available) {
                return Ok(found.to_string());
            }
        }
        Ok(locale(&fallback)?.to_string())
    }
    pub fn plural_category(
        &self,
        tag: String,
        count: f64,
        kind: Option<PluralType>,
    ) -> Result<Plural, IntlError> {
        let locale = locale(&tag)?;
        let ordinal = matches!(kind, Some(PluralType::Ordinal));
        let count = decimal(count)?;
        self.plurals.get(
            (locale.to_string(), ordinal),
            || {
                let prefs = (&locale).into();
                if ordinal {
                    data::construct!(
                        self.data,
                        PluralRules::try_new_ordinal,
                        try_new_ordinal_with_buffer_provider,
                        prefs
                    )
                } else {
                    data::construct!(
                        self.data,
                        PluralRules::try_new_cardinal,
                        try_new_cardinal_with_buffer_provider,
                        prefs
                    )
                }
                .map_err(|_| IntlError::data())
            },
            |rules| rules.category_for(&count).into(),
        )
    }
    pub fn format_number(
        &self,
        tag: String,
        value: f64,
        options: Option<numbers::Options>,
    ) -> Result<String, IntlError> {
        let locale = locale(&tag)?;
        let options = options.unwrap_or_default();
        let value = numbers::value(value, &options)?;
        self.numbers.get(
            (locale.to_string(), options.clone()),
            || numbers::Formatter::new(self.data, &locale, &options),
            |formatter| formatter.format(&value),
        )
    }
    pub fn format_date_time(
        &self,
        tag: String,
        epoch: i64,
        options: Option<temporal::DateOptions>,
    ) -> Result<String, IntlError> {
        self.temporal
            .date(self.data, locale(&tag)?, epoch, options.unwrap_or_default())
    }
    pub fn format_relative_time(
        &self,
        tag: String,
        value: f64,
        options: temporal::RelativeOptions,
    ) -> Result<String, IntlError> {
        self.temporal
            .relative(self.data, locale(&tag)?, decimal(value)?, options)
    }
    pub fn compare_strings(
        &self,
        tag: String,
        a: String,
        b: String,
        options: Option<CompareOptions>,
    ) -> Result<i32, IntlError> {
        self.with_collator(tag, options, |collator| {
            match collator.as_borrowed().compare(&a, &b) {
                Ordering::Less => -1,
                Ordering::Equal => 0,
                Ordering::Greater => 1,
            }
        })
    }
    pub fn sort_strings(
        &self,
        tag: String,
        mut items: Vec<String>,
        options: Option<CompareOptions>,
    ) -> Result<Vec<String>, IntlError> {
        self.with_collator(tag, options, |collator| {
            let collator = collator.as_borrowed();
            items.sort_by(|a, b| collator.compare(a, b));
            items
        })
    }
}

fn closest<'a>(
    requested: &LanguageIdentifier,
    available: &'a [LanguageIdentifier],
) -> Option<&'a LanguageIdentifier> {
    if requested.language == Language::UNKNOWN {
        return None;
    }
    let expander = LocaleExpander::new_extended();
    let mut expanded = requested.clone();
    expander.maximize(&mut expanded);
    available
        .iter()
        .filter(|value| value.language == requested.language)
        .min_by_key(|candidate| {
            let mut id = (*candidate).clone();
            expander.maximize(&mut id);
            (id.script != expanded.script, id.region != expanded.region)
        })
}

/// Registers the locale-aware formatter bindings.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(Intl::plugin());
}

/// Registers a generated application with its own embedded ICU data.
#[cfg(feature = "embedded-data")]
pub fn register_with_data(registrar: &mut PluginRegistrar, data: &'static [u8]) {
    let _ = data::CONFIGURED.set(data);
    register(registrar);
}
