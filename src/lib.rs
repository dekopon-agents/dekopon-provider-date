//! Date formatting owns no clock: only authorized invocation reads the broker's clock import.

pub mod date;

use clap::Parser;
use date::{DateError, DateInput, RawInput};
use dekopon_provider_sdk::provider::{Capability, Clock, Proposal, Provider, Stdout, Usage};
use dekopon_provider_sdk::{EffectKind, RiskLevel};
use std::io::Write;

pub struct DateProvider;
pub struct Now;
const DESCRIPTION: &str =
    "Formats the fresh broker clock with explicit timezone and local calendar-day offsets";

#[derive(Parser)]
#[command(name = "date", about = DESCRIPTION)]
pub struct DateArgs {
    #[arg(short = 'u', long = "utc", conflicts_with = "timezone")]
    utc: bool,
    #[arg(long, value_name = "NAME")]
    timezone: Option<String>,
    #[arg(long, value_name = "N", allow_hyphen_values = true)]
    days: Option<String>,
    #[arg(value_name = "+FORMAT")]
    format: Option<String>,
}

impl Provider for DateProvider {
    const ID: &'static str = "date";
    const COMMAND_WORDS: &'static [&'static str] = &["date"];
    const DESCRIPTION: &'static str = DESCRIPTION;
    type Args = DateArgs;
    type Capabilities = (Now,);

    fn propose(args: Self::Args, _stdin_piped: bool) -> Result<Proposal<Self>, Usage> {
        let mut words = Vec::new();
        if args.utc {
            words.push("--utc".to_owned());
        }
        if let Some(zone) = args.timezone {
            words.extend(["--timezone".to_owned(), zone]);
        }
        if let Some(days) = args.days {
            words.extend(["--days".to_owned(), days]);
        }
        if let Some(format) = args.format {
            words.push(format);
        }
        RawInput::from_argv(&words)
            .map(Proposal::to::<Now>)
            .map_err(|error| Usage::new(error.message().to_owned()))
    }
}

impl Capability for Now {
    type Provider = DateProvider;
    const NAME: &'static str = "now";
    const DESCRIPTION: &'static str = DESCRIPTION;
    const EFFECT: EffectKind = EffectKind::ReadOnly;
    const RISK: RiskLevel = RiskLevel::Low;
    type Input = RawInput;
    type Needs = Clock;
    type Error = DateError;

    fn run(input: RawInput, clock: Clock, out: &mut Stdout) -> Result<(), DateError> {
        let input = DateInput::from_raw(input)?;
        let rendered = input.render(clock.now_unix_millis())?;
        out.write_all(rendered.as_bytes())
            .and_then(|_| out.write_all(b"\n"))
            .map_err(|_| DateError::new("output-limit", "date output could not be written"))
    }
}

#[allow(unsafe_code)]
mod export {
    dekopon_provider_sdk::export!(super::DateProvider);
}

#[cfg(test)]
mod tests;
