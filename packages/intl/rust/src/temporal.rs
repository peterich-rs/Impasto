use fixed_decimal::Decimal;
use icu::{
    datetime::{
        DateTimeFormatter, DateTimeFormatterPreferences,
        fieldsets::{T, YMD, YMDE, YMDET, YMDT},
        input::{Date, DateTime, Time},
        options::{Length, TimePrecision},
        preferences::HourCycle,
    },
    locale::Locale,
};
use icu_experimental::relativetime::{
    RelativeTimeFormatter, RelativeTimeFormatterOptions, options::Numeric,
};
use impasto::ImpastoType;
use jiff::{Timestamp, tz::TimeZone};
use writeable::Writeable as _;

use crate::{
    Cache, IntlError,
    data::{Data, construct},
};

#[derive(Clone, Copy, Eq, Hash, PartialEq, ImpastoType)]
enum DateTimeStyle {
    Short,
    Medium,
    Long,
    Full,
}
#[derive(Clone, Default, Eq, Hash, PartialEq, ImpastoType)]
pub(crate) struct DateOptions {
    date_style: Option<DateTimeStyle>,
    time_style: Option<DateTimeStyle>,
    time_zone: Option<String>,
    hour12: Option<bool>,
}
#[derive(Clone, Copy, Eq, Hash, PartialEq, ImpastoType)]
enum Unit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Quarter,
    Year,
}
#[derive(Clone, Copy, Eq, Hash, PartialEq, ImpastoType)]
enum RelativeStyle {
    Long,
    Short,
    Narrow,
}
#[derive(Clone, Copy, Eq, Hash, PartialEq, ImpastoType)]
enum RelativeNumeric {
    Always,
    Auto,
}
#[derive(Clone, Eq, Hash, PartialEq, ImpastoType)]
pub(crate) struct RelativeOptions {
    unit: Unit,
    style: Option<RelativeStyle>,
    numeric: Option<RelativeNumeric>,
}

enum DateFormat {
    Date(DateTimeFormatter<YMD>),
    FullDate(DateTimeFormatter<YMDE>),
    Time(DateTimeFormatter<T>),
    Combined(DateTimeFormatter<YMDT>),
    FullCombined(DateTimeFormatter<YMDET>),
}
pub(crate) struct Formatters {
    date: Cache<(String, DateOptions), DateFormat>,
    relative: Cache<(String, RelativeOptions), RelativeTimeFormatter>,
}
impl Formatters {
    pub(crate) fn new() -> Self {
        Self {
            date: Cache::new(),
            relative: Cache::new(),
        }
    }
    pub(crate) fn date(
        &self,
        data: Option<&Data>,
        locale: Locale,
        epoch: i64,
        opts: DateOptions,
    ) -> Result<String, IntlError> {
        let zone = opts
            .time_zone
            .as_deref()
            .map_or_else(|| Ok(TimeZone::system()), TimeZone::get)
            .map_err(|_| IntlError("invalid-timezone", "Unknown IANA time zone"))?;
        let stamp = Timestamp::from_millisecond(epoch)
            .map_err(|_| IntlError::invalid())?
            .to_zoned(zone);
        let invalid = |_| IntlError::invalid();
        let day = Date::try_new_iso(
            i32::from(stamp.year()),
            stamp.month().cast_unsigned(),
            stamp.day().cast_unsigned(),
        )
        .map_err(invalid)?;
        let time = Time::try_new(
            stamp.hour().cast_unsigned(),
            stamp.minute().cast_unsigned(),
            stamp.second().cast_unsigned(),
            stamp.subsec_nanosecond().cast_unsigned(),
        )
        .map_err(|_| IntlError::invalid())?;
        self.date.get(
            (locale.to_string(), opts.clone()),
            || {
                let mut prefs: DateTimeFormatterPreferences = (&locale).into();
                if let Some(hour12) = opts.hour12 {
                    prefs.hour_cycle = Some(if hour12 {
                        HourCycle::H12
                    } else {
                        HourCycle::H23
                    });
                }
                let date = opts.date_style;
                let time = opts.time_style;
                let formatter = match (date, time) {
                    (Some(DateTimeStyle::Full), None) => construct!(
                        data,
                        DateTimeFormatter::try_new,
                        try_new_with_buffer_provider,
                        prefs,
                        YMDE::long()
                    )
                    .map(DateFormat::FullDate),
                    (Some(date), None) => construct!(
                        data,
                        DateTimeFormatter::try_new,
                        try_new_with_buffer_provider,
                        prefs,
                        YMD::for_length(length(date))
                    )
                    .map(DateFormat::Date),
                    (None, Some(time)) => construct!(
                        data,
                        DateTimeFormatter::try_new,
                        try_new_with_buffer_provider,
                        prefs,
                        T::for_length(length(time)).with_time_precision(precision(Some(time)))
                    )
                    .map(DateFormat::Time),
                    (Some(DateTimeStyle::Full), Some(time)) => construct!(
                        data,
                        DateTimeFormatter::try_new,
                        try_new_with_buffer_provider,
                        prefs,
                        YMDET::long().with_time_precision(precision(Some(time)))
                    )
                    .map(DateFormat::FullCombined),
                    (date, time) => construct!(
                        data,
                        DateTimeFormatter::try_new,
                        try_new_with_buffer_provider,
                        prefs,
                        YMDT::for_length(length(date.unwrap_or(DateTimeStyle::Medium)))
                            .with_time_precision(precision(time))
                    )
                    .map(DateFormat::Combined),
                };
                formatter.map_err(|_| IntlError::data())
            },
            |formatter| {
                let date_time = DateTime { date: day, time };
                match formatter {
                    DateFormat::Date(format) => format
                        .format(&date_time.date)
                        .write_to_string()
                        .into_owned(),
                    DateFormat::FullDate(format) => format
                        .format(&date_time.date)
                        .write_to_string()
                        .into_owned(),
                    DateFormat::Time(format) => format
                        .format(&date_time.time)
                        .write_to_string()
                        .into_owned(),
                    DateFormat::Combined(format) => {
                        format.format(&date_time).write_to_string().into_owned()
                    }
                    DateFormat::FullCombined(format) => {
                        format.format(&date_time).write_to_string().into_owned()
                    }
                }
            },
        )
    }

    pub(crate) fn relative(
        &self,
        data: Option<&Data>,
        locale: Locale,
        value: Decimal,
        opts: RelativeOptions,
    ) -> Result<String, IntlError> {
        self.relative.get(
            (locale.to_string(), opts.clone()),
            || {
                let prefs = (&locale).into();
                let style = opts.style.unwrap_or(RelativeStyle::Long);
                let numeric = if opts.numeric == Some(RelativeNumeric::Auto) {
                    Numeric::Auto
                } else {
                    Numeric::Always
                };
                let mut options = RelativeTimeFormatterOptions::default();
                options.numeric = numeric;
                macro_rules! relative {
                    ($built_in:ident, $buffer:ident) => {
                        construct!(
                            data,
                            RelativeTimeFormatter::$built_in,
                            $buffer,
                            prefs,
                            options
                        )
                    };
                }
                let formatter = match (style, opts.unit) {
                    (RelativeStyle::Long, Unit::Second) => relative!(
                        try_new_long_second,
                        try_new_long_second_with_buffer_provider
                    ),
                    (RelativeStyle::Long, Unit::Minute) => relative!(
                        try_new_long_minute,
                        try_new_long_minute_with_buffer_provider
                    ),
                    (RelativeStyle::Long, Unit::Hour) => {
                        relative!(try_new_long_hour, try_new_long_hour_with_buffer_provider)
                    }
                    (RelativeStyle::Long, Unit::Day) => {
                        relative!(try_new_long_day, try_new_long_day_with_buffer_provider)
                    }
                    (RelativeStyle::Long, Unit::Week) => {
                        relative!(try_new_long_week, try_new_long_week_with_buffer_provider)
                    }
                    (RelativeStyle::Long, Unit::Month) => {
                        relative!(try_new_long_month, try_new_long_month_with_buffer_provider)
                    }
                    (RelativeStyle::Long, Unit::Quarter) => relative!(
                        try_new_long_quarter,
                        try_new_long_quarter_with_buffer_provider
                    ),
                    (RelativeStyle::Long, Unit::Year) => {
                        relative!(try_new_long_year, try_new_long_year_with_buffer_provider)
                    }
                    (RelativeStyle::Short, Unit::Second) => relative!(
                        try_new_short_second,
                        try_new_short_second_with_buffer_provider
                    ),
                    (RelativeStyle::Short, Unit::Minute) => relative!(
                        try_new_short_minute,
                        try_new_short_minute_with_buffer_provider
                    ),
                    (RelativeStyle::Short, Unit::Hour) => {
                        relative!(try_new_short_hour, try_new_short_hour_with_buffer_provider)
                    }
                    (RelativeStyle::Short, Unit::Day) => {
                        relative!(try_new_short_day, try_new_short_day_with_buffer_provider)
                    }
                    (RelativeStyle::Short, Unit::Week) => {
                        relative!(try_new_short_week, try_new_short_week_with_buffer_provider)
                    }
                    (RelativeStyle::Short, Unit::Month) => relative!(
                        try_new_short_month,
                        try_new_short_month_with_buffer_provider
                    ),
                    (RelativeStyle::Short, Unit::Quarter) => relative!(
                        try_new_short_quarter,
                        try_new_short_quarter_with_buffer_provider
                    ),
                    (RelativeStyle::Short, Unit::Year) => {
                        relative!(try_new_short_year, try_new_short_year_with_buffer_provider)
                    }
                    (RelativeStyle::Narrow, Unit::Second) => relative!(
                        try_new_narrow_second,
                        try_new_narrow_second_with_buffer_provider
                    ),
                    (RelativeStyle::Narrow, Unit::Minute) => relative!(
                        try_new_narrow_minute,
                        try_new_narrow_minute_with_buffer_provider
                    ),
                    (RelativeStyle::Narrow, Unit::Hour) => relative!(
                        try_new_narrow_hour,
                        try_new_narrow_hour_with_buffer_provider
                    ),
                    (RelativeStyle::Narrow, Unit::Day) => {
                        relative!(try_new_narrow_day, try_new_narrow_day_with_buffer_provider)
                    }
                    (RelativeStyle::Narrow, Unit::Week) => relative!(
                        try_new_narrow_week,
                        try_new_narrow_week_with_buffer_provider
                    ),
                    (RelativeStyle::Narrow, Unit::Month) => relative!(
                        try_new_narrow_month,
                        try_new_narrow_month_with_buffer_provider
                    ),
                    (RelativeStyle::Narrow, Unit::Quarter) => relative!(
                        try_new_narrow_quarter,
                        try_new_narrow_quarter_with_buffer_provider
                    ),
                    (RelativeStyle::Narrow, Unit::Year) => relative!(
                        try_new_narrow_year,
                        try_new_narrow_year_with_buffer_provider
                    ),
                };
                formatter.map_err(|_| IntlError::data())
            },
            |format| format.format(value).write_to_string().into_owned(),
        )
    }
}
fn length(style: DateTimeStyle) -> Length {
    match style {
        DateTimeStyle::Short => Length::Short,
        DateTimeStyle::Medium => Length::Medium,
        DateTimeStyle::Long | DateTimeStyle::Full => Length::Long,
    }
}

fn precision(style: Option<DateTimeStyle>) -> TimePrecision {
    if style == Some(DateTimeStyle::Short) {
        TimePrecision::Minute
    } else {
        TimePrecision::Second
    }
}
