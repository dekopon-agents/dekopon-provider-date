use dekopon_broker::{
    AuthenticatedContext, Broker, BrokerLimits, CapabilityRoute, ConstraintCatalog, ConstraintSet,
    CredentialStore, IdentityDirectory, InMemoryAuditLog, InvocationRequest, PolicyEngine,
    PolicyWorld,
};
use dekopon_broker_host::{BrokerHostLimits, BrokerProviderRegistry, asset::AssetInputs};
use dekopon_broker_protocol::{Streams, TraceParent};
use dekopon_capability::{ExecutionConstraints, InvocationOutcome};
use dekopon_core::{Actor, PrincipalId};
use dekopon_date_provider::DateProvider;
use dekopon_provider_sdk::{CommandRunOutcome, EffectKind, RiskLevel, provider};
use dekopon_provider_sdk_testkit::{Harness, Native, conformance};
use serde_json::json;
use std::{
    io::Read,
    os::{fd::OwnedFd, unix::net::UnixStream},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tracing::{
    Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{Layer, layer::Context, prelude::*};

fn component() -> PathBuf {
    std::env::var_os("DEKOPON_PROVIDER_COMPONENT")
        .expect("DEKOPON_PROVIDER_COMPONENT must point at built component")
        .into()
}

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

#[derive(Clone, Default)]
struct ClockReads(Arc<Mutex<Vec<u64>>>);
#[derive(Default)]
struct Fields {
    clock: bool,
    millis: Option<u64>,
}
impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "event" && value == "provider_clock_read" {
            self.clock = true;
        }
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "unix_millis" {
            self.millis = Some(value);
        }
    }
    fn record_debug(&mut self, _field: &Field, _value: &dyn std::fmt::Debug) {}
}
impl<S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>> Layer<S> for ClockReads {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        if fields.clock {
            self.0
                .lock()
                .unwrap()
                .push(fields.millis.expect("clock event has unix_millis"));
        }
    }
}
fn principal(name: &str) -> PrincipalId {
    name.parse().unwrap()
}
fn caller(name: &str) -> AuthenticatedContext {
    AuthenticatedContext::attested(
        principal(name),
        Actor::Agent {
            agent: "date-test".parse().unwrap(),
        },
        principal("gateway"),
        "slack.t0123abc.u9xyz".parse().unwrap(),
    )
    .unwrap()
}
fn request(id: &str, input: serde_json::Value) -> InvocationRequest {
    InvocationRequest {
        id: id.parse().unwrap(),
        capability: "date.now".parse().unwrap(),
        trace_parent: TraceParent::new([7; 16], [3; 8], 1).unwrap(),
        input,
        secret_use: None,
    }
}
fn streams() -> (AssetInputs, UnixStream) {
    let (stdout, peer) = UnixStream::pair().unwrap();
    (
        AssetInputs {
            streams: Some(Streams {
                stdin: None,
                stdout: OwnedFd::from(stdout),
            }),
            ..AssetInputs::default()
        },
        peer,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn cedar_refusal_and_invalid_input_never_read_clock_but_authorized_component_does() {
    tokio::task::spawn_blocking(authorization_and_pure_proposals_guard_clock_reads)
        .await
        .unwrap();
    let reads = ClockReads::default();
    tracing_subscriber::registry().with(reads.clone()).init();
    let registry = BrokerProviderRegistry::load(
        [component()],
        BrokerHostLimits {
            max_input_bytes: 4096,
            max_output_bytes: 4096,
            fuel: 32_000_000,
            max_timeout: Duration::from_secs(10),
            ..BrokerHostLimits::default()
        },
    )
    .await
    .unwrap();
    let manifest = registry.manifests().next().unwrap();
    assert_eq!(manifest.id.as_str(), "date");
    assert_eq!(manifest.capabilities[0].id.as_str(), "date.now");
    for args in [vec![], vec!["--help".to_owned()], vec!["+%Q".to_owned()]] {
        registry.run_command("date", &args, false).await.unwrap();
    }
    assert!(
        reads.0.lock().unwrap().is_empty(),
        "proposal/help cannot read clock"
    );
    let world = PolicyWorld::new(
        [principal("date-reader"), principal("denied-reader")],
        [("date.now".parse().unwrap(), "date".parse().unwrap())],
    )
    .unwrap();
    let audit = Arc::new(InMemoryAuditLog::new(32).unwrap());
    let broker = Broker::new(
        registry,
        principal("test-broker"),
        "date-test-policy".to_owned(),
        PolicyEngine::new(include_str!("../examples/date.cedar"), &world).unwrap(),
        ConstraintCatalog::new([(
            "date.now".parse().unwrap(),
            ConstraintSet {
                route: CapabilityRoute::Generic,
                provider: "date".parse().unwrap(),
                effect: EffectKind::ReadOnly,
                risk: RiskLevel::Low,
                credential: None,
                constraints: ExecutionConstraints {
                    timeout_ms: 10_000,
                    ..ExecutionConstraints::default()
                },
            },
        )])
        .unwrap(),
        CredentialStore::empty(),
        IdentityDirectory::empty(),
        Arc::clone(&audit),
        BrokerLimits::default(),
    )
    .unwrap();
    let denied = broker
        .invoke(
            &caller("denied-reader"),
            None,
            None,
            request("denied", json!({"format":"%s"})),
            AssetInputs::default(),
        )
        .await
        .unwrap();
    assert_eq!(denied.result.outcome, InvocationOutcome::Denied);
    assert_eq!(denied.result.error.as_deref(), Some("policy-denied"));
    assert!(reads.0.lock().unwrap().is_empty());
    assert!(
        audit
            .records()
            .iter()
            .all(|event| !matches!(event, dekopon_broker::AuditEvent::Execution { .. }))
    );
    let (invalid_streams, mut invalid_peer) = streams();
    let invalid = broker
        .invoke(
            &caller("date-reader"),
            None,
            None,
            request("invalid", json!({"format":"%Q"})),
            invalid_streams,
        )
        .await
        .unwrap();
    assert_eq!(
        invalid.result.outcome,
        InvocationOutcome::Failed,
        "{invalid:?}"
    );
    let mut invalid_stdout = Vec::new();
    invalid_peer.read_to_end(&mut invalid_stdout).unwrap();
    assert!(invalid_stdout.is_empty());
    assert!(
        reads.0.lock().unwrap().is_empty(),
        "invalid input cannot read clock"
    );
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (assets, mut peer) = streams();
    let allowed = broker
        .invoke(
            &caller("date-reader"),
            None,
            None,
            request("allowed", json!({"format":"%s"})),
            assets,
        )
        .await
        .unwrap();
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert_eq!(
        allowed.result.outcome,
        InvocationOutcome::Succeeded,
        "{allowed:?}"
    );
    let mut stdout = Vec::new();
    peer.read_to_end(&mut stdout).unwrap();
    let text = std::str::from_utf8(&stdout).unwrap();
    let seconds: u64 = text.trim_end_matches('\n').parse().unwrap();
    assert!(text.ends_with('\n'));
    assert!((before..=after).contains(&seconds));
    let captured = reads.0.lock().unwrap().clone();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0] / 1000, seconds);
    assert!(
        audit
            .records()
            .iter()
            .any(|event| matches!(event, dekopon_broker::AuditEvent::Execution { .. }))
    );
}
