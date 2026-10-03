use dekopon_date_provider::DateProvider;
use dekopon_provider_sdk::{CommandRunOutcome, EffectKind, RiskLevel, provider};
use dekopon_provider_sdk_testkit::{Harness, Native, conformance};
use serde_json::json;
use std::{
    path::PathBuf,
    time::{Duration, UNIX_EPOCH},
};

fn component() -> PathBuf {
    std::env::var_os("DEKOPON_PROVIDER_COMPONENT")
        .expect("DEKOPON_PROVIDER_COMPONENT must point at built component")
        .into()
}

#[test]
fn authorization_and_pure_proposals_guard_clock_reads() {
    conformance::<DateProvider>(component()).unwrap();
    let manifest = provider::manifest::<DateProvider>().unwrap();
    assert_eq!(manifest.id.as_str(), "date");
    assert_eq!(manifest.command_words, ["date"]);
    assert_eq!(manifest.capabilities.len(), 1);
    assert_eq!(manifest.capabilities[0].id.as_str(), "date.now");
    assert_eq!(manifest.capabilities[0].effect, EffectKind::ReadOnly);
    assert_eq!(manifest.capabilities[0].risk, RiskLevel::Low);
    let CommandRunOutcome::Proposed {
        capability,
        input,
        secret_use,
    } = provider::command::<DateProvider>(&[], true)
    else {
        panic!("proposal")
    };
    assert_eq!(capability.as_str(), "date.now");
    assert_eq!(input, json!({"timezone":"UTC","days":0}));
    assert!(secret_use.is_none());
    assert!(matches!(
        provider::command::<DateProvider>(&["--help".into()], false),
        CommandRunOutcome::Rendered { status: 0, .. }
    ));
    let instant = UNIX_EPOCH + Duration::from_millis(1_704_074_584_567);
    let native = Native::<DateProvider>::new().clock(instant);
    let invalid = native.call("date.now", &json!({"format":"%Q"}).to_string());
    assert_ne!(invalid.status, 0);
    assert!(invalid.stdout.is_empty());
    let unknown = native.call("clock.now", "{}");
    assert_ne!(unknown.status, 0);
    let output = native.call(
        "date.now",
        &json!({"timezone":"America/New_York", "format":"%F %H:%M:%S %z %Z %s"}).to_string(),
    );
    assert_eq!(output.status, 0, "{}", output.stderr);
    assert_eq!(output.stdout, b"2023-12-31 21:03:04 -0500 EST 1704074584\n");
    let bounded = native.call("date.now", &json!({"format":"%s".repeat(128)}).to_string());
    assert_eq!(bounded.status, 0, "{}", bounded.stderr);
    assert!(bounded.stdout.len() <= 2049);
    let component = Harness::<DateProvider>::get(component())
        .clock(instant)
        .call("date.now", json!({"format":"%s"}))
        .unwrap();
    assert_eq!(component.status, 0, "{}", component.stderr);
    assert_eq!(component.stdout, b"1704074584\n");
    assert!(component.http_calls.is_empty());
    assert_eq!(Harness::<DateProvider>::compiled_identities(), 1);
}
