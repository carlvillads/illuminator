# Changelog

All notable changes to Illuminator will be documented in this file.

This format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1] - 2026-10-22

### Added

- Add stale histograms while counting new frames.

### Changed

- Assume millimeters for volume geometry by default.
- Update assumeing millimeters warning to be more clear.
- Make each layer solid outside of the window by default.

### Fixed

- Fix missing title bars on Linux window managers.
- Fix mouse pointer alignment in Linux.
- Prevent that the frame number on 4D series can point past the last frame available.
- Fix header-image pairing for the Analyze format.
- Fix UI jumping when scrubbing a 4D series.


## [0.1.0] - 2026-10-19

### Added
- Initial release

### Removed
- Nothing

[Unreleased]: https://github.com/carlvillads/illuminator/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/carlvillads/illuminator/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/carlvillads/illuminator/releases/tag/v0.1.0
