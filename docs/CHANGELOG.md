# Changelog

## 0.1.1 (2026-11-25)

- Load banked SNA files, recognize 128K SZX and select the correct Z80 version-specific memory layout. Report incomplete snapshots on startup.
- Restore interrupt enable state from Z80 snapshots and read external configuration beside the executable.
- Use the audio device’s native sample format and restore the original byte when a trainer omits it.
- Find the bundled runner when launching from another directory and preserve existing output if writing fails.

## 0.1.0 (2026-01-29)

- Initial release of the Windows and Linux snapshot bundler and runner.
