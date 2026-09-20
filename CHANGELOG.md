# Changelog

## [Unreleased]

## [0.2.0] - 2026-09-20

### Changed
- Move to provider SDK 0.18.0; caller behavior is unchanged.

### Added
- Initial bounded date command backed only by the authorized broker clock, with IANA timezones,
  supported strftime conversions, checked local calendar-day offsets, and pure command proposals.
- Shared CI/release callers and deterministic component/native broker authorization conformance.
