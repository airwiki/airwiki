# Inventory of Components Not Managed by Cargo

This inventory covers components outside Cargo that are involved in the Windows installer. The
artifacts are identified by SHA-256. The included legal texts were copied from the exact artifacts
or tags listed below, with line endings normalized to LF and a single trailing newline added. The
application, the MCP bridge, and their Rust dependencies are covered by
`THIRD_PARTY_LICENSES.md`.

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

## NSIS 3.09

- Artifact pinned by `cargo-packager 0.11.8`:
  `https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.9/nsis-3.09.zip`
- Origin declared by the mirror:
  `https://sourceforge.net/projects/nsis/files/NSIS%203/3.09/nsis-3.09.zip/download`
- Pinned ZIP SHA-256: `f5dc52eef1f3884230520199bac6f36b82d643d86b003ce51bd24b05c6ba7c91`.
- Licenses declared by `nsis-3.09/COPYING`: zlib/libpng for the core, plug-ins, and documentation
  except where otherwise noted; bzip2 for the bzip2 module; Common Public License 1.0 for the LZMA
  module, with the special linking exception included by its authors.
- Included legal text: `non-cargo/NSIS-3.09-COPYING.txt`, normalized SHA-256
  `1aab7a7da0a0d0f8a7857be09fe403ec807eb55c60c1264f1bbd17144482a222`.

The installer uses the `Stubs/lzma_solid-x86-unicode` stub from the same ZIP, SHA-256
`62677d44c9721779c2219571a5d3afdf4fcf4668b5dc475f5f5668d31d3e8ae9`. Therefore the CPL-1.0
text and the LZMA exception are mandatory parts of the distributed notices.

## nsis-tauri-utils 0.2.1

- Pinned release:
  `https://github.com/tauri-apps/nsis-tauri-utils/releases/tag/nsis_tauri_utils-v0.2.1`.
- Tag/commit: `c3a4447060a260c5e4e09d94284948c4f864da02`.
- DLL pinned by `cargo-packager 0.11.8`: SHA-256
  `0eed48313a7f904d7cc1977b70000ab3f11f18cadc8e6a69b807d288ca71f9db`.
- License declared by the workspace at that tag: `MIT OR Apache-2.0`.
- Legal texts included from that tag:
  - `non-cargo/nsis-tauri-utils-0.2.1-LICENSE_MIT.txt`, normalized SHA-256
    `20ae1ba81c7eddc620dfe2de650f6a453b4979f843c2482abfe8764264a24a49`.
  - `non-cargo/nsis-tauri-utils-0.2.1-LICENSE_APACHE-2.0.txt`, normalized SHA-256
    `809fa1ed21450f59827d1e9aec720bbc4b687434fa22283c6cb5dd82a47ab9c0`.

## NSIS-ApplicationID 1.1: excluded from the artifact

`cargo-packager 0.11.8` would normally download the mirror at
`https://github.com/tauri-apps/binary-releases/releases/download/nsis-plugins-v0/NSIS-ApplicationID.zip`.
The mirror release identifies its origin as
`https://github.com/connectiblutz/NSIS-ApplicationID/releases/tag/1.1`.

- Mirror ZIP SHA-256: `1c2772b0edfb0f96a7524734d6c8fac1fc011f26221faf88f3ed2c950f0c06c0`.
- Upstream tag/commit: `ad7e5084c69342d8f9fa7c66c6a135ca04e3c284`.
- DLL selected by the packager: `ReleaseUnicode/ApplicationID.dll`, SHA-256
  `f6851dcbf0a39edecd8a46564bc455e5273736c3dbcb02b954c201c79ccdf117`.

The tag, repository, release, and both ZIP files contain neither a license nor a verifiable
redistribution grant. A code copyright does not substitute for a license. Release preparation
therefore does not download that ZIP. It creates only the path required by the internal
`cargo-packager 0.11.8` check, as an empty, non-executable sentinel (SHA-256
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`). The managed template
does not invoke `ApplicationID::Set`; NSIS does not incorporate that plug-in into the installer.
Normal shortcut creation continues without an explicit AppUserModelID.

The legal gate fails closed with
`missing_verified_redistribution_license: nsis-applicationid-1.1` if the template references
`ApplicationID::` again. Reintroduction would require a verifiable public grant that covers the
distributed code and a legal text pinned by hash.

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
