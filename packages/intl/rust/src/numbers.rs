use fixed_decimal::Decimal;
use icu::decimal::{CompactDecimalFormatter, DecimalFormatter, options::GroupingStrategy};
use icu::locale::Locale;
use icu_experimental::dimension::{
    currency::{CurrencyType, formatter::CurrencyFormatter},
    percent::formatter::PercentFormatter,
};
use impasto::ImpastoType;
use writeable::Writeable as _;

use crate::{
    IntlError,
    data::{Data, construct},
    decimal,
};

#[derive(Clone, Copy, Eq, Hash, PartialEq, ImpastoType)]
enum Style {
    Decimal,
    Currency,
    Percent,
}
#[derive(Clone, Copy, Eq, Hash, PartialEq, ImpastoType)]
enum Display {
    Symbol,
    NarrowSymbol,
    Code,
    Name,
}
#[derive(Clone, Copy, Eq, Hash, PartialEq, ImpastoType)]
enum Notation {
    Standard,
    CompactShort,
    CompactLong,
}

#[derive(Clone, Default, Eq, Hash, PartialEq, ImpastoType)]
pub(crate) struct Options {
    style: Option<Style>,
    currency: Option<String>,
    currency_display: Option<Display>,
    notation: Option<Notation>,
    minimum_fraction_digits: Option<u8>,
    maximum_fraction_digits: Option<u8>,
    use_grouping: Option<bool>,
}

pub(crate) enum Formatter {
    Decimal(DecimalFormatter),
    Compact(CompactDecimalFormatter),
    Currency(CurrencyFormatter<DecimalFormatter>),
    CompactCurrency(CurrencyFormatter<CompactDecimalFormatter>),
    Percent(PercentFormatter<DecimalFormatter>),
}

pub(crate) fn value(value: f64, options: &Options) -> Result<Decimal, IntlError> {
    let mut value = decimal(value)?;
    if options.style == Some(Style::Percent) {
        value.absolute.multiply_pow10(2);
    }
    let minimum = options.minimum_fraction_digits.unwrap_or(0);
    let maximum = options.maximum_fraction_digits.unwrap_or(minimum.max(3));
    if minimum > maximum || maximum > 20 {
        return Err(IntlError::invalid());
    }
    if options.style != Some(Style::Currency) || options.maximum_fraction_digits.is_some() {
        value.round(-i16::from(maximum));
        value.absolute.trim_end();
    }
    value.absolute.pad_end(-i16::from(minimum));
    Ok(value)
}

impl Formatter {
    pub(crate) fn new(
        data: Option<&Data>,
        locale: &Locale,
        options: &Options,
    ) -> Result<Self, IntlError> {
        let grouping = if options.use_grouping == Some(false) {
            GroupingStrategy::Never
        } else {
            GroupingStrategy::Auto
        };
        let compact = options.notation.unwrap_or(Notation::Standard);
        match options.style.unwrap_or(Style::Decimal) {
            Style::Decimal => match compact {
                Notation::Standard => construct!(
                    data,
                    DecimalFormatter::try_new,
                    try_new_with_buffer_provider,
                    locale.into(),
                    grouping.into()
                )
                .map(Self::Decimal),
                Notation::CompactShort => construct!(
                    data,
                    CompactDecimalFormatter::try_new_short,
                    try_new_short_with_buffer_provider,
                    locale.into(),
                    grouping.into()
                )
                .map(Self::Compact),
                Notation::CompactLong => construct!(
                    data,
                    CompactDecimalFormatter::try_new_long,
                    try_new_long_with_buffer_provider,
                    locale.into(),
                    grouping.into()
                )
                .map(Self::Compact),
            }
            .map_err(|_| IntlError::data()),
            Style::Percent => construct!(
                data,
                PercentFormatter::try_new,
                try_new_with_buffer_provider,
                locale.into(),
                Default::default()
            )
            .map(Self::Percent)
            .map_err(|_| IntlError::data()),
            Style::Currency => {
                let currency = options
                    .currency
                    .as_deref()
                    .filter(|code| code.len() == 3 && code.bytes().all(|b| b.is_ascii_alphabetic()))
                    .and_then(|code| code.to_ascii_lowercase().parse::<CurrencyType>().ok())
                    .ok_or(IntlError(
                        "invalid-currency",
                        "Currency must be a three-letter ISO code",
                    ))?;
                let prefs = locale.into();
                let display = options.currency_display.unwrap_or(Display::Symbol);
                let format = match (compact, display) {
                    (Notation::Standard, Display::Symbol) => construct!(
                        data,
                        CurrencyFormatter::try_new_symbol,
                        try_new_symbol_with_buffer_provider,
                        prefs,
                        currency,
                        Default::default()
                    )
                    .map(Self::Currency),
                    (Notation::Standard, Display::NarrowSymbol) => construct!(
                        data,
                        CurrencyFormatter::try_new_symbol_narrow,
                        try_new_symbol_narrow_with_buffer_provider,
                        prefs,
                        currency,
                        Default::default()
                    )
                    .map(Self::Currency),
                    (Notation::Standard, Display::Code) => construct!(
                        data,
                        CurrencyFormatter::try_new_code,
                        try_new_code_with_buffer_provider,
                        prefs,
                        currency,
                        Default::default()
                    )
                    .map(Self::Currency),
                    (Notation::Standard, Display::Name) => construct!(
                        data,
                        CurrencyFormatter::try_new_name,
                        try_new_name_with_buffer_provider,
                        prefs,
                        currency
                    )
                    .map(Self::Currency),
                    (Notation::CompactShort, Display::Symbol) => construct!(
                        data,
                        CurrencyFormatter::try_new_compact_symbol,
                        try_new_compact_symbol_with_buffer_provider,
                        prefs,
                        currency,
                        Default::default()
                    )
                    .map(Self::CompactCurrency),
                    (Notation::CompactShort, Display::NarrowSymbol) => construct!(
                        data,
                        CurrencyFormatter::try_new_compact_symbol_narrow,
                        try_new_compact_symbol_narrow_with_buffer_provider,
                        prefs,
                        currency,
                        Default::default()
                    )
                    .map(Self::CompactCurrency),
                    (Notation::CompactShort, Display::Code) => construct!(
                        data,
                        CurrencyFormatter::try_new_compact_code,
                        try_new_compact_code_with_buffer_provider,
                        prefs,
                        currency,
                        Default::default()
                    )
                    .map(Self::CompactCurrency),
                    (Notation::CompactShort, Display::Name) => construct!(
                        data,
                        CurrencyFormatter::try_new_compact_name,
                        try_new_compact_name_with_buffer_provider,
                        prefs,
                        currency
                    )
                    .map(Self::CompactCurrency),
                    (Notation::CompactLong, Display::Symbol) => construct!(
                        data,
                        CurrencyFormatter::try_new_compact_long_symbol,
                        try_new_compact_long_symbol_with_buffer_provider,
                        prefs,
                        currency,
                        Default::default()
                    )
                    .map(Self::CompactCurrency),
                    (Notation::CompactLong, Display::NarrowSymbol) => construct!(
                        data,
                        CurrencyFormatter::try_new_compact_long_symbol_narrow,
                        try_new_compact_long_symbol_narrow_with_buffer_provider,
                        prefs,
                        currency,
                        Default::default()
                    )
                    .map(Self::CompactCurrency),
                    (Notation::CompactLong, Display::Code) => construct!(
                        data,
                        CurrencyFormatter::try_new_compact_long_code,
                        try_new_compact_long_code_with_buffer_provider,
                        prefs,
                        currency,
                        Default::default()
                    )
                    .map(Self::CompactCurrency),
                    (Notation::CompactLong, Display::Name) => construct!(
                        data,
                        CurrencyFormatter::try_new_compact_long_name,
                        try_new_compact_long_name_with_buffer_provider,
                        prefs,
                        currency
                    )
                    .map(Self::CompactCurrency),
                };
                format.map_err(|_| IntlError::data())
            }
        }
    }
    pub(crate) fn format(&self, value: &Decimal) -> String {
        match self {
            Self::Decimal(format) => format.format(value).write_to_string().into_owned(),
            Self::Compact(format) => format.format(value).write_to_string().into_owned(),
            Self::Currency(format) => format.format_fixed_decimal(value).to_string(),
            Self::CompactCurrency(format) => format.format_fixed_decimal(value).to_string(),
            Self::Percent(format) => format.format(value).write_to_string().into_owned(),
        }
    }
}
