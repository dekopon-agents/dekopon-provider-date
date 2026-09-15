//! Shared CLI/invocation validation and pure rendering with an injected Unix-millisecond reading.

use std::fmt;

use chrono::{DateTime, Datelike, Days, LocalResult, Offset, SecondsFormat, TimeZone, Utc};
use chrono_tz::Tz;
use dekopon_provider_sdk::ProviderError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(crate) const MAX_FORMAT_BYTES: usize = 256;
pub(crate) const MAX_OUTPUT_BYTES: usize = 2048;
pub(crate) const MAX_DAYS: i64 = 36_600;
const MAX_ARGV: usize = 8;
const MAX_ARG_BYTES: usize = 1024;
const MAX_TIMEZONE_BYTES: usize = 64;
const MAX_MILLIS: u64 = 253_402_300_799_999;

fn invalid(message: &'static str) -> ProviderError {
    ProviderError::new("invalid-input", message)
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<String>,
    #[serde(default = "utc")]
    timezone: String,
    days: i64,
}

fn utc() -> String {
    "UTC".to_owned()
}

impl RawInput {
    pub(crate) fn from_argv(argv: &[String]) -> Result<Self, ProviderError> {
        if argv.len() > MAX_ARGV
            || argv.iter().any(|arg| arg.len() > MAX_ARG_BYTES)
            || argv.iter().map(String::len).sum::<usize>() > MAX_ARG_BYTES
        {
            return Err(invalid("argv exceeds 8 arguments or 1024 bytes"));
        }
        let mut input = Self {
            timezone: utc(),
            ..Self::default()
        };
        let mut seen_timezone = false;
        let mut seen_days = false;
        let mut args = argv.iter();
        while let Some(arg) = args.next() {
            if let Some(format) = arg.strip_prefix('+') {
                if input.format.replace(format.to_owned()).is_some() {
                    return Err(invalid("only one +FORMAT is allowed"));
                }
            } else if arg == "-u"
                || arg == "--utc"
                || arg == "--timezone"
                || arg.starts_with("--timezone=")
            {
                if seen_timezone {
                    return Err(invalid("timezone options cannot be repeated or combined"));
                }
                seen_timezone = true;
                input.timezone = if arg == "--timezone" {
                    args.next()
                        .ok_or_else(|| invalid("--timezone requires an IANA name"))?
                        .clone()
                } else if let Some(zone) = arg.strip_prefix("--timezone=") {
                    zone.to_owned()
                } else {
                    utc()
                };
            } else if arg == "--days" || arg.starts_with("--days=") {
                if seen_days {
                    return Err(invalid("--days cannot be repeated"));
                }
                seen_days = true;
                let days = if arg == "--days" {
                    args.next()
                        .ok_or_else(|| invalid("--days requires a signed integer"))?
                } else {
                    arg.strip_prefix("--days=").expect("prefix checked")
                };
                input.days = days.parse().map_err(|error| {
                    ProviderError::new(
                        "invalid-input",
                        format!("--days requires a signed integer: {error}"),
                    )
                })?;
            } else {
                return Err(invalid("unknown option or extra argument"));
            }
        }
        input.validate()?;
        Ok(input)
    }

    fn validate(&self) -> Result<DateInput, ProviderError> {
        let format = self.format.as_deref().map(DateFormat::parse).transpose()?;
        if self.timezone.len() > MAX_TIMEZONE_BYTES {
            return Err(invalid("timezone exceeds 64 bytes"));
        }
        let timezone = self.timezone.parse().map_err(|error| {
            ProviderError::new("invalid-input", format!("invalid IANA timezone: {error}"))
        })?;
        if !(-MAX_DAYS..=MAX_DAYS).contains(&self.days) {
            return Err(invalid("days must be between -36600 and 36600"));
        }
        Ok(DateInput {
            format,
            timezone,
            days: DayOffset(self.days),
        })
    }
}

struct DateFormat(String);
struct DayOffset(i64);

impl DateFormat {
    fn parse(format: &str) -> Result<Self, ProviderError> {
        if format.len() > MAX_FORMAT_BYTES || !format.bytes().all(|b| (b' '..=b'~').contains(&b)) {
            return Err(invalid("format must be at most 256 printable ASCII bytes"));
        }
        let mut chars = format.chars();
        while let Some(character) = chars.next() {
            if character == '%'
                && !matches!(
                    chars.next(),
                    Some('F' | 'Y' | 'm' | 'd' | 'H' | 'M' | 'S' | 'z' | 'Z' | 's' | '%')
                )
            {
                return Err(invalid(
                    "unsupported format conversion; supported: %F %Y %m %d %H %M %S %z %Z %s %%",
                ));
            }
        }
        Ok(Self(format.to_owned()))
    }
}

pub(crate) struct DateInput {
    format: Option<DateFormat>,
    timezone: Tz,
    days: DayOffset,
}

impl DateInput {
    pub(crate) fn from_value(value: Value) -> Result<Self, ProviderError> {
        // Inspect lengths before serde can clone strings. Unknown field/type errors remain bounded.
        let object = value
            .as_object()
            .ok_or_else(|| invalid("input must be an object"))?;
        if object.len() > 3
            || object
                .keys()
                .any(|key| !matches!(key.as_str(), "format" | "timezone" | "days"))
        {
            return Err(invalid("input accepts only format, timezone, and days"));
        }
        for (key, limit) in [
            ("format", MAX_FORMAT_BYTES),
            ("timezone", MAX_TIMEZONE_BYTES),
        ] {
            if let Some(value) = object.get(key) {
                let text = value
                    .as_str()
                    .ok_or_else(|| invalid("format and timezone must be strings"))?;
                if text.len() > limit {
                    return Err(invalid("format or timezone exceeds its byte limit"));
                }
            }
        }
        if object
            .get("days")
            .is_some_and(|days| days.as_i64().is_none())
        {
            return Err(invalid("days must be a signed integer"));
        }
        let raw: RawInput = serde_json::from_value(value).map_err(|error| {
            ProviderError::new("invalid-input", format!("invalid date input: {error}"))
        })?;
        raw.validate()
    }

    pub(crate) fn render(&self, millis: u64) -> Result<String, ProviderError> {
        if millis > MAX_MILLIS {
            return Err(ProviderError::new(
                "clock-out-of-range",
                "host clock is past year 9999",
            ));
        }
        let now = DateTime::<Utc>::from_timestamp_millis(millis as i64)
            .ok_or_else(|| ProviderError::new("clock-out-of-range", "invalid host timestamp"))?
            .with_timezone(&self.timezone);
        // Zero preserves the actual instant even in a fold. Nonzero offsets preserve wall time,
        // not elapsed seconds, and reject a destination with zero or two corresponding instants.
        let target = if self.days.0 == 0 {
            now
        } else {
            let days = Days::new(self.days.0.unsigned_abs());
            let local = if self.days.0 < 0 {
                now.naive_local().checked_sub_days(days)
            } else {
                now.naive_local().checked_add_days(days)
            }
            .ok_or_else(|| invalid("calendar-day offset overflows"))?;
            match self.timezone.from_local_datetime(&local) {
                LocalResult::Single(target) => target,
                LocalResult::Ambiguous(_, _) => {
                    return Err(ProviderError::new(
                        "ambiguous-local-time",
                        "offset lands in a timezone fold",
                    ));
                }
                LocalResult::None => {
                    return Err(ProviderError::new(
                        "nonexistent-local-time",
                        "offset lands in a timezone gap",
                    ));
                }
            }
        };
        if !(1..=9999).contains(&target.year())
            || !(1..=9999).contains(&target.with_timezone(&Utc).year())
        {
            return Err(invalid(
                "result must have local and UTC years between 0001 and 9999",
            ));
        }
        let mut output = BoundedOutput(String::new());
        match &self.format {
            Some(format) => target
                .format(&format.0)
                .write_to(&mut output)
                .map_err(|error| {
                    ProviderError::new(
                        "output-limit",
                        format!("date output exceeds 2048 bytes: {error}"),
                    )
                })?,
            None => {
                if target.offset().fix().local_minus_utc() % 60 != 0 {
                    return Err(ProviderError::new(
                        "unsupported-offset",
                        "RFC3339 cannot represent a timezone offset with seconds; use +%F or +%s",
                    ));
                }
                output.0 = target.to_rfc3339_opts(SecondsFormat::Secs, true);
            }
        }
        Ok(output.0)
    }
}

struct BoundedOutput(String);
impl fmt::Write for BoundedOutput {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if text.len() > MAX_OUTPUT_BYTES - self.0.len() {
            return Err(fmt::Error);
        }
        self.0.push_str(text);
        Ok(())
    }
}

pub(crate) fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "format": {"type": "string", "maxLength": MAX_FORMAT_BYTES,
                "description": "Printable ASCII strftime subset: %F %Y %m %d %H %M %S %z %Z %s %% (no flags or widths). Omit for RFC3339."},
            "timezone": {"type": "string", "maxLength": MAX_TIMEZONE_BYTES, "default": "UTC",
                "description": "Case-sensitive IANA timezone from bundled chrono-tz data"},
            "days": {"type": "integer", "minimum": -MAX_DAYS, "maximum": MAX_DAYS, "default": 0,
                "description": "Local calendar days; nonzero offsets reject timezone gaps/folds"}
        },
        "additionalProperties": false
    })
}
