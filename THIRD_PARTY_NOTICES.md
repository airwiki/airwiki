# Third-party notices

AirWiki source is licensed under Apache-2.0 and incorporates or downloads third-party components under their own terms. The project license does not replace those terms; distributed artifacts include this notice, the root `LICENSE`, and the generated legal inventory.

## Distributed or downloaded runtime assets

| Component | Version / revision | Pinned file size | License | Source |
| --- | --- | ---: | --- | --- |
| llama.cpp | `b9946` / `fb30ba9a6c5b4674174d06aed14794832ab33278` | macOS archive; Windows source archive: 36,865,897 bytes | MIT and vendored terms below | [ggml-org/llama.cpp](https://github.com/ggml-org/llama.cpp/tree/fb30ba9a6c5b4674174d06aed14794832ab33278) |
| Gemma 4 E2B Q4 | `69536a21d70340464240401ba38223d805f6a709` | 3,349,514,112 bytes | Apache-2.0 | [google/gemma-4-E2B-it-qat-q4_0-gguf](https://huggingface.co/google/gemma-4-E2B-it-qat-q4_0-gguf/tree/69536a21d70340464240401ba38223d805f6a709) |
| Gemma 4 E4B Q4 | `7edc6763a77bbca236126a361613b834c5ea0f7a` | 5,154,939,136 bytes | Apache-2.0 | [google/gemma-4-E4B-it-qat-q4_0-gguf](https://huggingface.co/google/gemma-4-E4B-it-qat-q4_0-gguf/tree/7edc6763a77bbca236126a361613b834c5ea0f7a) |
| Qwen3-1.7B-GGUF Q8_0, legacy fallback | `90862c4b9d2787eaed51d12237eafdfe7c5f6077` | 1,834,426,016 bytes | Apache-2.0 | [Qwen/Qwen3-1.7B-GGUF](https://huggingface.co/Qwen/Qwen3-1.7B-GGUF/tree/90862c4b9d2787eaed51d12237eafdfe7c5f6077) |
| multilingual-e5-small | `614241f622f53c4eeff9890bdc4f31cfecc418b3` | 487,352,505 bytes across five files | MIT | [intfloat/multilingual-e5-small](https://huggingface.co/intfloat/multilingual-e5-small/tree/614241f622f53c4eeff9890bdc4f31cfecc418b3) |
| mMARCO mMiniLMv2 L12 H384 v1 | `1427fd652930e4ba29e8149678df786c240d8825` | 135,704,242 bytes on macOS or 135,704,241 bytes on Windows, across five files | Apache-2.0 | [cross-encoder/mmarco-mMiniLMv2-L12-H384-v1](https://huggingface.co/cross-encoder/mmarco-mMiniLMv2-L12-H384-v1/tree/1427fd652930e4ba29e8149678df786c240d8825) |

On first use, the application downloads only the Gemma core selected for the detected hardware and any missing files from the pinned embedding and relevance snapshots. A previously installed and verified Qwen model is retained as a legacy fallback; it is not downloaded by a clean installation. The desktop enables the `fastembed-runtime` integration; CI uses deterministic providers and does not download model weights.

### llama.cpp Windows source build

The Windows llama.cpp runtime is built from the pinned source ZIP (SHA-256
`7a36a3e384ad29ce4ffbac0051f31b7265105d7d8c3240e5ab9a859e952ec3a2`) with OpenMP and shared
libraries disabled, the static MSVC runtime, AVX2 and reproducible-build flags. The package
contains only `llama-server.exe` plus its authenticated build manifest; it does not redistribute
the upstream Windows binary archive or an OpenMP DLL. In addition to llama.cpp's MIT license,
the linked server includes nlohmann/json and cpp-httplib under MIT, miniaudio under public domain
or MIT No Attribution terms, stb_image under public domain or MIT terms, and
`sheredom/subprocess.h` and `common/base64.hpp` under their public-domain dedications. The linked
amalgamated nlohmann/json header also preserves the MIT notices for Evan Nemerson's Hedley,
Florian Loitsch's Grisu2 implementation and Björn/Bjoern Hoehrmann's UTF-8 DFA. The linked
CPU backend incorporates YaRN under MIT (Copyright 2023 Jeffrey Quesnelle and Bowen Peng), and the
multimodal image code adapts Pillow's `ImagingResample` algorithm under the MIT-CMU license. The
AVX2 path adapts Arm Optimized Routines under MIT; FP16 conversion carries the MIT notices of
Facebook, Georgia Institute of Technology and Google; the tokenizer carries the MIT notice of
`cmp-nct/ggllm.cpp`; and the audio path carries the MIT notices of `whisper.cpp` and OpenAI
Whisper. A deterministic, hash-pinned Windows source patch removes the separate unlicensed
`yglukhov/libimage`-derived bicubic routine before compilation and delegates to the licensed
Pillow path. Exact
source and normalized legal-text
hashes are recorded in `licenses/NON_CARGO_COMPONENTS.md`; the corresponding texts are under
`licenses/non-cargo/`.

Runtime and package verification are independent release gates. A public candidate requires the
applicable legal review, reproducible build receipt, package verification, and platform signing
checks documented in `docs/release-checklist.md`. Source-tree notices alone do not close those
gates.

## Pinned multimodal assets not installed by the text-only application

The catalog records these projectors so a later multimodal pipeline can remain reproducible. They are not downloaded, distributed, loaded, or exposed as an install option by the current text-only application.

| Component | Revision | Pinned file size | SHA-256 | License |
| --- | --- | ---: | --- | --- |
| Gemma 4 E2B multimodal projector | `69536a21d70340464240401ba38223d805f6a709` | 986,833,312 bytes | `58c187648007cab392bd5678b87e862c3e8794017deb945feea2cf256195e96a` | Apache-2.0 |
| Gemma 4 E4B multimodal projector | `7edc6763a77bbca236126a361613b834c5ea0f7a` | 991,551,904 bytes | `c6398448d84a4836fdedf58f9775979e69ae0cc4dfdf4d697b5597693a555b12` | Apache-2.0 |

## Windows installer components

The Windows package is produced with the NSIS 3.09 toolchain pinned by
`cargo-packager 0.11.8` and uses its LZMA solid Unicode stub. NSIS core components are licensed
under zlib/libpng; the embedded LZMA module is under Common Public License 1.0 with the special
NSIS linking exception. The installer also embeds `nsis-tauri-utils 0.2.1`, licensed under
MIT OR Apache-2.0.

The complete artifact provenance, SHA-256 values and verified upstream texts are installed under
`licenses/NON_CARGO_COMPONENTS.md` and `licenses/non-cargo/`. In the source tree they live under
`resources/licenses/`.

`NSIS-ApplicationID 1.1` has no verifiable redistribution license in its upstream tag or release.
AirWiki does not invoke or embed that optional plug-in. The release gate fails closed if it
is referenced again without a verified license.

The package verifier uses the official 7-Zip 26.02 x64 MSI as a build-time inspection tool. The
MSI and its selected `7z.exe`/`7z.dll` files are pinned by SHA-256, staged through an
administrative image without installing 7-Zip, and are not distributed in the application or
installer. 7-Zip's upstream `License.txt` covers GNU LGPL 2.1 or later, the unRAR restriction,
BSD-2-Clause, BSD-3-Clause and public-domain files as assigned there. Its exact normalized text and
the raw MSI payload hash are recorded in `licenses/NON_CARGO_COMPONENTS.md` and
`licenses/non-cargo/7-Zip-26.02-License.txt`.

## Rust dependencies

The complete reproducible distributable inventory is packaged as
`licenses/THIRD_PARTY_LICENSES.md` (source path:
`resources/licenses/THIRD_PARTY_LICENSES.md`). It contains the exact transitive
closure of the application and MCP bridge for macOS arm64 and Windows x64,
SPDX expressions, upstream sources, and every `LICENSE`, `COPYING`, `NOTICE`,
or equivalent file found in the crates. Identical texts are stored once and
referenced by SHA-256.

Selected primary components:

| Component | Version in current lockfile | License expression | Source |
| --- | --- | --- | --- |
| eframe / egui | 0.35.0 | MIT OR Apache-2.0 | [emilk/egui](https://github.com/emilk/egui) |
| egui_commonmark | 0.24.0 | MIT OR Apache-2.0 | [lampsitter/egui_commonmark](https://github.com/lampsitter/egui_commonmark) |
| egui_graphs | 0.31.0 | MIT | [blitzarx1/egui_graphs](https://github.com/blitzarx1/egui_graphs) |
| pulldown-cmark | 0.13.4 | MIT | [pulldown-cmark/pulldown-cmark](https://github.com/pulldown-cmark/pulldown-cmark) |
| rust-libp2p | 0.56.0 | MIT | [libp2p/rust-libp2p](https://github.com/libp2p/rust-libp2p) |
| rmcp | 2.2.0 | Apache-2.0 | [modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk) |
| fastembed | 5.17.2 | Apache-2.0 | [Anush008/fastembed-rs](https://github.com/Anush008/fastembed-rs) |
| rusqlite | 0.37.0 | MIT | [rusqlite/rusqlite](https://github.com/rusqlite/rusqlite) |
| lopdf | 0.44.0 | MIT | [J-F-Liu/lopdf](https://github.com/J-F-Liu/lopdf) |
| cargo-packager | 0.11.8, build-time | MIT OR Apache-2.0 | [crabnebula-dev/cargo-packager](https://github.com/crabnebula-dev/cargo-packager) |

SQLite itself is bundled through `libsqlite3-sys` and is dedicated to the public domain by its authors; see [sqlite.org/copyright.html](https://www.sqlite.org/copyright.html).

`Cargo.lock` pins versions but is not the legal inventory. The generated
artifact uses `cargo metadata --locked` and the files present in each package
source. CI verifies that it is current and checks advisories, sources, and SPDX
expressions with `cargo-audit` and `cargo-deny`. An allowlist does not relicense
any component.

Installers include the generated inventory, copies of the common MIT and Apache
License 2.0 texts, pinned non-Cargo license texts, and this notice. When a
runtime is updated or restaged, its upstream legal files must remain with it.
