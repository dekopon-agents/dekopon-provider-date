use chrono::DateTime;
use dekopon_provider_sdk::{CommandRunOutcome, EffectKind, RiskLevel, provider};
use dekopon_provider_sdk_testkit::Native;

use serde_json::{Value, json};

use super::{
    DESCRIPTION, DateProvider,
    date::{DateInput, MAX_DAYS, MAX_OUTPUT_BYTES, RawInput},
};

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

fn millis(instant: &str) -> u64 {
    DateTime::parse_from_rfc3339(instant)
        .unwrap()
        .timestamp_millis()
        .try_into()
        .unwrap()
}

fn render(instant: &str, input: Value) -> String {
    DateInput::from_value(input)
        .unwrap()
        .render(millis(instant))
        .unwrap()
}

#[test]
fn default_and_named_timezone_can_have_different_dates() {
    let instant = "2024-01-01T02:03:04.567Z";
    assert_eq!(render(instant, json!({})), "2024-01-01T02:03:04Z");
    assert_eq!(
        render(instant, json!({"timezone":"America/New_York"})),
        "2023-12-31T21:03:04-05:00"
    );
    assert_eq!(
        render(
            instant,
            json!({"format":"%F %Y/%m/%d %H:%M:%S %z %Z %s %%"})
        ),
        "2024-01-01 2024/01/01 02:03:04 +0000 UTC 1704074584 %"
    );
    assert_eq!(
        render(
            instant,
            json!({"timezone":"America/New_York", "format":"%F %z %Z %s"})
        ),
        "2023-12-31 -0500 EST 1704074584"
    );
}

#[test]
fn calendar_offsets_cross_leap_day_and_year_boundary() {
    for (instant, days, expected) in [
        ("2024-03-01T12:00:00Z", -1, "2024-02-29"),
        ("2024-02-28T12:00:00Z", 1, "2024-02-29"),
        ("2023-12-31T12:00:00Z", 1, "2024-01-01"),
        ("2024-01-01T12:00:00Z", -1, "2023-12-31"),
        ("2024-02-29T12:00:00Z", 0, "2024-02-29"),
    ] {
        assert_eq!(
            render(instant, json!({"days":days,"format":"%F"})),
            expected
        );
    }
}

#[test]
fn local_days_cross_dst_in_23_or_25_hours_not_86400_seconds() {
    for (instant, expected, delta) in [
        (
            "2024-03-09T17:00:00Z",
            "2024-03-10T12:00:00-04:00",
            23 * 3600,
        ),
        (
            "2024-11-02T16:00:00Z",
            "2024-11-03T12:00:00-05:00",
            25 * 3600,
        ),
    ] {
        let input = json!({"timezone":"America/New_York", "days":1});
        assert_eq!(render(instant, input.clone()), expected);
        let target = DateTime::parse_from_rfc3339(&render(instant, input)).unwrap();
        assert_eq!(target.timestamp() - (millis(instant) / 1000) as i64, delta);
        assert_eq!(
            render(expected, json!({"timezone":"UTC","days":-1,"format":"%H"})),
            if delta == 23 * 3600 { "16" } else { "17" }
        );
    }
}

#[test]
fn nonzero_offsets_reject_destination_gaps_and_folds_even_for_date_only() {
    for (instant, days, code) in [
        ("2024-03-09T07:30:00Z", 1, "nonexistent-local-time"),
        ("2024-03-11T06:30:00Z", -1, "nonexistent-local-time"),
        ("2024-11-02T05:30:00Z", 1, "ambiguous-local-time"),
        ("2024-11-04T06:30:00Z", -1, "ambiguous-local-time"),
        ("2011-12-29T22:00:00Z", 1, "nonexistent-local-time"),
    ] {
        let zone = if instant.starts_with("2011") {
            "Pacific/Apia"
        } else {
            "America/New_York"
        };
        let input =
            DateInput::from_value(json!({"timezone":zone,"days":days,"format":"%F"})).unwrap();
        assert_eq!(input.render(millis(instant)).unwrap_err().code(), code);
    }
    for (instant, expected) in [
        ("2024-11-03T05:30:00Z", "2024-11-03T01:30:00-04:00"),
        ("2024-11-03T06:30:00Z", "2024-11-03T01:30:00-05:00"),
    ] {
        assert_eq!(
            render(instant, json!({"timezone":"America/New_York","days":0})),
            expected
        );
    }
}

#[test]
fn input_validation_is_shared_and_cannot_be_bypassed_by_invoke() {
    for input in [
        json!(null),
        json!([]),
        json!({"extra":true}),
        json!({"format":null}),
        json!({"timezone":"unknown"}),
        json!({"timezone":"america/new_york"}),
        json!({"timezone":"x".repeat(65)}),
        json!({"format":"x".repeat(257)}),
        json!({"days":MAX_DAYS+1}),
        json!({"days":-MAX_DAYS-1}),
        json!({"days":u64::MAX}),
        json!({"days":1.5}),
        json!({"days":"7"}),
    ] {
        assert!(DateInput::from_value(input.clone()).is_err(), "{input}");
        let native = Native::<DateProvider>::new();
        let result = native.call("date.now", &input.to_string());
        assert_ne!(
            result.status, 0,
            "input={input}, stdout={:?}",
            result.stdout
        );
        assert!(result.stdout.is_empty());
    }
    let unknown = Native::<DateProvider>::new().call("clock.other", "{}");
    assert_ne!(unknown.status, 0);
    for format in [
        "%",
        "%Q",
        "%n",
        "%t",
        "%c",
        "%:z",
        "%-d",
        "%99999999Y",
        "%EY",
        "%Od",
        "line\n",
        "\0",
        "é",
    ] {
        assert!(
            DateInput::from_value(json!({"format":format})).is_err(),
            "{format:?}"
        );
        assert!(RawInput::from_argv(&argv(&[&format!("+{format}")])).is_err());
    }
}

#[test]
fn argv_rejects_unknown_missing_extra_and_conflicting_arguments() {
    for words in [
        vec!["--unknown"],
        vec!["today"],
        vec!["--timezone"],
        vec!["--timezone="],
        vec!["--days"],
        vec!["--days="],
        vec!["--days=huge"],
        vec!["--days=36601"],
        vec!["--days=99999999999999999999999999"],
        vec!["+%F", "+%s"],
        vec!["-u", "--utc"],
        vec!["--timezone", "UTC", "-u"],
        vec!["--days=1", "--days=2"],
        vec!["--help", "extra"],
        vec!["+%F", "extra"],
    ] {
        assert!(RawInput::from_argv(&argv(&words)).is_err(), "{words:?}");
        assert!(
            matches!(
                provider::command::<DateProvider>(&argv(&words), false),
                CommandRunOutcome::Rendered { status: 2, .. }
            ),
            "CLI accepted {words:?}"
        );
    }
    assert!(RawInput::from_argv(&vec!["-u".to_owned(); 9]).is_err());
    assert!(RawInput::from_argv(&["x".repeat(1025)]).is_err());
    assert!(RawInput::from_argv(&["x".repeat(600), "y".repeat(600)]).is_err());
}

#[test]
fn proposals_help_and_manifest_are_exact_and_pure() {
    let command = |words: &[&str], piped| provider::command::<DateProvider>(&argv(words), piped);
    let CommandRunOutcome::Proposed {
        capability,
        input,
        secret_use,
    } = command(&[], true)
    else {
        panic!("proposal")
    };
    assert_eq!(capability.as_str(), "date.now");
    assert_eq!(input, json!({"timezone":"UTC","days":0}));
    assert!(secret_use.is_none());
    assert_eq!(command(&[], true), command(&[], false));
    for words in [vec!["-u"], vec!["--utc"], vec!["--timezone=UTC"]] {
        assert_eq!(command(&words, false), command(&[], false));
    }
    let CommandRunOutcome::Proposed { input, .. } = command(
        &["--timezone", "America/New_York", "--days=-7", "+%F %H:%M"],
        false,
    ) else {
        panic!("proposal")
    };
    assert_eq!(
        input,
        json!({"timezone":"America/New_York","days":-7,"format":"%F %H:%M"})
    );
    assert!(
        matches!(command(&["--help"], false), CommandRunOutcome::Rendered { status: 0, stdout, .. } if stdout.contains("date"))
    );
    for words in [vec!["--help", "extra"], vec!["+%F", "--help"]] {
        assert!(matches!(
            command(&words, false),
            CommandRunOutcome::Rendered { status: 2, .. }
        ));
    }
    let sentinel = "x".repeat(4096);
    let unknown = format!("--{sentinel}");
    let outcome = command(&[&unknown], false);
    let rendered = format!("{outcome:?}");
    assert!(
        !rendered.contains(&sentinel),
        "unknown argv leaked into usage"
    );
    assert!(rendered.len() < 512, "usage error was unbounded");
    let manifest = provider::manifest::<DateProvider>().unwrap();
    assert_eq!(manifest.id.as_str(), "date");
    assert_eq!(manifest.description, DESCRIPTION);
    assert_eq!(manifest.command_words, ["date"]);
    assert_eq!(manifest.capabilities.len(), 1);
    let capability = &manifest.capabilities[0];
    assert_eq!(capability.id.as_str(), "date.now");
    assert_eq!(capability.effect, EffectKind::ReadOnly);
    assert_eq!(capability.risk, RiskLevel::Low);
    assert_eq!(capability.input_schema["additionalProperties"], false);
    assert!(
        capability.input_schema["properties"]
            .get("format")
            .is_some()
    );
}

#[test]
fn range_limits_and_maximal_format_stay_bounded() {
    let input = DateInput::from_value(json!({})).unwrap();
    assert_eq!(input.render(0).unwrap(), "1970-01-01T00:00:00Z");
    assert_eq!(
        input.render(253_402_300_799_999).unwrap(),
        "9999-12-31T23:59:59Z"
    );
    for millis in [253_402_300_800_000, i64::MAX as u64, u64::MAX] {
        assert_eq!(
            input.render(millis).unwrap_err().code(),
            "clock-out-of-range"
        );
    }
    for days in [-MAX_DAYS, MAX_DAYS] {
        assert!(
            DateInput::from_value(json!({"days":days}))
                .unwrap()
                .render(0)
                .is_ok()
        );
    }
    assert!(
        DateInput::from_value(json!({"days":1}))
            .unwrap()
            .render(253_402_300_799_999)
            .is_err()
    );
    assert!(
        DateInput::from_value(json!({"timezone":"Pacific/Kiritimati"}))
            .unwrap()
            .render(253_402_300_799_999)
            .is_err()
    );
    let formatted = render("9999-12-31T00:00:00Z", json!({"format":"%s".repeat(128)}));
    assert!(formatted.len() <= MAX_OUTPUT_BYTES);
    assert_eq!(formatted, "253402214400".repeat(128));
    assert_eq!(
        render("1970-01-01T00:00:00Z", json!({"days":-1,"format":"%s"})),
        "-86400"
    );
    assert_eq!(render("2024-01-01T00:00:00Z", json!({"format":""})), "");
}

#[test]
fn historical_second_offsets_refuse_default_rfc3339_but_keep_explicit_date_and_seconds() {
    let instant = "1970-01-01T01:00:00Z";
    let input = DateInput::from_value(json!({"timezone":"Africa/Monrovia"})).unwrap();
    assert_eq!(
        input.render(millis(instant)).unwrap_err().code(),
        "unsupported-offset"
    );
    assert_eq!(
        render(
            instant,
            json!({"timezone":"Africa/Monrovia", "format":"%F %H:%M:%S %z %Z %s"})
        ),
        "1970-01-01 00:15:30 -0045 MMT 3600"
    );
    let offset =
        DateInput::from_value(json!({"timezone":"America/New_York", "days":-36600})).unwrap();
    assert_eq!(offset.render(0).unwrap_err().code(), "unsupported-offset");
    assert_eq!(
        DateInput::from_value(json!({"timezone":"America/New_York", "days":-36600,"format":"%z"}))
            .unwrap()
            .render(0)
            .unwrap(),
        "-0456"
    );
    assert_eq!(
        render(
            "2024-01-01T01:00:00Z",
            json!({"timezone":"Africa/Monrovia"})
        ),
        "2024-01-01T01:00:00Z"
    );
}

#[test]
fn invalid_day_type_errors_do_not_echo_unbounded_input() {
    let error = match DateInput::from_value(json!({"days":"sentinel".repeat(10000)})) {
        Ok(_) => panic!("wrong type must fail"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "invalid-input");
    assert_eq!(error.message(), "days must be a signed integer");
}
