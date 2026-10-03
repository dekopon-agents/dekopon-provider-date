use dekopon_date_provider::DateProvider;
use dekopon_provider_sdk_testkit::{Harness, conformance};
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
fn fixed_clock_component_formats_and_rejects_invalid_inputs() {
    conformance::<DateProvider>(component()).unwrap();
    let instant = UNIX_EPOCH + Duration::from_millis(1_704_074_584_567);
    for (input, expected) in [
        (json!({}), "2024-01-01T02:03:04Z\n"),
        (json!({"format":"%s"}), "1704074584\n"),
        (
            json!({"timezone":"America/New_York", "format":"%F"}),
            "2023-12-31\n",
        ),
        (json!({"format":""}), "\n"),
    ] {
        let result = Harness::<DateProvider>::get(component())
            .clock(instant)
            .call("date.now", input)
            .unwrap();
        assert_eq!(result.status, 0, "{}", result.stderr);
        assert_eq!(result.stdout, expected.as_bytes());
        assert!(result.http_calls.is_empty());
    }
    assert!(
        Harness::<DateProvider>::get(component())
            .clock(instant)
            .call("clock.now", json!({}))
            .is_err()
    );
    for (capability, input) in [
        ("date.now", json!({"format":"%Q"})),
        ("date.now", json!({"days":36601})),
        ("date.now", json!({"timezone":"bad"})),
        ("date.now", json!({"extra":true})),
    ] {
        let result = Harness::<DateProvider>::get(component())
            .clock(instant)
            .call(capability, input)
            .unwrap();
        assert_ne!(result.status, 0);
        assert!(result.stdout.is_empty());
    }
    let too_late = Harness::<DateProvider>::get(component())
        .clock(UNIX_EPOCH + Duration::from_millis(253_402_300_800_000))
        .call("date.now", json!({}))
        .unwrap();
    assert_ne!(too_late.status, 0);
    assert!(too_late.stdout.is_empty());
    assert_eq!(Harness::<DateProvider>::compiled_identities(), 1);
}
