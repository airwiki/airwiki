# Inventory of Components Not Managed by Cargo

This inventory covers distributed assets and build tools outside Cargo. The artifacts are
identified by SHA-256. The included legal texts were copied from the exact artifacts
or tags listed below, with line endings normalized to LF and a single trailing newline added. The
application, the MCP bridge, and their Rust dependencies are covered by
`THIRD_PARTY_LICENSES.md`.

## SPDX 2.3 JSON Schema: non-distributed release validator

Release metadata is checked against the official SPDX 2.3 JSON Schema from
`spdx/spdx-spec` commit `aadf3b0b8dbbabdb4d880b0fc714255fea436ff7` (tag `v2.3`).
The exact source is `packaging/schemas/spdx-2.3.schema.json`, SHA-256
`3ec6cd5b8ba0c9a3e821da48536fa1b814567dc7e4376efe98d3e7b2a7a8d230`, under
CC-BY-3.0. It is a source-tree build validator and is not installed or
published as a release asset.

## Bundled interface fonts

The desktop WebView bundles two variable WOFF2 files and never fetches fonts from a CDN or the
network. Both are redistributed unmodified under the SIL Open Font License 1.1.

- Space Grotesk 2.0.0, tag/commit `7220f5d04813fe83babe76d4fd23e02275021280`:
  `apps/desktop/ui/src/assets/fonts/SpaceGrotesk-Variable.woff2`, 49,256 bytes, SHA-256
  `8e085aa438094f11487a836652edd5c054fa6a96f63fc7c282105ee3a4b08c07`. The exact upstream
  `OFL.txt` is included as `non-cargo/Space-Grotesk-2.0.0-OFL.txt`, SHA-256
  `564ce565c371c5e5bbf286006565a7c9aa55a9f56e7ca58d56e05d649dd61a72`.
- Atkinson Hyperlegible Next, commit `7925f50f649b3813257faf2f4c0b381011f434f1`:
  `apps/desktop/ui/src/assets/fonts/AtkinsonHyperlegibleNext-Variable.woff2`, 48,188 bytes,
  SHA-256 `abde1ad5cf78b9ac575ef90d991f2e9101eb0b3b6668bde9a00e2e1e27d99afd`. The exact upstream
  `OFL.txt` is included as `non-cargo/Atkinson-Hyperlegible-Next-7925f50-OFL.txt`, SHA-256
  `aca6a428580965d2297d1b718042dd427c2a9443ece3b0d02d758e161e0c4030`.

## Third-party product marks

The desktop displays these marks only beside the corresponding local integration so users can
identify the client they are connecting. They are not AirWiki branding, do not imply sponsorship
or endorsement, and are never recolored, combined with the AirWiki mark, or used for an unknown
MCP client. Product and company names and marks remain the property of their respective owners.

- ChatGPT and Codex artwork was sourced from the official OpenAI ChatGPT macOS application
  `26.810.41047` (`6570`). The source files `icon-chatgpt.png` and
  `icon-codex-dark-color.png` have SHA-256
  `3453947a9ce2709b7ec51c0559c7eb976e4ac53b232b607d1d81b0d1d1048b61` and
  `69fb4384e161be8a20dcb94a9ac34aea4fbfaeb67514110a71e7b0732eccb0fc`. They were scaled
  proportionally to 256 px without cropping or recoloring. Distributed files:
  `apps/desktop/ui/src/assets/brands/chatgpt.png`, 51,573 bytes, SHA-256
  `29a63f80864a00daa15dd1a721b81e0aea59d10cb1827fb023e7587ebcd90c1e`; and
  `apps/desktop/ui/src/assets/brands/codex.png`, 43,899 bytes, SHA-256
  `051c1731e00275c8750fab436141b166c59cce519410681c34dfeca16fda1040`.
  Use is subject to the [OpenAI design guidelines and Marks usage
  terms](https://openai.com/brand/).
- Claude Desktop and Claude Code use `ClaudeIcon-Rounded.svg` and `Claude Spark - Clay.svg`
  from Anthropic's official newsroom press kit. The pinned 26,465,941-byte press-kit ZIP has
  SHA-256 `c68ac92df86c825f95177e24016fcc9a8863a3fd4ca344fe6f0700b2c1e07151`.
  The SVGs are distributed unmodified as
  `apps/desktop/ui/src/assets/brands/claude-desktop.svg`, 3,064 bytes, SHA-256
  `059e22f525d67c6258c4f64514f0b0e717c914df8a706936d0299d5e6b8082d9`; and
  `apps/desktop/ui/src/assets/brands/claude-code.svg`, 2,580 bytes, SHA-256
  `6d53db4be375e899c937c26cf16684a80d6e869b1928d72b37748bef2560e219`.
- Gemini CLI uses the official IDE companion icon from
  `google-gemini/gemini-cli` commit `5411f113cafae26161b4969b0237b8e1e024e2c2`. The upstream
  46,696-byte PNG has SHA-256
  `351e9f5b1bf863d738cd7be4ed040a625a1419450ae7fc490143e4042b7c2438`; it was scaled
  proportionally to 256 px without cropping or recoloring. Distributed file:
  `apps/desktop/ui/src/assets/brands/gemini-cli.png`, 28,218 bytes, SHA-256
  `28cfe81a91a7c58906f87970a2185e98707f391a079fe5455a5b71d48345baa1`. The upstream
  repository is Apache-2.0; Google and Gemini marks remain subject to Google's trademark terms.

## llama.cpp b9946: Windows runtime built from source

The Windows x64 runtime is built in CI from the exact commit
`fb30ba9a6c5b4674174d06aed14794832ab33278` (tag `b9946`). The upstream binary ZIP is not
redistributed: that archive contains an OpenMP library under `debug_nonredist` for which an
adequate redistribution grant was not verified. The gate rejects any reference to, or file from,
that payload.

- Pinned source:
  `https://github.com/ggml-org/llama.cpp/archive/fb30ba9a6c5b4674174d06aed14794832ab33278.zip`.
- Size: `36865897` bytes; SHA-256:
  `7a36a3e384ad29ce4ffbac0051f31b7265105d7d8c3240e5ab9a859e952ec3a2`.
- Policy: `GGML_OPENMP=OFF`, `BUILD_SHARED_LIBS=OFF`, static MSVC runtime
  `MultiThreaded` (`/MT`), minimum AVX2 CPU, and `/experimental:deterministic` plus `/Brepro`
  with remapped paths.
- Allowed imports in the final PE: `ADVAPI32.dll`, `KERNEL32.dll`, `SHELL32.dll`, and
  `WS2_32.dll`. No OpenMP, MSVC, or UCRT DLLs are distributed with the server.
- Reviewed toolchain family: Visual Studio 17.14, VC Tools 14.44/MSVC 19.44, Windows SDK
  `10.0.26100.0`, CMake `3.31.6-msvc6`, and Ninja `1.12.1` or `1.13.2`. Each candidate's
  manifest records the exact versions and hashes used, including `cmd.exe`, `curl.exe`,
  `tar.exe`, the C/C++/ASM compilers, `rc.exe`, and `mt.exe`; every participating system
  executable is validated as a Microsoft-signed binary.

The distributed runtime contains only `llama-server.exe` and `BUILD-MANIFEST.json`. The manifest
authenticates the source, policy, toolchain, imports, smoke test, size, and executable SHA-256. That
SHA-256 is embedded in the desktop application during the same build; packaging, signing, final
verification, and clean installation verify the binding again. There is no fixed executable hash
independent of the approved toolchain.

Legal inventory for code linked into `llama-server`:

- llama.cpp `LICENSE`, MIT: raw and normalized SHA-256
  `94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d`; text included in
  `non-cargo/llama.cpp-b9946-LICENSE.txt`.
- nlohmann/json `licenses/LICENSE-jsonhpp`, MIT: raw and normalized SHA-256
  `c0d068392ea65358b798b8c165103560f06e9e3b38c4ab4e2d8810a7b931af86`; text included in
  `non-cargo/llama.cpp-b9946-nlohmann-json-LICENSE.txt`.
- The amalgamated header `vendor/nlohmann/json.hpp`, SHA-256
  `aaf127c04cb31c406e5b04a63f1ae89369fccde6d8fa7cdda1ed4f32dfc5de63`, retains additional
  MIT notices that do not appear in `LICENSE-jsonhpp`: Hedley by Evan Nemerson, Grisu2 by Florian
  Loitsch, and the UTF-8 DFA by Björn/Bjoern Hoehrmann. Their exact notices and the MIT text are
  included respectively in `non-cargo/llama.cpp-b9946-nlohmann-hedley-MIT.txt` (SHA-256
  `152eed9e946af6706ff1c8c4bb4389bf7308f88912e02925a81e389f417f8456`),
  `non-cargo/llama.cpp-b9946-nlohmann-grisu2-MIT.txt` (SHA-256
  `c3a2d400b346f928e2bfcc95f4191a33ad76810708cb6e3f57cef8c483617d93`), and
  `non-cargo/llama.cpp-b9946-nlohmann-utf8-dfa-MIT.txt` (SHA-256
  `61517e0071eecedba4424636a5474ddda21e5bb721e749c8883a105e2b8b6dad`).
- `vendor/cpp-httplib/LICENSE`, MIT: raw SHA-256
  `4b45cbe16d7b71b89ae6127e26e0d90a029198ca5e958ad8e3d0b8bbed364d8b`; normalized text
  included in `non-cargo/llama.cpp-b9946-cpp-httplib-LICENSE.txt`, SHA-256
  `f8c53951438545b8ed61176d9071bd1039e81502f9ec9590b85ccd5c71a08473`.
- `vendor/miniaudio/miniaudio.h`, public domain or MIT No Attribution at the recipient's option:
  source SHA-256 `ac7af4de748b7e26b777f37e01cee313a308a7296a3eb080e2906b320cc55c89`; extracted legal
  text included in `non-cargo/llama.cpp-b9946-miniaudio-LICENSE.txt`, SHA-256
  `8ee059f719506d610d0e11e15a36d5c6fd9a55801931b80215f9d26ed019e0d1`.
- `vendor/stb/stb_image.h`, public domain or MIT at the recipient's option: source SHA-256
  `594c2fe35d49488b4382dbfaec8f98366defca819d916ac95becf3e75f4200b3`; extracted legal text
  included in `non-cargo/llama.cpp-b9946-stb-image-LICENSE.txt`, SHA-256
  `36df9677aa6a2ae37a01c7aaa39c3206fa02a4e06bb5037ebe89e5828b931f31`.
- `vendor/sheredom/subprocess.h`, public domain: source SHA-256
  `0bf208a408ba2c7e63739d62a0a492a13f90b0113214776835c855629ef90043`; extracted declaration
  included in `non-cargo/llama.cpp-b9946-sheredom-subprocess-LICENSE.txt`, SHA-256
  `0bc26379d10e8dc97d4bab5b007391e3ce25454f080fd0f2b12be4afe238e6df`.
- `common/base64.hpp`, Unlicense/public domain: source SHA-256
  `57f595aa0a206c4dec9a84b90a3416028a242da4dd8f219afc0859a6ccb7efe7`; declaration included
  in `non-cargo/llama.cpp-b9946-base64-UNLICENSE.txt`, SHA-256
  `88d9b4eb60579c191ec391ca04c16130572d7eedc4a86daa58bf28c6e14c9bcd`.
- `ggml/src/ggml-cpu/ops.cpp` incorporates the YaRN algorithm by Jeffrey Quesnelle and Bowen Peng,
  MIT: compiled-source SHA-256
  `701c57328cc54ec1979a1dcd120b46c36928e9c4d6d017c86d042f9725cf98f6`; text included in
  `non-cargo/llama.cpp-b9946-yarn-MIT.txt`, SHA-256
  `707b81ce28e1d0952791be53d4561b7a6ccbb9ec14abd4819b5dbedc3ceb1564`.
- `tools/mtmd/mtmd-image.cpp` adapts Pillow's `ImagingResample` algorithm, MIT-CMU:
  compiled-source SHA-256
  `84d130afea62061871e8daef3fe8188415d4bcea0bcf9278955083700f951a65`; attribution and text
  included in `non-cargo/llama.cpp-b9946-pillow-LICENSE.txt`, SHA-256
  `15181e7363dca9aed78b79bebebc7fde7f1814b8bd311ea3b87ae8ccadfc185b`.
- `ggml/src/ggml-cpu/vec.h`, AVX2 branch, adapts Arm Optimized Routines under the MIT option:
  source SHA-256
  `926330bae1c5d003bd654035426e31381fafcdca23ffcc23201d219dbb97cbeb`; text and Arm Copyright
  included in `non-cargo/llama.cpp-b9946-arm-optimized-routines-MIT.txt`, SHA-256
  `5129a8a7ed5b589626bf0327a1174cdc806994105ed7521925c21420fe17c485`.
- `ggml/src/ggml-impl.h` adapts Maratyszcza FP16, MIT: source SHA-256
  `2ed56e264202906d107e26d08eabb242d3107b026ebfb78096fa1e5f94bdbbb8`; copyrights for
  Facebook, Georgia Institute of Technology, and Google included in
  `non-cargo/llama.cpp-b9946-fp16-MIT.txt`, SHA-256
  `b2948afc330c07e5d780f0a2fb5c8c8738c5ba2869b68e4a0e98059fcaf81587`.
- `src/llama-vocab.cpp` adapts `cmp-nct/ggllm.cpp`, MIT: source SHA-256
  `3c649e905f838ee8f2ffd877bc1701e278f35948e17b0233c2bd350091c58670`; attribution included
  in `non-cargo/llama.cpp-b9946-ggllm-MIT.txt`, SHA-256
  `97bd5b8595175a711f3a44f523504eb5e931ee2baa9602197ce7c5c55c02ab85`.
- `tools/mtmd/mtmd-audio.cpp` declares code copied from `whisper.cpp` and references the OpenAI
  Whisper preprocessor: source SHA-256
  `22ae060fedb63689d3924a625b3b9a6a4488b89d692761a18bb67e380b0c0548`. Both MIT texts are
  included conservatively in `non-cargo/llama.cpp-b9946-whisper.cpp-MIT.txt`
  (`94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d`) and
  `non-cargo/llama.cpp-b9946-openai-whisper-MIT.txt`
  (`b5d65a59060e68c4ff940e1eddfa6f94b2d68fdf58ed7f4dd57721c997e35e9d`).

The original `tools/mtmd/mtmd-image.cpp` source also contained a routine adapted from
`yglukhov/bicubic-interpolation-image-processing` without a verifiable license. The Windows build
does not distribute it: after verifying the preceding input hash, it applies an exact patch that
removes the routine and delegates to the licensed Pillow path. The patched source hash is
`7c0cfa47bd61a9202824a9610cdc1168c2edd868e7c2e115f80e9eba70037f0f`; the build policy and
manifest authenticate both hashes.

## NSIS 3.11

- Artifact pinned by Tauri bundler 2.9.4:
  `https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.11/nsis-3.11.zip`
- Pinned ZIP SHA-256: `c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1`.
- Licenses declared by `nsis-3.11/COPYING`: zlib/libpng for the core, plug-ins, and documentation
  except where otherwise noted; bzip2 for the bzip2 module; Common Public License 1.0 for the LZMA
  module, with the special linking exception included by its authors.
- Included legal text: `non-cargo/NSIS-3.11-COPYING.txt`, SHA-256
  `dc0f74a312c08ffc900548a67ae9a3670ed28ad25a3afda1fe0504da16f89361`.

The installer uses the `Stubs/lzma_solid-x86-unicode` stub from the same ZIP, SHA-256
`a0d065b62d34be5f0aaaf7c162e101a5e25d7cd3eb10a13fdb37f91b02ebfce2`. Therefore the CPL-1.0
text and the LZMA exception are mandatory parts of the distributed notices.

## nsis-tauri-utils 0.5.3

- Pinned release:
  `https://github.com/tauri-apps/nsis-tauri-utils/releases/tag/nsis_tauri_utils-v0.5.3`.
- Tag/commit: `13d9edd27b69310e108d6fbd49f90992f8a05390`.
- DLL pinned by Tauri bundler 2.9.4: SHA-256
  `5ba143b5db4a87d32d6e7802e033330aae56cbceabe0d1e3ba41948385ad4709`.
- License declared by the workspace at that tag: `MIT OR Apache-2.0`.
- Legal texts included from that tag:
  - `non-cargo/nsis-tauri-utils-0.5.3-LICENSE_MIT.txt`, SHA-256
    `1c1020fa10a6bf318717e82c911bcc54ebdfb9bb280460ae332bcb2f82f57fbe`.
  - `non-cargo/nsis-tauri-utils-0.5.3-LICENSE_APACHE-2.0.txt`, SHA-256
    `0d542e0c8804e39aa7f37eb00da5a762149dc682d7829451287e11b938e94594`.

Tauri v2 no longer requires the unlicensed `NSIS-ApplicationID` compatibility plug-in. AirWiki
does not download, stage, invoke, or distribute it. Both the toolchain and template policy gates
reject any future reintroduction.

## 7-Zip 26.02 x64: non-distributed verification tool

Packaging and the final verifier use `7z.exe` solely to inspect the generated NSIS payload. The
tool is not included in the application or published installer and is not obtained from `PATH` or
a system installation.

- Pinned official MSI:
  `https://github.com/ip7z/7zip/releases/download/26.02/7z2602-x64.msi`.
- Size: `1999872` bytes; SHA-256:
  `db407a4f6d4999e5c7bc00ce8a882be94717b56e7fa68140fe3f12605d91643e`.
- The MSI is opened as an ephemeral administrative image through `msiexec /a`; it does not
  install 7-Zip or modify the Registry.
- Files selected and verified before use:
  - `7z.exe`: `576000` bytes; SHA-256
    `83967f1b02b43c4efeda302795722c809e0e81b8307de73558d10484d5676a7d`; x64 PE.
  - `7z.dll`: `1906688` bytes; SHA-256
    `69fd4df057985c40e510e2fac182881c7f85e90aa13ec703f763a8fdb2ce61f8`; x64 PE.
  - `License.txt`: `6031` bytes; raw SHA-256
    `519ac0a4bded9c18ea02e0afb71f663d8c47373bd9facd3ac96a79f51d77765d`.
- Included legal text, normalized to LF with one trailing newline:
  `non-cargo/7-Zip-26.02-License.txt`; SHA-256
  `32369594a3a9f7c643d124035120eaa6a7707e75e57c4386ef509f801447bc49`.

The upstream text declares GNU LGPL 2.1 or later for most of the code, an additional unRAR
restriction for RAR support, 2-clause and 3-clause BSD licenses for the files it identifies, and
some public-domain files. The pinned text is authoritative for the exact per-file assignment.
