# Changelog

All notable changes to Illuminator will be documented in this file.

This format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Bump gpui-kit version from 0.7.1 to 0.8.1
- Change tooltip delay from 600ms to 300ms

## [0.2.0] - 2026-10-28

### Added

- Add a time series window (<kbd>Shift</kbd>+<kbd>G</kbd>, and the button next to "Curve at cursor").
- Measure the time series at, or in a cube around the cursor ($3 \times 3 \times 3$, $5 \times 5 \times 5$ or $7 \times 7 \times 7$ voxels), or inside label map labels.
- Show the mean, minimum, or maximum of the area in each frame.
- Click or drag on the time series to go to that frame.
- Copy or save the time series as CSV with voxel count, mean, std. dev., minimum, and maximum of each frame.
- Add Arch Linux build

### Changed

- Update app icon.
- Share one window layout between the histogram and time series windows.
- Update toolbar icon behaviour.
- Major refactor of variable and function names.
- Change message when no volume is open.
- Update cache handling to delete temporary files on app close.
- Label statistics CSV is saved as `<name>_statistics.csv` instead of `<name>_stats.csv`

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

[Unreleased]: https://github.com/carlvillads/illuminator/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/carlvillads/illuminator/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/carlvillads/illuminator/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/carlvillads/illuminator/releases/tag/v0.1.0
