# Zexe (ZX Executable)

Zexe is a modern project designed to turn ZX Spectrum snapshots into standalone, self-contained executable files for Windows and Linux. It allows you to package your favorite ZX Spectrum games into a single binary that includes both the emulator and the game data, making them as easy to run as any other native application.

## Components

The project consists of two main tools:

1.  **zexe-runner**: A specialized, lightweight ZX Spectrum emulator built in Rust using [rustzx-core](https://github.com/rustzx/rustzx). It is optimized for performance, low-latency audio, and features a clean OpenGL-based renderer.
2.  **zexe-bundler**: A packaging utility that attaches a ZX Spectrum snapshot and configuration metadata to the runner, creating the final standalone executable.

## Features

- **Portable**: Generates a single file with no external dependencies required.
- **Multi-Platform**: Native support for both Windows and Linux.
- **Snapshot Format Support**: Handles `.sna`, `.z80`, and `.szx` snapshot formats.
- **Spectrum 48K/128K Support**: Automatic selection of 48K or 128K modes based on the input snapshot.
- **High-Quality Rendering**: Uses OpenGL (via `glow`) for smooth scaling, with support for built-in filtering (Nearest, Linear, Scanlines).
- **RetroArch Shader Support**: Support for external and embedded RetroArch-compatible `.glsl` shaders for advanced post-processing.
- **Joystick Mapping**: Support for Kempston, Sinclair, and Cursor joysticks mapped to the cursor keys.
- **OSD (On-Screen Display)**: Semi-transparent overlay for volume control and status information.

## Compilation

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (latest stable version recommended)
- A C compiler (for some dependencies)
- Development headers for OpenGL and ALSA (on Linux)

### Windows

You can use the provided batch script to build both components and package them into the `dist` folder:

```batch
build_dist.bat
```

Or manually:
```powershell
cd zexe-runner
cargo build --release
cd ../zexe-bundler
cargo build --release
```

### Linux

You can use the provided shell script:

```bash
chmod +x build_dist.sh
./build_dist.sh
```

Or manually:
```bash
cd zexe-runner && cargo build --release
cd ../zexe-bundler && cargo build --release
```

*Note: The Linux script will attempt to use `upx` to compress the resulting binaries if it is installed.*

## Usage Guide

To create a standalone game, you need the `zexe-bundler` and `zexe-runner` binaries (found in the `dist` folder after building).

### Creating a Standalone Executable

Run the bundler with your game snapshot. The bundler will also automatically look for `.glsl`, `.pok`, and `.json` files with the same name as the input if you don't provide them explicitly.

**Windows:**
```powershell
.\dist\zexe-bundler.exe my_game.z80
```
This will generate `my_game.exe`.

**Linux:**
```bash
./dist/zexe-bundler my_game.z80
```
This will generate `my_game`.

**Customizing the Output:**
If you want to specify a different output name:
```powershell
.\dist\zexe-bundler.exe my_game.z80 --output different_name.exe
```

**Customizing the Runner:**
By default, the bundler looks for `zexe-runner` (or `zexe-runner.exe`) in the current working directory. If it is located elsewhere, use the `--runner` flag:
```bash
./zexe-bundler game.z80 --runner ./path/to/zexe-runner
```

**With optional components:**
```bash
./dist/zexe-bundler game.z80 --output game.exe --shader crt.glsl --pokes cheats.pok --config custom.json
```

### Runtime Controls

Once running the executable, the following hotkeys are available:

- **ESC**: Exit the application.
- **F1**: Show version info on OSD.
- **F2**: Cycle filtering modes (Nearest, Linear, Scanlines, Shaders).
- **F3**: Cycle joystick modes (Kempston, Sinclair 1/2, Cursor, Off).
- **F4**: Cycle border modes (Full, Minimal, None).
- **F5**: Toggle Fullscreen.
- **F6**: Toggle POKEs (cheats) if a `.pok` file is loaded.
- **F7 / F8**: Decrease / Increase volume.
- **F9**: Toggle Mute.
- **F10**: Toggle between 1x speed and Full Speed (Warpspeed).

### Keyboard Joysticks
When a joystick mode is active (**F3**), the **Arrow Keys** and **Alt Left** are automatically mapped to the corresponding ZX Spectrum inputs:
- **Kempston**: Arrow keys + Alt Left (Fire).
- **Sinclair 1 / 2**: Arrow keys + Alt Left (Fire).
- **Cursor**: Arrow keys + Alt Left (Fire / Key 0).

*Note: You can still use the standard numeric keys (1-0) for Sinclair joysticks if you prefer, as they are part of the standard keyboard mapping.*

## Advanced Features

### Custom Shaders
Zexe supports custom OpenGL shaders for post-processing. To ensure compatibility, shaders should follow the **RetroArch (.glsl)** standard format.
- **Internal**: Use the `--shader` flag with `zexe-bundler` to embed a `.glsl` file.
- **External/Fallback**: The runner will look for shaders in the following order:
  1. An embedded shader in the executable.
  2. A file with the same name as the executable (e.g., `my_game.glsl`).
  3. A file named `shader.glsl` in the same directory as the executable.

### Cheats (POKEs)
You can include POKEs to cheat in games.
- **Internal**: Use the `--pokes` flag with `zexe-bundler` to embed a `.pok` file.
- **External/Fallback**: The runner will look for a `.pok` file with the same name as your executable (e.g., `my_game.pok`) in the same directory.

### Configuration
A `config.json` can be embedded using the `--config` flag or placed in the same directory as a fallback.
- **Order of preference**: Embedded configuration > `my_game.json` > `config.json`.
- **Available settings**:
  - `fullscreen`: true/false
  - `filtering`: "Nearest", "Linear", "Scanlines", "Embedded", "Custom"
  - `joystick`: "Kempston", "Sinclair1", "Sinclair2", "Cursor", "Off"
  - `border`: "Full", "Minimal", "None"
  - `cheats_enabled`: true/false
  - `volume`: 0-200 (100 is default)

## License

This project is licensed under the GPLv3 License - see the LICENSE file for details.
