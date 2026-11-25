# Third-party components

The runner uses RustZX for Spectrum emulation, winit and glutin for window management, glow for OpenGL, and CPAL and ringbuf for sound. The release uses the versions recorded in Cargo.lock. License texts for the Rust dependencies are included under docs/licenses in the binary packages.

The Sinclair Spectrum firmware is supplied by rustzx-core. Copyright belongs to Amstrad. The firmware may be redistributed with emulators and may not be sold separately.

## RustZX

```text
The MIT License (MIT)

Copyright (c) 2016 Vladyslav Nikonov

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
