# Third-party code used by the engine

Nothing listed here is vendored into git. CMake fetches the SDKs at configure
time (`FetchContent`, see `engine/CMakeLists.txt`) into the build tree
(`engine/build*/_deps/`, or `target/saine-third-party/` for cargo builds),
both of which are git-ignored. Offline builds can point CMake at local
checkouts with `-DFETCHCONTENT_SOURCE_DIR_SAINE_CLAP=<dir>` and
`-DFETCHCONTENT_SOURCE_DIR_SAINE_VST3SDK=<dir>`.

| Component | Source | Pinned tag (commit) | License | What we use |
|---|---|---|---|---|
| CLAP | https://github.com/free-audio/clap | `1.2.10` (`195b42a004144fab0b3cf95e9c067187d15365b7`, 2026-07-13) | MIT | Headers only (`include/clap/…`): hosting, scanner, test plugins |
| VST 3 SDK | https://github.com/steinbergmedia/vst3sdk | `v3.8.1_build_84` (`3cdf9ca5d1f5b1b21e0a86832aa4abe55607bd96`, 2026-08-11); submodules `pluginterfaces` (`4f547e8e`), `public.sdk` (`586dc5e6`) | MIT (since VST 3.8) | `pluginterfaces` headers + IID definitions (`funknown.cpp`, `coreiids.cpp`, `vstinitiids.cpp`, `commoniids.cpp`); host helpers from `public.sdk/source/vst/hosting` (`hostclasses.cpp`, `pluginterfacesupport.cpp`) with `vst/utility/stringconvert.cpp` and `common/commonstringconvert.cpp` |
| signalsmith-stretch | https://github.com/Signalsmith-Audio/signalsmith-stretch | `1.4.0` (`a670068d9aeb64913331d5cc29337b19a457a7df`, 2026-09-25) | MIT | Header-only time stretcher (`signalsmith-stretch.h`), wrapped by `src/warp.cpp` for warped audio clips |
| signalsmith-linear | https://github.com/Signalsmith-Audio/linear | `0.6.4` (`de55e6a50ffcf6f8f43f649692d94691c7025151`, 2026-09-25) | MIT | Header-only FFT / STFT used by signalsmith-stretch (vDSP backend on macOS via Accelerate) |
| Audio Units, CoreAudioKit, AppKit | Apple system frameworks | macOS SDK | Apple SDK terms | AU hosting (`AudioToolbox`), `AUGenericView` / `AUCocoaUIBase` editors, NSWindow |

Notes:

* Only the `pluginterfaces` and `public.sdk` submodules of the VST 3 SDK are
  checked out (no `base`, `vstgui4`, `tutorials`, `doc`). The ASIO SDK is a
  separate product (GPLv3 / proprietary) and is **not** used.
* "VST" is a trademark of Steinberg Media Technologies GmbH. The MIT license
  covers the SDK code; use of the VST logo/trademark is governed separately by
  Steinberg's usage guidelines (`VST3_Usage_Guidelines.pdf` in the SDK). We
  ship no logo.
* The test plugins in `engine/test_plugins/` are original code written for
  this project against the SDK interfaces; they are built into the build tree
  only and are not distributed.

## CLAP — MIT License

```
MIT License

Copyright (c) 2021 Alexandre BIQUE

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

## VST 3 SDK — MIT License

(Text of `LICENSE.txt` at `v3.8.1_build_84`; the `pluginterfaces` and
`public.sdk` submodules carry the same license.)

```
MIT License

Copyright (c) 2026, Steinberg Media Technologies GmbH

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

## signalsmith-stretch — MIT License

```
MIT License

Copyright (c) 2022 Geraint Luff / Signalsmith Audio Ltd.

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

## signalsmith-linear — MIT License

```
MIT License

Copyright (c) 2025 Signalsmith Audio

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
