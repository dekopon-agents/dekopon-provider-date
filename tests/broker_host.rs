//! Actual published 0.15.2 broker + Cedar + component boundary, not a mock clock host.
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use dekopon_broker::{
    AuthenticatedContext, Broker, BrokerLimits, CapabilityRoute, ConstraintCatalog, ConstraintSet,
    CredentialStore, IdentityDirectory, InMemoryAuditLog, InvocationRequest, PolicyEngine,
    PolicyWorld,
};
use dekopon_broker_host::{BrokerHostLimits, BrokerProviderRegistry};
use dekopon_capability::{EffectKind, ExecutionConstraints, InvocationOutcome};
use dekopon_core::{Actor, PrincipalId, RiskLevel};
use dekopon_provider_sdk::{CommandRunOutcome, ProviderApiVersion};
use serde_json::json;
use tracing::{
    Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{Layer, layer::Context, prelude::*};

#[derive(Clone, Default)]
struct ClockReads(Arc<Mutex<Vec<(u64, String)>>>);
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
    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        if fields.clock {
            self.0.lock().unwrap().push((
                fields.millis.expect("clock event carries millis"),
                ctx.event_span(event)
                    .expect("clock read has invoke parent")
                    .name()
                    .to_owned(),
            ));
        }
    }
}

fn principal(name: &str) -> PrincipalId {
    name.parse().unwrap()
}
fn caller(name: &str) -> AuthenticatedContext {
    AuthenticatedContext::new(
        principal(name),
        Actor::Service {
            principal: principal(name),
        },
    )
    .unwrap()
}
fn request(id: &str, input: serde_json::Value) -> InvocationRequest {
    InvocationRequest {
        id: id.parse().unwrap(),
        capability: "clock.now".parse().unwrap(),
        trace_parent: "00-0000000000000000000000000000f1c7-00000000000000f1-00"
            .parse()
            .unwrap(),
        input,
        secret_use: None,
    }
}
fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[tokio::test(flavor = "multi_thread")]
async fn clock_only_after_cedar_authorization_with_no_http_storage_or_secret_grant() {
    let reads = ClockReads::default();
    tracing_subscriber::registry().with(reads.clone()).init();
    let path = PathBuf::from(
        std::env::var_os("DEKOPON_PROVIDER_COMPONENT")
            .expect("DEKOPON_PROVIDER_COMPONENT must point at the built component"),
    );
    let bytes = std::fs::metadata(&path).unwrap().len();
    assert!(bytes < 4 * 1024 * 1024, "component bytes: {bytes}");
    let registry = BrokerProviderRegistry::load(
        [path],
        BrokerHostLimits {
            max_memory_bytes: 32 * 1024 * 1024,
            max_input_bytes: 4096,
            max_output_bytes: 4096,
            fuel: 32_000_000,
            max_timeout: Duration::from_secs(10),
            ..BrokerHostLimits::default()
        },
    )
    .await
    .expect("real host describes component with clock disabled");
    let manifest = registry.manifests().next().unwrap();
    assert_eq!(manifest.api_version, ProviderApiVersion::V1Alpha1);
    assert_eq!(manifest.id.as_str(), "date");
    assert_eq!(manifest.command_words, ["date"]);
    assert_eq!(manifest.capabilities.len(), 1);
    assert_eq!(manifest.capabilities[0].id.as_str(), "clock.now");
    assert_eq!(manifest.capabilities[0].effect, EffectKind::ReadOnly);
    assert_eq!(manifest.capabilities[0].risk, RiskLevel::Low);
    for args in [vec![], vec!["--help".to_owned()], vec!["+%Q".to_owned()]] {
        let outcome = registry
            .run_command("date", &args, None)
            .await
            .expect("clock-disabled command run succeeds");
        if args.is_empty() {
            assert_eq!(
                outcome,
                CommandRunOutcome::Proposed {
                    capability: "clock.now".parse().unwrap(),
                    input: json!({"timezone":"UTC","days":0}),
                    secret_use: None
                }
            );
        }
    }
    assert!(reads.0.lock().unwrap().is_empty());
    let world = PolicyWorld::new(
        [principal("date-reader"), principal("denied-reader")],
        [("clock.now".parse().unwrap(), "date".parse().unwrap())],
    )
    .unwrap();
    let audit = Arc::new(InMemoryAuditLog::new(32).unwrap());
    let broker = Broker::new(
        registry,
        principal("test-broker"),
        "date-test-policy".to_owned(),
        PolicyEngine::new(include_str!("../examples/date.cedar"), &world).unwrap(),
        ConstraintCatalog::new([(
            "clock.now".parse().unwrap(),
            ConstraintSet {
                route: CapabilityRoute::Generic,
                provider: "date".parse().unwrap(),
                effect: EffectKind::ReadOnly,
                risk: RiskLevel::Low,
                credential: None,
                credential_by_agent: BTreeMap::new(),
                constraints: ExecutionConstraints {
                    timeout_ms: 10_000,
                    max_output_bytes: 4096,
                    http: None,
                    storage: None,
                    secret_use: None,
                },
            },
        )])
        .unwrap(),
        CredentialStore::empty(),
        IdentityDirectory::empty(),
        audit.clone(),
        BrokerLimits::default(),
    )
    .unwrap();
    let denied = broker
        .invoke(
            &caller("denied-reader"),
            None,
            None,
            request("denied", json!({"format":"%s"})),
        )
        .await
        .unwrap();
    assert_eq!(denied.outcome, InvocationOutcome::Denied);
    assert!(denied.output.is_none());
    assert!(
        reads.0.lock().unwrap().is_empty(),
        "Cedar denial must not invoke the clock"
    );
    let invalid = broker
        .invoke(
            &caller("date-reader"),
            None,
            None,
            request("invalid", json!({"format":"%Q"})),
        )
        .await
        .unwrap();
    assert_eq!(invalid.outcome, InvocationOutcome::Failed);
    assert!(
        reads.0.lock().unwrap().is_empty(),
        "invalid direct input must fail before clock"
    );
    let before = unix_seconds();
    let result = broker
        .invoke(
            &caller("date-reader"),
            None,
            None,
            request("allowed", json!({"format":"%s"})),
        )
        .await
        .unwrap();
    let after = unix_seconds();
    assert_eq!(result.outcome, InvocationOutcome::Succeeded, "{result:?}");
    let text = result.output.unwrap();
    let seconds: u64 = text.as_str().unwrap().parse().unwrap();
    assert!((before..=after).contains(&seconds));
    let captured = reads.0.lock().unwrap().clone();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].0 / 1000, seconds);
    assert_eq!(captured[0].1, "provider.invoke");
    let output = broker
        .invoke(
            &caller("date-reader"),
            None,
            None,
            request("bounded", json!({"format":"%s".repeat(128)})),
        )
        .await
        .unwrap();
    assert_eq!(output.outcome, InvocationOutcome::Succeeded);
    assert!(output.output.unwrap().as_str().unwrap().len() <= 2048);
    assert_eq!(reads.0.lock().unwrap().len(), 2);
    assert!(!audit.records().await.is_empty());
    eprintln!(
        "real broker 0.15.2: component={bytes} bytes; clock reads=2; denied/invalid/proposals/help reads=0; no HTTP/storage/secret grants"
    );
}
