//! Date formatting owns no clock: only authorized invocation reads the broker's clock import.

mod date;

use date::{DateInput, RawInput};
use dekopon_provider_sdk::{
    CapabilityId, CommandRun, EffectKind, Provider, ProviderApiVersion, ProviderCapability,
    ProviderError, ProviderManifest, RiskLevel,
};
use serde_json::{Value, json};

mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "provider",
        generate_all,
        pub_export_macro: true,
    });
}

struct DateProvider;
const NOW: &str = "clock.now";
const HELP: &str = include_str!("help.txt");
const DESCRIPTION: &str =
    "Formats the fresh broker clock with explicit timezone and local calendar-day offsets";

impl Provider for DateProvider {
    fn manifest() -> ProviderManifest {
        ProviderManifest {
            api_version: ProviderApiVersion::V1Alpha1,
            id: "date".parse().expect("static provider ID"),
            description: DESCRIPTION.to_owned(),
            command_words: vec!["date".to_owned()],
            capabilities: vec![ProviderCapability {
                id: NOW.parse().expect("static capability ID"),
                description: DESCRIPTION.to_owned(),
                effect: EffectKind::ReadOnly,
                risk: RiskLevel::Low,
                input_schema: date::schema(),
            }],
        }
    }

    fn invoke(capability: &CapabilityId, input: Value) -> Result<Value, ProviderError> {
        if capability.as_str() != NOW {
            return Err(ProviderError::new(
                "unsupported",
                "date implements only clock.now",
            ));
        }
        // The schema is metadata, not enforcement. Validate again before touching the host.
        let input = DateInput::from_value(input)?;
        let millis = dekopon_provider_clock::now_unix_millis();
        // The shell prints a JSON string verbatim and adds one newline; do not double it here.
        Ok(json!(input.render(millis)?))
    }

    fn run_command(argv: &[String], _stdin: Option<&str>) -> Result<CommandRun, ProviderError> {
        if argv.len() == 1 && argv[0] == "--help" {
            return Ok(CommandRun::rendered(HELP, 0));
        }
        match RawInput::from_argv(argv) {
            Ok(input) => Ok(CommandRun::proposal(
                NOW.parse().expect("static capability ID"),
                serde_json::to_value(input).expect("bounded input serializes"),
            )),
            Err(error) => Ok(CommandRun::rendered_error(
                format!("date: {}\nTry 'date --help'.\n", error.message()),
                2,
            )),
        }
    }
}

dekopon_provider_sdk::export_provider_with_cli!(DateProvider, bindings);

#[cfg(test)]
mod tests;
