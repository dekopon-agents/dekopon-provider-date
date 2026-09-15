//! Deterministic component execution: sole clock import, no ambient linker, and exact shell text.
use std::{path::PathBuf, sync::Mutex};

use dekopon_core::SecretUseProposal;
use dekopon_provider_sdk::{CommandRunOutcome, ComponentResponse};
use dekopon_shell::{CapabilityCallResult, CapabilityInvoker, CommandRun, Interpreter, Limits};
use serde_json::{Value, json};
use wasmtime::{
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder,
    component::{Component, HasSelf, Linker},
};

mod bindings {
    wasmtime::component::bindgen!({ path: "wit", world: "provider" });
}
struct State {
    millis: u64,
    reads: usize,
    limits: StoreLimits,
}
impl bindings::dekopon::clock::wall::Host for State {
    fn now_unix_millis(&mut self) -> u64 {
        self.reads += 1;
        self.millis
    }
}
struct Guest(Mutex<(Store<State>, bindings::Provider)>);
impl Guest {
    fn new() -> Self {
        let path = PathBuf::from(
            std::env::var_os("DEKOPON_PROVIDER_COMPONENT")
                .expect("DEKOPON_PROVIDER_COMPONENT must point at the built component"),
        );
        let mut config = Config::new();
        config.wasm_component_model(true).consume_fuel(true);
        let engine = Engine::new(&config).unwrap();
        let component = Component::from_file(&engine, path).unwrap();
        let imports = component
            .component_type()
            .imports(&engine)
            .map(|(name, _)| name.to_owned())
            .collect::<Vec<_>>();
        assert_eq!(imports, ["dekopon:clock/wall@1.0.0"]);
        let mut store = Store::new(
            &engine,
            State {
                millis: 1_704_074_584_567,
                reads: 0,
                limits: StoreLimitsBuilder::new()
                    .memory_size(32 * 1024 * 1024)
                    .build(),
            },
        );
        store.limiter(|state| &mut state.limits);
        store.set_fuel(32_000_000).unwrap();
        let mut linker = Linker::new(&engine);
        let error = bindings::Provider::instantiate(&mut store, &component, &linker)
            .err()
            .expect("empty linker refuses clock import");
        assert!(format!("{error:#}").contains("dekopon:clock/wall"));
        bindings::Provider::add_to_linker::<_, HasSelf<_>>(&mut linker, |state: &mut State| state)
            .unwrap();
        let provider = bindings::Provider::instantiate(&mut store, &component, &linker).unwrap();
        let manifest = provider.call_describe(&mut store).unwrap();
        let manifest: dekopon_provider_sdk::ProviderManifest =
            serde_json::from_str(&manifest).unwrap();
        assert_eq!(manifest.id.as_str(), "date");
        assert_eq!(store.data().reads, 0);
        Self(Mutex::new((store, provider)))
    }
    fn invoke_wire(&self, capability: &str, input: &str) -> ComponentResponse {
        let mut guest = self.0.lock().unwrap();
        let (store, provider) = &mut *guest;
        store.set_fuel(32_000_000).unwrap();
        let output = provider
            .call_invoke(&mut *store, capability, input)
            .unwrap();
        assert!(output.len() <= 4096);
        serde_json::from_str(&output).unwrap()
    }
}
impl CapabilityInvoker for Guest {
    fn granted(&self) -> Vec<String> {
        vec!["clock.now".to_owned()]
    }
    fn command_words(&self) -> Vec<String> {
        vec!["date".to_owned()]
    }
    fn run_command(&self, word: &str, argv: &[String], stdin: Option<&str>) -> Option<CommandRun> {
        assert_eq!(word, "date");
        let mut guest = self.0.lock().unwrap();
        let (store, provider) = &mut *guest;
        store.set_fuel(32_000_000).unwrap();
        let reads = store.data().reads;
        let encoded = provider.call_run_command(&mut *store, argv, stdin).unwrap();
        assert_eq!(store.data().reads, reads, "command runs are pure");
        Some(
            match serde_json::from_str::<CommandRunOutcome>(&encoded).unwrap() {
                CommandRunOutcome::Proposed {
                    capability,
                    input,
                    secret_use,
                } => CommandRun::Proposed {
                    capability: capability.to_string(),
                    input,
                    secret_use,
                },
                CommandRunOutcome::Rendered {
                    stdout,
                    stderr,
                    status,
                } => CommandRun::Rendered {
                    stdout,
                    stderr,
                    status,
                },
                other => panic!("unexpected command outcome: {other:?}"),
            },
        )
    }
    fn invoke(
        &self,
        capability: &str,
        input: Value,
        secret_use: Option<SecretUseProposal>,
    ) -> CapabilityCallResult {
        assert!(secret_use.is_none());
        match self.invoke_wire(capability, &input.to_string()) {
            ComponentResponse::Succeeded { output } => CapabilityCallResult::Succeeded(output),
            other => panic!("unexpected invoke response: {other:?}"),
        }
    }
}

#[test]
fn exact_component_text_and_shell_substitutions_with_a_fixed_clock() {
    let guest = Guest::new();
    let interpreter = Interpreter::new(Limits::default());
    for (script, expected) in [
        ("date", "2024-01-01T02:03:04Z"),
        ("date +%s", "1704074584"),
        (
            "date --timezone America/New_York '+%F %H:%M:%S %z %Z %%'",
            "2023-12-31 21:03:04 -0500 EST %",
        ),
        (
            "today=$(date --timezone America/New_York +%F); echo \"[$today]\"",
            "[2023-12-31]",
        ),
        ("date +%F; date +%s", "2024-01-01\n1704074584"),
        ("date +; echo end", "\nend"),
    ] {
        let outcome = interpreter.run(script, &guest);
        assert_eq!(outcome.exit_code.get(), 0, "{script}: {outcome:?}");
        // ScriptOutcome strips the final line terminator; interior separators are exactly one LF.
        assert_eq!(outcome.output, expected, "{script}");
    }
    let outcome = interpreter.run("date --help", &guest);
    assert_eq!(outcome.exit_code.get(), 0);
    assert_eq!(
        outcome.output,
        include_str!("../src/help.txt").trim_end_matches('\n')
    );
}

#[test]
fn direct_component_invalid_inputs_and_host_overflow_fail_without_traps() {
    let guest = Guest::new();
    for (capability, input) in [
        ("clock.other", "{}"),
        ("clock.now", "not-json"),
        ("clock.now", "{\"format\":\"%Q\"}"),
        ("clock.now", "{\"days\":36601}"),
        ("clock.now", "{\"timezone\":\"bad\"}"),
    ] {
        assert!(matches!(
            guest.invoke_wire(capability, input),
            ComponentResponse::Failed { .. }
        ));
    }
    assert_eq!(guest.0.lock().unwrap().0.data().reads, 0);
    guest.0.lock().unwrap().0.data_mut().millis = u64::MAX;
    let ComponentResponse::Failed { error } = guest.invoke_wire("clock.now", "{}") else {
        panic!("overflow must fail")
    };
    assert_eq!(error.code, "clock-out-of-range");
    guest.0.lock().unwrap().0.data_mut().millis = 253_402_214_400_000;
    assert_eq!(
        guest.invoke_wire("clock.now", &json!({"format":"%s".repeat(128)}).to_string()),
        ComponentResponse::Succeeded {
            output: json!("253402214400".repeat(128))
        }
    );
}
