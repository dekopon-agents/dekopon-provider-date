//! Date formatting owns no clock: only authorized invocation reads the broker's clock import.

pub mod date;

use clap::{ArgMatches, Command, CommandFactory, FromArgMatches, Parser, error::ErrorKind};
use date::{DateError, DateInput, RawInput};
use dekopon_provider_sdk::provider::{Capability, Clock, Proposal, Provider, Stdout, Usage};
use dekopon_provider_sdk::{EffectKind, RiskLevel};
use std::io::Write;

pub struct DateProvider;
pub struct Now;
const DESCRIPTION: &str =
    "Formats the fresh broker clock with explicit timezone and local calendar-day offsets";

#[derive(Parser)]
#[command(name = "date", about = DESCRIPTION, disable_help_flag = true, trailing_var_arg = true)]
struct Grammar {
    #[arg(allow_hyphen_values = true)]
    words: Vec<String>,
}

pub struct DateArgs(RawInput);

impl CommandFactory for DateArgs {
    fn command() -> Command {
        Grammar::command()
    }
    fn command_for_update() -> Command {
        Grammar::command_for_update()
    }
}
impl FromArgMatches for DateArgs {
    fn from_arg_matches(matches: &ArgMatches) -> Result<Self, clap::Error> {
        let args = Grammar::from_arg_matches(matches)?;
        if args.words.first().is_some_and(|word| word == "--help") {
            return if args.words.len() == 1 {
                Err(clap::Error::raw(
                    ErrorKind::DisplayHelp,
                    include_str!("help.txt"),
                ))
            } else {
                Err(clap::Error::raw(
                    ErrorKind::ArgumentConflict,
                    "date: --help must stand alone",
                ))
            };
        }
        if args.words.iter().any(|word| word == "--help") {
            return Err(clap::Error::raw(
                ErrorKind::ArgumentConflict,
                "date: --help must stand alone",
            ));
        }
        RawInput::from_argv(&args.words).map(Self).map_err(|_| {
            clap::Error::raw(
                ErrorKind::InvalidValue,
                "date: invalid arguments (try date --help)",
            )
        })
    }
    fn update_from_arg_matches(&mut self, matches: &ArgMatches) -> Result<(), clap::Error> {
        *self = Self::from_arg_matches(matches)?;
        Ok(())
    }
}
impl Parser for DateArgs {}

impl Provider for DateProvider {
    const ID: &'static str = "date";
    const COMMAND_WORDS: &'static [&'static str] = &["date"];
    const DESCRIPTION: &'static str = DESCRIPTION;
    type Args = DateArgs;
    type Capabilities = (Now,);

    fn propose(args: Self::Args, _stdin_piped: bool) -> Result<Proposal<Self>, Usage> {
        Ok(Proposal::to::<Now>(args.0))
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
