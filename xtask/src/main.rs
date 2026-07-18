use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Instant,
};

use airwiki_core::{
    EvidenceDecision, EvidenceRelevanceProvider, FastEmbedMmarcoReranker, MMARCO_RERANKER_REVISION,
    PinnedMmarcoRerankerSnapshot, RELEVANCE_CANDIDATE_LIMIT, RelevanceInput,
};
use airwiki_inference::{AssetManager, InstallEvent, MMARCO_REVISION, platform_relevance_model};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use minisign_verify::{PublicKey as MinisignPublicKey, Signature as MinisignSignature};
use serde::{Deserialize, Serialize};
use serde_yaml::Value as YamlValue;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter, write::SimpleFileOptions};

mod docs;
mod retrieval;

const LICENSE_REPORT: &str = "resources/licenses/THIRD_PARTY_LICENSES.md";
const NON_CARGO_LICENSE_INVENTORY: &str = "resources/licenses/NON_CARGO_COMPONENTS.md";
const APPLICATION_ID_LICENSE_ERROR: &str =
    "missing_verified_redistribution_license: nsis-applicationid-1.1";
const VERIFIED_NON_CARGO_LEGAL_TEXTS: [(&str, &str); 21] = [
    (
        "resources/licenses/non-cargo/NSIS-3.09-COPYING.txt",
        "1aab7a7da0a0d0f8a7857be09fe403ec807eb55c60c1264f1bbd17144482a222",
    ),
    (
        "resources/licenses/non-cargo/nsis-tauri-utils-0.2.1-LICENSE_APACHE-2.0.txt",
        "809fa1ed21450f59827d1e9aec720bbc4b687434fa22283c6cb5dd82a47ab9c0",
    ),
    (
        "resources/licenses/non-cargo/nsis-tauri-utils-0.2.1-LICENSE_MIT.txt",
        "20ae1ba81c7eddc620dfe2de650f6a453b4979f843c2482abfe8764264a24a49",
    ),
    (
        "resources/licenses/non-cargo/7-Zip-26.02-License.txt",
        "32369594a3a9f7c643d124035120eaa6a7707e75e57c4386ef509f801447bc49",
    ),
    (
        "resources/licenses/non-cargo/llama.cpp-b9946-LICENSE.txt",
        "94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d",
    ),
    (
        "resources/licenses/non-cargo/llama.cpp-b9946-nlohmann-json-LICENSE.txt",
        "c0d068392ea65358b798b8c165103560f06e9e3b38c4ab4e2d8810a7b931af86",
    ),
    (
        "resources/licenses/non-cargo/llama.cpp-b9946-nlohmann-hedley-MIT.txt",
        "152eed9e946af6706ff1c8c4bb4389bf7308f88912e02925a81e389f417f8456",
    ),
    (
        "resources/licenses/non-cargo/llama.cpp-b9946-nlohmann-grisu2-MIT.txt",
        "c3a2d400b346f928e2bfcc95f4191a33ad76810708cb6e3f57cef8c483617d93",
    ),
    (
        "resources/licenses/non-cargo/llama.cpp-b9946-nlohmann-utf8-dfa-MIT.txt",
        "61517e0071eecedba4424636a5474ddda21e5bb721e749c8883a105e2b8b6dad",
    ),
    (
        "resources/licenses/non-cargo/llama.cpp-b9946-cpp-httplib-LICENSE.txt",
        "f8c53951438545b8ed61176d9071bd1039e81502f9ec9590b85ccd5c71a08473",
    ),
    (
        "resources/licenses/non-cargo/llama.cpp-b9946-miniaudio-LICENSE.txt",
        "8ee059f719506d610d0e11e15a36d5c6fd9a55801931b80215f9d26ed019e0d1",
    ),
    (
        "resources/licenses/non-cargo/llama.cpp-b9946-stb-image-LICENSE.txt",
        "36df9677aa6a2ae37a01c7aaa39c3206fa02a4e06bb5037ebe89e5828b931f31",
    ),
    (
        "resources/licenses/non-cargo/llama.cpp-b9946-sheredom-subprocess-LICENSE.txt",
        "0bc26379d10e8dc97d4bab5b007391e3ce25454f080fd0f2b12be4afe238e6df",
    ),
    (
        "resources/licenses/non-cargo/llama.cpp-b9946-base64-UNLICENSE.txt",
        "88d9b4eb60579c191ec391ca04c16130572d7eedc4a86daa58bf28c6e14c9bcd",
    ),
    (
        "resources/licenses/non-cargo/llama.cpp-b9946-yarn-MIT.txt",
        "707b81ce28e1d0952791be53d4561b7a6ccbb9ec14abd4819b5dbedc3ceb1564",
    ),
    (
        "resources/licenses/non-cargo/llama.cpp-b9946-pillow-LICENSE.txt",
        "15181e7363dca9aed78b79bebebc7fde7f1814b8bd311ea3b87ae8ccadfc185b",
    ),
    (
        "resources/licenses/non-cargo/llama.cpp-b9946-arm-optimized-routines-MIT.txt",
        "5129a8a7ed5b589626bf0327a1174cdc806994105ed7521925c21420fe17c485",
    ),
    (
        "resources/licenses/non-cargo/llama.cpp-b9946-fp16-MIT.txt",
        "b2948afc330c07e5d780f0a2fb5c8c8738c5ba2869b68e4a0e98059fcaf81587",
    ),
    (
        "resources/licenses/non-cargo/llama.cpp-b9946-ggllm-MIT.txt",
        "97bd5b8595175a711f3a44f523504eb5e931ee2baa9602197ce7c5c55c02ab85",
    ),
    (
        "resources/licenses/non-cargo/llama.cpp-b9946-whisper.cpp-MIT.txt",
        "94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d",
    ),
    (
        "resources/licenses/non-cargo/llama.cpp-b9946-openai-whisper-MIT.txt",
        "b5d65a59060e68c4ff940e1eddfa6f94b2d68fdf58ed7f4dd57721c997e35e9d",
    ),
];
const DISTRIBUTED_PACKAGES: [&str; 3] = [
    "airwiki-desktop",
    "airwiki-mcp-bridge",
    "airwiki-windows-firewall-helper",
];
const MAX_LEGAL_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_LEGAL_SEARCH_DEPTH: usize = 8;
const DISTRIBUTION_TARGETS: [&str; 2] = ["aarch64-apple-darwin", "x86_64-pc-windows-msvc"];
const MAX_MCPB_BINARY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MCPB_UNCOMPRESSED_BYTES: u64 = 128 * 1024 * 1024;
const MAX_MCPB_LEGAL_FILES: usize = 1_024;
const MCPB_NAME: &str = "airwiki";
const MCPB_TOOL: &str = "search_airwiki";
const MAX_UPDATER_KEY_OR_SIGNATURE_BYTES: u64 = 16 * 1024;
const UPDATER_PUBLIC_KEY_ENV: &str = "AIRWIKI_UPDATER_PUBLIC_KEY";

#[tokio::main]
async fn main() -> Result<()> {
    let mut arguments = std::env::args().skip(1);
    let command = arguments.next().unwrap_or_else(|| "help".to_owned());
    match command.as_str() {
        "fetch-runtime" => fetch_runtime().await,
        "relevance" => match arguments.next().as_deref() {
            Some("validate") => validate_relevance_fixture(),
            Some("evaluate") => {
                ensure!(
                    arguments.next().as_deref() == Some("--snapshot"),
                    "relevance evaluate expects `--snapshot <path>`"
                );
                let snapshot = arguments
                    .next()
                    .context("relevance evaluate is missing the snapshot path")?;
                ensure!(
                    arguments.next().is_none(),
                    "relevance evaluate received unexpected arguments"
                );
                evaluate_relevance(Path::new(&snapshot)).await
            }
            Some(other) => bail!("unknown relevance command: {other}"),
            None => bail!("missing relevance command; expected `validate` or `evaluate`"),
        },
        "retrieval" => match arguments.next().as_deref() {
            Some("corpus") => match arguments.next().as_deref() {
                Some("validate") => {
                    ensure!(
                        arguments.next().is_none(),
                        "retrieval corpus validate received unexpected arguments"
                    );
                    retrieval::validate_answerability_corpus()
                }
                Some("verify") => {
                    ensure!(
                        arguments.next().as_deref() == Some("--source-root"),
                        "retrieval corpus verify expects `--source-root <path>`"
                    );
                    let source_root = arguments
                        .next()
                        .context("retrieval corpus verify is missing the source root")?;
                    ensure!(
                        arguments.next().is_none(),
                        "retrieval corpus verify received unexpected arguments"
                    );
                    retrieval::verify_answerability_corpus(Path::new(&source_root))
                }
                Some(other) => bail!("unknown retrieval corpus command: {other}"),
                None => bail!("missing retrieval corpus command; expected `validate` or `verify`"),
            },
            Some("validate") => {
                ensure!(
                    arguments.next().is_none(),
                    "retrieval validate received unexpected arguments"
                );
                retrieval::validate().await
            }
            Some("evaluate") => {
                ensure!(
                    arguments.next().as_deref() == Some("--phase"),
                    "retrieval evaluate expects `--phase development|final` first"
                );
                let phase = arguments
                    .next()
                    .context("retrieval evaluate is missing the evaluation phase")?;
                let phase = retrieval::EvaluationPhase::parse(&phase)?;
                ensure!(
                    arguments.next().as_deref() == Some("--embedding-snapshot"),
                    "retrieval evaluate expects `--embedding-snapshot <path>` after the phase"
                );
                let embedding_snapshot = arguments
                    .next()
                    .context("retrieval evaluate is missing the embedding snapshot path")?;
                ensure!(
                    arguments.next().as_deref() == Some("--relevance-snapshot"),
                    "retrieval evaluate expects `--relevance-snapshot <path>` second"
                );
                let relevance_snapshot = arguments
                    .next()
                    .context("retrieval evaluate is missing the relevance snapshot path")?;
                ensure!(
                    arguments.next().is_none(),
                    "retrieval evaluate received unexpected arguments"
                );
                retrieval::evaluate(
                    phase,
                    Path::new(&embedding_snapshot),
                    Path::new(&relevance_snapshot),
                )
                .await
            }
            Some("evaluate-selector") => {
                ensure!(
                    arguments.next().as_deref() == Some("--phase"),
                    "retrieval evaluate-selector expects `--phase development` first"
                );
                let phase = arguments
                    .next()
                    .context("retrieval evaluate-selector is missing the evaluation phase")?;
                let phase = retrieval::EvaluationPhase::parse(&phase)?;
                ensure!(
                    arguments.next().as_deref() == Some("--data-root"),
                    "retrieval evaluate-selector expects `--data-root <path>` after the phase"
                );
                let data_root = arguments
                    .next()
                    .context("retrieval evaluate-selector is missing the data root")?;
                ensure!(
                    arguments.next().as_deref() == Some("--llama-server"),
                    "retrieval evaluate-selector expects `--llama-server <path>` after the data root"
                );
                let llama_server = arguments
                    .next()
                    .context("retrieval evaluate-selector is missing llama-server")?;
                ensure!(
                    arguments.next().as_deref() == Some("--model-id"),
                    "retrieval evaluate-selector expects `--model-id <catalog-id>` last"
                );
                let model_id = arguments
                    .next()
                    .context("retrieval evaluate-selector is missing the model ID")?;
                ensure!(
                    arguments.next().is_none(),
                    "retrieval evaluate-selector received unexpected arguments"
                );
                retrieval::evaluate_selector(
                    phase,
                    Path::new(&data_root),
                    Path::new(&llama_server),
                    &model_id,
                )
                .await
            }
            Some("evaluate-answerability") => {
                ensure!(
                    arguments.next().as_deref() == Some("--source-root"),
                    "retrieval evaluate-answerability expects `--source-root <directory>` first"
                );
                let source_root = arguments.next().context(
                    "retrieval evaluate-answerability is missing the corpus source root",
                )?;
                ensure!(
                    arguments.next().as_deref() == Some("--data-root"),
                    "retrieval evaluate-answerability expects `--data-root <directory>` after the source root"
                );
                let data_root = arguments
                    .next()
                    .context("retrieval evaluate-answerability is missing the data root")?;
                ensure!(
                    arguments.next().as_deref() == Some("--llama-server"),
                    "retrieval evaluate-answerability expects `--llama-server <path>` after the data root"
                );
                let llama_server = arguments
                    .next()
                    .context("retrieval evaluate-answerability is missing llama-server")?;
                ensure!(
                    arguments.next().as_deref() == Some("--model-id"),
                    "retrieval evaluate-answerability expects `--model-id <catalog-id>` last"
                );
                let model_id = arguments
                    .next()
                    .context("retrieval evaluate-answerability is missing the model ID")?;
                ensure!(
                    arguments.next().is_none(),
                    "retrieval evaluate-answerability received unexpected arguments"
                );
                retrieval::evaluate_answerability(
                    Path::new(&source_root),
                    Path::new(&data_root),
                    Path::new(&llama_server),
                    &model_id,
                )
                .await
            }
            Some("evaluate-reviewed-anchors") => {
                ensure!(
                    arguments.next().as_deref() == Some("--data-root"),
                    "retrieval evaluate-reviewed-anchors expects `--data-root <directory>` first"
                );
                let data_root = arguments
                    .next()
                    .context("retrieval evaluate-reviewed-anchors is missing the data root")?;
                ensure!(
                    arguments.next().as_deref() == Some("--llama-server"),
                    "retrieval evaluate-reviewed-anchors expects `--llama-server <path>` after the data root"
                );
                let llama_server = arguments
                    .next()
                    .context("retrieval evaluate-reviewed-anchors is missing llama-server")?;
                ensure!(
                    arguments.next().as_deref() == Some("--model-id"),
                    "retrieval evaluate-reviewed-anchors expects `--model-id <catalog-id>` last"
                );
                let model_id = arguments
                    .next()
                    .context("retrieval evaluate-reviewed-anchors is missing the model ID")?;
                ensure!(
                    arguments.next().is_none(),
                    "retrieval evaluate-reviewed-anchors received unexpected arguments"
                );
                retrieval::evaluate_reviewed_anchors(
                    Path::new(&data_root),
                    Path::new(&llama_server),
                    &model_id,
                )
                .await
            }
            Some("evaluate-mini-graph") => {
                ensure!(
                    arguments.next().is_none(),
                    "retrieval evaluate-mini-graph received unexpected arguments"
                );
                retrieval::evaluate_mini_graph()
            }
            Some("evaluate-real-mini-graph") => {
                ensure!(
                    arguments.next().as_deref() == Some("--embedding-snapshot"),
                    "retrieval evaluate-real-mini-graph expects `--embedding-snapshot <directory>`"
                );
                let embedding_snapshot = arguments.next().context(
                    "retrieval evaluate-real-mini-graph is missing the embedding snapshot path",
                )?;
                ensure!(
                    arguments.next().is_none(),
                    "retrieval evaluate-real-mini-graph received unexpected arguments"
                );
                retrieval::evaluate_real_mini_graph(Path::new(&embedding_snapshot)).await
            }
            Some(other) => bail!("unknown retrieval command: {other}"),
            None => bail!(
                "missing retrieval command; expected `corpus`, `validate`, `evaluate`, `evaluate-selector`, `evaluate-answerability`, `evaluate-reviewed-anchors`, `evaluate-mini-graph` or `evaluate-real-mini-graph`"
            ),
        },
        "licenses" => match arguments.next().as_deref() {
            Some("generate") => generate_licenses(false),
            Some("check") => generate_licenses(true),
            Some(other) => bail!("unknown licenses command: {other}"),
            None => bail!("missing licenses command; expected `generate` or `check`"),
        },
        "docs" => match arguments.next().as_deref() {
            Some("check") => {
                ensure!(
                    arguments.next().is_none(),
                    "docs check received unexpected arguments"
                );
                let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .context("xtask manifest directory has no repository parent")?;
                docs::check(repository_root)
            }
            Some(other) => bail!("unknown docs command: {other}"),
            None => bail!("missing docs command; expected `check`"),
        },
        "mcpb" => match arguments.next().as_deref() {
            Some("build") => {
                let request = parse_mcpb_request(arguments.collect())?;
                build_mcpb(&request)
            }
            Some("verify") => {
                let request = parse_mcpb_request(arguments.collect())?;
                verify_mcpb(&request.output, request.target, &request.bridge)
            }
            Some(other) => bail!("unknown mcpb command: {other}"),
            None => bail!("missing mcpb command; expected `build` or `verify`"),
        },
        "packaging" => match arguments.next().as_deref() {
            Some("verify-windows-uninstaller") => verify_windows_uninstaller(),
            Some("verify-updater-signature") => {
                let request = parse_updater_signature_request(arguments.collect())?;
                let public_key = std::env::var(UPDATER_PUBLIC_KEY_ENV)
                    .with_context(|| format!("{UPDATER_PUBLIC_KEY_ENV} is required"))?;
                verify_updater_signature(&request, &public_key)
            }
            Some(other) => bail!("unknown packaging command: {other}"),
            None => bail!(
                "missing packaging command; expected `verify-windows-uninstaller` or `verify-updater-signature`"
            ),
        },
        "help" | "--help" | "-h" => {
            println!("cargo run -p xtask -- fetch-runtime");
            println!("cargo run --locked -p xtask -- relevance validate");
            println!("cargo run --locked -p xtask -- relevance evaluate --snapshot <directory>");
            println!("cargo run --locked -p xtask -- retrieval validate");
            println!("cargo run --locked -p xtask -- retrieval corpus validate");
            println!(
                "cargo run --locked -p xtask -- retrieval corpus verify --source-root <directory>"
            );
            println!(
                "cargo run --locked -p xtask -- retrieval evaluate --phase development|final --embedding-snapshot <directory> --relevance-snapshot <directory>"
            );
            println!(
                "cargo run --locked -p xtask -- retrieval evaluate-selector --phase development --data-root <directory> --llama-server <path> --model-id <catalog-id>"
            );
            println!(
                "cargo run --locked -p xtask -- retrieval evaluate-answerability --source-root <directory> --data-root <directory> --llama-server <path> --model-id <catalog-id>"
            );
            println!(
                "cargo run --locked -p xtask -- retrieval evaluate-reviewed-anchors --data-root <directory> --llama-server <path> --model-id <catalog-id>"
            );
            println!("cargo run --release --locked -p xtask -- retrieval evaluate-mini-graph");
            println!(
                "cargo run --release --locked -p xtask -- retrieval evaluate-real-mini-graph --embedding-snapshot <directory>"
            );
            println!("cargo run --locked -p xtask -- licenses generate");
            println!("cargo run --locked -p xtask -- licenses check");
            println!("cargo run --locked -p xtask -- docs check");
            println!("cargo run --locked -p xtask -- packaging verify-windows-uninstaller");
            println!(
                "cargo run --locked -p xtask -- packaging verify-updater-signature --artifact <path> --signature <path>"
            );
            println!(
                "cargo run --locked -p xtask -- mcpb build --target <triple> --bridge <path> --output <path>"
            );
            println!(
                "cargo run --locked -p xtask -- mcpb verify --target <triple> --bridge <path> --output <path>"
            );
            Ok(())
        }
        unknown => bail!("unknown xtask command: {unknown}"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum McpbTarget {
    MacosArm64,
    WindowsX64,
}

impl McpbTarget {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "aarch64-apple-darwin" => Ok(Self::MacosArm64),
            "x86_64-pc-windows-msvc" => Ok(Self::WindowsX64),
            _ => bail!("unsupported MCPB target `{value}`"),
        }
    }

    const fn triple(self) -> &'static str {
        match self {
            Self::MacosArm64 => "aarch64-apple-darwin",
            Self::WindowsX64 => "x86_64-pc-windows-msvc",
        }
    }

    const fn platform(self) -> &'static str {
        match self {
            Self::MacosArm64 => "darwin",
            Self::WindowsX64 => "win32",
        }
    }

    const fn binary_entry(self) -> &'static str {
        match self {
            Self::MacosArm64 => "server/airwiki-mcp-bridge",
            Self::WindowsX64 => "server/airwiki-mcp-bridge.exe",
        }
    }
}

fn mcpb_bundle_command(target: McpbTarget) -> String {
    format!("${{__dirname}}/{}", target.binary_entry())
}

#[derive(Debug)]
struct McpbRequest {
    target: McpbTarget,
    bridge: PathBuf,
    output: PathBuf,
}

fn parse_mcpb_request(arguments: Vec<String>) -> Result<McpbRequest> {
    ensure!(
        arguments.len().is_multiple_of(2),
        "MCPB arguments must be `--target`, `--bridge`, and `--output` pairs"
    );
    let mut values = BTreeMap::new();
    for pair in arguments.chunks_exact(2) {
        let key = pair[0].as_str();
        ensure!(
            matches!(key, "--target" | "--bridge" | "--output"),
            "unknown MCPB argument `{key}`"
        );
        ensure!(
            values.insert(key.to_owned(), pair[1].clone()).is_none(),
            "duplicate MCPB argument `{key}`"
        );
    }
    let target = values
        .remove("--target")
        .context("missing MCPB `--target`")?;
    let output = values
        .remove("--output")
        .context("missing MCPB `--output`")?;
    let bridge = values
        .remove("--bridge")
        .map(PathBuf::from)
        .context("missing MCPB `--bridge`")?;
    Ok(McpbRequest {
        target: McpbTarget::parse(&target)?,
        bridge,
        output: PathBuf::from(output),
    })
}

fn build_mcpb(request: &McpbRequest) -> Result<()> {
    ensure!(
        request.output.extension().and_then(|value| value.to_str()) == Some("mcpb"),
        "MCPB output must use the `.mcpb` extension"
    );
    let bridge_bytes = read_regular_file(&request.bridge, MAX_MCPB_BINARY_BYTES)?;
    validate_bridge_architecture(&bridge_bytes, request.target)?;
    let entries = expected_mcpb_entries(request.target, bridge_bytes)?;

    if let Some(parent) = request.output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating MCPB output directory {}", parent.display()))?;
    }
    let filename = request
        .output
        .file_name()
        .and_then(|value| value.to_str())
        .context("MCPB output has no valid filename")?;
    let temporary = request.output.with_file_name(format!(".{filename}.part"));
    if temporary.exists() {
        fs::remove_file(&temporary)
            .with_context(|| format!("removing stale {}", temporary.display()))?;
    }

    let file =
        File::create(&temporary).with_context(|| format!("creating {}", temporary.display()))?;
    let mut archive = ZipWriter::new(file);
    for (name, (content, mode)) in entries {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(9))
            .last_modified_time(DateTime::default())
            .unix_permissions(mode);
        archive
            .start_file(&name, options)
            .with_context(|| format!("adding `{name}` to MCPB"))?;
        archive
            .write_all(&content)
            .with_context(|| format!("writing `{name}` to MCPB"))?;
    }
    let file = archive.finish().context("finishing MCPB archive")?;
    file.sync_all().context("syncing MCPB archive")?;
    replace_file(&temporary, &request.output)?;
    verify_mcpb(&request.output, request.target, &request.bridge)?;

    let digest = hex::encode(Sha256::digest(
        fs::read(&request.output)
            .with_context(|| format!("hashing {}", request.output.display()))?,
    ));
    println!(
        "built {} MCPB at {} (SHA-256 {digest})",
        request.target.triple(),
        request.output.display()
    );
    Ok(())
}

fn verify_mcpb(path: &Path, target: McpbTarget, expected_bridge: &Path) -> Result<()> {
    let expected_bridge = read_regular_file(expected_bridge, MAX_MCPB_BINARY_BYTES)?;
    validate_bridge_architecture(&expected_bridge, target)?;
    let expected_entries = expected_mcpb_entries(target, expected_bridge)?;
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("reading MCPB metadata for {}", path.display()))?;
    ensure!(
        metadata.file_type().is_file(),
        "MCPB must be a regular file"
    );
    ensure!(
        metadata.len() <= MAX_MCPB_UNCOMPRESSED_BYTES,
        "MCPB exceeds the archive size limit"
    );
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut archive = ZipArchive::new(file).context("opening MCPB zip archive")?;
    let expected = expected_entries.keys().cloned().collect::<BTreeSet<_>>();
    let mut entries = BTreeMap::<String, Vec<u8>>::new();
    let mut total_size = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .with_context(|| format!("reading MCPB entry {index}"))?;
        let name = entry.name().to_owned();
        let enclosed = entry
            .enclosed_name()
            .with_context(|| format!("MCPB entry `{name}` escapes the bundle"))?;
        ensure!(
            enclosed.to_string_lossy().replace('\\', "/") == name,
            "MCPB entry `{name}` is not normalized"
        );
        ensure!(!entry.is_dir(), "MCPB entry `{name}` must be a file");
        ensure!(
            !entry.is_symlink(),
            "MCPB entry `{name}` must not be a symlink"
        );
        let (_, expected_mode) = expected_entries
            .get(&name)
            .with_context(|| format!("unexpected MCPB entry `{name}`"))?;
        ensure!(
            entry.unix_mode().map(|mode| mode & 0o777) == Some(*expected_mode),
            "MCPB entry `{name}` has unexpected permissions"
        );
        ensure!(
            !entries.contains_key(&name),
            "duplicate MCPB entry `{name}`"
        );
        total_size = total_size
            .checked_add(entry.size())
            .context("MCPB uncompressed size overflow")?;
        ensure!(
            total_size <= MAX_MCPB_UNCOMPRESSED_BYTES,
            "MCPB uncompressed payload exceeds the limit"
        );
        let mut content = Vec::with_capacity(
            usize::try_from(entry.size()).context("MCPB entry is too large for this platform")?,
        );
        entry
            .read_to_end(&mut content)
            .with_context(|| format!("reading MCPB entry `{name}`"))?;
        entries.insert(name, content);
    }
    ensure!(
        entries.keys().cloned().collect::<BTreeSet<_>>() == expected,
        "MCPB is missing a required entry"
    );
    let manifest = entries
        .get("manifest.json")
        .context("MCPB manifest is missing")?;
    validate_mcpb_manifest(manifest, target)?;
    let bridge = entries
        .get(target.binary_entry())
        .context("MCPB bridge is missing")?;
    validate_bridge_architecture(bridge, target)?;
    for (name, (expected_content, _)) in expected_entries {
        ensure!(
            entries.get(&name) == Some(&expected_content),
            "MCPB entry `{name}` differs from the verified workspace input"
        );
    }
    Ok(())
}

fn expected_mcpb_entries(
    target: McpbTarget,
    bridge_bytes: Vec<u8>,
) -> Result<BTreeMap<String, (Vec<u8>, u32)>> {
    let root = workspace_root();
    validate_non_cargo_legal_inventory(&root)?;

    let mut entries = BTreeMap::new();
    entries.insert(
        "LICENSE".to_owned(),
        (
            read_regular_file(&root.join("LICENSE"), MAX_LEGAL_FILE_BYTES)?,
            0o644,
        ),
    );
    entries.insert(
        "THIRD_PARTY_NOTICES.md".to_owned(),
        (
            read_regular_file(&root.join("THIRD_PARTY_NOTICES.md"), MAX_LEGAL_FILE_BYTES)?,
            0o644,
        ),
    );
    entries.insert("manifest.json".to_owned(), (mcpb_manifest(target)?, 0o644));
    entries.insert(target.binary_entry().to_owned(), (bridge_bytes, 0o755));
    collect_mcpb_legal_entries(
        &root.join("resources/licenses"),
        Path::new(""),
        0,
        &mut entries,
    )?;
    Ok(entries)
}

fn collect_mcpb_legal_entries(
    directory: &Path,
    relative_directory: &Path,
    depth: usize,
    entries: &mut BTreeMap<String, (Vec<u8>, u32)>,
) -> Result<()> {
    let metadata = fs::symlink_metadata(directory)
        .with_context(|| format!("reading MCPB legal directory {}", directory.display()))?;
    ensure!(
        metadata.file_type().is_dir(),
        "MCPB legal source {} must be a directory",
        directory.display()
    );
    let mut children = fs::read_dir(directory)
        .with_context(|| format!("reading MCPB legal directory {}", directory.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    children.sort_by_key(std::fs::DirEntry::file_name);

    for child in children {
        let name = child
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("MCPB legal source contains a non-UTF-8 filename"))?;
        ensure!(
            !name.starts_with('.') && !name.contains(['/', '\\']),
            "MCPB legal source contains an unsafe filename `{name}`"
        );
        let file_type = child
            .file_type()
            .with_context(|| format!("reading type of {}", child.path().display()))?;
        ensure!(
            !file_type.is_symlink(),
            "MCPB legal source {} must not be a symlink",
            child.path().display()
        );
        let relative = relative_directory.join(&name);
        if file_type.is_dir() {
            ensure!(
                depth < MAX_LEGAL_SEARCH_DEPTH,
                "MCPB legal source exceeds the directory depth limit"
            );
            collect_mcpb_legal_entries(&child.path(), &relative, depth + 1, entries)?;
            continue;
        }
        ensure!(
            file_type.is_file(),
            "MCPB legal source {} is not a regular file",
            child.path().display()
        );
        ensure!(
            entries.len() < MAX_MCPB_LEGAL_FILES,
            "MCPB contains too many legal files"
        );
        let relative = relative
            .to_str()
            .context("MCPB legal source contains a non-UTF-8 path")?
            .replace('\\', "/");
        let entry_name = format!("licenses/{relative}");
        ensure!(
            entries
                .insert(
                    entry_name.clone(),
                    (
                        read_regular_file(&child.path(), MAX_LEGAL_FILE_BYTES)?,
                        0o644
                    ),
                )
                .is_none(),
            "duplicate MCPB legal entry `{entry_name}`"
        );
    }
    Ok(())
}

fn mcpb_manifest(target: McpbTarget) -> Result<Vec<u8>> {
    let command = mcpb_bundle_command(target);
    let manifest = serde_json::json!({
        "manifest_version": "0.3",
        "name": MCPB_NAME,
        "display_name": "AirWiki",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Search human-reviewed knowledge explicitly approved for external chat.",
        "long_description": "Connects Claude Desktop to the local AirWiki application. AirWiki must remain open; the extension is read-only and does not grant access to collections.",
        "author": { "name": "AirWiki contributors" },
        "license": "Apache-2.0",
        "server": {
            "type": "binary",
            "entry_point": target.binary_entry(),
            "mcp_config": {
                "command": command,
                "args": ["--client", "claude-desktop"],
                "env": {}
            }
        },
        "tools": [{
            "name": MCPB_TOOL,
            "description": "Search read-only evidence approved for external chat."
        }],
        "tools_generated": false,
        "prompts_generated": false,
        "compatibility": {
            "claude_desktop": ">=1.0.0",
            "platforms": [target.platform()]
        }
    });
    let mut bytes = serde_json::to_vec_pretty(&manifest).context("encoding MCPB manifest")?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn validate_mcpb_manifest(bytes: &[u8], target: McpbTarget) -> Result<()> {
    let manifest: serde_json::Value =
        serde_json::from_slice(bytes).context("decoding MCPB manifest")?;
    ensure!(
        manifest
            .get("manifest_version")
            .and_then(serde_json::Value::as_str)
            == Some("0.3"),
        "MCPB manifest version must be 0.3"
    );
    ensure!(
        manifest.get("name").and_then(serde_json::Value::as_str) == Some(MCPB_NAME),
        "MCPB manifest has an unexpected name"
    );
    let server = manifest
        .get("server")
        .and_then(serde_json::Value::as_object)
        .context("MCPB server configuration is missing")?;
    ensure!(
        object_has_exact_keys(server, &["entry_point", "mcp_config", "type"]),
        "MCPB server contains unsupported fields"
    );
    ensure!(
        server.get("type").and_then(serde_json::Value::as_str) == Some("binary"),
        "MCPB server must be a binary"
    );
    ensure!(
        server
            .get("entry_point")
            .and_then(serde_json::Value::as_str)
            == Some(target.binary_entry()),
        "MCPB entry point does not match the target"
    );
    let config = server
        .get("mcp_config")
        .and_then(serde_json::Value::as_object)
        .context("MCPB mcp_config is missing")?;
    ensure!(
        object_has_exact_keys(config, &["args", "command", "env"]),
        "MCPB mcp_config contains unsupported execution fields"
    );
    ensure!(
        config.get("command").and_then(serde_json::Value::as_str)
            == Some(mcpb_bundle_command(target).as_str()),
        "MCPB command must resolve the packaged bridge from the bundle root"
    );
    ensure!(
        config.get("args") == Some(&serde_json::json!(["--client", "claude-desktop"])),
        "MCPB command arguments are not the fixed Claude client tag"
    );
    ensure!(
        config.get("env") == Some(&serde_json::json!({})),
        "MCPB command environment must be empty"
    );
    ensure!(
        manifest
            .get("compatibility")
            .and_then(|value| value.get("platforms"))
            == Some(&serde_json::json!([target.platform()])),
        "MCPB platform does not match the target"
    );
    let tools = manifest
        .get("tools")
        .and_then(serde_json::Value::as_array)
        .context("MCPB tools are missing")?;
    ensure!(tools.len() == 1, "MCPB must declare exactly one tool");
    ensure!(
        tools[0].get("name").and_then(serde_json::Value::as_str) == Some(MCPB_TOOL),
        "MCPB declares an unexpected tool"
    );
    ensure!(
        bytes == mcpb_manifest(target)?,
        "MCPB manifest differs from the deterministic project manifest"
    );
    Ok(())
}

fn object_has_exact_keys(
    object: &serde_json::Map<String, serde_json::Value>,
    expected: &[&str],
) -> bool {
    object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
}

fn read_regular_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("reading metadata for {}", path.display()))?;
    ensure!(
        metadata.file_type().is_file(),
        "{} must be a regular file",
        path.display()
    );
    ensure!(
        metadata.len() <= max_bytes,
        "{} exceeds the {} byte limit",
        path.display(),
        max_bytes
    );
    fs::read(path).with_context(|| format!("reading {}", path.display()))
}

fn validate_bridge_architecture(bytes: &[u8], target: McpbTarget) -> Result<()> {
    match target {
        McpbTarget::MacosArm64 => {
            ensure!(bytes.len() >= 8, "bridge Mach-O header is truncated");
            ensure!(
                bytes[..4] == [0xcf, 0xfa, 0xed, 0xfe],
                "bridge is not a 64-bit little-endian Mach-O"
            );
            ensure!(
                bytes[4..8] == [0x0c, 0x00, 0x00, 0x01],
                "bridge Mach-O is not arm64"
            );
        }
        McpbTarget::WindowsX64 => {
            ensure!(bytes.len() >= 0x40, "bridge PE header is truncated");
            ensure!(&bytes[..2] == b"MZ", "bridge is not a PE executable");
            let offset_bytes: [u8; 4] = bytes[0x3c..0x40]
                .try_into()
                .context("bridge PE offset is truncated")?;
            let pe_offset = usize::try_from(u32::from_le_bytes(offset_bytes))
                .context("bridge PE offset is invalid")?;
            let header_end = pe_offset
                .checked_add(6)
                .context("bridge PE header offset overflow")?;
            ensure!(bytes.len() >= header_end, "bridge PE header is truncated");
            ensure!(
                &bytes[pe_offset..pe_offset + 4] == b"PE\0\0",
                "bridge has an invalid PE signature"
            );
            ensure!(
                bytes[pe_offset + 4..pe_offset + 6] == [0x64, 0x86],
                "bridge PE is not x86_64"
            );
        }
    }
    Ok(())
}

const RELEVANCE_FIXTURE: &str = "fixtures/relevance/answerability.json";
const RELEVANCE_REPORT_DIRECTORY: &str = "target/evals";
const RELEVANCE_EXPANDED_BATCH_SIZES: [usize; 4] = [8, 10, 40, 80];
const MIN_CALIBRATION_DOMAINS: usize = 2;
const MIN_HOLDOUT_DOMAINS: usize = 7;
const EXPECTED_RELEVANCE_CASE_IDS: [&str; 17] = [
    "calibration_atlas_budget_absent",
    "calibration_atlas_recovery",
    "calibration_atlas_conflict",
    "calibration_atlas_owner_cross_language",
    "calibration_atlas_date_cross_language",
    "calibration_injection_requested",
    "calibration_injection_authority",
    "holdout_leave_days_cross_language",
    "holdout_expense_approver_cross_language",
    "holdout_event_date",
    "holdout_device_restart",
    "holdout_nebula_budget_absent",
    "holdout_sensor_conflict",
    "holdout_quasar_code_absent",
    "holdout_rto_definition_cross_language",
    "holdout_policy_exception",
    "holdout_unrelated_injection",
];
const REQUIRED_RELEVANCE_TAGS: [RelevanceTag; 4] = [
    RelevanceTag::Absent,
    RelevanceTag::Conflict,
    RelevanceTag::Injection,
    RelevanceTag::CrossLanguage,
];

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RelevanceFixture {
    schema_version: u32,
    cases: Vec<RelevanceCase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RelevanceCase {
    id: String,
    split: RelevanceSplit,
    domain: String,
    tags: Vec<RelevanceTag>,
    question: String,
    candidates: Vec<RelevanceCandidate>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum RelevanceSplit {
    Calibration,
    Holdout,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum RelevanceTag {
    Direct,
    Absent,
    Conflict,
    Injection,
    CrossLanguage,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RelevanceCandidate {
    id: String,
    relevant: bool,
    title: String,
    heading: String,
    text: String,
}

impl RelevanceCandidate {
    fn as_input(&self) -> RelevanceInput {
        RelevanceInput {
            title: self.title.clone(),
            heading: self.heading.clone(),
            text: self.text.clone(),
        }
    }
}

#[derive(Debug)]
struct LoadedRelevanceFixture {
    fixture: RelevanceFixture,
    sha256: String,
}

#[derive(Debug, Default, Clone, Copy, Serialize)]
struct ClassificationCounts {
    true_positive: u32,
    false_positive: u32,
    true_negative: u32,
    false_negative: u32,
}

impl ClassificationCounts {
    fn record(&mut self, expected: bool, actual: bool) {
        match (expected, actual) {
            (true, true) => self.true_positive = self.true_positive.saturating_add(1),
            (false, true) => self.false_positive = self.false_positive.saturating_add(1),
            (false, false) => self.true_negative = self.true_negative.saturating_add(1),
            (true, false) => self.false_negative = self.false_negative.saturating_add(1),
        }
    }

    fn recall(self) -> Option<f64> {
        let denominator = self.true_positive.saturating_add(self.false_negative);
        (denominator > 0).then(|| f64::from(self.true_positive) / f64::from(denominator))
    }
}

#[derive(Debug, Serialize)]
struct RelevanceCaseReport {
    id: String,
    split: RelevanceSplit,
    domain: String,
    tags: Vec<RelevanceTag>,
    counts: ClassificationCounts,
}

#[derive(Debug, Serialize)]
struct RelevanceExpandedBatchReport {
    size: usize,
    cases_checked: usize,
    failed_case_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct RelevanceStabilityReport {
    permutation_cases_checked: usize,
    permutation_failed_case_ids: Vec<String>,
    expanded_batches: Vec<RelevanceExpandedBatchReport>,
}

impl RelevanceStabilityReport {
    fn new() -> Self {
        Self {
            permutation_cases_checked: 0,
            permutation_failed_case_ids: Vec::new(),
            expanded_batches: RELEVANCE_EXPANDED_BATCH_SIZES
                .into_iter()
                .map(|size| RelevanceExpandedBatchReport {
                    size,
                    cases_checked: 0,
                    failed_case_ids: Vec::new(),
                })
                .collect(),
        }
    }

    fn passed(&self) -> bool {
        self.permutation_failed_case_ids.is_empty()
            && self
                .expanded_batches
                .iter()
                .all(|batch| batch.failed_case_ids.is_empty())
    }
}

#[derive(Debug, Serialize)]
struct RelevanceEvaluationReport {
    schema_version: u32,
    candidate_limit: usize,
    profile_id: String,
    policy_id: String,
    fixture_sha256: String,
    target_os: String,
    target_arch: String,
    artifact_filename: String,
    artifact_sha256: String,
    artifact_revision: String,
    thread_count: usize,
    elapsed_ms: u128,
    calibration: ClassificationCounts,
    holdout: ClassificationCounts,
    stability: RelevanceStabilityReport,
    passed: bool,
    cases: Vec<RelevanceCaseReport>,
}

fn load_relevance_fixture() -> Result<LoadedRelevanceFixture> {
    let path = workspace_root().join(RELEVANCE_FIXTURE);
    let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let fixture: RelevanceFixture =
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    validate_relevance_fixture_data(&fixture)?;
    Ok(LoadedRelevanceFixture {
        fixture,
        sha256: hex::encode(Sha256::digest(&bytes)),
    })
}

fn validate_relevance_fixture() -> Result<()> {
    let loaded = load_relevance_fixture()?;
    println!(
        "validated {} relevance cases (SHA-256 {})",
        loaded.fixture.cases.len(),
        loaded.sha256
    );
    Ok(())
}

fn validate_relevance_fixture_data(fixture: &RelevanceFixture) -> Result<()> {
    ensure!(
        fixture.schema_version == 2,
        "unsupported relevance fixture schema"
    );
    let expected_ids = EXPECTED_RELEVANCE_CASE_IDS
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut case_ids = BTreeSet::new();
    let mut splits = BTreeSet::new();
    let mut domains = BTreeMap::<RelevanceSplit, BTreeSet<&str>>::new();
    let mut tags_by_split = BTreeMap::<RelevanceSplit, BTreeSet<RelevanceTag>>::new();
    for case in &fixture.cases {
        ensure!(!case.id.trim().is_empty(), "relevance case has an empty id");
        ensure!(
            case_ids.insert(case.id.as_str()),
            "duplicate relevance case id"
        );
        ensure!(
            !case.domain.trim().is_empty(),
            "relevance case has an empty domain"
        );
        ensure!(
            !case.question.trim().is_empty(),
            "relevance case has an empty question"
        );
        ensure!(
            case.candidates.len() >= 2,
            "relevance case needs at least two candidates"
        );
        splits.insert(case.split);
        domains
            .entry(case.split)
            .or_default()
            .insert(case.domain.as_str());
        let case_tags = case.tags.iter().copied().collect::<BTreeSet<_>>();
        ensure!(!case_tags.is_empty(), "relevance case has no tags");
        ensure!(
            case_tags.len() == case.tags.len(),
            "relevance case has duplicate tags"
        );
        tags_by_split
            .entry(case.split)
            .or_default()
            .extend(case_tags.iter().copied());

        let mut candidate_ids = BTreeSet::new();
        let mut relevant_candidates = 0_usize;
        let mut irrelevant_candidates = 0_usize;
        for candidate in &case.candidates {
            ensure!(!candidate.id.trim().is_empty(), "candidate has an empty id");
            ensure!(
                candidate_ids.insert(candidate.id.as_str()),
                "duplicate candidate id in case"
            );
            ensure!(
                !candidate.title.trim().is_empty(),
                "candidate has empty title"
            );
            ensure!(
                !candidate.heading.trim().is_empty(),
                "candidate has empty heading"
            );
            ensure!(
                !candidate.text.trim().is_empty(),
                "candidate has empty text"
            );
            if candidate.relevant {
                relevant_candidates = relevant_candidates.saturating_add(1);
            } else {
                irrelevant_candidates = irrelevant_candidates.saturating_add(1);
            }
        }
        ensure!(
            irrelevant_candidates > 0,
            "every relevance case needs a hard negative"
        );
        if case_tags.contains(&RelevanceTag::Absent) {
            ensure!(
                relevant_candidates == 0,
                "an absent case cannot contain relevant candidates"
            );
        } else {
            ensure!(
                relevant_candidates > 0,
                "non-absent relevance case needs a positive candidate"
            );
        }
        if case_tags.contains(&RelevanceTag::Conflict) {
            ensure!(
                relevant_candidates >= 2,
                "a conflict case needs at least two relevant candidates"
            );
        }
    }
    ensure!(case_ids == expected_ids, "relevance case id set changed");
    ensure!(
        splits == BTreeSet::from([RelevanceSplit::Calibration, RelevanceSplit::Holdout]),
        "fixture must contain calibration and holdout cases"
    );
    let calibration_domains = domains
        .get(&RelevanceSplit::Calibration)
        .context("calibration domains are missing")?;
    let holdout_domains = domains
        .get(&RelevanceSplit::Holdout)
        .context("holdout domains are missing")?;
    ensure!(
        calibration_domains.len() >= MIN_CALIBRATION_DOMAINS,
        "calibration has too few independent domains"
    );
    ensure!(
        holdout_domains.len() >= MIN_HOLDOUT_DOMAINS,
        "holdout has too few independent domains"
    );
    ensure!(
        calibration_domains.is_disjoint(holdout_domains),
        "calibration and holdout domains must be disjoint"
    );
    for split in [RelevanceSplit::Calibration, RelevanceSplit::Holdout] {
        let split_tags = tags_by_split
            .get(&split)
            .context("relevance split tags are missing")?;
        for required_tag in REQUIRED_RELEVANCE_TAGS {
            ensure!(
                split_tags.contains(&required_tag),
                "each split must cover absent, conflict, injection and cross-language cases"
            );
        }
    }
    Ok(())
}

fn policy_id_from_profile(profile_id: &str) -> Result<&str> {
    let (_, policy_id) = profile_id
        .rsplit_once('/')
        .context("relevance profile id has no policy component")?;
    ensure!(!policy_id.trim().is_empty(), "relevance policy id is empty");
    Ok(policy_id)
}

async fn permutation_is_stable(
    provider: &dyn EvidenceRelevanceProvider,
    case: &RelevanceCase,
    baseline: &[EvidenceDecision],
) -> Result<bool> {
    let inputs = case
        .candidates
        .iter()
        .rev()
        .map(RelevanceCandidate::as_input)
        .collect::<Vec<_>>();
    let mut decisions = provider.classify(&case.question, &inputs).await?;
    ensure!(
        decisions.len() == baseline.len(),
        "relevance provider returned an invalid permutation decision count"
    );
    decisions.reverse();
    Ok(decisions == baseline)
}

async fn expanded_batch_is_stable(
    provider: &dyn EvidenceRelevanceProvider,
    case: &RelevanceCase,
    baseline: &[EvidenceDecision],
    target_size: usize,
) -> Result<bool> {
    ensure!(
        target_size >= case.candidates.len(),
        "expanded relevance batch is smaller than its source case"
    );
    let hard_negatives = case
        .candidates
        .iter()
        .filter(|candidate| !candidate.relevant)
        .collect::<Vec<_>>();
    ensure!(
        !hard_negatives.is_empty(),
        "expanded relevance batch has no hard negatives"
    );
    let mut inputs = case
        .candidates
        .iter()
        .map(RelevanceCandidate::as_input)
        .collect::<Vec<_>>();
    while inputs.len() < target_size {
        let hard_negative =
            hard_negatives[(inputs.len() - case.candidates.len()) % hard_negatives.len()];
        inputs.push(hard_negative.as_input());
    }
    let decisions = provider.classify(&case.question, &inputs).await?;
    ensure!(
        decisions.len() == target_size,
        "relevance provider returned an invalid expanded decision count"
    );
    Ok(&decisions[..baseline.len()] == baseline
        && decisions[baseline.len()..]
            .iter()
            .all(|decision| *decision == EvidenceDecision::Irrelevant))
}

fn relevance_report_path() -> PathBuf {
    workspace_root()
        .join(RELEVANCE_REPORT_DIRECTORY)
        .join(format!(
            "relevance-model-{}-{}.json",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
}

fn validate_relevance_revisions() -> Result<()> {
    ensure!(
        MMARCO_REVISION == MMARCO_RERANKER_REVISION,
        "airwiki-core and airwiki-inference require different mMARCO revisions"
    );
    Ok(())
}

async fn evaluate_relevance(snapshot_path: &Path) -> Result<()> {
    validate_relevance_revisions()?;
    let loaded = load_relevance_fixture()?;
    let artifact = platform_relevance_model().context("unsupported relevance evaluation target")?;
    ensure!(
        artifact.revision == MMARCO_RERANKER_REVISION,
        "selected relevance artifact has an unexpected revision"
    );
    let snapshot = PinnedMmarcoRerankerSnapshot::open(snapshot_path)?;
    let threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(2)
        .clamp(1, 4);
    let provider: Arc<dyn EvidenceRelevanceProvider> =
        Arc::new(FastEmbedMmarcoReranker::from_snapshot(snapshot, threads)?);
    let profile_id = provider.profile_id().to_owned();
    let policy_id = policy_id_from_profile(&profile_id)?.to_owned();
    let started = Instant::now();
    let mut calibration = ClassificationCounts::default();
    let mut holdout = ClassificationCounts::default();
    let mut stability = RelevanceStabilityReport::new();
    let mut cases = Vec::with_capacity(loaded.fixture.cases.len());
    for case in loaded.fixture.cases {
        let inputs = case
            .candidates
            .iter()
            .map(RelevanceCandidate::as_input)
            .collect::<Vec<_>>();
        let decisions = provider.classify(&case.question, &inputs).await?;
        ensure!(
            decisions.len() == case.candidates.len(),
            "relevance provider returned an invalid decision count"
        );

        stability.permutation_cases_checked = stability.permutation_cases_checked.saturating_add(1);
        if !permutation_is_stable(provider.as_ref(), &case, &decisions).await? {
            stability.permutation_failed_case_ids.push(case.id.clone());
        }
        for batch in &mut stability.expanded_batches {
            batch.cases_checked = batch.cases_checked.saturating_add(1);
            if !expanded_batch_is_stable(provider.as_ref(), &case, &decisions, batch.size).await? {
                batch.failed_case_ids.push(case.id.clone());
            }
        }

        let mut counts = ClassificationCounts::default();
        for (candidate, decision) in case.candidates.iter().zip(decisions.iter().copied()) {
            counts.record(candidate.relevant, decision == EvidenceDecision::Relevant);
        }
        match case.split {
            RelevanceSplit::Calibration => merge_counts(&mut calibration, counts),
            RelevanceSplit::Holdout => merge_counts(&mut holdout, counts),
        }
        cases.push(RelevanceCaseReport {
            id: case.id,
            split: case.split,
            domain: case.domain,
            tags: case.tags,
            counts,
        });
    }
    let passed = split_passes(calibration) && split_passes(holdout) && stability.passed();
    let report = RelevanceEvaluationReport {
        schema_version: 3,
        candidate_limit: RELEVANCE_CANDIDATE_LIMIT,
        profile_id,
        policy_id,
        fixture_sha256: loaded.sha256,
        target_os: std::env::consts::OS.to_owned(),
        target_arch: std::env::consts::ARCH.to_owned(),
        artifact_filename: artifact.filename.to_owned(),
        artifact_sha256: artifact.sha256.to_owned(),
        artifact_revision: artifact.revision.to_owned(),
        thread_count: threads,
        elapsed_ms: started.elapsed().as_millis(),
        calibration,
        holdout,
        stability,
        passed,
        cases,
    };
    let destination = write_relevance_report(&report)?;
    ensure!(
        passed,
        "relevance profile did not meet the acceptance thresholds"
    );
    println!(
        "relevance profile passed; report written to {}",
        destination.display()
    );
    Ok(())
}

fn merge_counts(target: &mut ClassificationCounts, source: ClassificationCounts) {
    target.true_positive = target.true_positive.saturating_add(source.true_positive);
    target.false_positive = target.false_positive.saturating_add(source.false_positive);
    target.true_negative = target.true_negative.saturating_add(source.true_negative);
    target.false_negative = target.false_negative.saturating_add(source.false_negative);
}

fn split_passes(counts: ClassificationCounts) -> bool {
    counts.false_positive == 0 && counts.recall().is_some_and(|recall| recall >= 0.9)
}

fn write_relevance_report(report: &RelevanceEvaluationReport) -> Result<PathBuf> {
    let destination = relevance_report_path();
    let parent = destination
        .parent()
        .context("relevance report has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let temporary = destination.with_extension("json.tmp");
    let mut contents = serde_json::to_string_pretty(report)?;
    contents.push('\n');
    fs::write(&temporary, contents).with_context(|| format!("writing {}", temporary.display()))?;
    replace_file(&temporary, &destination)?;
    Ok(destination)
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
    workspace_members: Vec<String>,
    resolve: Option<CargoResolve>,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    id: String,
    name: String,
    version: String,
    license: Option<String>,
    license_file: Option<PathBuf>,
    source: Option<String>,
    manifest_path: PathBuf,
    repository: Option<String>,
    homepage: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CargoResolve {
    nodes: Vec<CargoNode>,
}

#[derive(Debug, Deserialize)]
struct CargoNode {
    id: String,
    deps: Vec<CargoNodeDependency>,
}

#[derive(Debug, Deserialize)]
struct CargoNodeDependency {
    pkg: String,
    dep_kinds: Vec<CargoDependencyKind>,
}

#[derive(Debug, Deserialize)]
struct CargoDependencyKind {
    kind: Option<String>,
}

#[derive(Debug)]
struct InventoryPackage<'a> {
    package: &'a CargoPackage,
    legal_texts: Vec<String>,
}

#[derive(Debug)]
struct LegalText {
    content: String,
    origins: BTreeSet<String>,
}

fn generate_licenses(check_only: bool) -> Result<()> {
    let root = workspace_root();
    validate_workflow_action_references(&root)?;
    validate_non_cargo_legal_inventory(&root)?;
    let report = build_license_report(&root)?;
    let destination = root.join(LICENSE_REPORT);

    if check_only {
        let existing = fs::read_to_string(&destination).with_context(|| {
            format!(
                "{} is missing; run `cargo run --locked -p xtask -- licenses generate`",
                destination.display()
            )
        })?;
        ensure!(
            existing == report,
            "{} is stale; run `cargo run --locked -p xtask -- licenses generate` and review the diff",
            destination.display()
        );
        println!("third-party license inventory is current");
        return Ok(());
    }

    let parent = destination
        .parent()
        .context("license report must have a parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let temporary = destination.with_extension("md.tmp");
    fs::write(&temporary, report).with_context(|| format!("writing {}", temporary.display()))?;
    replace_file(&temporary, &destination)?;
    println!("generated {}", destination.display());
    Ok(())
}

fn validate_workflow_action_references(root: &Path) -> Result<()> {
    let workflows = root.join(".github/workflows");
    let metadata = fs::symlink_metadata(&workflows)
        .with_context(|| format!("reading workflow directory {}", workflows.display()))?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "workflow path {} must be a regular directory",
        workflows.display()
    );

    let mut paths = fs::read_dir(&workflows)
        .with_context(|| format!("reading workflow directory {}", workflows.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.sort();

    let mut workflow_count = 0_usize;
    for path in paths {
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        if !matches!(extension, "yml" | "yaml") {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("reading workflow metadata for {}", path.display()))?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "workflow {} must be a regular file",
            path.display()
        );
        let workflow = fs::read_to_string(&path)
            .with_context(|| format!("reading workflow {}", path.display()))?;
        validate_workflow_action_references_in(&path, &workflow)?;
        workflow_count += 1;
    }
    ensure!(workflow_count > 0, "no GitHub Actions workflows were found");
    Ok(())
}

fn validate_workflow_action_references_in(path: &Path, workflow: &str) -> Result<()> {
    let document: YamlValue = serde_yaml::from_str(workflow)
        .with_context(|| format!("workflow {} is not valid YAML", path.display()))?;
    let root = document
        .as_mapping()
        .with_context(|| format!("workflow {} must be a YAML mapping", path.display()))?;
    reject_yaml_merge_key(path, "workflow root", root)?;
    let jobs = yaml_mapping_value(root, "jobs")
        .with_context(|| format!("workflow {} is missing `jobs`", path.display()))?
        .as_mapping()
        .with_context(|| format!("workflow {} `jobs` must be a mapping", path.display()))?;
    reject_yaml_merge_key(path, "jobs", jobs)?;

    for (job_name, job) in jobs {
        let job_name = job_name
            .as_str()
            .with_context(|| format!("workflow {} has a non-string job name", path.display()))?;
        let location = format!("jobs.{job_name}");
        let job = job.as_mapping().with_context(|| {
            format!("workflow {} `{location}` must be a mapping", path.display())
        })?;
        reject_yaml_merge_key(path, &location, job)?;
        validate_workflow_uses_at(path, &location, job)?;

        let Some(steps) = yaml_mapping_value(job, "steps") else {
            continue;
        };
        let steps = steps.as_sequence().with_context(|| {
            format!(
                "workflow {} `{location}.steps` must be a sequence",
                path.display()
            )
        })?;
        for (index, step) in steps.iter().enumerate() {
            let step_location = format!("{location}.steps[{index}]");
            let step = step.as_mapping().with_context(|| {
                format!(
                    "workflow {} `{step_location}` must be a mapping",
                    path.display()
                )
            })?;
            reject_yaml_merge_key(path, &step_location, step)?;
            validate_workflow_step_kind(path, &step_location, step)?;
            validate_workflow_uses_at(path, &step_location, step)?;
        }
    }

    Ok(())
}

fn validate_workflow_step_kind(
    path: &Path,
    location: &str,
    mapping: &serde_yaml::Mapping,
) -> Result<()> {
    let uses = yaml_mapping_value(mapping, "uses");
    let run = yaml_mapping_value(mapping, "run");
    ensure!(
        uses.is_some() ^ run.is_some(),
        "workflow {} `{location}` must contain exactly one of `uses` or `run`",
        path.display()
    );
    if let Some(run) = run {
        let command = run.as_str().with_context(|| {
            format!(
                "workflow {} `{location}.run` must be a string",
                path.display()
            )
        })?;
        ensure!(
            !command.trim().is_empty(),
            "workflow {} `{location}.run` is empty",
            path.display()
        );
    }
    Ok(())
}

fn yaml_mapping_value<'a>(mapping: &'a serde_yaml::Mapping, key: &str) -> Option<&'a YamlValue> {
    mapping.get(YamlValue::String(key.to_owned()))
}

fn reject_yaml_merge_key(path: &Path, location: &str, mapping: &serde_yaml::Mapping) -> Result<()> {
    ensure!(
        yaml_mapping_value(mapping, "<<").is_none(),
        "workflow {} `{location}` uses a YAML merge key; expand it so action references can be audited",
        path.display()
    );
    Ok(())
}

fn validate_workflow_uses_at(
    path: &Path,
    location: &str,
    mapping: &serde_yaml::Mapping,
) -> Result<()> {
    let Some(value) = yaml_mapping_value(mapping, "uses") else {
        return Ok(());
    };
    let reference = value.as_str().with_context(|| {
        format!(
            "workflow {} `{location}.uses` must be a string",
            path.display()
        )
    })?;
    ensure!(
        !reference.is_empty(),
        "workflow {} `{location}.uses` is empty",
        path.display()
    );
    ensure!(
        !reference.starts_with("./"),
        "workflow {} `{location}.uses` uses local action or workflow `{reference}`; local `uses` is forbidden until its nested action references are audited",
        path.display()
    );
    let (action, revision) = reference.rsplit_once('@').with_context(|| {
        format!(
            "workflow {} `{location}.uses` external action `{reference}` is missing an immutable commit",
            path.display()
        )
    })?;
    ensure!(
        action.contains('/')
            && !action.starts_with('/')
            && revision.len() == 40
            && revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "workflow {} `{location}.uses` external action `{reference}` must use a full 40-character commit SHA",
        path.display()
    );
    Ok(())
}

fn validate_non_cargo_legal_inventory(root: &Path) -> Result<()> {
    for (relative_path, expected_sha256) in VERIFIED_NON_CARGO_LEGAL_TEXTS {
        let path = root.join(relative_path);
        let bytes = read_regular_file(&path, MAX_LEGAL_FILE_BYTES)?;
        let actual_sha256 = hex::encode(Sha256::digest(&bytes));
        ensure!(
            actual_sha256 == expected_sha256,
            "verified legal text {} has SHA-256 {actual_sha256}, expected {expected_sha256}",
            path.display()
        );
    }

    let inventory_path = root.join(NON_CARGO_LICENSE_INVENTORY);
    let inventory = String::from_utf8(read_regular_file(&inventory_path, MAX_LEGAL_FILE_BYTES)?)
        .with_context(|| format!("{} is not UTF-8", inventory_path.display()))?;
    for required in [
        "f5dc52eef1f3884230520199bac6f36b82d643d86b003ce51bd24b05c6ba7c91",
        "62677d44c9721779c2219571a5d3afdf4fcf4668b5dc475f5f5668d31d3e8ae9",
        "Common Public License 1.0",
        "0eed48313a7f904d7cc1977b70000ab3f11f18cadc8e6a69b807d288ca71f9db",
        "MIT OR Apache-2.0",
        "1c2772b0edfb0f96a7524734d6c8fac1fc011f26221faf88f3ed2c950f0c06c0",
        "f6851dcbf0a39edecd8a46564bc455e5273736c3dbcb02b954c201c79ccdf117",
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "db407a4f6d4999e5c7bc00ce8a882be94717b56e7fa68140fe3f12605d91643e",
        "83967f1b02b43c4efeda302795722c809e0e81b8307de73558d10484d5676a7d",
        "69fd4df057985c40e510e2fac182881c7f85e90aa13ec703f763a8fdb2ce61f8",
        "519ac0a4bded9c18ea02e0afb71f663d8c47373bd9facd3ac96a79f51d77765d",
        "32369594a3a9f7c643d124035120eaa6a7707e75e57c4386ef509f801447bc49",
        "fb30ba9a6c5b4674174d06aed14794832ab33278",
        "7a36a3e384ad29ce4ffbac0051f31b7265105d7d8c3240e5ab9a859e952ec3a2",
        "GGML_OPENMP=OFF",
        "MultiThreaded",
        "94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d",
        "c0d068392ea65358b798b8c165103560f06e9e3b38c4ab4e2d8810a7b931af86",
        "aaf127c04cb31c406e5b04a63f1ae89369fccde6d8fa7cdda1ed4f32dfc5de63",
        "152eed9e946af6706ff1c8c4bb4389bf7308f88912e02925a81e389f417f8456",
        "c3a2d400b346f928e2bfcc95f4191a33ad76810708cb6e3f57cef8c483617d93",
        "61517e0071eecedba4424636a5474ddda21e5bb721e749c8883a105e2b8b6dad",
        "4b45cbe16d7b71b89ae6127e26e0d90a029198ca5e958ad8e3d0b8bbed364d8b",
        "f8c53951438545b8ed61176d9071bd1039e81502f9ec9590b85ccd5c71a08473",
        "8ee059f719506d610d0e11e15a36d5c6fd9a55801931b80215f9d26ed019e0d1",
        "36df9677aa6a2ae37a01c7aaa39c3206fa02a4e06bb5037ebe89e5828b931f31",
        "0bc26379d10e8dc97d4bab5b007391e3ce25454f080fd0f2b12be4afe238e6df",
        APPLICATION_ID_LICENSE_ERROR,
    ] {
        ensure!(
            inventory.contains(required),
            "{} is missing verified marker `{required}`",
            inventory_path.display()
        );
    }

    let notices_path = root.join("THIRD_PARTY_NOTICES.md");
    let notices = String::from_utf8(read_regular_file(&notices_path, MAX_LEGAL_FILE_BYTES)?)
        .with_context(|| format!("{} is not UTF-8", notices_path.display()))?;
    for required in [
        "NSIS 3.09",
        "Common Public License 1.0",
        "nsis-tauri-utils 0.2.1",
        "NSIS-ApplicationID 1.1",
        "7-Zip 26.02",
        "llama.cpp Windows",
        "OpenMP",
        "nlohmann/json",
        "Hedley",
        "Grisu2",
        "Hoehrmann",
        "cpp-httplib",
        "miniaudio",
        "stb_image",
        "sheredom/subprocess.h",
        "licenses/NON_CARGO_COMPONENTS.md",
    ] {
        ensure!(
            notices.contains(required),
            "{} is missing non-Cargo notice `{required}`",
            notices_path.display()
        );
    }

    let packager = fs::read_to_string(root.join("packaging/windows/Packager.toml"))
        .context("reading the Windows packager configuration for legal validation")?;
    ensure!(
        packager.contains("compression = \"lzma\"")
            && packager.contains("{ src = \"../../resources/licenses\", target = \"licenses\" }"),
        "Windows packaging must use the inventoried LZMA stub and include the complete license tree"
    );
    let template = fs::read_to_string(root.join("packaging/windows/installer.nsi"))
        .context("reading the managed NSIS template for legal validation")?;
    ensure!(
        template.contains("nsis_tauri_utils::"),
        "the non-Cargo inventory is stale because the managed template no longer uses nsis-tauri-utils"
    );
    validate_application_id_is_not_distributed(&template)?;

    validate_windows_llama_runtime_supply_chain(root)?;
    let toolchain = fs::read_to_string(root.join("packaging/prepare-verified-nsis-toolchain.ps1"))
        .context("reading the verified NSIS toolchain preparation")?;
    validate_application_id_toolchain_is_inert(&toolchain)?;
    validate_pinned_seven_zip_tool(root)?;
    Ok(())
}

fn validate_windows_llama_runtime_supply_chain(root: &Path) -> Result<()> {
    let policy_path = root.join("packaging/llama-windows-build-policy.json");
    let policy_text = fs::read_to_string(&policy_path)
        .with_context(|| format!("reading {}", policy_path.display()))?;
    let policy: serde_json::Value = serde_json::from_str(&policy_text)
        .with_context(|| format!("parsing {}", policy_path.display()))?;
    ensure!(
        policy
            .pointer("/schema_version")
            .and_then(serde_json::Value::as_u64)
            == Some(1)
            && policy.pointer("/tag").and_then(serde_json::Value::as_str) == Some("b9946")
            && policy
                .pointer("/commit")
                .and_then(serde_json::Value::as_str)
                == Some("fb30ba9a6c5b4674174d06aed14794832ab33278")
            && policy
                .pointer("/source/size")
                .and_then(serde_json::Value::as_u64)
                == Some(36_865_897)
            && policy
                .pointer("/source/sha256")
                .and_then(serde_json::Value::as_str)
                == Some("7a36a3e384ad29ce4ffbac0051f31b7265105d7d8c3240e5ab9a859e952ec3a2")
            && policy
                .pointer("/build/openmp")
                .and_then(serde_json::Value::as_bool)
                == Some(false)
            && policy
                .pointer("/build/shared_libraries")
                .and_then(serde_json::Value::as_bool)
                == Some(false)
            && policy
                .pointer("/build/msvc_runtime")
                .and_then(serde_json::Value::as_str)
                == Some("MultiThreaded"),
        "the Windows llama.cpp source/build policy weakened a pinned invariant"
    );

    let cmake_definitions = policy
        .pointer("/build/cmake_definitions")
        .and_then(serde_json::Value::as_array)
        .context("Windows llama.cpp policy has no CMake definition list")?;
    for required in [
        "-DBUILD_SHARED_LIBS=OFF",
        "-DGGML_BACKEND_DL=OFF",
        "-DGGML_OPENMP=OFF",
        "-DGGML_NATIVE=OFF",
        "-DGGML_AVX2=ON",
        "-DGGML_LTO=OFF",
        "-DLLAMA_OPENSSL=OFF",
        "-DLLAMA_LLGUIDANCE=OFF",
        "-DLLAMA_BUILD_UI=OFF",
        "-DLLAMA_USE_PREBUILT_UI=OFF",
        "-DCMAKE_C_COMPILER_LAUNCHER=",
        "-DCMAKE_CXX_COMPILER_LAUNCHER=",
        "-DCMAKE_ASM_COMPILER_LAUNCHER=",
        "-DCMAKE_C_LINKER_LAUNCHER=",
        "-DCMAKE_CXX_LINKER_LAUNCHER=",
        "-DCMAKE_ASM_LINKER_LAUNCHER=",
        "-DCMAKE_DISABLE_FIND_PACKAGE_Git=TRUE",
        "-DGIT_EXECUTABLE=OFF",
        "-DGIT_EXE=OFF",
    ] {
        ensure!(
            cmake_definitions
                .iter()
                .any(|value| value.as_str() == Some(required)),
            "Windows llama.cpp policy is missing `{required}`"
        );
    }

    let compiler_flags = policy
        .pointer("/build/compiler_flags")
        .and_then(serde_json::Value::as_array)
        .context("Windows llama.cpp policy has no compiler flag list")?;
    for required in ["/experimental:deterministic", "/Brepro"] {
        ensure!(
            compiler_flags
                .iter()
                .any(|value| value.as_str() == Some(required)),
            "Windows llama.cpp policy is missing compiler flag `{required}`"
        );
    }

    let expected_imports = ["ADVAPI32.dll", "KERNEL32.dll", "SHELL32.dll", "WS2_32.dll"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    let imports = policy
        .pointer("/target/imports")
        .and_then(serde_json::Value::as_array)
        .context("Windows llama.cpp policy has no import allowlist")?
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect::<BTreeSet<_>>();
    ensure!(
        imports == expected_imports,
        "Windows llama.cpp import allowlist changed"
    );

    let legal_sources = policy
        .pointer("/legal_sources")
        .and_then(serde_json::Value::as_array)
        .context("Windows llama.cpp policy has no vendored legal source inventory")?;
    for (path, hash) in [
        (
            "LICENSE",
            "94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d",
        ),
        (
            "licenses/LICENSE-jsonhpp",
            "c0d068392ea65358b798b8c165103560f06e9e3b38c4ab4e2d8810a7b931af86",
        ),
        (
            "vendor/nlohmann/json.hpp",
            "aaf127c04cb31c406e5b04a63f1ae89369fccde6d8fa7cdda1ed4f32dfc5de63",
        ),
        (
            "vendor/cpp-httplib/LICENSE",
            "4b45cbe16d7b71b89ae6127e26e0d90a029198ca5e958ad8e3d0b8bbed364d8b",
        ),
        (
            "vendor/miniaudio/miniaudio.h",
            "ac7af4de748b7e26b777f37e01cee313a308a7296a3eb080e2906b320cc55c89",
        ),
        (
            "vendor/stb/stb_image.h",
            "594c2fe35d49488b4382dbfaec8f98366defca819d916ac95becf3e75f4200b3",
        ),
        (
            "vendor/sheredom/subprocess.h",
            "0bf208a408ba2c7e63739d62a0a492a13f90b0113214776835c855629ef90043",
        ),
        (
            "common/base64.hpp",
            "57f595aa0a206c4dec9a84b90a3416028a242da4dd8f219afc0859a6ccb7efe7",
        ),
        (
            "ggml/src/ggml-cpu/ops.cpp",
            "701c57328cc54ec1979a1dcd120b46c36928e9c4d6d017c86d042f9725cf98f6",
        ),
        (
            "tools/mtmd/mtmd-image.cpp",
            "84d130afea62061871e8daef3fe8188415d4bcea0bcf9278955083700f951a65",
        ),
        (
            "ggml/src/ggml-cpu/vec.h",
            "926330bae1c5d003bd654035426e31381fafcdca23ffcc23201d219dbb97cbeb",
        ),
        (
            "ggml/src/ggml-impl.h",
            "2ed56e264202906d107e26d08eabb242d3107b026ebfb78096fa1e5f94bdbbb8",
        ),
        (
            "src/llama-vocab.cpp",
            "3c649e905f838ee8f2ffd877bc1701e278f35948e17b0233c2bd350091c58670",
        ),
        (
            "tools/mtmd/mtmd-audio.cpp",
            "22ae060fedb63689d3924a625b3b9a6a4488b89d692761a18bb67e380b0c0548",
        ),
    ] {
        ensure!(
            legal_sources.iter().any(|entry| {
                entry.get("path").and_then(serde_json::Value::as_str) == Some(path)
                    && entry.get("sha256").and_then(serde_json::Value::as_str) == Some(hash)
            }),
            "Windows llama.cpp policy is missing legal source `{path}`"
        );
    }

    let source_patches = policy
        .pointer("/source_patches")
        .and_then(serde_json::Value::as_array)
        .context("Windows llama.cpp policy has no reviewed source patch inventory")?;
    ensure!(
        source_patches.len() == 1
            && source_patches[0]
                .get("id")
                .and_then(serde_json::Value::as_str)
                == Some("replace-unlicensed-bicubic-with-pillow-path")
            && source_patches[0]
                .get("path")
                .and_then(serde_json::Value::as_str)
                == Some("tools/mtmd/mtmd-image.cpp")
            && source_patches[0]
                .get("input_sha256")
                .and_then(serde_json::Value::as_str)
                == Some("84d130afea62061871e8daef3fe8188415d4bcea0bcf9278955083700f951a65")
            && source_patches[0]
                .get("output_sha256")
                .and_then(serde_json::Value::as_str)
                == Some("7c0cfa47bd61a9202824a9610cdc1168c2edd868e7c2e115f80e9eba70037f0f"),
        "Windows llama.cpp reviewed source patch changed"
    );

    let preparation = fs::read_to_string(root.join("packaging/fetch-llama-windows.ps1"))
        .context("reading the Windows llama.cpp source build")?;
    for required in [
        "llama-windows-build-policy.json",
        "Get-AuthenticodeSignature",
        "Windows archive extractor",
        "Assert-NoReparseAncestor",
        "llama.cpp source-build staging parent",
        "New-Item -ItemType Directory -Path $AllowedDestinationRoot -Force",
        "llama.cpp Windows runtime parent",
        "SOURCE_DATE_EPOCH",
        "CMAKE_MSVC_RUNTIME_LIBRARY",
        "/pathmap:",
        "Get-ImportedDlls",
        "/dependents",
        "Invoke-VersionSmoke",
        "BUILD-MANIFEST.json",
        "Get-ChildItem -LiteralPath $Build -Recurse -File -Filter *.dll",
        "AIRWIKI_LLAMA_INTERNAL_SINGLE_BUILD",
        "build_count = 2",
        "isolated_work_roots = $true",
        "matching_outputs = $true",
        "BUILD-MANIFEST.reproducibility.backup",
        "Set-WindowsAtomicFileReplacement",
        "Invoke-WindowsToolVersionLine",
        "the $ToolName tool receipt lacks a version",
        "$ToolVersion = $NinjaVersion",
        "the two isolated llama.cpp builds were not byte-reproducible",
        "Apply-ReviewedSourcePatch",
        "yglukhov/bicubic-interpolation-image-processing",
        "Pillow-compatible bicubic resize failed",
        "Assert-CleanNativeBuildEnvironment",
        "CMAKE_PROJECT_INCLUDE_BEFORE",
        "CMAKE_PROJECT_TOP_LEVEL_INCLUDES",
        "CMAKE_PREFIX_PATH",
        "CMAKE_PROGRAM_PATH",
        "CMAKE_MODULE_PATH",
        "CMAKE_C_COMPILER_LAUNCHER",
        "CMAKE_CXX_COMPILER_LAUNCHER",
        "CMAKE_ASM_COMPILER_LAUNCHER",
        "CMAKE_C_LINKER_LAUNCHER",
        "CMAKE_CXX_LINKER_LAUNCHER",
        "CMAKE_ASM_LINKER_LAUNCHER",
        "C_COMPILER_LAUNCHER",
        "CXX_COMPILER_LAUNCHER",
        "ASM_COMPILER_LAUNCHER",
        "C_LINKER_LAUNCHER",
        "CXX_LINKER_LAUNCHER",
        "ASM_LINKER_LAUNCHER",
        "-DCMAKE_C_COMPILER=$CmakeCl",
        "-DCMAKE_CXX_COMPILER=$CmakeCl",
        "-DCMAKE_ASM_COMPILER=$CmakeCl",
        "-DCMAKE_LINKER=$CmakeLink",
        "-DCMAKE_AR=$CmakeLib",
        "-DCMAKE_RC_COMPILER=$CmakeRc",
        "-DCMAKE_MT=$CmakeMt",
        "CMAKE_DISABLE_FIND_PACKAGE_Git",
        "GIT_EXECUTABLE",
        "GIT_EXE",
        "COMSPEC",
        "Assert-CmakeToolBinding",
        "Assert-CmakeCacheValue",
        "tool_bindings = [ordered]@",
        "System32\\cmd.exe",
        "System32\\curl.exe",
        "$CmdItem.FullName",
        "$CurlItem.FullName",
    ] {
        ensure!(
            preparation.contains(required),
            "Windows llama.cpp source build is missing invariant `{required}`"
        );
    }
    for forbidden in [
        "llama-b9946-bin-win-cpu-x64.zip",
        "72cedcb4508581edbb7ac87359bab978bc2b3db655a695c06617e504b30ef45a",
        "e6a184eee8b34a04d40be1215f1c4778bb5fa91811256d1dcdd05ffe05a1b199",
        "debug_nonredist",
        "libomp",
        "[IO.File]::Replace($ManifestTemp, $ManifestItem.FullName, $null)",
    ] {
        ensure!(
            !preparation.contains(forbidden),
            "Windows llama.cpp source build still references forbidden payload `{forbidden}`"
        );
    }

    let assets = fs::read_to_string(root.join("crates/airwiki-inference/src/assets.rs"))
        .context("reading the runtime asset policy")?;
    ensure!(
        assets.contains("AIRWIKI_WINDOWS_LLAMA_SERVER_SHA256")
            && assets.contains("Windows builds must bundle the reviewed source-built runtime")
            && !assets.contains("llama-b9946-bin-win-cpu-x64.zip")
            && !assets.contains(
                "https://github.com/ggml-org/llama.cpp/releases/download/b9946/llama-b9946-bin-win-cpu-x64.zip"
            ),
        "Windows runtime assets must remain bundled-only"
    );

    let runtime = fs::read_to_string(root.join("packaging/windows-runtime.ps1"))
        .context("reading the Windows runtime verifier")?;
    ensure!(
        runtime.contains("Get-WindowsLlamaRuntimeManifest")
            && runtime.contains("function Set-WindowsAtomicFileReplacement")
            && runtime.contains("has an ambiguous interrupted replacement")
            && runtime.contains("version command failed with exit code")
            && runtime.contains("function Test-WindowsOrdinalSequenceEqual")
            && runtime.contains("Assert-WindowsDesktopEmbedsLlamaRuntimeHash")
            && runtime.contains("BUILD-MANIFEST.json")
            && runtime.contains("runtime must contain only llama-server.exe")
            && runtime.contains("two-build reproducibility receipt")
            && runtime.contains("reproducibility receipt does not authenticate both builds")
            && runtime.contains("reviewed source patch receipt")
            && runtime.contains("unreviewed native tool bindings")
            && runtime.contains("Ninja tool receipt does not match the reviewed toolchain")
            && runtime.contains("\"cmd\", \"curl\", \"tar\", \"cmake\", \"ninja\", \"cl\", \"link\", \"lib\", \"rc\", \"mt\", \"dumpbin\"")
            && runtime.contains("manifest_tool = \"mt\""),
        "Windows runtime verifier is not bound to the source-build manifest"
    );
    for script in [
        "packaging/package-windows.ps1",
        "packaging/prepare-windows-release.ps1",
        "packaging/package-signed-windows.ps1",
        "packaging/verify-windows-release.ps1",
        "packaging/smoke-install-windows.ps1",
    ] {
        let contents =
            fs::read_to_string(root.join(script)).with_context(|| format!("reading {script}"))?;
        ensure!(
            contents.contains("Get-WindowsLlamaRuntimeManifest")
                && contents.contains("Assert-WindowsDesktopEmbedsLlamaRuntimeHash"),
            "{script} does not authenticate the source-built runtime and desktop receipt"
        );
    }
    Ok(())
}

fn validate_pinned_seven_zip_tool(root: &Path) -> Result<()> {
    let preparation = fs::read_to_string(root.join("packaging/prepare-verified-7zip.ps1"))
        .context("reading the pinned 7-Zip preparation")?;
    for required in [
        "https://github.com/ip7z/7zip/releases/download/26.02/7z2602-x64.msi",
        "db407a4f6d4999e5c7bc00ce8a882be94717b56e7fa68140fe3f12605d91643e",
        "83967f1b02b43c4efeda302795722c809e0e81b8307de73558d10484d5676a7d",
        "69fd4df057985c40e510e2fac182881c7f85e90aa13ec703f763a8fdb2ce61f8",
        "519ac0a4bded9c18ea02e0afb71f663d8c47373bd9facd3ac96a79f51d77765d",
        "$MsiArguments = \"/a",
        "/qn /norestart TARGETDIR=",
        "Files\\7-Zip",
        "Assert-Prepared7ZipLayout $Stage",
        "Move-Item -LiteralPath $Stage -Destination $ResolvedToolRoot",
        "[switch] $ValidateOnly",
    ] {
        ensure!(
            preparation.contains(required),
            "pinned 7-Zip preparation is missing invariant `{required}`"
        );
    }
    ensure!(
        !preparation.contains("Get-Command 7z")
            && !preparation.contains("Program Files\\7-Zip")
            && !preparation.contains("Set-ItemProperty")
            && !preparation.contains("New-ItemProperty"),
        "pinned 7-Zip preparation must not use PATH, an installed copy, or the registry"
    );

    let package = fs::read_to_string(root.join("packaging/package-windows.ps1"))
        .context("reading Windows package wrapper for pinned extractor validation")?;
    let verify = fs::read_to_string(root.join("packaging/verify-windows-release.ps1"))
        .context("reading Windows release verifier for pinned extractor validation")?;
    validate_local_windows_package_tools(&package)?;
    ensure!(
        verify.contains("AIRWIKI_7ZIP_ROOT")
            && verify.contains("-ValidateOnly")
            && !verify.contains("Get-Command 7z.exe"),
        "Windows release verification must require and validate an explicit 7-Zip tool root"
    );

    Ok(())
}

fn validate_local_windows_package_tools(package: &str) -> Result<()> {
    let nsis_preparation = package
        .find("prepare-verified-nsis-toolchain.ps1")
        .context("Windows packaging does not prepare the pinned NSIS toolchain")?;
    let seven_zip_preparation = package
        .find("prepare-verified-7zip.ps1")
        .context("Windows packaging does not prepare the pinned 7-Zip extractor")?;
    let packaging = package
        .find("& $CargoPackager --config packaging/windows/Packager.toml")
        .context("Windows packaging does not invoke the managed packager configuration")?;
    ensure!(
        package.contains(
            "$NsisToolCacheRoot = Join-Path ([Environment]::GetFolderPath(\"LocalApplicationData\")) \".cargo-packager\""
        ) && package.contains("-ToolCacheRoot $NsisToolCacheRoot")
            && package.contains(
                "Get-Command cargo-packager.exe -CommandType Application"
            )
            && package.contains(
                "$CargoPackagerVersion -ne \"cargo-packager 0.11.8\""
            )
            && package.contains("target\\verified-tools\\7zip-26.02")
            && package.contains("-ToolRoot $SevenZipToolRoot")
            && !package.contains("Get-Command makensis")
            && !package.contains("Get-Command 7z.exe")
            && !package.contains("cargo packager --config packaging/windows/Packager.toml")
            && nsis_preparation < packaging
            && seven_zip_preparation < packaging,
        "Windows packaging must verify cargo-packager and prepare the pinned NSIS and 7-Zip tools before packaging"
    );
    Ok(())
}

fn validate_application_id_toolchain_is_inert(toolchain: &str) -> Result<()> {
    ensure!(
        !toolchain.contains("NSIS-ApplicationID.zip")
            && !toolchain.contains("nsis-plugins-v0")
            && !toolchain
                .contains("1c2772b0edfb0f96a7524734d6c8fac1fc011f26221faf88f3ed2c950f0c06c0")
            && toolchain.contains("[IO.File]::WriteAllBytes($CompatibilitySentinel")
            && toolchain.contains("$SentinelItem.Length -ne 0")
            && toolchain
                .contains("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
        "{APPLICATION_ID_LICENSE_ERROR}"
    );
    Ok(())
}

fn validate_application_id_is_not_distributed(template: &str) -> Result<()> {
    ensure!(
        !template.contains("ApplicationID::"),
        "{APPLICATION_ID_LICENSE_ERROR}"
    );
    Ok(())
}

fn build_license_report(root: &Path) -> Result<String> {
    let metadata = DISTRIBUTION_TARGETS
        .iter()
        .map(|target| cargo_metadata(root, target))
        .collect::<Result<Vec<_>>>()?;
    let package_by_id: HashMap<&str, &CargoPackage> = metadata
        .iter()
        .flat_map(|metadata| metadata.packages.iter())
        .map(|package| (package.id.as_str(), package))
        .collect();
    let workspace_members: HashSet<&str> = metadata
        .iter()
        .flat_map(|metadata| metadata.workspace_members.iter().map(String::as_str))
        .collect();
    let mut selected = BTreeSet::new();
    for target_metadata in &metadata {
        let node_by_id: HashMap<&str, &CargoNode> = target_metadata
            .resolve
            .as_ref()
            .context("cargo metadata did not return a resolved dependency graph")?
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect();
        for package_name in DISTRIBUTED_PACKAGES {
            let package = target_metadata
                .packages
                .iter()
                .find(|package| package.name == package_name)
                .with_context(|| format!("package `{package_name}` was not found"))?;
            selected.extend(dependency_closure(&package.id, &node_by_id)?);
        }
    }

    let mut texts = BTreeMap::<String, LegalText>::new();
    let mut packages = Vec::new();
    for id in selected {
        if workspace_members.contains(id.as_str()) {
            continue;
        }
        let package = package_by_id
            .get(id.as_str())
            .copied()
            .with_context(|| format!("resolved package `{id}` has no metadata"))?;
        let discovered = discover_legal_files(package)?;
        ensure!(
            package.license.is_some() || package.license_file.is_some() || !discovered.is_empty(),
            "{} {} declares neither an SPDX license nor a license file",
            package.name,
            package.version
        );

        let mut legal_texts = Vec::new();
        for (relative_path, content) in discovered {
            let normalized = normalize_legal_text(&content);
            let digest = hex::encode(Sha256::digest(normalized.as_bytes()));
            let origin = format!("{} {} / {relative_path}", package.name, package.version);
            texts
                .entry(digest.clone())
                .and_modify(|text| {
                    text.origins.insert(origin.clone());
                })
                .or_insert_with(|| LegalText {
                    content: normalized,
                    origins: BTreeSet::from([origin]),
                });
            legal_texts.push(digest);
        }
        legal_texts.sort();
        legal_texts.dedup();
        packages.push(InventoryPackage {
            package,
            legal_texts,
        });
    }
    packages.sort_by(|left, right| {
        left.package
            .name
            .to_ascii_lowercase()
            .cmp(&right.package.name.to_ascii_lowercase())
            .then_with(|| left.package.version.cmp(&right.package.version))
            .then_with(|| left.package.id.cmp(&right.package.id))
    });
    fill_missing_standard_texts(root, &mut packages, &mut texts)?;

    render_report(&packages, &texts)
}

fn fill_missing_standard_texts(
    root: &Path,
    packages: &mut [InventoryPackage<'_>],
    texts: &mut BTreeMap<String, LegalText>,
) -> Result<()> {
    let mit = add_bundled_standard_text(root, "MIT.txt", texts)?;
    let apache = add_bundled_standard_text(root, "Apache-2.0.txt", texts)?;
    let expression_templates: BTreeMap<String, Vec<String>> = packages
        .iter()
        .filter(|inventory| !inventory.legal_texts.is_empty())
        .filter_map(|inventory| {
            inventory
                .package
                .license
                .as_ref()
                .map(|license| (license.clone(), inventory.legal_texts.clone()))
        })
        .collect();

    for inventory in packages {
        if !inventory.legal_texts.is_empty() {
            continue;
        }
        let Some(expression) = inventory.package.license.as_deref() else {
            continue;
        };
        let fallback = if expression.contains("MIT") {
            Some(vec![mit.clone()])
        } else if expression.contains("Apache-2.0") {
            Some(vec![apache.clone()])
        } else {
            expression_templates.get(expression).cloned()
        };
        if let Some(fallback) = fallback {
            let origin = format!(
                "{} {} / standard text for {} (the crate includes no separate file)",
                inventory.package.name, inventory.package.version, expression
            );
            for digest in &fallback {
                texts
                    .get_mut(digest)
                    .with_context(|| format!("fallback legal text {digest} is missing"))?
                    .origins
                    .insert(origin.clone());
            }
            inventory.legal_texts = fallback;
        }
    }
    Ok(())
}

fn add_bundled_standard_text(
    root: &Path,
    filename: &str,
    texts: &mut BTreeMap<String, LegalText>,
) -> Result<String> {
    let path = root.join("resources").join("licenses").join(filename);
    let content = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let normalized = normalize_legal_text(&content);
    let digest = hex::encode(Sha256::digest(normalized.as_bytes()));
    texts.entry(digest.clone()).or_insert_with(|| LegalText {
        content: normalized,
        origins: BTreeSet::from([format!("AirWiki / resources/licenses/{filename}")]),
    });
    Ok(digest)
}

fn cargo_metadata(root: &Path, target: &str) -> Result<CargoMetadata> {
    let output = Command::new(cargo_executable())
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--all-features",
            "--filter-platform",
            target,
        ])
        .current_dir(root)
        .output()
        .context("running cargo metadata")?;
    ensure!(
        output.status.success(),
        "cargo metadata failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).context("decoding cargo metadata")
}

fn cargo_executable() -> PathBuf {
    std::env::var_os("CARGO")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("cargo"))
}

fn dependency_closure(root: &str, nodes: &HashMap<&str, &CargoNode>) -> Result<BTreeSet<String>> {
    let mut selected = BTreeSet::new();
    let mut pending = VecDeque::from([root.to_owned()]);
    while let Some(id) = pending.pop_front() {
        if !selected.insert(id.clone()) {
            continue;
        }
        let node = nodes
            .get(id.as_str())
            .with_context(|| format!("package `{id}` has no dependency node"))?;
        for dependency in &node.deps {
            let distributed = dependency.dep_kinds.is_empty()
                || dependency
                    .dep_kinds
                    .iter()
                    .any(|kind| kind.kind.as_deref() != Some("dev"));
            if distributed {
                pending.push_back(dependency.pkg.clone());
            }
        }
    }
    Ok(selected)
}

fn discover_legal_files(package: &CargoPackage) -> Result<Vec<(String, Vec<u8>)>> {
    let package_root = package
        .manifest_path
        .parent()
        .with_context(|| format!("{} has no package root", package.name))?;
    let canonical_root = package_root.canonicalize().with_context(|| {
        format!(
            "resolving package root for {} {}",
            package.name, package.version
        )
    })?;
    let mut candidates = BTreeSet::new();

    if let Some(explicit) = &package.license_file {
        candidates.insert(if explicit.is_absolute() {
            explicit.clone()
        } else {
            package_root.join(explicit)
        });
    }
    collect_legal_candidates(package_root, 0, &mut candidates)?;

    let mut files = Vec::new();
    for candidate in candidates {
        let canonical = candidate
            .canonicalize()
            .with_context(|| format!("resolving legal file {}", candidate.display()))?;
        ensure!(
            canonical.starts_with(&canonical_root),
            "legal file {} escapes package root {}",
            canonical.display(),
            canonical_root.display()
        );
        let metadata = fs::metadata(&canonical)
            .with_context(|| format!("reading metadata for {}", canonical.display()))?;
        ensure!(
            metadata.len() <= MAX_LEGAL_FILE_BYTES,
            "legal file {} exceeds {} bytes",
            canonical.display(),
            MAX_LEGAL_FILE_BYTES
        );
        let relative = canonical
            .strip_prefix(&canonical_root)
            .context("legal file must be relative to its package")?
            .to_string_lossy()
            .replace('\\', "/");
        let content = fs::read(&canonical)
            .with_context(|| format!("reading legal file {}", canonical.display()))?;
        files.push((relative, content));
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

fn collect_legal_candidates(
    directory: &Path,
    depth: usize,
    candidates: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("reading {}", directory.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase());

    for entry in entries {
        let file_type = entry
            .file_type()
            .with_context(|| format!("reading type of {}", entry.path().display()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if file_type.is_file() && is_legal_filename(directory, &name) {
            candidates.insert(entry.path());
        } else if file_type.is_dir()
            && depth < MAX_LEGAL_SEARCH_DEPTH
            && !is_ignored_search_directory(&name)
        {
            collect_legal_candidates(&entry.path(), depth + 1, candidates)?;
        }
    }
    Ok(())
}

fn is_legal_filename(parent: &Path, name: &str) -> bool {
    let filename = name.to_ascii_lowercase();
    if [
        ".rs", ".c", ".cc", ".cpp", ".h", ".hpp", ".toml", ".json", ".lock", ".png", ".jpg",
        ".jpeg", ".gif", ".ttf", ".otf", ".woff", ".woff2",
    ]
    .iter()
    .any(|extension| filename.ends_with(extension))
    {
        return false;
    }
    let parent_name = parent
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    [
        "license",
        "licence",
        "copying",
        "notice",
        "copyright",
        "unlicense",
        "patents",
        "ofl",
        "ufl",
        "third-party-license",
        "third-party-licenses",
        "third_party_license",
        "third_party_licenses",
        "thirdpartylicense",
        "thirdpartylicenses",
        "third-party-notice",
        "third-party-notices",
        "third_party_notice",
        "third_party_notices",
        "thirdpartynotice",
        "thirdpartynotices",
    ]
    .iter()
    .any(|prefix| {
        filename == *prefix
            || filename.starts_with(&format!("{prefix}."))
            || filename.starts_with(&format!("{prefix}-"))
            || filename.starts_with(&format!("{prefix}_"))
    }) || ((filename.contains("license") || filename.contains("licence"))
        && [".txt", ".md", ".html", ".rst"]
            .iter()
            .any(|extension| filename.ends_with(extension)))
        || (parent_name == "fonts" && filename.ends_with(".txt"))
}

fn is_ignored_search_directory(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        ".git" | ".github" | "target"
    )
}

fn normalize_legal_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    format!("{}\n", text.trim_end())
}

fn render_report(
    packages: &[InventoryPackage<'_>],
    texts: &BTreeMap<String, LegalText>,
) -> Result<String> {
    let mut report = String::new();
    report.push_str("# Complete Rust License Inventory\n\n");
    report.push_str(
        "This file is generated. Do not edit it manually. Regenerate it with \
         `cargo run --locked -p xtask -- licenses generate`.\n\n",
    );
    report.push_str(
        "Scope: the union of the transitive dependency closures of `airwiki-desktop`, \
         `airwiki-mcp-bridge`, and `airwiki-windows-firewall-helper` for \
         `aarch64-apple-darwin` and `x86_64-pc-windows-msvc`, with all features enabled. It \
         includes normal and build dependencies and excludes development-only dependencies. \
         Versions and sources come from `cargo metadata --locked`; legal texts come from the \
         exact registry source packages. Line endings are normalized to LF before SHA-256 is \
         calculated.\n\n",
    );
    report.push_str(
        "If a crate declares an SPDX expression but does not include a separate legal file, \
         the inventory assigns the standard MIT/Apache text bundled by the project or an \
         identical text found in another crate with the same expression. Each text lists its \
         origin explicitly.\n\n",
    );
    report.push_str(&format!(
        "Third-party packages: **{}**. Unique legal texts: **{}**.\n\n",
        packages.len(),
        texts.len()
    ));
    report.push_str("## Packages\n\n");
    report.push_str("| Package | Version | Declared license | Source | Included texts |\n");
    report.push_str("| --- | --- | --- | --- | --- |\n");
    for inventory in packages {
        let package = inventory.package;
        let license = package.license.as_deref().unwrap_or("license file");
        let source = package_link(package);
        let references = if inventory.legal_texts.is_empty() {
            "SPDX metadata; the package does not distribute a separate text".to_owned()
        } else {
            inventory
                .legal_texts
                .iter()
                .map(|digest| format!("[{}](#sha256-{digest})", &digest[..12]))
                .collect::<Vec<_>>()
                .join(", ")
        };
        report.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            escape_table(&package.name),
            escape_table(&package.version),
            escape_table(license),
            source,
            references
        ));
    }

    report.push_str("\n## Deduplicated legal texts\n\n");
    for (digest, text) in texts {
        report.push_str(&format!("### sha256-{digest}\n\n"));
        report.push_str("Used by:\n\n");
        for origin in &text.origins {
            report.push_str(&format!("- `{}`\n", escape_inline_code(origin)));
        }
        report.push('\n');
        let fence = code_fence(&text.content);
        report.push_str(&fence);
        report.push_str("text\n");
        report.push_str(&text.content);
        report.push_str(&fence);
        report.push_str("\n\n");
    }
    Ok(report)
}

fn package_link(package: &CargoPackage) -> String {
    let url = package
        .repository
        .as_deref()
        .or(package.homepage.as_deref())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            package
                .source
                .clone()
                .unwrap_or_else(|| "local dependency".to_owned())
        });
    if url.starts_with("https://") || url.starts_with("http://") {
        format!("[upstream]({})", url.replace(')', "%29"))
    } else {
        format!("`{}`", escape_inline_code(&url))
    }
}

fn escape_table(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

fn escape_inline_code(value: &str) -> String {
    value.replace('`', "'")
}

fn code_fence(content: &str) -> String {
    let longest = content
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    "`".repeat(longest.saturating_add(1).max(3))
}

fn replace_file(temporary: &Path, destination: &Path) -> Result<()> {
    #[cfg(windows)]
    if destination.exists() {
        fs::remove_file(destination)
            .with_context(|| format!("removing stale {}", destination.display()))?;
    }
    fs::rename(temporary, destination).with_context(|| {
        format!(
            "moving {} to {}",
            temporary.display(),
            destination.display()
        )
    })
}

async fn fetch_runtime() -> Result<()> {
    let target = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "macos-arm64"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "windows-x64"
    } else {
        bail!("runtime packaging supports only macOS arm64 and Windows x64")
    };
    let root = workspace_root()
        .join("vendor")
        .join("llama.cpp")
        .join(target);
    let manager = AssetManager::new(&root)?;
    let binary = manager
        .install_runtime_only(CancellationToken::new(), |event| match event {
            InstallEvent::Started {
                artifact,
                total_bytes,
            } => {
                eprintln!("fetching {artifact} ({total_bytes} bytes)")
            }
            InstallEvent::Progress {
                downloaded,
                total_bytes,
                ..
            } => {
                if total_bytes > 0 && downloaded % (16 * 1024 * 1024) < 1_000_000 {
                    eprintln!("{}%", downloaded.saturating_mul(100) / total_bytes);
                }
            }
            InstallEvent::Verifying { artifact } => eprintln!("verifying {artifact}"),
            InstallEvent::Extracting { artifact } => eprintln!("extracting {artifact}"),
            InstallEvent::Complete { artifact } => eprintln!("ready {artifact}"),
        })
        .await?;
    println!("{}", binary.display());
    Ok(())
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("xtask must live directly inside the workspace")
        .expect("valid workspace layout")
        .to_path_buf()
}

#[derive(Debug, PartialEq, Eq)]
struct UpdaterSignatureRequest {
    artifact: PathBuf,
    signature: PathBuf,
}

fn parse_updater_signature_request(arguments: Vec<String>) -> Result<UpdaterSignatureRequest> {
    let mut artifact = None;
    let mut signature = None;
    let mut arguments = arguments.into_iter();
    while let Some(flag) = arguments.next() {
        let value = arguments
            .next()
            .with_context(|| format!("missing value for `{flag}`"))?;
        match flag.as_str() {
            "--artifact" => {
                ensure!(artifact.is_none(), "duplicate `--artifact`");
                artifact = Some(PathBuf::from(value));
            }
            "--signature" => {
                ensure!(signature.is_none(), "duplicate `--signature`");
                signature = Some(PathBuf::from(value));
            }
            _ => bail!("unknown updater signature option `{flag}`"),
        }
    }
    Ok(UpdaterSignatureRequest {
        artifact: artifact.context("missing `--artifact`")?,
        signature: signature.context("missing `--signature`")?,
    })
}

fn verify_updater_signature(
    request: &UpdaterSignatureRequest,
    encoded_public_key: &str,
) -> Result<()> {
    let public_key_text = decode_updater_box(encoded_public_key, "updater public key")?;
    let public_key =
        MinisignPublicKey::decode(&public_key_text).context("decoding the updater public key")?;
    let encoded_signature = read_small_regular_utf8(
        &request.signature,
        MAX_UPDATER_KEY_OR_SIGNATURE_BYTES,
        "updater signature",
    )?;
    let signature_text = decode_updater_box(&encoded_signature, "updater signature")?;
    let signature =
        MinisignSignature::decode(&signature_text).context("decoding the updater signature")?;

    let metadata = fs::symlink_metadata(&request.artifact).with_context(|| {
        format!(
            "inspecting updater artifact `{}`",
            request.artifact.display()
        )
    })?;
    ensure!(
        metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
        "updater artifact must be a regular file"
    );
    let mut artifact = File::open(&request.artifact)
        .with_context(|| format!("opening updater artifact `{}`", request.artifact.display()))?;
    let mut verifier = public_key
        .verify_stream(&signature)
        .context("preparing updater signature verification")?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = artifact
            .read(&mut buffer)
            .context("reading updater artifact for signature verification")?;
        if read == 0 {
            break;
        }
        verifier.update(&buffer[..read]);
    }
    verifier
        .finalize()
        .context("updater signature does not match the artifact")?;
    Ok(())
}

fn decode_updater_box(encoded: &str, label: &str) -> Result<String> {
    ensure!(
        encoded.len() as u64 <= MAX_UPDATER_KEY_OR_SIGNATURE_BYTES,
        "{label} exceeds the size limit"
    );
    let decoded = BASE64_STANDARD
        .decode(encoded.trim())
        .with_context(|| format!("decoding base64 {label}"))?;
    ensure!(
        decoded.len() as u64 <= MAX_UPDATER_KEY_OR_SIGNATURE_BYTES,
        "decoded {label} exceeds the size limit"
    );
    String::from_utf8(decoded).with_context(|| format!("{label} is not UTF-8"))
}

fn read_small_regular_utf8(path: &Path, limit: u64, label: &str) -> Result<String> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspecting {label} `{}`", path.display()))?;
    ensure!(
        metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
        "{label} must be a regular file"
    );
    ensure!(metadata.len() <= limit, "{label} exceeds the size limit");
    fs::read_to_string(path).with_context(|| format!("reading {label} `{}`", path.display()))
}

fn verify_windows_uninstaller() -> Result<()> {
    let root = workspace_root();
    let config = fs::read_to_string(root.join("packaging/windows/Packager.toml"))
        .context("reading the Windows packager configuration")?;
    let template = fs::read_to_string(root.join("packaging/windows/installer.nsi"))
        .context("reading the managed Windows NSIS template")?;
    let smoke = fs::read_to_string(root.join("packaging/smoke-install-windows.ps1"))
        .context("reading the Windows built-installer smoke matrix")?;
    let updater = fs::read_to_string(root.join("apps/desktop/src/updater.rs"))
        .context("reading the Windows updater implementation")?;
    verify_windows_installer_preflight_sources(&template)?;
    verify_windows_installer_smoke_sources(&smoke)?;
    verify_windows_uninstaller_sources(&config, &template)?;
    verify_windows_update_handoff_sources(&template, &updater)
}

fn powershell_function_range(source: &str, name: &str) -> Result<std::ops::Range<usize>> {
    let marker = format!("function {name}");
    let start = source
        .find(&marker)
        .with_context(|| format!("PowerShell source has no {name} function"))?;
    let body_start = start + marker.len();
    let remainder = &source[body_start..];
    let end = remainder
        .find("\nfunction ")
        .or_else(|| remainder.find("\ntry {"))
        .map(|offset| body_start + offset)
        .with_context(|| format!("PowerShell {name} function is not terminated"))?;
    Ok(start..end)
}

fn powershell_function<'a>(source: &'a str, name: &str) -> Result<&'a str> {
    let range = powershell_function_range(source, name)?;
    source
        .get(range)
        .with_context(|| format!("PowerShell {name} function offsets are invalid"))
}

struct PowerShellSourceViews {
    normalized: String,
    code: String,
}

fn powershell_source_views(source: &str) -> Result<PowerShellSourceViews> {
    #[derive(Clone, Copy)]
    enum State {
        Code,
        SingleQuoted,
        DoubleQuoted,
        LineComment,
        BlockComment,
    }

    fn push_masked(output: &mut Vec<u8>, byte: u8) {
        output.push(if byte == b'\n' { b'\n' } else { b' ' });
    }

    let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
    let bytes = normalized.as_bytes();
    let mut code = Vec::with_capacity(normalized.len());
    let mut state = State::Code;
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        let next = bytes.get(index + 1).copied();
        match state {
            State::Code => match (byte, next) {
                (b'@', Some(b'\'' | b'"')) => {
                    bail!("Windows built-installer smoke matrix forbids PowerShell here-strings")
                }
                (b'<', Some(b'#')) => {
                    code.extend_from_slice(b"  ");
                    state = State::BlockComment;
                    index += 2;
                }
                (b'#', _) => {
                    code.push(b' ');
                    state = State::LineComment;
                    index += 1;
                }
                (b'\'', _) => {
                    code.push(b' ');
                    state = State::SingleQuoted;
                    index += 1;
                }
                (b'"', _) => {
                    code.push(b' ');
                    state = State::DoubleQuoted;
                    index += 1;
                }
                (b'`', Some(escaped)) => {
                    code.push(byte);
                    code.push(escaped);
                    index += 2;
                }
                _ => {
                    code.push(byte);
                    index += 1;
                }
            },
            State::SingleQuoted => {
                push_masked(&mut code, byte);
                if byte == b'\'' {
                    if next == Some(b'\'') {
                        code.push(b' ');
                        index += 2;
                    } else {
                        state = State::Code;
                        index += 1;
                    }
                } else {
                    index += 1;
                }
            }
            State::DoubleQuoted => {
                push_masked(&mut code, byte);
                if byte == b'`' {
                    if let Some(escaped) = next {
                        push_masked(&mut code, escaped);
                        index += 2;
                    } else {
                        index += 1;
                    }
                } else {
                    if byte == b'"' {
                        state = State::Code;
                    }
                    index += 1;
                }
            }
            State::LineComment => {
                push_masked(&mut code, byte);
                if byte == b'\n' {
                    state = State::Code;
                }
                index += 1;
            }
            State::BlockComment => match (byte, next) {
                (b'#', Some(b'>')) => {
                    code.extend_from_slice(b"  ");
                    state = State::Code;
                    index += 2;
                }
                _ => {
                    push_masked(&mut code, byte);
                    index += 1;
                }
            },
        }
    }

    ensure!(
        matches!(state, State::Code | State::LineComment),
        "Windows built-installer smoke matrix contains an unterminated PowerShell string or comment"
    );
    let code = String::from_utf8(code).context("masking PowerShell source")?;
    ensure!(
        code.len() == normalized.len(),
        "PowerShell source scanner did not preserve normalized offsets"
    );
    Ok(PowerShellSourceViews { normalized, code })
}

fn powershell_executable_exact(
    source: &PowerShellSourceViews,
    exact: &str,
    executable_marker: &str,
) -> bool {
    powershell_executable_exact_slices(
        source.normalized.as_str(),
        source.code.as_str(),
        exact,
        executable_marker,
    )
}

fn powershell_executable_exact_in_function(
    source: &PowerShellSourceViews,
    name: &str,
    exact: &str,
    executable_marker: &str,
) -> Result<bool> {
    let range = powershell_function_range(source.code.as_str(), name)?;
    let normalized = source
        .normalized
        .get(range.clone())
        .with_context(|| format!("PowerShell {name} normalized offsets are invalid"))?;
    let code = source
        .code
        .get(range)
        .with_context(|| format!("PowerShell {name} executable offsets are invalid"))?;
    Ok(powershell_executable_exact_slices(
        normalized,
        code,
        exact,
        executable_marker,
    ))
}

fn powershell_executable_exact_slices(
    normalized: &str,
    code: &str,
    exact: &str,
    executable_marker: &str,
) -> bool {
    let Some(marker_offset) = exact.find(executable_marker) else {
        return false;
    };
    normalized.match_indices(exact).any(|(offset, _)| {
        code.get(offset + marker_offset..)
            .is_some_and(|candidate| candidate.starts_with(executable_marker))
    })
}

fn verify_powershell_mutation_process_guards(code: &str) -> Result<()> {
    let mutation_starts = [
        "Start-Process ",
        "[IO.File]::WriteAllText(",
        "New-Item ",
        "New-ItemProperty ",
        "Remove-Item ",
        "Remove-ItemProperty ",
        "Set-ItemProperty ",
        "Remove-AirWikiWindowsStagingPath `",
    ];
    let lines: Vec<_> = code.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if mutation_starts.iter().any(|mutation| {
            trimmed.starts_with(mutation) || trimmed.contains(&format!("= {mutation}"))
        }) {
            let previous = index.checked_sub(1).and_then(|prior| lines.get(prior));
            ensure!(
                previous.is_some_and(|prior| prior
                    .trim_start()
                    .starts_with("Assert-MutationProcessPrecondition ")),
                "Windows built-installer smoke matrix requires an immediate exact-path process precondition before every installer, uninstaller, file, and registry mutation"
            );
        }
    }
    Ok(())
}

fn verify_windows_installer_smoke_sources(smoke: &str) -> Result<()> {
    let source = powershell_source_views(smoke)?;
    let smoke = source.normalized.as_str();
    let code = source.code.as_str();
    verify_powershell_mutation_process_guards(code)?;

    let mutation_guard = powershell_function(code, "Assert-MutationProcessPrecondition")?;
    let no_process_guard = powershell_function(code, "Assert-NoDesktopProcess")?;
    ensure!(
        mutation_guard
            .matches("Assert-NoDesktopProcess $CaseId")
            .count()
            == 1
            && no_process_guard
                .contains("@(Get-DesktopProcessesAtExactPath $DesktopExecutable).Count -ne 0"),
        "Windows built-installer smoke matrix reusable mutation guard must fail closed on every process at the exact installed desktop path"
    );

    for executable_marker in [
        "Assert-WindowsClientInstallerGateHost",
        "Invoke-Installer",
        "Invoke-RejectedInstaller",
        "Assert-CleanRejection",
        "New-OwnedWixFixture",
        "Remove-OwnedRegistryFixture",
        "Assert-RejectedInstallerPreservedState",
        "ProductType -ne 1",
    ] {
        ensure!(
            code.contains(executable_marker),
            "Windows built-installer smoke matrix is missing executable case `{executable_marker}`"
        );
    }

    for forbidden_gate in [
        "$env:CI",
        "$env:GITHUB_ACTIONS",
        "$env:RUNNER_ENVIRONMENT",
        "$env:RUNNER_TEMP",
        "GitHub-hosted",
    ] {
        ensure!(
            !smoke.contains(forbidden_gate),
            "Windows built-installer smoke matrix must not depend on CI identity or hosted-runner state"
        );
    }

    let host = powershell_function(code, "Assert-WindowsClientInstallerGateHost")?;
    ensure!(
        host.contains("$AuthorizeDestructiveClientInstallerGate")
            && host.contains("Win32_OperatingSystem")
            && host.contains("Win32_Processor")
            && host.contains("ProductType -ne 1")
            && host.contains("[version]$Os.Version -lt [version]")
            && host.contains("Architecture -ne 9")
            && host.contains("[Environment]::Is64BitProcess")
            && host.contains("WindowsBuiltInRole]::Administrator"),
        "Windows built-installer smoke matrix must require authorization, client Windows 10, elevation, native AMD64, and a 64-bit process"
    );
    ensure!(
        powershell_executable_exact_in_function(
            &source,
            "Assert-WindowsClientInstallerGateHost",
            r#"if ([int]$Os.ProductType -ne 1 -or [version]$Os.Version -lt [version]"10.0") {"#,
            "if ([int]$Os.ProductType -ne 1 -or [version]$Os.Version -lt [version]",
        )?,
        "Windows built-installer smoke matrix must anchor the active Windows 10 literal in the executable client host gate"
    );

    let host_call_marker = "$ClientOsVersion = Assert-WindowsClientInstallerGateHost";
    ensure!(
        code.matches(host_call_marker).count() == 1,
        "Windows built-installer smoke matrix must execute the host gate exactly once"
    );
    let host_call = code
        .find(host_call_marker)
        .context("Windows built-installer smoke matrix has no host-gate call")?;
    let owner_write = code
        .find("[IO.File]::WriteAllText($OwnerMarker, $OwnerToken")
        .context("Windows built-installer smoke matrix has no owner-marker write")?;
    let first_installer = code
        .find("Assert-CleanRejection @(")
        .context("Windows built-installer smoke matrix has no first installer case")?;
    let first_registry_fixture = code
        .find("$WixOne = New-OwnedWixFixture")
        .context("Windows built-installer smoke matrix has no first registry fixture")?;
    ensure!(
        host_call < owner_write
            && owner_write < first_installer
            && host_call < first_registry_fixture
            && powershell_executable_exact(
                &source,
                "    try {\n        $CurrentCase = \"owner-marker\"\n        Assert-MutationProcessPrecondition $CurrentCase\n        [IO.File]::WriteAllText($OwnerMarker, $OwnerToken, [Text.UTF8Encoding]::new($false))",
                "[IO.File]::WriteAllText($OwnerMarker, $OwnerToken",
            )
            && code
                .contains("$ProgramDataRoot = (Resolve-Path -LiteralPath $env:ProgramData).Path")
            && code.contains("[IO.Path]::IsPathRooted($InstallDir)")
            && code.contains("$InstallDir -match"),
        "Windows built-installer smoke matrix must gate the host before any owned state under resolved ProgramData"
    );
    ensure!(
        powershell_executable_exact(
            &source,
            r#"if (-not [IO.Path]::IsPathRooted($InstallDir) -or $InstallDir -match '\s') {"#,
            "if (-not [IO.Path]::IsPathRooted($InstallDir) -or $InstallDir -match",
        ),
        "Windows built-installer smoke matrix must anchor the active whitespace regex in the executable install-path gate"
    );

    let invoke = powershell_function(code, "Invoke-Installer")?;
    ensure!(
        code.matches("$Process = Start-Process -FilePath $Installer")
            .count()
            == 1
            && invoke.contains("Assert-InstallerArguments $Arguments")
            && invoke.contains("Assert-MutationProcessPrecondition $CaseId")
            && invoke.contains("$Process.WaitForExit($InstallerWaitMilliseconds)")
            && invoke.contains("$Process.WaitForExit($ProcessCleanupWaitMilliseconds)")
            && invoke.contains("$Process.Kill()")
            && invoke.contains("$ExitCode = $Process.ExitCode")
            && invoke.contains("$Process.Dispose()")
            && invoke.contains("$ExitCode -ne $ExpectedExit")
            && !invoke.contains("-Wait")
            && !invoke.contains("$Process.WaitForExit()"),
        "Windows built-installer smoke matrix must use one bounded direct installer wait, exact exit code, retained timeout cleanup, and disposal"
    );
    let argument_gate = powershell_function(code, "Assert-InstallerArguments")?;
    ensure!(
        argument_gate.contains("$Arguments[$Arguments.Count - 1] -cne")
            && !smoke.contains("\"/D=$InstallDir\","),
        "Windows built-installer smoke matrix must keep /D as the final installer argument"
    );
    ensure!(
        powershell_executable_exact_in_function(
            &source,
            "Assert-InstallerArguments",
            "if ($Arguments.Count -eq 0 -or\n        $Arguments[$Arguments.Count - 1] -cne \"/D=$InstallDir\") {",
            "if ($Arguments.Count -eq 0 -or",
        )?,
        "Windows built-installer smoke matrix must anchor the active final /D literal in the executable installer-argument gate"
    );

    let rejected_invoke = powershell_function(code, "Invoke-RejectedInstaller")?;
    let observer_start = powershell_function(code, "Start-RejectedDesktopProcessObserver")?;
    let observer_read = powershell_function(code, "Get-RejectedDesktopProcessObserverEvents")?;
    let observer_stop = powershell_function(code, "Stop-RejectedDesktopProcessObserver")?;
    ensure!(
        rejected_invoke.contains("$Observer = Start-RejectedDesktopProcessObserver")
            && rejected_invoke.contains("Invoke-Installer $Arguments $ExpectedExit $CaseId")
            && rejected_invoke.contains("Assert-RejectedDesktopProcessObserverClear $Observer")
            && rejected_invoke.contains("finally {")
            && rejected_invoke.contains("Stop-RejectedDesktopProcessObserver $Observer")
            && observer_start.contains("[System.Management.ManagementEventWatcher]::new($Query)")
            && observer_start.contains("Register-ObjectEvent")
            && observer_start.contains("-ErrorAction Stop | Out-Null")
            && observer_start.contains("$Subscriptions = @(Get-EventSubscriber -ErrorAction Stop")
            && observer_start.contains("$Subscriptions.Count -ne 1")
            && observer_start.contains("$Subscription = $Subscriptions[0]")
            && observer_start.contains("$Watcher.Start()")
            && observer_start.contains("catch {")
            && observer_start.contains("Unregister-Event -SubscriptionId")
            && observer_start.contains("$Watcher.Stop()")
            && observer_start.contains("$Watcher.Dispose()")
            && observer_read.contains("Get-EventSubscriber -ErrorAction Stop")
            && observer_read.contains("Get-Event -ErrorAction Stop")
            && observer_stop.contains("$Record.Watcher.Stop()")
            && observer_stop
                .contains("Wait-Event -SourceIdentifier $Record.SourceIdentifier -Timeout 1")
            && observer_stop.contains("Get-RejectedDesktopProcessObserverEvents $Record")
            && observer_stop.contains("Unregister-Event")
            && observer_stop.contains("Remove-Event")
            && observer_stop.contains("$Record.Watcher.Dispose()")
            && observer_stop.contains("$FinalEvents = @(Get-Event -ErrorAction Stop")
            && observer_stop
                .contains("if ($FinalEvents.Count -ne 0) {\n            $Observed = $true")
            && observer_stop.contains("$TeardownFailed")
            && observer_stop.contains("$Observed"),
        "Windows built-installer smoke matrix rejection cases require an OS-native process-start observer with checked setup, read, and finally teardown"
    );
    ensure!(
        powershell_executable_exact_in_function(
            &source,
            "Start-RejectedDesktopProcessObserver",
            "$Query = [System.Management.WqlEventQuery]::new(\n            \"SELECT * FROM Win32_ProcessStartTrace WHERE ProcessName = 'airwiki.exe'\"\n        )",
            "$Query = [System.Management.WqlEventQuery]::new(",
        )?,
        "Windows built-installer smoke matrix must anchor the exact active process-start WQL event class and airwiki.exe filter"
    );

    for rejection_helper in [
        "Assert-CleanRejection",
        "Assert-RejectedInstallerPreservedState",
    ] {
        let helper = powershell_function(code, rejection_helper)?;
        ensure!(
            helper
                .matches("Invoke-RejectedInstaller $Arguments 2 $CaseId")
                .count()
                == 1
                && helper.matches("Assert-NoDesktopProcess $CaseId").count() == 2
                && !helper.contains("Invoke-Installer ")
                && !helper.contains("Start-Process"),
            "Windows built-installer smoke matrix must require exit 2 through the process-start observer plus before/after exact-path checks in {rejection_helper}"
        );
    }

    ensure!(
        powershell_executable_exact(
            &source,
            "Assert-CleanRejection @(\"/P\", \"/R\", \"/AIRWIKIUPDATE\", \"/NS\", \"/D=$InstallDir\") \"clean /AIRWIKIUPDATE rejection\"",
            "Assert-CleanRejection",
        ) && powershell_executable_exact(
            &source,
            "Assert-CleanRejection @(\"/P\", \"/R\", \"/AIRWIKIUPDATE\", \"/NS\", \"/D=$InstallDir\") \"single WiX /AIRWIKIUPDATE rejection\"",
            "Assert-CleanRejection",
        ) && powershell_executable_exact(
            &source,
            "Assert-RejectedInstallerPreservedState `\n                @(\"/R\", \"/AIRWIKIUPDATE\", \"/NS\", \"/D=$InstallDir\") `\n                \"/AIRWIKIUPDATE requires /P\"",
            "Assert-RejectedInstallerPreservedState",
        ) && powershell_executable_exact(
            &source,
            "Assert-RejectedInstallerPreservedState `\n                @(\"/P\", \"/R\", \"/AIRWIKIUPDATE\", \"/NS\", \"/D=$InstallDir\") `\n                \"/AIRWIKIUPDATE rejects same-version replay\"",
            "Assert-RejectedInstallerPreservedState",
        ) && !smoke.contains("@(\"/S\", \"/P\", \"/AIRWIKIUPDATE\""),
        "Windows built-installer smoke matrix must use the exact /P /R /AIRWIKIUPDATE handoff"
    );
    ensure!(
        powershell_executable_exact(
            &source,
            "$WixTwo = New-OwnedWixFixture \"two\" \"0.1.8\"",
            "$WixTwo = New-OwnedWixFixture",
        ),
        "Windows built-installer smoke matrix must create a second WiX key for multiple WiX rejection"
    );
    ensure!(
        code.contains(
            "Remove-ItemProperty -LiteralPath $UninstallRegistryPath -Name $RequiredName"
        ),
        "Windows built-installer smoke matrix must execute each partial NSIS key mutation"
    );

    let fingerprint = powershell_function(code, "Get-RegistryFingerprint")?;
    ensure!(
        fingerprint.contains("$Subkeys = @($Key.GetSubKeyNames() | Sort-Object)")
            && fingerprint.contains("$Key.GetValueNames() | Sort-Object")
            && fingerprint.contains("kind = [string]$Key.GetValueKind($Name)")
            && fingerprint.contains("subkeys = @($Subkeys); values = @($Values)"),
        "Windows built-installer smoke matrix registry fingerprint must cover sorted structure, kinds, and values"
    );

    let marker = powershell_function(code, "Assert-OwnedRegistryMarker")?;
    ensure!(
        marker.contains("[string]$Record.Owner -cne $RegistryOwner")
            && marker.contains("$Key = Get-Item -LiteralPath $Record.Path -ErrorAction Stop")
            && marker.contains("$MarkerValue -cne $RegistryOwner")
            && marker.contains("$MarkerKind -ne [Microsoft.Win32.RegistryValueKind]::String"),
        "Windows built-installer smoke matrix requires an exact trusted typed owner marker for deletion authority"
    );
    ensure!(
        powershell_executable_exact_in_function(
            &source,
            "Assert-OwnedRegistryMarker",
            "$MarkerValue = [string]$Key.GetValue(\n        \"AirWikiTestOwner\",\n        $null,\n        [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames\n    )",
            "$MarkerValue = [string]$Key.GetValue(",
        )? && powershell_executable_exact_in_function(
            &source,
            "Assert-OwnedRegistryMarker",
            "$MarkerKind = $Key.GetValueKind(\"AirWikiTestOwner\")",
            "$MarkerKind = $Key.GetValueKind(",
        )?,
        "Windows built-installer smoke matrix must anchor both exact active owner marker reads"
    );

    let new_fixture = powershell_function(code, "New-OwnedWixFixture")?;
    let key_create = new_fixture
        .find("New-Item -Path $Path -ErrorAction Stop")
        .context("Windows built-installer smoke matrix does not create a fresh WiX key")?;
    let record = new_fixture
        .find("$Record = [pscustomobject]")
        .context("Windows built-installer smoke matrix does not create an ownership record")?;
    let registration = new_fixture
        .find("$OwnedRegistryFixtures.Add($Record) | Out-Null")
        .context("Windows built-installer smoke matrix does not register owned WiX state")?;
    let first_value = new_fixture
        .find("New-ItemProperty `")
        .context("Windows built-installer smoke matrix has no WiX owner marker")?;
    let marker_readback = new_fixture
        .find("Assert-OwnedRegistryMarker $Record")
        .context("Windows built-installer smoke matrix does not read back the WiX owner marker")?;
    let marker_trust = new_fixture
        .find("$Record.MarkerTrusted = $true")
        .context("Windows built-installer smoke matrix does not trust the verified marker")?;
    let first_fingerprint = new_fixture
        .find("$Record.Fingerprint = Get-RegistryFingerprint $Path")
        .context("Windows built-installer smoke matrix does not fingerprint the trusted key")?;
    ensure!(
        key_create < record
            && record < registration
            && registration < first_value
            && first_value < marker_readback
            && marker_readback < marker_trust
            && marker_trust < first_fingerprint
            && !new_fixture.contains("New-Item -Path $Path -Force")
            && new_fixture.contains("MarkerTrusted = $false")
            && new_fixture.contains("Fingerprint = $null")
            && new_fixture
                .matches("$Record.Fingerprint = Get-RegistryFingerprint $Path")
                .count()
                == 5,
        "Windows built-installer smoke matrix must register a fresh key without deletion authority, verify the typed marker, then refresh its trusted fingerprint after every mutation"
    );
    ensure!(
        powershell_executable_exact_in_function(
            &source,
            "New-OwnedWixFixture",
            "New-ItemProperty `\n        -LiteralPath $Path `\n        -Name AirWikiTestOwner `\n        -Value $RegistryOwner `\n        -PropertyType String | Out-Null",
            "New-ItemProperty `",
        )?,
        "Windows built-installer smoke matrix must anchor the exact active owner marker write with REG_SZ property type"
    );
    for mutation in [
        "New-ItemProperty -LiteralPath $Path -Name DisplayName -Value \"AirWiki\" | Out-Null\n    $Record.Fingerprint = Get-RegistryFingerprint $Path",
        "New-ItemProperty -LiteralPath $Path -Name Publisher -Value $ExpectedPublisher | Out-Null\n    $Record.Fingerprint = Get-RegistryFingerprint $Path",
        "New-ItemProperty -LiteralPath $Path -Name DisplayVersion -Value $Version | Out-Null\n    $Record.Fingerprint = Get-RegistryFingerprint $Path",
        "New-ItemProperty -LiteralPath $Path -Name UninstallString -Value \"msiexec.exe /x {$RegistryOwner}\" | Out-Null\n    $Record.Fingerprint = Get-RegistryFingerprint $Path",
    ] {
        ensure!(
            powershell_executable_exact(&source, mutation, "New-ItemProperty"),
            "Windows built-installer smoke matrix must refresh the exact fingerprint after each fixture mutation"
        );
    }

    let remove_fixture = powershell_function(code, "Remove-OwnedRegistryFixture")?;
    let owned_record_count = remove_fixture
        .find("$OwnedRecordCount = @($OwnedRegistryFixtures | Where-Object {")
        .context("Windows built-installer smoke matrix has no owned-record count")?;
    let owned_path_count = remove_fixture
        .find("$OwnedPathCount = @($OwnedRegistryFixtures | Where-Object {")
        .context("Windows built-installer smoke matrix has no owned-path count")?;
    let unique_ownership_gate = remove_fixture
        .find("$OwnedRecordCount -ne 1 -or $OwnedPathCount -ne 1")
        .context("Windows built-installer smoke matrix has no unique-ownership gate")?;
    let fingerprint_gate = remove_fixture
        .find("(Get-RegistryFingerprint $Record.Path) -cne $Record.Fingerprint")
        .context("Windows built-installer smoke matrix has no fixture fingerprint gate")?;
    let fixture_delete = remove_fixture
        .find("Remove-Item -LiteralPath $Record.Path -Recurse -Force")
        .context("Windows built-installer smoke matrix has no owned fixture removal")?;
    let trusted_delete = "    if ((Get-RegistryFingerprint $Record.Path) -cne $Record.Fingerprint) {\n        throw \"owned registry fixture changed; refusing deletion\"\n    }\n    Assert-MutationProcessPrecondition $CurrentCase\n    Remove-Item -LiteralPath $Record.Path -Recurse -Force";
    ensure!(
        owned_record_count < unique_ownership_gate
            && owned_path_count < unique_ownership_gate
            && unique_ownership_gate < fingerprint_gate
            && fingerprint_gate < fixture_delete
            && remove_fixture
                .contains("$Record.MarkerTrusted -ne $true -or $null -eq $Record.Fingerprint")
            && remove_fixture
                .find("Assert-OwnedRegistryMarker $Record")
                .is_some_and(|owner_gate| unique_ownership_gate < owner_gate
                    && owner_gate < fingerprint_gate)
            && remove_fixture
                .matches("[object]::ReferenceEquals($_, $Record)")
                .count()
                == 1
            && remove_fixture
                .matches("[string]$_.Path -ceq [string]$Record.Path")
                .count()
                == 1
            && powershell_executable_exact(
                &source,
                trusted_delete,
                "(Get-RegistryFingerprint $Record.Path)",
            ),
        "Windows built-installer smoke matrix must prove one owned record, one path, and the trusted typed owner marker immediately before exact fixture deletion authority"
    );

    let cleanup = powershell_function(code, "Invoke-CleanupStep")?;
    ensure!(
        cleanup.contains("catch {")
            && cleanup.contains("$CleanupFailures.Add($Label) | Out-Null")
            && !cleanup.contains("Exception")
            && !cleanup.contains("Write-Error")
            && powershell_executable_exact(
                &source,
                "foreach ($Record in @($OwnedRegistryFixtures)) {\n            Invoke-CleanupStep \"owned registry fixture\" { Remove-OwnedRegistryFixture $Record }\n        }",
                "Invoke-CleanupStep",
            )
            && powershell_executable_exact(
                &source,
                "Invoke-CleanupStep \"desktop-process-check\" {\n            if ($null -ne $OwnedDesktopProcess -or\n                @(Get-DesktopProcessesAtExactPath $DesktopExecutable).Count -ne 0)",
                "Invoke-CleanupStep",
            )
            && powershell_executable_exact(
                &source,
                "Invoke-CleanupStep \"owned-desktop-process\" {\n            Stop-RegisteredDesktopProcess $DesktopExecutable\n        }",
                "Invoke-CleanupStep",
            )
            && code
                .contains("if ($InstallCleanupAllowed -and\n            $UninstallerVerified -and")
            && code.contains("if ($InstallCleanupAllowed -and $OwnerMarkerCreated)")
            && code.contains("$OwnedRegistryFixtures.Count -ne 0")
            && code.contains("$CleanupFailures.Count -ne 0"),
        "Windows built-installer smoke matrix must attempt independent cleanup, require exact process exit, and accumulate stable categories"
    );

    ensure!(
        powershell_executable_exact(
            &source,
            "        } finally {\n            Restore-VerifiedInstall \"same-version silent repair recovery\"\n        }",
            "Restore-VerifiedInstall",
        ) && powershell_executable_exact(
            &source,
            "        } finally {\n            Restore-VerifiedInstall \"strictly newer silent upgrade recovery\"\n        }",
            "Restore-VerifiedInstall",
        ) && powershell_executable_exact(
            &source,
            "        } finally {\n            Restore-VerifiedInstall \"silent downgrade rejection recovery\"\n        }\n\n        $CoexistingWix",
            "Restore-VerifiedInstall",
        ) && powershell_executable_exact(
            &source,
            "            } finally {\n                Assert-MutationProcessPrecondition $CurrentCase\n                Set-ItemProperty -LiteralPath $UninstallRegistryPath -Name $RequiredName -Value $RequiredValue\n                Restore-VerifiedInstall \"partial NSIS key rejection recovery: $RequiredName\"\n            }",
            "Set-ItemProperty",
        ) && [
            "Restore-VerifiedInstall \"WiX plus NSIS coexistence rejection recovery\"",
            "Restore-VerifiedInstall \"/AIRWIKIUPDATE same-version replay recovery\"",
            "Restore-VerifiedInstall \"/AIRWIKIUPDATE requires /P recovery\"",
            "Restore-VerifiedInstall \"invalid installed version rejection recovery\"",
        ]
        .iter()
        .all(|recovery| powershell_executable_exact(
            &source,
            recovery,
            "Restore-VerifiedInstall",
        )),
        "Windows built-installer smoke matrix must restore and reverify every temporary rejection fixture in finally"
    );

    let wait_process = powershell_function(code, "Wait-OwnedDesktopProcess")?;
    let stop_process = powershell_function(code, "Stop-OwnedDesktopProcess")?;
    let stop_registered = powershell_function(code, "Stop-RegisteredDesktopProcess")?;
    ensure!(
        wait_process.contains("[System.Diagnostics.Process]::GetProcessById(")
            && wait_process.contains("$SafeHandle = $Process.SafeHandle")
            && wait_process.contains("$SafeHandle.IsInvalid")
            && wait_process.contains("$SafeHandle.IsClosed")
            && wait_process.contains("$Process.MainModule.FileName")
            && wait_process
                .contains("StartTimeUtcTicks = $Process.StartTime.ToUniversalTime().Ticks")
            && wait_process.contains("Process = $Process")
            && wait_process.contains("SafeHandle = $SafeHandle")
            && wait_process.contains("ProcessId = $Process.Id")
            && wait_process.contains("$script:OwnedDesktopProcess = $Record")
            && wait_process.contains("$Process.Dispose()")
            && stop_process.contains("$Process = $Record.Process")
            && stop_process
                .contains("[object]::ReferenceEquals($Record.SafeHandle, $Process.SafeHandle)")
            && stop_process.contains("$Process.MainModule.FileName")
            && stop_process.contains(
                "$ActualPath.Equals(\n            $Record.ExecutablePath,\n            [StringComparison]::OrdinalIgnoreCase\n        )"
            )
            && stop_process.contains("$ActualStartTimeUtcTicks -ne $Record.StartTimeUtcTicks")
            && stop_process.contains("$Process.Kill()")
            && stop_process.contains("$Process.WaitForExit($ProcessCleanupWaitMilliseconds)")
            && stop_process.contains("finally {")
            && stop_process.contains("$Process.Dispose()")
            && stop_process.contains("$Record.Process = $null")
            && stop_process.contains("$Record.SafeHandle = $null")
            && !stop_process.contains("GetProcessById")
            && !stop_process.contains("Get-CimInstance")
            && code
                .matches("[System.Diagnostics.Process]::GetProcessById(")
                .count()
                == 1
            && !code.contains("Get-Process -Name")
            && !code.contains("Stop-Process")
            && powershell_executable_exact(
                &source,
                "$OwnedDesktopProcess = Wait-OwnedDesktopProcess $DesktopExecutable\n            Assert-InstalledRelease \"/AIRWIKIUPDATE strictly newer install verification\"",
                "$OwnedDesktopProcess = Wait-OwnedDesktopProcess",
            )
            && stop_registered.contains("$null -ne $OwnedDesktopProcess")
            && stop_registered.contains("$Record = $OwnedDesktopProcess")
            && stop_registered.contains("$Record.ExecutablePath.Equals(")
            && stop_registered
                .matches("Stop-OwnedDesktopProcess $Record")
                .count()
                == 1
            && stop_registered.contains("finally {")
            && stop_registered.contains("$Record.Process.Dispose()")
            && stop_registered.contains("$Record.Process = $null")
            && stop_registered.contains("$Record.SafeHandle = $null")
            && stop_registered
                .contains("@(Get-DesktopProcessesAtExactPath $ExpectedPath).Count -ne 0")
            && !stop_registered.contains("foreach ("),
        "Windows built-installer smoke matrix must own, revalidate, and stop only the exact restarted PID and path"
    );
    ensure!(
        powershell_executable_exact(
            &source,
            "Assert-NoDesktopProcess \"/AIRWIKIUPDATE strictly newer install\"\n        $UpdaterRestartCleanupAllowed = $false\n        try {\n            Set-InstalledVersionFixture \"0.1.9\"",
            "Assert-NoDesktopProcess",
        ) && powershell_executable_exact(
            &source,
            "Invoke-Installer `\n                @(\"/P\", \"/R\", \"/AIRWIKIUPDATE\", \"/NS\", \"/D=$InstallDir\") `\n                0 `\n                \"/AIRWIKIUPDATE strictly newer install\"\n            $OwnedDesktopProcess = Wait-OwnedDesktopProcess $DesktopExecutable",
            "Invoke-Installer",
        ) && powershell_executable_exact(
            &source,
            "        } finally {\n            Invoke-CleanupStep \"updater-restart-process\" {\n                Stop-RegisteredDesktopProcess $DesktopExecutable\n                $script:UpdaterRestartCleanupAllowed = $true\n            }\n            if ($UpdaterRestartCleanupAllowed) {\n                Invoke-CleanupStep \"updater-install-recovery\" {\n                    Restore-VerifiedInstall \"/AIRWIKIUPDATE strictly newer install recovery\"\n                }\n            }\n        }",
            "Invoke-CleanupStep",
        ),
        "Windows built-installer smoke matrix must check the restart precondition, supervise the exact updater handoff, and recover without broad process ownership"
    );

    ensure!(
        powershell_executable_exact(
            &source,
            "[Console]::Out.WriteLine(\n        \"WINDOWS_INSTALLER_MATRIX_PASS os_version=$ClientOsVersion installer_sha256=$InstallerHash transition_matrix=pass uninstaller=pass owned_residue=0\"\n    )",
            "[Console]::Out.WriteLine(",
        ) && powershell_executable_exact(
            &source,
            "[Console]::Error.WriteLine(\"WINDOWS_INSTALLER_MATRIX_FAIL case=$CurrentCase\")",
            "[Console]::Error.WriteLine(",
        ) && code.contains("exit 1")
            && !code.contains("Write-Error $_")
            && !code.contains("$_.Exception")
            && !code.contains("Exception.Message"),
        "Windows built-installer smoke matrix must emit only fixed sanitized success or failure evidence"
    );

    Ok(())
}

fn nsis_function<'a>(template: &'a str, name: &str) -> Result<&'a str> {
    let marker = format!("Function {name}");
    let start = template
        .find(&marker)
        .with_context(|| format!("NSIS template has no {name} function"))?;
    let end = template[start..]
        .find("FunctionEnd")
        .map(|offset| start + offset)
        .with_context(|| format!("NSIS {name} function is not terminated"))?;
    Ok(&template[start..end])
}

fn verify_windows_installer_preflight_sources(template: &str) -> Result<()> {
    ensure!(
        template.contains("!include WinVer.nsh"),
        "NSIS preflight must include WinVer.nsh for the Windows 10 gate"
    );
    ensure!(
        template.contains(
            "!if \"${INSTALLMODE}\" != \"currentUser\"\n  !error \"AirWiki 0.2.0 supports only currentUser Windows installs.\"\n!endif"
        ),
        "NSIS template must reject non-currentUser modes at compile time"
    );
    ensure!(
        template.contains(
            "!if \"${ALLOWDOWNGRADES}\" != \"false\"\n  !error \"AirWiki 0.2.0 does not support Windows downgrades.\"\n!endif"
        ),
        "NSIS template must reject ALLOWDOWNGRADES=true at compile time"
    );
    let platform = nsis_function(template, "EnforceSupportedWindows")?;
    ensure!(
        platform.matches("Call RejectUnsupportedPlatform").count() == 3
            && !platform.contains("SetErrorLevel")
            && !platform.contains("Abort"),
        "all three platform failures must route through the shared rejection helper"
    );
    ensure!(
        platform.contains(
            "  ${IfNot} ${AtLeastWin10}\n    StrCpy $PlatformRejectionMessage \"$(UnsupportedWindowsVersion)\"\n    Call RejectUnsupportedPlatform\n  ${EndIf}"
        ),
        "NSIS preflight must reject hosts older than Windows 10"
    );
    ensure!(
        platform.contains(
            "  ${If} ${IsServerOS}\n    StrCpy $PlatformRejectionMessage \"$(UnsupportedWindowsServer)\"\n    Call RejectUnsupportedPlatform\n  ${EndIf}"
        ),
        "NSIS preflight must reject Windows Server"
    );
    ensure!(
        platform.contains(
            "  ${IfNot} ${IsNativeAMD64}\n    StrCpy $PlatformRejectionMessage \"$(UnsupportedWindowsArchitecture)\"\n    Call RejectUnsupportedPlatform\n  ${EndIf}"
        ) && !platform.contains("${RunningX64}"),
        "NSIS preflight must require native AMD64 rather than emulation"
    );
    ensure!(
        template.contains("LangString UnsupportedWindowsVersion ${LANG_ENGLISH}")
            && template.contains("LangString UnsupportedWindowsVersion ${LANG_SPANISH}")
            && template.contains("LangString UnsupportedWindowsServer ${LANG_ENGLISH}")
            && template.contains("LangString UnsupportedWindowsServer ${LANG_SPANISH}")
            && template.contains("LangString UnsupportedWindowsArchitecture ${LANG_ENGLISH}")
            && template.contains("LangString UnsupportedWindowsArchitecture ${LANG_SPANISH}"),
        "unsupported-platform messages must exist in English and Spanish"
    );
    let reject = nsis_function(template, "RejectUnsupportedPlatform")?;
    let silent_jump = reject
        .find("IfSilent platform_reject_abort")
        .context("platform rejection must omit MessageBox only in silent mode")?;
    let message_box = reject
        .find("MessageBox MB_OK|MB_ICONSTOP \"$PlatformRejectionMessage\"")
        .context("interactive platform rejection needs a blocking localized MessageBox")?;
    let abort_label = reject
        .find("platform_reject_abort:")
        .context("silent platform rejection needs a shared abort label")?;
    let exit_status = reject
        .find("SetErrorLevel 2")
        .context("platform rejection must set exit code 2")?;
    let abort = reject
        .find("Abort")
        .context("platform rejection must abort")?;
    ensure!(
        silent_jump < message_box
            && message_box < abort_label
            && abort_label < exit_status
            && exit_status < abort,
        "platform rejection must omit MessageBox only in silent mode and always set exit code 2 before Abort"
    );

    let classify = nsis_function(template, "ClassifyExistingInstallation")?;
    ensure!(
        template.contains("!define VERSION_SENTINEL \"__airwiki_invalid_semver__\"")
            && classify.contains(
                "nsis_tauri_utils::SemverCompare \"$InstalledVersion\" \"${VERSION_SENTINEL}\""
            )
            && classify
                .contains("nsis_tauri_utils::SemverCompare \"${VERSION}\" \"$InstalledVersion\""),
        "NSIS preflight must use the fixed invalid SemVer sentinel before comparison"
    );
    ensure!(
        classify.contains("IntOp $WixMetadataCount $WixMetadataCount + 1"),
        "NSIS classifier must scan all WiX candidates"
    );
    ensure!(
        classify.contains("    ${If} $WixMetadataCount == 1\n      StrCpy $WixCandidateKey"),
        "NSIS classifier must retain the single WiX candidate"
    );
    ensure!(
        classify.contains(
            "    ${If} $NsisMetadataState == \"${NSIS_METADATA_PARTIAL}\"\n      Goto classify_reject"
        ),
        "NSIS classifier must reject a partial NSIS key"
    );
    ensure!(
        classify.contains("    ${If} $WixMetadataCount > 1\n      Goto classify_reject"),
        "NSIS classifier must reject multiple WiX entries"
    );
    ensure!(
        classify.contains(
            "    ${If} $WixMetadataCount == 1\n      ${If} $NsisMetadataState != \"${NSIS_METADATA_ABSENT}\""
        ),
        "NSIS classifier must reject WiX and NSIS coexistence"
    );

    let init = nsis_function(template, ".onInit")?;
    let first_action = init
        .strip_prefix("Function .onInit")
        .context("NSIS .onInit declaration is malformed")?
        .trim_start();
    ensure!(
        first_action.starts_with("Call EnforceSupportedWindows"),
        "supported-platform enforcement must be the first executable .onInit action"
    );
    let platform_call = init
        .find("Call EnforceSupportedWindows")
        .context("NSIS must enforce the supported platform from .onInit")?;
    let option_parse = init
        .find("${GetOptions} $CMDLINE \"/P\"")
        .context("NSIS must parse passive mode after the platform gate")?;
    let context = init
        .find("!insertmacro SetContext")
        .context("NSIS must select its fixed registry context")?;
    let classify_call = init
        .find("Call ClassifyExistingInstallation")
        .context("NSIS must classify before every installer page")?;
    let policy_call = init
        .find("Call EnforceInstallPolicy")
        .context("NSIS must enforce install policy from .onInit")?;
    let language = init
        .find("!insertmacro MUI_LANGDLL_DISPLAY")
        .context("NSIS language selector marker is missing")?;
    let restore = init
        .find("Call RestorePreviousInstallLocation")
        .context("NSIS install-location restoration marker is missing")?;
    ensure!(
        platform_call < option_parse
            && option_parse < context
            && context < classify_call
            && classify_call < policy_call
            && policy_call < language
            && policy_call < restore,
        "NSIS platform and version policy must run before the language selector and every write"
    );
    ensure!(
        !nsis_function(template, "PageReinstall")?.contains("nsis_tauri_utils::SemverCompare"),
        "NSIS reinstall presentation must consume, not calculate, the version relation"
    );
    let policy = nsis_function(template, "EnforceInstallPolicy")?;
    ensure!(
        policy.contains(
            "  ${If} $InstallVersionRelation == \"${RELATION_OLDER}\"\n    SetErrorLevel 2\n    Abort\n  ${EndIf}"
        ),
        "NSIS preflight must reject every downgrade before installer sections"
    );
    let page_leave = nsis_function(template, "PageLeaveReinstall")?;
    ensure!(
        policy.contains("$ExistingInstallKind == \"wix\"")
            && policy.contains("$SilentMode == 1")
            && policy.contains("$PassiveMode == 1")
            && policy.contains("$UpdaterMode == 1")
            && page_leave.contains("$ExistingInstallKind == \"wix\""),
        "WiX migration must be interactive, explicitly confirmed, and unavailable to /S, /P, or /AIRWIKIUPDATE"
    );
    let wix_dispatch = page_leave
        .find(
            "  ${If} $ExistingInstallKind == \"wix\"\n    ${If} $R1 != ${BST_CHECKED}\n      Abort\n    ${EndIf}\n    Goto reinst_uninstall\n  ${EndIf}",
        )
        .context("WiX migration requires an explicitly checked migration choice")?;
    let legacy_dispatch = page_leave
        .find("  StrCmp $R5 \"1\"")
        .context("NSIS reinstall page has no legacy version dispatcher")?;
    ensure!(
        page_leave[..wix_dispatch].trim_end()
            == "Function PageLeaveReinstall\n  ${NSD_GetState} $R2 $R1"
            && wix_dispatch < legacy_dispatch,
        "WiX migration must dispatch before the legacy dispatcher"
    );
    let wix_exec = page_leave
        .find("      ExecWait '$R1' $0")
        .context("WiX uninstall may execute only from PageLeaveReinstall")?;
    let hide = page_leave
        .find("    HideWindow")
        .context("NSIS reinstall leave function does not hide the installer")?;
    ensure!(
        template.matches("ExecWait '$R1' $0").count() == 1
            && wix_dispatch < hide
            && hide < wix_exec,
        "WiX uninstall may execute only from PageLeaveReinstall after the explicitly checked migration choice"
    );
    Ok(())
}

fn verify_windows_update_handoff_sources(template: &str, updater: &str) -> Result<()> {
    const STRICT_UPDATER_BLOCK: &str = r#"  ${If} $UpdaterMode == 1
    ${If} $PassiveMode != 1
      SetErrorLevel 2
      Abort
    ${EndIf}
    ${If} $ExistingInstallKind != "nsis"
      SetErrorLevel 2
      Abort
    ${EndIf}
    ${If} $InstallVersionRelation != "${RELATION_NEWER}"
      SetErrorLevel 2
      Abort
    ${EndIf}
  ${EndIf}"#;
    const DOWNGRADE_REJECTION: &str = r#"  ${If} $InstallVersionRelation == "${RELATION_OLDER}"
    SetErrorLevel 2
    Abort
  ${EndIf}"#;

    ensure!(
        template.contains("Var UpdaterMode")
            && template.contains("StrCpy $UpdaterMode 0")
            && template.contains("${GetOptions} $CMDLINE \"/AIRWIKIUPDATE\" $UpdaterMode")
            && template.contains(STRICT_UPDATER_BLOCK),
        "NSIS in-app updates must require passive mode and a strictly newer embedded version"
    );
    ensure!(
        template.contains(DOWNGRADE_REJECTION),
        "NSIS must reject every downgrade before installer sections"
    );

    let wait_start = template
        .find("Function WaitForAirWikiUpdateShutdown")
        .context("NSIS template has no bounded updater shutdown wait")?;
    let wait_end = template[wait_start..]
        .find("FunctionEnd")
        .map(|offset| wait_start + offset)
        .context("NSIS updater shutdown wait is not terminated")?;
    let wait = &template[wait_start..wait_end];
    let update_option = wait
        .find("${GetOptions} $CMDLINE \"/AIRWIKIUPDATE\" $R0")
        .context("NSIS updater shutdown wait does not require /AIRWIKIUPDATE")?;
    let counter = wait
        .find("StrCpy $R1 0")
        .context("NSIS updater shutdown wait has no bounded counter")?;
    let process_check = wait
        .find("nsis_tauri_utils::FindProcess \"${MAINBINARYNAME}.exe\"")
        .context("NSIS updater shutdown wait does not observe the desktop process")?;
    let increment = wait
        .find("IntOp $R1 $R1 + 1")
        .context("NSIS updater shutdown wait does not advance its counter")?;
    let bound = wait
        .find("${If} $R1 >= 50")
        .context("NSIS updater shutdown wait is not bounded to 50 attempts")?;
    let sleep = wait
        .find("Sleep 100")
        .context("NSIS updater shutdown wait does not yield between attempts")?;
    ensure!(
        update_option < counter
            && counter < process_check
            && process_check < increment
            && increment < bound
            && bound < sleep,
        "NSIS updater shutdown wait must parse /AIRWIKIUPDATE then poll for at most five seconds"
    );

    let install_start = template
        .find("Section Install")
        .context("NSIS template has no install section")?;
    let install_end = template[install_start..]
        .find("SectionEnd")
        .map(|offset| install_start + offset)
        .context("NSIS install section is not terminated")?;
    let install = &template[install_start..install_end];
    let graceful_wait = install
        .find("Call WaitForAirWikiUpdateShutdown")
        .context("NSIS install section does not wait for a clean updater shutdown")?;
    let recovery = install
        .find("!insertmacro CheckIfAppIsRunning")
        .context("NSIS install section has no stuck-process recovery")?;
    ensure!(
        graceful_wait < recovery,
        "NSIS must wait for clean updater shutdown before stuck-process recovery"
    );

    ensure!(
        updater.contains(
            "const WINDOWS_INSTALLER_ARGS: [&str; 3] = [\"/P\", \"/R\", \"/AIRWIKIUPDATE\"]"
        ) && updater.contains("launch_locked_windows_process(&package, &WINDOWS_INSTALLER_ARGS)")
            && updater.contains("CreateProcessW(")
            && updater.contains("PROC_THREAD_ATTRIBUTE_HANDLE_LIST")
            && updater.contains("SetHandleInformation(")
            && updater.contains("package.preserve_after_launch()"),
        "Windows updater must launch the locked NSIS directly with inherited guards and /P /R /AIRWIKIUPDATE"
    );
    ensure!(
        updater.contains(".custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)")
            && updater.contains("compare_staged_package(&mut installer, package)?")
            && updater.contains(
                "verify_open_artifact_publisher_matches_current_executable(package.file(), package.path())"
            ),
        "Windows updater must revalidate exact bytes and publisher on the final non-reparse read-only handle"
    );
    ensure!(
        updater.contains(
            "expected_windows_update_version(&update.version, env!(\"CARGO_PKG_VERSION\"))"
        ) && updater.contains("FILE_VER_GET_NEUTRAL")
            && updater.contains("MAX_WINDOWS_VERSION_INFO_BYTES")
            && updater.contains("read_locked_windows_versions(package)")
            && updater.contains("fixed_info.dwFileVersionMS")
            && updater.contains("fixed_info.dwProductVersionMS")
            && updater.contains("validate_embedded_windows_versions"),
        "Windows updater must bind both signed PE versions to the manifest before launch"
    );
    let normalized = updater.to_ascii_lowercase();
    ensure!(
        !normalized.contains("powershell") && !updater.contains("process::exit"),
        "Windows updater must not use PowerShell or terminate the process from the worker"
    );
    Ok(())
}

fn verify_windows_uninstaller_sources(config: &str, template: &str) -> Result<()> {
    let install_modes = config
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("installMode ="))
        .collect::<Vec<_>>();
    ensure!(
        install_modes == ["installMode = \"currentUser\""],
        "Windows installer configuration must remain currentUser-only"
    );
    let downgrade_settings = config
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("allowDowngrades ="))
        .collect::<Vec<_>>();
    ensure!(
        downgrade_settings == ["allowDowngrades = false"],
        "Windows installer configuration must disable downgrades"
    );
    ensure!(
        template.contains("!define INSTALLMODE \"{{install_mode}}\"")
            && template.contains("!define ALLOWDOWNGRADES \"{{allow_downgrades}}\""),
        "Windows installer template must consume the verified install mode and downgrade policy"
    );
    ensure!(
        config.contains("template = \"installer.nsi\""),
        "Windows packaging must select the managed NSIS template"
    );
    let appdata_paths = toml_string_array(config, "appdataPaths")?;
    ensure!(
        appdata_paths
            == [
                "$LOCALAPPDATA/airwiki/AirWiki".to_owned(),
                "$APPDATA/airwiki/AirWiki".to_owned(),
            ],
        "Windows appdataPaths must contain exactly the two managed data roots"
    );
    ensure!(
        template.contains("cargo-packager 0.11.8's default NSIS template"),
        "the managed NSIS template must record its pinned upstream base"
    );

    let uninstall_start = template
        .find("Section Uninstall")
        .context("NSIS template has no uninstall section")?;
    let uninstall_end = template[uninstall_start..]
        .find("SectionEnd")
        .map(|offset| uninstall_start + offset)
        .context("NSIS uninstall section is not terminated")?;
    let uninstall = &template[uninstall_start..uninstall_end];

    let autostart_read = uninstall
        .find("ReadRegStr $R0 HKCU \"${AUTOSTARTKEY}\" \"${AUTOSTARTVALUENAME}\"")
        .context("uninstaller does not read the exact managed autostart value")?;
    let expected_command = uninstall
        .find("StrCpy $R1 \"$\\\"$INSTDIR\\${MAINBINARYNAME}.exe$\\\" --background\"")
        .context("uninstaller does not reconstruct the exact managed autostart command")?;
    let exact_gate = uninstall
        .find("StrCmp $R0 $R1 0 autostart_cleanup_done")
        .context("autostart removal is not guarded by an exact comparison")?;
    let autostart_delete = uninstall
        .find("DeleteRegValue HKCU \"${AUTOSTARTKEY}\" \"${AUTOSTARTVALUENAME}\"")
        .context("uninstaller does not remove the exact managed autostart value")?;
    ensure!(
        autostart_read < expected_command
            && expected_command < exact_gate
            && exact_gate < autostart_delete,
        "autostart cleanup must read, reconstruct, compare, then delete"
    );
    ensure!(
        !uninstall.contains("DeleteRegKey HKCU \"${AUTOSTARTKEY}\""),
        "uninstaller must never delete the Windows Run key"
    );
    let autostart_mutations = uninstall
        .lines()
        .map(str::trim)
        .filter(|line| {
            let normalized = line.to_ascii_lowercase();
            (normalized.starts_with("deletereg") || normalized.starts_with("writereg"))
                && line.contains("${AUTOSTARTKEY}")
        })
        .collect::<Vec<_>>();
    ensure!(
        autostart_mutations
            == ["DeleteRegValue HKCU \"${AUTOSTARTKEY}\" \"${AUTOSTARTVALUENAME}\""],
        "uninstaller must contain exactly one guarded autostart mutation"
    );

    let firewall_gate = uninstall
        .find("${If} $RemoveFirewallCheckboxState == ${BST_CHECKED}")
        .context("firewall cleanup is not opt-in")?;
    let helper_remove = uninstall
        .find(
            "ExecShellWait \"runas\" \"$INSTDIR\\${FIREWALLHELPER}\" \"remove\" SW_SHOWNORMAL $R0",
        )
        .context("firewall cleanup does not elevate the packaged helper's closed remove command")?;
    let payload_delete = uninstall
        .find("; Delete the app directory and its content from disk")
        .context("uninstall payload deletion marker is missing")?;
    ensure!(
        firewall_gate < helper_remove && helper_remove < payload_delete,
        "the signed helper must run only after opt-in and before its payload is deleted"
    );
    let firewall_cleanup = &uninstall[firewall_gate..payload_delete];
    ensure!(
        !firewall_cleanup.contains("Abort")
            && !firewall_cleanup.contains("Quit")
            && !firewall_cleanup.contains("netsh")
            && !firewall_cleanup.contains("powershell"),
        "firewall cleanup failure must not block uninstall or bypass the signed helper"
    );
    ensure!(
        !template.contains("SendMessage $RemoveFirewallCheckbox ${BM_SETCHECK}")
            && !template.contains("SendMessage $DeleteAppDataCheckbox ${BM_SETCHECK}"),
        "firewall and data deletion choices must remain unchecked by default"
    );

    let data_cleanup = uninstall
        .find("; Delete app data")
        .map(|offset| &uninstall[offset..])
        .context("app-data cleanup block is missing")?;
    let data_gate = data_cleanup
        .find("${If} $DeleteAppDataCheckboxState == 1")
        .context("app-data deletion is not guarded by explicit confirmation")?;
    let recursive_delete = data_cleanup
        .find("RmDir /r")
        .context("app-data cleanup has no managed paths")?;
    let data_gate_end = data_cleanup[data_gate..]
        .find("${EndIf}")
        .map(|offset| data_gate + offset)
        .context("app-data confirmation gate is not terminated")?;
    ensure!(
        data_gate < recursive_delete && recursive_delete < data_gate_end,
        "app data must only be recursively deleted inside the confirmation gate"
    );
    let recursive_deletes = uninstall
        .lines()
        .map(str::trim)
        .filter(|line| line.to_ascii_lowercase().starts_with("rmdir /r "))
        .collect::<Vec<_>>();
    ensure!(
        recursive_deletes == ["RmDir /r \"{{unescape_dollar_sign this}}\""],
        "uninstaller must contain no recursive deletion outside managed appdataPaths"
    );
    Ok(())
}

fn toml_string_array(config: &str, key: &str) -> Result<Vec<String>> {
    let marker = format!("{key} = [");
    ensure!(
        config.lines().filter(|line| line.trim() == marker).count() == 1,
        "{key} string array must appear exactly once"
    );
    let start = config
        .lines()
        .position(|line| line.trim() == marker)
        .with_context(|| format!("missing {key} string array"))?;
    let mut values = Vec::new();
    let mut closed = false;
    for line in config.lines().skip(start + 1) {
        let line = line.trim();
        if line == "]" {
            closed = true;
            break;
        }
        ensure!(!line.is_empty(), "{key} contains an empty array entry");
        let value = line
            .strip_suffix(',')
            .unwrap_or(line)
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .with_context(|| format!("{key} contains a non-string entry"))?;
        ensure!(
            !value.contains('"') && !value.contains('\\'),
            "{key} contains an escaped or ambiguous path"
        );
        values.push(value.to_owned());
    }
    ensure!(closed, "{key} string array is not terminated");
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use airwiki_inference::MACOS_LLAMA_SERVER_SHA256;

    const TEST_UPDATER_PUBLIC_KEY: &str = "untrusted comment: minisign public key E7620F1842B4E81F\n\
         RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
    const TEST_UPDATER_SIGNATURE: &str = "untrusted comment: signature from minisign secret key\n\
         RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=\n\
         trusted comment: timestamp:1556193335\tfile:test\n\
         y/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg==";

    fn windows_uninstaller_sources() -> (String, String) {
        let root = workspace_root();
        let config = fs::read_to_string(root.join("packaging/windows/Packager.toml")).unwrap();
        let template = fs::read_to_string(root.join("packaging/windows/installer.nsi")).unwrap();
        (config, template)
    }

    fn windows_update_handoff_sources() -> (String, String) {
        let root = workspace_root();
        let template = fs::read_to_string(root.join("packaging/windows/installer.nsi")).unwrap();
        let updater = fs::read_to_string(root.join("apps/desktop/src/updater.rs")).unwrap();
        (template, updater)
    }

    fn windows_installer_smoke_source() -> String {
        fs::read_to_string(workspace_root().join("packaging/smoke-install-windows.ps1")).unwrap()
    }

    fn updater_signature_fixture(artifact: &[u8]) -> (tempfile::TempDir, UpdaterSignatureRequest) {
        let directory = tempfile::tempdir().unwrap();
        let artifact_path = directory.path().join("artifact.exe");
        let signature_path = directory.path().join("artifact.exe.sig");
        fs::write(&artifact_path, artifact).unwrap();
        fs::write(
            &signature_path,
            BASE64_STANDARD.encode(TEST_UPDATER_SIGNATURE),
        )
        .unwrap();
        let request = UpdaterSignatureRequest {
            artifact: artifact_path,
            signature: signature_path,
        };
        (directory, request)
    }

    #[test]
    fn updater_signature_verification_accepts_the_signed_final_bytes() {
        let (_directory, request) = updater_signature_fixture(b"test");
        let public_key = BASE64_STANDARD.encode(TEST_UPDATER_PUBLIC_KEY);

        verify_updater_signature(&request, &public_key).unwrap();
    }

    #[test]
    fn updater_signature_verification_rejects_a_modified_artifact() {
        let (_directory, request) = updater_signature_fixture(b"modified");
        let public_key = BASE64_STANDARD.encode(TEST_UPDATER_PUBLIC_KEY);

        let error = verify_updater_signature(&request, &public_key).unwrap_err();

        assert!(error.to_string().contains("does not match"));
    }

    #[test]
    fn updater_signature_request_rejects_duplicate_artifact_options() {
        let error = parse_updater_signature_request(vec![
            "--artifact".to_owned(),
            "one".to_owned(),
            "--artifact".to_owned(),
            "two".to_owned(),
            "--signature".to_owned(),
            "signature".to_owned(),
        ])
        .unwrap_err();

        assert!(error.to_string().contains("duplicate `--artifact`"));
    }

    #[test]
    fn windows_uninstaller_preserves_cleanup_consent_and_exact_ownership() {
        let (config, template) = windows_uninstaller_sources();

        verify_windows_uninstaller_sources(&config, &template).unwrap();
    }

    #[test]
    fn windows_update_handoff_waits_cleanly_then_keeps_stuck_process_recovery() {
        let (template, updater) = windows_update_handoff_sources();

        verify_windows_update_handoff_sources(&template, &updater).unwrap();
    }

    #[test]
    fn windows_installer_preflight_is_not_owned_by_the_reinstall_page() {
        let (template, _) = windows_update_handoff_sources();
        let unsafe_template = template.replacen(
            "  Call ClassifyExistingInstallation\n  Call EnforceInstallPolicy",
            "  Call EnforceInstallPolicy",
            1,
        );

        let error = verify_windows_installer_preflight_sources(&unsafe_template).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("classify before every installer page")
        );
    }

    #[test]
    fn windows_installer_preflight_smoke_executes_the_transition_matrix() {
        let smoke = windows_installer_smoke_source();

        verify_windows_installer_smoke_sources(&smoke).unwrap();
    }

    #[test]
    fn windows_installer_preflight_smoke_accepts_a_crlf_checkout() {
        let smoke = windows_installer_smoke_source().replace('\n', "\r\n");

        verify_windows_installer_smoke_sources(&smoke).unwrap();
    }

    #[test]
    fn windows_installer_preflight_smoke_anchors_the_active_windows_version_literal() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let active =
            r#"if ([int]$Os.ProductType -ne 1 -or [version]$Os.Version -lt [version]"10.0") {"#;
        let unsafe_smoke = smoke.replacen(
            active,
            r#"if ([int]$Os.ProductType -ne 1 -or [version]$Os.Version -lt [version]"6.1") {
        $null = '[version]$Os.Version -lt [version]"10.0"'"#,
            1,
        );
        assert_ne!(unsafe_smoke, smoke, "Windows-version fixture did not match");

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("Windows 10 literal"));
    }

    #[test]
    fn windows_installer_preflight_smoke_anchors_the_active_whitespace_regex() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let active =
            r#"if (-not [IO.Path]::IsPathRooted($InstallDir) -or $InstallDir -match '\s') {"#;
        let unsafe_smoke = smoke.replacen(
            active,
            r#"if (-not [IO.Path]::IsPathRooted($InstallDir) -or $InstallDir -match '^$') {
        $null = "$InstallDir -match '\s'""#,
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "whitespace-regex fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("whitespace regex"));
    }

    #[test]
    fn windows_installer_preflight_smoke_anchors_the_active_final_install_argument() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let active = r#"$Arguments[$Arguments.Count - 1] -cne "/D=$InstallDir") {"#;
        let unsafe_smoke = smoke.replacen(
            active,
            r#"$Arguments[$Arguments.Count - 1] -cne "/D=$($Arguments[$Arguments.Count - 1].Substring(3))") {
            $null = '$Arguments[$Arguments.Count - 1] -cne "/D=$InstallDir"'"#,
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "final-install-argument fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("final /D literal"));
    }

    #[test]
    fn windows_installer_preflight_smoke_anchors_the_active_process_start_event_class() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let active = r#""SELECT * FROM Win32_ProcessStartTrace WHERE ProcessName = 'airwiki.exe'"
        )"#;
        let unsafe_smoke = smoke.replacen(
            active,
            r#""SELECT * FROM Win32_ProcessStopTrace WHERE ProcessName = 'airwiki.exe'"
        )
        $null = "SELECT * FROM Win32_ProcessStartTrace WHERE ProcessName = 'airwiki.exe'""#,
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "process-start event-class fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("process-start WQL"));
    }

    #[test]
    fn windows_installer_preflight_smoke_anchors_the_active_process_name_filter() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let active = r#""SELECT * FROM Win32_ProcessStartTrace WHERE ProcessName = 'airwiki.exe'"
        )"#;
        let unsafe_smoke = smoke.replacen(
            active,
            r#""SELECT * FROM Win32_ProcessStartTrace WHERE ProcessName = 'unrelated.exe'"
        )
        $null = "SELECT * FROM Win32_ProcessStartTrace WHERE ProcessName = 'airwiki.exe'""#,
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "process-name filter fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("process-start WQL"));
    }

    #[test]
    fn windows_installer_preflight_smoke_anchors_the_active_owner_value_read() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let active = r#"$MarkerValue = [string]$Key.GetValue(
        "AirWikiTestOwner",
        $null,
        [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames
    )"#;
        let unsafe_smoke = smoke.replacen(
            active,
            r#"$MarkerValue = [string]$Key.GetValue(
        "UnexpectedOwner",
        $null,
        [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames
    )
    $null = '$MarkerValue = [string]$Key.GetValue(
        "AirWikiTestOwner",
        $null,
        [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames
    )'"#,
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "owner-value read fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("owner marker reads"));
    }

    #[test]
    fn windows_installer_preflight_smoke_anchors_the_active_owner_kind_read() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let active = r#"$MarkerKind = $Key.GetValueKind("AirWikiTestOwner")"#;
        let unsafe_smoke = smoke.replacen(
            active,
            r#"$MarkerKind = $Key.GetValueKind("UnexpectedOwner")
    $null = '$MarkerKind = $Key.GetValueKind("AirWikiTestOwner")'"#,
            1,
        );
        assert_ne!(unsafe_smoke, smoke, "owner-kind read fixture did not match");

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("owner marker reads"));
    }

    #[test]
    fn windows_installer_preflight_smoke_anchors_the_active_owner_marker_write() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let active = r#"New-ItemProperty `
        -LiteralPath $Path `
        -Name AirWikiTestOwner `
        -Value $RegistryOwner `
        -PropertyType String | Out-Null"#;
        let unsafe_smoke = smoke.replacen(
            active,
            r#"New-ItemProperty `
        -LiteralPath $Path `
        -Name UnexpectedOwner `
        -Value $RegistryOwner `
        -PropertyType String | Out-Null
    $null = 'New-ItemProperty `
        -LiteralPath $Path `
        -Name AirWikiTestOwner `
        -Value $RegistryOwner `
        -PropertyType String | Out-Null'"#,
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "owner-marker write fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("owner marker write"));
    }

    #[test]
    fn windows_installer_preflight_smoke_anchors_the_active_reg_sz_property_type() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let active = r#"New-ItemProperty `
        -LiteralPath $Path `
        -Name AirWikiTestOwner `
        -Value $RegistryOwner `
        -PropertyType String | Out-Null"#;
        let unsafe_smoke = smoke.replacen(
            active,
            r#"New-ItemProperty `
        -LiteralPath $Path `
        -Name AirWikiTestOwner `
        -Value $RegistryOwner `
        -PropertyType ExpandString | Out-Null
    $null = 'New-ItemProperty `
        -LiteralPath $Path `
        -Name AirWikiTestOwner `
        -Value $RegistryOwner `
        -PropertyType String | Out-Null'"#,
            1,
        );
        assert_ne!(unsafe_smoke, smoke, "REG_SZ write fixture did not match");

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("owner marker write"));
    }

    #[test]
    fn windows_installer_preflight_smoke_rejects_a_commented_host_gate_call() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "$ClientOsVersion = Assert-WindowsClientInstallerGateHost",
            "# $ClientOsVersion = Assert-WindowsClientInstallerGateHost",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "commented host-gate mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_rejects_a_commented_second_wix_call() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "$WixTwo = New-OwnedWixFixture \"two\" \"0.1.8\"",
            "# $WixTwo = New-OwnedWixFixture \"two\" \"0.1.8\"",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "commented second-WiX mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_rejects_a_commented_partial_nsis_mutation() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "Remove-ItemProperty -LiteralPath $UninstallRegistryPath -Name $RequiredName",
            "# Remove-ItemProperty -LiteralPath $UninstallRegistryPath -Name $RequiredName",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "commented partial-NSIS mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_rejects_an_inert_host_gate_call() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let statement = "$ClientOsVersion = Assert-WindowsClientInstallerGateHost";
        let unsafe_smoke = smoke.replacen(statement, &format!("'{statement}'"), 1);
        assert_ne!(unsafe_smoke, smoke, "inert host-gate fixture did not match");

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("host gate exactly once"));
    }

    #[test]
    fn windows_installer_preflight_smoke_rejects_an_inert_rejection_call() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let statement = "Assert-CleanRejection @(\"/P\", \"/R\", \"/AIRWIKIUPDATE\", \"/NS\", \"/D=$InstallDir\") \"clean /AIRWIKIUPDATE rejection\"";
        let unsafe_smoke = smoke.replacen(statement, &format!("'{statement}'"), 1);
        assert_ne!(
            unsafe_smoke, smoke,
            "inert rejection-call fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("exact /P /R /AIRWIKIUPDATE handoff")
        );
    }

    #[test]
    fn windows_installer_preflight_smoke_rejects_an_inert_ownership_registration() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let statement = "$OwnedRegistryFixtures.Add($Record) | Out-Null";
        let unsafe_smoke = smoke.replacen(statement, &format!("'{statement}'"), 1);
        assert_ne!(
            unsafe_smoke, smoke,
            "inert ownership-registration fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("register owned WiX state"));
    }

    #[test]
    fn windows_installer_preflight_smoke_rejects_an_inert_recovery_call() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let statement = "Restore-VerifiedInstall \"/AIRWIKIUPDATE requires /P recovery\"";
        let unsafe_smoke = smoke.replacen(statement, &format!("'{statement}'"), 1);
        assert_ne!(
            unsafe_smoke, smoke,
            "inert recovery-call fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("restore and reverify"));
    }

    #[test]
    fn windows_installer_preflight_smoke_rejects_an_inert_cleanup_call() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let statement = "$CleanupFailures.Add($Label) | Out-Null";
        let unsafe_smoke = smoke.replacen(statement, &format!("'{statement}'"), 1);
        assert_ne!(
            unsafe_smoke, smoke,
            "inert cleanup-call fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("independent cleanup"));
    }

    #[test]
    fn windows_installer_preflight_smoke_rejects_here_strings() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let unsafe_smoke = smoke.replacen(
            "    $ClientOsVersion = Assert-WindowsClientInstallerGateHost",
            "@'\n$ClientOsVersion = Assert-WindowsClientInstallerGateHost\n'@",
            1,
        );
        assert_ne!(unsafe_smoke, smoke, "here-string fixture did not match");

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("PowerShell here-strings"));
    }

    #[test]
    fn windows_installer_preflight_smoke_requires_a_typed_owner_marker_before_deletion() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let deletion = "    Assert-MutationProcessPrecondition $CurrentCase\n    Remove-Item -LiteralPath $Record.Path -Recurse -Force";
        let unsafe_smoke = smoke.replacen(
            deletion,
            "    Assert-MutationProcessPrecondition $CurrentCase\n    Remove-ItemProperty -LiteralPath $Record.Path -Name AirWikiTestOwner -ErrorAction Stop\n    Assert-MutationProcessPrecondition $CurrentCase\n    New-ItemProperty -LiteralPath $Record.Path -Name AirWikiTestOwner -Value $RegistryOwner -PropertyType ExpandString -ErrorAction Stop | Out-Null\n    $Record.Fingerprint = Get-RegistryFingerprint $Record.Path\n    Assert-MutationProcessPrecondition $CurrentCase\n    Remove-Item -LiteralPath $Record.Path -Recurse -Force",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "typed-owner-marker mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("typed owner marker"));
    }

    #[test]
    fn windows_installer_preflight_smoke_keeps_fingerprint_untrusted_until_marker_readback() {
        let smoke = windows_installer_smoke_source();
        let safe_order = "    Assert-OwnedRegistryMarker $Record\n    $Record.MarkerTrusted = $true\n    $Record.Fingerprint = Get-RegistryFingerprint $Path";
        let unsafe_order = "    $Record.Fingerprint = Get-RegistryFingerprint $Path\n    Assert-OwnedRegistryMarker $Record\n    $Record.MarkerTrusted = $true";
        let unsafe_smoke = smoke.replacen(safe_order, unsafe_order, 1);
        assert_ne!(
            unsafe_smoke, smoke,
            "trusted-fingerprint order fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_guards_registry_mutations_immediately() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let sink = "        Set-ItemProperty -LiteralPath $UninstallRegistryPath -Name DisplayVersion -Value $Version";
        let unsafe_smoke = smoke.replacen(sink, &format!("        $null = Get-Date\n{sink}"), 1);
        assert_ne!(
            unsafe_smoke, smoke,
            "registry-mutation precondition fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("immediate exact-path process precondition")
        );
    }

    #[test]
    fn windows_installer_preflight_smoke_uses_the_exact_path_in_its_reusable_guard() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let unsafe_smoke = smoke.replacen(
            "function Assert-MutationProcessPrecondition([string] $CaseId) {\n    Assert-NoDesktopProcess $CaseId\n}",
            "function Assert-MutationProcessPrecondition([string] $CaseId) {\n    $null = $CaseId\n}",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "reusable exact-path guard fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("reusable mutation guard"));
    }

    #[test]
    fn windows_installer_preflight_smoke_guards_file_mutations_immediately() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let sink =
            "    [IO.File]::WriteAllText($Notice, $Sentinel, [Text.UTF8Encoding]::new($false))";
        let unsafe_smoke = smoke.replacen(sink, &format!("    $null = Get-Date\n{sink}"), 1);
        assert_ne!(
            unsafe_smoke, smoke,
            "file-mutation precondition fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("immediate exact-path process precondition")
        );
    }

    #[test]
    fn windows_installer_preflight_smoke_guards_process_invocations_immediately() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let sink =
            "    $Process = Start-Process -FilePath $Installer -ArgumentList $Arguments -PassThru";
        let unsafe_smoke = smoke.replacen(sink, &format!("    $null = Get-Date\n{sink}"), 1);
        assert_ne!(
            unsafe_smoke, smoke,
            "process-invocation precondition fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("immediate exact-path process precondition")
        );
    }

    #[test]
    fn windows_installer_preflight_smoke_guards_uninstaller_invocations_immediately() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let sink = "        $UninstallProcess = Start-Process `";
        let unsafe_smoke = smoke.replacen(sink, &format!("        $null = Get-Date\n{sink}"), 1);
        assert_ne!(
            unsafe_smoke, smoke,
            "uninstaller precondition fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("immediate exact-path process precondition")
        );
    }

    #[test]
    fn windows_installer_preflight_smoke_observes_rejected_process_starts_in_finally() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let opener = "function Assert-CleanRejection([string[]] $Arguments, [string] $CaseId) {";
        let unsafe_smoke = smoke.replacen(
            opener,
            &format!(
                "{opener}\n    Invoke-Installer -Arguments $Arguments -ExpectedExit 2 -CaseId \"$CaseId-unobserved\""
            ),
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "rejected-process observer fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("process-start observer"));
    }

    #[test]
    fn windows_installer_preflight_smoke_rejects_an_inert_process_observer_start() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let statement = "$Observer = Start-RejectedDesktopProcessObserver";
        let unsafe_smoke = smoke.replacen(statement, &format!("'{statement}'"), 1);
        assert_ne!(
            unsafe_smoke, smoke,
            "inert process-observer fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("process-start observer"));
    }

    #[test]
    fn windows_installer_preflight_smoke_retrieves_the_registered_event_subscriber() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let unsafe_smoke = smoke.replacen(
            "        $Subscriptions = @(Get-EventSubscriber -ErrorAction Stop | Where-Object {\n            $_.SourceIdentifier -ceq $SourceIdentifier\n        })\n        if ($Subscriptions.Count -ne 1) {\n            throw \"rejected-process observer subscription setup was ambiguous\"\n        }\n        $Subscription = $Subscriptions[0]",
            "        $Subscription = $null",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "event-subscriber retrieval fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("process-start observer"));
    }

    #[test]
    fn windows_installer_preflight_smoke_drains_observer_events_after_unsubscribe() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let unsafe_smoke = smoke.replacen(
            "        if ($FinalEvents.Count -ne 0) {\n            $Observed = $true\n        }",
            "        $null = $FinalEvents.Count",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "observer final-drain fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("process-start observer"));
    }

    #[test]
    fn windows_installer_preflight_smoke_retains_a_safe_process_handle() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let statement =
            "            $OwnedDesktopProcess = Wait-OwnedDesktopProcess $DesktopExecutable";
        let unsafe_smoke = smoke.replacen(
            statement,
            &format!(
                "{statement}\n            $OwnedDesktopProcess = [pscustomobject]@{{\n                ProcessId = $OwnedDesktopProcess.ProcessId\n                ExecutablePath = $OwnedDesktopProcess.ExecutablePath\n            }}"
            ),
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "safe-process-handle fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("exact restarted PID and path"));
    }

    #[test]
    fn windows_installer_preflight_smoke_rejects_an_inert_safe_handle_acquisition() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let statement = "$SafeHandle = $Process.SafeHandle";
        let unsafe_smoke = smoke.replacen(statement, &format!("'{statement}'"), 1);
        assert_ne!(
            unsafe_smoke, smoke,
            "inert safe-handle fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("exact restarted PID and path"));
    }

    #[test]
    fn windows_installer_preflight_smoke_disposes_retained_process_when_stop_validation_fails() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let unsafe_smoke = smoke.replacen(
            "        } finally {\n            if ($null -ne $Record.Process) {\n                $Record.Process.Dispose()\n                $Record.Process = $null\n                $Record.SafeHandle = $null\n            }\n        }",
            "        }",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "retained-process fallback disposal fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("exact restarted PID and path"));
    }

    #[test]
    fn windows_installer_preflight_smoke_terminates_only_through_the_retained_process() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let unsafe_smoke = smoke.replacen(
            "        $Process.Kill()\n        if (-not $Process.WaitForExit($ProcessCleanupWaitMilliseconds)) {\n            throw \"owned desktop process did not exit within the bounded wait\"",
            "        Stop-Process -Id $Record.ProcessId -ErrorAction Stop\n        if ($false) {\n            throw \"owned desktop process did not exit within the bounded wait\"",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "retained-process termination fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_waits_only_for_the_direct_installer() {
        let smoke = windows_installer_smoke_source();
        verify_windows_installer_smoke_sources(&smoke).unwrap();
        let unsafe_smoke = smoke.replacen(
            "        $ExitCode = $Process.ExitCode",
            "        $Process.WaitForExit()\n        $ExitCode = $Process.ExitCode",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "direct-installer bounded-wait fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("bounded direct installer wait"));
    }

    #[test]
    fn windows_installer_preflight_smoke_requires_the_clean_airwiki_update_rejection_call() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "Assert-CleanRejection @(\"/P\", \"/R\", \"/AIRWIKIUPDATE\", \"/NS\", \"/D=$InstallDir\") \"clean /AIRWIKIUPDATE rejection\"",
            "# removed clean updater rejection call",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "clean /AIRWIKIUPDATE mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_requires_two_wix_rejection_fixtures() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "$WixTwo = New-OwnedWixFixture \"two\" \"0.1.8\"",
            "$WixTwo = $WixOne",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "multiple WiX mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_requires_the_partial_nsis_mutation_call() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "    Remove-ItemProperty -LiteralPath $UninstallRegistryPath -Name $RequiredName\n",
            "",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "partial NSIS mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_calls_the_host_gate_before_owned_state() {
        let smoke = windows_installer_smoke_source();
        let owner_write = "[IO.File]::WriteAllText($OwnerMarker, $OwnerToken, [Text.UTF8Encoding]::new($false))\n";
        let host_call = "$ClientOsVersion = Assert-WindowsClientInstallerGateHost\n";
        let without_owner_write = smoke.replacen(owner_write, "", 1);
        assert_ne!(
            without_owner_write, smoke,
            "host-order mutation fixture did not find the owner write"
        );
        let unsafe_smoke =
            without_owner_write.replacen(host_call, &format!("{owner_write}{host_call}"), 1);
        assert_ne!(
            unsafe_smoke, without_owner_write,
            "host-order mutation fixture did not find the host-gate call"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_requires_a_64_bit_powershell_process() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "[Environment]::Is64BitProcess",
            "$true # bypassed process architecture gate",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "64-bit-process mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_rejects_ci_identity_gates() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "$ErrorActionPreference = \"Stop\"",
            "$ErrorActionPreference = \"Stop\"\nif ($env:CI -ne \"true\") { throw \"CI only\" }",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "CI-identity mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("CI identity"));
    }

    #[test]
    fn windows_installer_preflight_smoke_fingerprints_direct_registry_subkeys() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "$Subkeys = @($Key.GetSubKeyNames() | Sort-Object)",
            "$Subkeys = @()",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "registry-subkey fingerprint mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_refreshes_fixture_ownership_after_each_mutation() {
        let smoke = windows_installer_smoke_source();
        let mutation = "    New-ItemProperty -LiteralPath $Path -Name DisplayVersion -Value $Version | Out-Null\n    $Record.Fingerprint = Get-RegistryFingerprint $Path\n";
        let unsafe_smoke = smoke.replacen(
            mutation,
            "    New-ItemProperty -LiteralPath $Path -Name DisplayVersion -Value $Version | Out-Null\n",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "ownership-refresh mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_registers_fixture_ownership_before_values() {
        let smoke = windows_installer_smoke_source();
        let registration = "    $OwnedRegistryFixtures.Add($Record) | Out-Null\n";
        let first_value = "    New-ItemProperty `\n        -LiteralPath $Path `\n        -Name AirWikiTestOwner `\n        -Value $RegistryOwner `\n        -PropertyType String | Out-Null\n";
        let without_registration = smoke.replacen(registration, "", 1);
        assert_ne!(
            without_registration, smoke,
            "ownership-order mutation fixture did not find registration"
        );
        let unsafe_smoke =
            without_registration.replacen(first_value, &format!("{first_value}{registration}"), 1);
        assert_ne!(
            unsafe_smoke, without_registration,
            "ownership-order mutation fixture did not find first value"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_requires_one_owned_record_and_path_before_delete() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "$OwnedRecordCount -ne 1 -or $OwnedPathCount -ne 1",
            "$OwnedRecordCount -gt 1 -or $OwnedPathCount -gt 1",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "unique-owned-record mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_accumulates_independent_cleanup_failures() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "        $CleanupFailures.Add($Label) | Out-Null",
            "        throw \"cleanup stopped at $Label\"",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "cleanup-accumulation mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_attempts_each_registry_cleanup_independently() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "        Invoke-CleanupStep \"owned registry fixture\" { Remove-OwnedRegistryFixture $Record }\n",
            "",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "independent-registry-cleanup mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_restores_after_the_downgrade_case() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "        } finally {\n            Restore-VerifiedInstall \"silent downgrade rejection recovery\"\n        }\n\n        $CoexistingWix",
            "        } finally {\n        }\n\n        $CoexistingWix",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "downgrade-recovery mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_restores_repair_state_in_finally() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "        } finally {\n            Restore-VerifiedInstall \"same-version silent repair recovery\"\n        }",
            "        }\n        Restore-VerifiedInstall \"same-version silent repair recovery\"",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "repair-finally mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_restores_upgrade_state_in_finally() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "        } finally {\n            Restore-VerifiedInstall \"strictly newer silent upgrade recovery\"\n        }",
            "        }\n        Restore-VerifiedInstall \"strictly newer silent upgrade recovery\"",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "upgrade-finally mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_restores_each_partial_nsis_case() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "    Restore-VerifiedInstall \"partial NSIS key rejection recovery: $RequiredName\"\n",
            "",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "partial-NSIS recovery mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_uses_the_exact_updater_handoff() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "@(\"/P\", \"/R\", \"/AIRWIKIUPDATE\", \"/NS\", \"/D=$InstallDir\")",
            "@(\"/S\", \"/P\", \"/AIRWIKIUPDATE\", \"/NS\", \"/D=$InstallDir\")",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "updater-handoff mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_requires_exact_policy_rejection_exit_code() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "    Invoke-RejectedInstaller $Arguments 2 $CaseId\n",
            "    Invoke-RejectedInstaller $Arguments 1 $CaseId\n",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "exact-rejection-code mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_routes_rejections_through_one_process_helper() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "    Invoke-RejectedInstaller $Arguments 2 $CaseId\n",
            "    Invoke-Installer $Arguments 2 $CaseId\n",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "single-process-helper mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_restores_downgrade_state_in_finally() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "        } finally {\n            Restore-VerifiedInstall \"silent downgrade rejection recovery\"\n        }",
            "        }\n        Restore-VerifiedInstall \"silent downgrade rejection recovery\"",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "downgrade-finally mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_restores_partial_nsis_state_in_finally() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "            } finally {\n                Assert-MutationProcessPrecondition $CurrentCase\n                Set-ItemProperty -LiteralPath $UninstallRegistryPath -Name $RequiredName -Value $RequiredValue\n                Restore-VerifiedInstall \"partial NSIS key rejection recovery: $RequiredName\"\n            }",
            "            }\n            Assert-MutationProcessPrecondition $CurrentCase\n            Set-ItemProperty -LiteralPath $UninstallRegistryPath -Name $RequiredName -Value $RequiredValue\n            Restore-VerifiedInstall \"partial NSIS key rejection recovery: $RequiredName\"",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "partial-NSIS-finally mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_supervises_the_exact_restarted_process() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "$OwnedDesktopProcess = Wait-OwnedDesktopProcess $DesktopExecutable",
            "$OwnedDesktopProcess = Get-Process -Name \"airwiki\"",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "exact-restarted-process mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_recovers_updater_state_in_finally() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "                Invoke-CleanupStep \"updater-install-recovery\" {\n                    Restore-VerifiedInstall \"/AIRWIKIUPDATE strictly newer install recovery\"\n                }",
            "                Restore-VerifiedInstall \"/AIRWIKIUPDATE strictly newer install recovery\"",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "updater-recovery-finally mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_fails_closed_when_restart_wait_fails() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "            Invoke-CleanupStep \"updater-restart-process\" {\n                Stop-RegisteredDesktopProcess $DesktopExecutable\n                $script:UpdaterRestartCleanupAllowed = $true\n            }",
            "            if ($null -ne $OwnedDesktopProcess) {\n                Stop-OwnedDesktopProcess $OwnedDesktopProcess\n            }",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "restart-wait cleanup mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_never_claims_all_exact_path_processes() {
        let smoke = windows_installer_smoke_source();
        let registered_only = "    if ($null -ne $OwnedDesktopProcess) {\n        $Record = $OwnedDesktopProcess\n        try {\n            if (-not $Record.ExecutablePath.Equals(\n                $ExpectedFullPath,\n                [StringComparison]::OrdinalIgnoreCase\n            )) {\n                throw \"registered desktop process path does not match the owned install\"\n            }\n            Stop-OwnedDesktopProcess $Record\n        } finally {\n            if ($null -ne $Record.Process) {\n                $Record.Process.Dispose()\n                $Record.Process = $null\n                $Record.SafeHandle = $null\n            }\n        }\n        $script:OwnedDesktopProcess = $null\n    }";
        let unsafe_smoke = smoke.replacen(
            registered_only,
            "    foreach ($OwnedDesktopProcess in @(Get-DesktopProcessesAtExactPath $ExpectedPath)) {\n        Stop-OwnedDesktopProcess $OwnedDesktopProcess\n    }",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "broad exact-path ownership mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_retries_exact_path_cleanup_in_outer_finally() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "        Invoke-CleanupStep \"owned-desktop-process\" {\n            Stop-RegisteredDesktopProcess $DesktopExecutable\n        }",
            "        if ($null -ne $OwnedDesktopProcess) {\n            Stop-OwnedDesktopProcess $OwnedDesktopProcess\n        }",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "outer exact-path cleanup mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_checks_restart_precondition_before_mutation() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "Assert-NoDesktopProcess \"/AIRWIKIUPDATE strictly newer install\"\n        $UpdaterRestartCleanupAllowed = $false\n        try {",
            "$UpdaterRestartCleanupAllowed = $false\n        try {\n            Assert-NoDesktopProcess \"/AIRWIKIUPDATE strictly newer install\"",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "restart-precondition order mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_stops_only_the_retained_process() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "$Process.Kill()\n        if (-not $Process.WaitForExit($ProcessCleanupWaitMilliseconds))",
            "Stop-Process -Name \"airwiki\" -ErrorAction Stop\n        if ($false)",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "retained-process stop mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_revalidates_pid_path_before_stop() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "$ActualPath.Equals(\n            $Record.ExecutablePath,\n            [StringComparison]::OrdinalIgnoreCase\n        )",
            "$true",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "PID-path revalidation mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_guards_install_cleanup_on_exact_process_exit() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "if ($InstallCleanupAllowed -and\n            $UninstallerVerified -and",
            "if ($UninstallerVerified -and",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "install-cleanup process guard mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_puts_owner_marker_under_cleanup_finally() {
        let smoke = windows_installer_smoke_source();
        let protected_write = "    try {\n        $CurrentCase = \"owner-marker\"\n        Assert-MutationProcessPrecondition $CurrentCase\n        [IO.File]::WriteAllText($OwnerMarker, $OwnerToken, [Text.UTF8Encoding]::new($false))";
        let unsafe_smoke = smoke.replacen(
            protected_write,
            "    $CurrentCase = \"owner-marker\"\n    Assert-MutationProcessPrecondition $CurrentCase\n    [IO.File]::WriteAllText($OwnerMarker, $OwnerToken, [Text.UTF8Encoding]::new($false))\n    try {",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "owner-marker-finally mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_checks_rejections_for_process_launches() {
        let smoke = windows_installer_smoke_source();
        let clean_rejection = powershell_function(&smoke, "Assert-CleanRejection").unwrap();
        let preserved_state =
            powershell_function(&smoke, "Assert-RejectedInstallerPreservedState").unwrap();
        assert_eq!(
            clean_rejection
                .matches("Assert-NoDesktopProcess $CaseId")
                .count(),
            2
        );
        assert_eq!(
            preserved_state
                .matches("Assert-NoDesktopProcess $CaseId")
                .count(),
            2
        );
        let unsafe_smoke = smoke.replacen(
            "function Assert-CleanRejection([string[]] $Arguments, [string] $CaseId) {\n    Assert-NoDesktopProcess $CaseId",
            "function Assert-CleanRejection([string[]] $Arguments, [string] $CaseId) {",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "rejection-process mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_smoke_suppresses_native_failure_details() {
        let smoke = windows_installer_smoke_source();
        let unsafe_smoke = smoke.replacen(
            "[Console]::Error.WriteLine(\"WINDOWS_INSTALLER_MATRIX_FAIL case=$CurrentCase\")",
            "Write-Error $_",
            1,
        );
        assert_ne!(
            unsafe_smoke, smoke,
            "sanitized-failure mutation fixture did not match"
        );

        let error = verify_windows_installer_smoke_sources(&unsafe_smoke).unwrap_err();

        assert!(error.to_string().contains("built-installer smoke matrix"));
    }

    #[test]
    fn windows_installer_preflight_requires_client_windows_10_and_native_amd64() {
        let (template, _) = windows_update_handoff_sources();
        for (needle, replacement, message) in [
            ("${AtLeastWin10}", "${RunningX64}", "Windows 10"),
            ("${IsServerOS}", "${AtLeastWin10}", "Windows Server"),
            ("${IsNativeAMD64}", "${RunningX64}", "native AMD64"),
        ] {
            let unsafe_template = template.replacen(needle, replacement, 1);
            let error = verify_windows_installer_preflight_sources(&unsafe_template).unwrap_err();
            assert!(error.to_string().contains(message));
        }
    }

    #[test]
    fn windows_installer_platform_gate_is_the_first_on_init_action() {
        let (template, _) = windows_update_handoff_sources();
        let unsafe_template = template.replacen(
            "Function .onInit\n  Call EnforceSupportedWindows",
            "Function .onInit\n  StrCpy $PassiveMode 0\n  Call EnforceSupportedWindows",
            1,
        );

        let error = verify_windows_installer_preflight_sources(&unsafe_template).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("first executable .onInit action")
        );
    }

    #[test]
    fn windows_installer_platform_gate_rejects_inverted_predicates() {
        let (template, _) = windows_update_handoff_sources();
        for (needle, replacement, message) in [
            (
                "  ${IfNot} ${AtLeastWin10}\n    StrCpy $PlatformRejectionMessage \"$(UnsupportedWindowsVersion)\"",
                "  ${If} ${AtLeastWin10}\n    StrCpy $PlatformRejectionMessage \"$(UnsupportedWindowsVersion)\"",
                "Windows 10",
            ),
            (
                "  ${If} ${IsServerOS}\n    StrCpy $PlatformRejectionMessage \"$(UnsupportedWindowsServer)\"",
                "  ${IfNot} ${IsServerOS}\n    StrCpy $PlatformRejectionMessage \"$(UnsupportedWindowsServer)\"",
                "Windows Server",
            ),
            (
                "  ${IfNot} ${IsNativeAMD64}\n    StrCpy $PlatformRejectionMessage \"$(UnsupportedWindowsArchitecture)\"",
                "  ${If} ${IsNativeAMD64}\n    StrCpy $PlatformRejectionMessage \"$(UnsupportedWindowsArchitecture)\"",
                "native AMD64",
            ),
        ] {
            let unsafe_template = template.replacen(needle, replacement, 1);
            assert_ne!(
                unsafe_template, template,
                "platform-polarity mutation fixture did not match `{needle}`"
            );
            let error = verify_windows_installer_preflight_sources(&unsafe_template).unwrap_err();
            assert!(error.to_string().contains(message));
        }
    }

    #[test]
    fn windows_installer_platform_rejection_is_localized_and_silent_safe() {
        let (template, _) = windows_update_handoff_sources();
        for (needle, replacement, message) in [
            (
                "LangString UnsupportedWindowsVersion ${LANG_SPANISH}",
                "LangString UnsupportedWindowsVersion ${LANG_ENGLISH}",
                "English and Spanish",
            ),
            (
                "IfSilent platform_reject_abort",
                "Goto platform_reject_abort",
                "omit MessageBox only in silent mode",
            ),
            (
                "Call RejectUnsupportedPlatform",
                "Abort",
                "all three platform failures",
            ),
            (
                "platform_reject_abort:\n    SetErrorLevel 2\n    Abort",
                "platform_reject_abort:\n    SetErrorLevel 1\n    Abort",
                "exit code 2",
            ),
            (
                "platform_reject_abort:\n    SetErrorLevel 2\n    Abort\nFunctionEnd",
                "platform_reject_abort:\n    SetErrorLevel 2\n    Return\nFunctionEnd",
                "must abort",
            ),
        ] {
            let unsafe_template = template.replacen(needle, replacement, 1);
            assert_ne!(
                unsafe_template, template,
                "platform-rejection mutation fixture did not match `{needle}`"
            );
            let error = verify_windows_installer_preflight_sources(&unsafe_template).unwrap_err();
            assert!(error.to_string().contains(message));
        }
    }

    #[test]
    fn windows_installer_preflight_precedes_the_first_runtime_write() {
        let (template, _) = windows_update_handoff_sources();
        let unsafe_template = template.replacen(
            "  Call EnforceInstallPolicy\n\n  !if \"${DISPLAYLANGUAGESELECTOR}\" == \"true\"\n    !insertmacro MUI_LANGDLL_DISPLAY\n  !endif",
            "  !if \"${DISPLAYLANGUAGESELECTOR}\" == \"true\"\n    !insertmacro MUI_LANGDLL_DISPLAY\n  !endif\n\n  Call EnforceInstallPolicy",
            1,
        );

        let error = verify_windows_installer_preflight_sources(&unsafe_template).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("before the language selector and every write")
        );
    }

    #[test]
    fn windows_installer_preflight_rejects_invalid_installed_semver() {
        let (template, _) = windows_update_handoff_sources();
        let unsafe_template = template.replace("__airwiki_invalid_semver__", "0.0.0");

        let error = verify_windows_installer_preflight_sources(&unsafe_template).unwrap_err();

        assert!(error.to_string().contains("invalid SemVer sentinel"));
    }

    #[test]
    fn windows_installer_preflight_rejects_ambiguous_registry_classification() {
        let (template, _) = windows_update_handoff_sources();
        for (needle, replacement, message) in [
            (
                "IntOp $WixMetadataCount $WixMetadataCount + 1",
                "IntOp $WixMetadataCount $WixMetadataCount + 0",
                "all WiX candidates",
            ),
            (
                "    ${If} $NsisMetadataState == \"${NSIS_METADATA_PARTIAL}\"\n      Goto classify_reject",
                "    ${If} $NsisMetadataState == \"${NSIS_METADATA_ABSENT}\"\n      Goto classify_reject",
                "partial NSIS",
            ),
            (
                "    ${If} $WixMetadataCount > 1\n      Goto classify_reject",
                "    ${If} $WixMetadataCount > 2\n      Goto classify_reject",
                "multiple WiX",
            ),
            (
                "    ${If} $WixMetadataCount == 1\n      ${If} $NsisMetadataState != \"${NSIS_METADATA_ABSENT}\"",
                "    ${If} $WixMetadataCount == 0\n      ${If} $NsisMetadataState != \"${NSIS_METADATA_ABSENT}\"",
                "WiX and NSIS coexistence",
            ),
            (
                "    ${If} $WixMetadataCount == 1\n      StrCpy $WixCandidateKey",
                "    ${If} $WixMetadataCount == 0\n      StrCpy $WixCandidateKey",
                "single WiX candidate",
            ),
        ] {
            let unsafe_template = template.replacen(needle, replacement, 1);
            assert_ne!(
                unsafe_template, template,
                "classification mutation fixture did not match `{needle}`"
            );
            let error = verify_windows_installer_preflight_sources(&unsafe_template).unwrap_err();
            assert!(error.to_string().contains(message));
        }
    }

    #[test]
    fn windows_installer_preflight_is_current_user_only_and_disables_downgrades() {
        let (config, template) = windows_uninstaller_sources();
        for (needle, replacement, message) in [
            (
                "installMode = \"currentUser\"",
                "installMode = \"both\"",
                "currentUser-only",
            ),
            (
                "allowDowngrades = false",
                "allowDowngrades = true",
                "disable downgrades",
            ),
        ] {
            let unsafe_config = config.replacen(needle, replacement, 1);
            assert_ne!(
                unsafe_config, config,
                "configuration mutation fixture did not match `{needle}`"
            );
            let error = verify_windows_uninstaller_sources(&unsafe_config, &template).unwrap_err();
            assert!(error.to_string().contains(message));
        }

        for (needle, replacement, message) in [
            (
                "!if \"${INSTALLMODE}\" != \"currentUser\"\n  !error \"AirWiki 0.2.0 supports only currentUser Windows installs.\"\n!endif",
                "!if \"${INSTALLMODE}\" != \"both\"\n  !error \"AirWiki 0.2.0 supports only currentUser Windows installs.\"\n!endif",
                "reject non-currentUser modes",
            ),
            (
                "!if \"${ALLOWDOWNGRADES}\" != \"false\"\n  !error \"AirWiki 0.2.0 does not support Windows downgrades.\"\n!endif",
                "!if \"${ALLOWDOWNGRADES}\" != \"true\"\n  !error \"AirWiki 0.2.0 does not support Windows downgrades.\"\n!endif",
                "reject ALLOWDOWNGRADES=true",
            ),
        ] {
            let unsafe_template = template.replacen(needle, replacement, 1);
            assert_ne!(
                unsafe_template, template,
                "compile-time policy mutation fixture did not match `{needle}`"
            );
            let error = verify_windows_installer_preflight_sources(&unsafe_template).unwrap_err();
            assert!(error.to_string().contains(message));
        }

        let unsafe_template = template.replacen(
            "  ${If} $InstallVersionRelation == \"${RELATION_OLDER}\"\n    SetErrorLevel 2\n    Abort\n  ${EndIf}",
            "  ${If} $InstallVersionRelation == \"${RELATION_SAME}\"\n    SetErrorLevel 2\n    Abort\n  ${EndIf}",
            1,
        );
        assert_ne!(
            unsafe_template, template,
            "downgrade-policy mutation fixture did not match"
        );
        let error = verify_windows_installer_preflight_sources(&unsafe_template).unwrap_err();
        assert!(error.to_string().contains("reject every downgrade"));
    }

    #[test]
    fn windows_installer_preflight_dispatches_wix_before_legacy_reinstall_logic() {
        let (template, _) = windows_update_handoff_sources();
        let unsafe_template = template.replacen(
            "Function PageLeaveReinstall\n  ${NSD_GetState} $R2 $R1\n\n  ${If} $ExistingInstallKind == \"wix\"",
            "Function PageLeaveReinstall\n  ${NSD_GetState} $R2 $R1\n\n  ${If} $ExistingInstallKind == \"wix\"\n    Goto reinst_done\n  ${EndIf}\n\n  ${If} $ExistingInstallKind == \"wix\"",
            1,
        );
        assert_ne!(
            unsafe_template, template,
            "WiX dispatcher-order mutation fixture did not match"
        );

        let error = verify_windows_installer_preflight_sources(&unsafe_template).unwrap_err();

        assert!(error.to_string().contains("before the legacy dispatcher"));
    }

    #[test]
    fn windows_installer_preflight_requires_explicit_wix_migration_selection() {
        let (template, _) = windows_update_handoff_sources();
        for (needle, replacement, message) in [
            (
                "    ${If} $R1 != ${BST_CHECKED}\n      Abort\n    ${EndIf}\n    Goto reinst_uninstall",
                "    ${If} $R1 == ${BST_CHECKED}\n      Abort\n    ${EndIf}\n    Goto reinst_uninstall",
                "explicitly checked migration choice",
            ),
            (
                "      ExecWait '$R1' $0",
                "      DetailPrint '$R1'",
                "only from PageLeaveReinstall",
            ),
        ] {
            let unsafe_template = template.replacen(needle, replacement, 1);
            assert_ne!(
                unsafe_template, template,
                "WiX selection mutation fixture did not match `{needle}`"
            );
            let error = verify_windows_installer_preflight_sources(&unsafe_template).unwrap_err();
            assert!(error.to_string().contains(message));
        }
    }

    #[test]
    fn windows_update_handoff_rejects_a_missing_private_update_flag() {
        let (template, updater) = windows_update_handoff_sources();
        let unsafe_template = template.replace(
            "${GetOptions} $CMDLINE \"/AIRWIKIUPDATE\" $R0",
            "${GetOptions} $CMDLINE \"/UNSAFE\" $R0",
        );

        let error = verify_windows_update_handoff_sources(&unsafe_template, &updater).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("does not require /AIRWIKIUPDATE")
        );
    }

    #[test]
    fn windows_update_handoff_rejects_replay_or_downgrade_in_updater_mode() {
        let (template, updater) = windows_update_handoff_sources();
        let unsafe_template = template.replace(
            "${If} $InstallVersionRelation != \"${RELATION_NEWER}\"",
            "${If} $InstallVersionRelation == \"${RELATION_OLDER}\"",
        );
        assert_ne!(unsafe_template, template);

        let error = verify_windows_update_handoff_sources(&unsafe_template, &updater).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("strictly newer embedded version")
        );
    }

    #[test]
    fn windows_update_handoff_rejects_downgrade_policy_bypass() {
        let (template, updater) = windows_update_handoff_sources();
        let unsafe_template = template.replace(
            "${If} $InstallVersionRelation == \"${RELATION_OLDER}\"",
            "${If} $InstallVersionRelation == \"${RELATION_SAME}\"",
        );
        assert_ne!(unsafe_template, template);

        let error = verify_windows_update_handoff_sources(&unsafe_template, &updater).unwrap_err();

        assert!(error.to_string().contains("every downgrade"));
    }

    #[test]
    fn windows_update_handoff_rejects_forced_recovery_before_the_clean_wait() {
        let (template, updater) = windows_update_handoff_sources();
        let unsafe_template = template.replace(
            "Call WaitForAirWikiUpdateShutdown\n  !insertmacro CheckIfAppIsRunning",
            "!insertmacro CheckIfAppIsRunning\n  Call WaitForAirWikiUpdateShutdown",
        );

        let error = verify_windows_update_handoff_sources(&unsafe_template, &updater).unwrap_err();

        assert!(error.to_string().contains("before stuck-process recovery"));
    }

    #[test]
    fn windows_update_handoff_rejects_an_incomplete_nsis_argument_contract() {
        let (template, updater) = windows_update_handoff_sources();
        let unsafe_updater = updater.replace(
            "const WINDOWS_INSTALLER_ARGS: [&str; 3] = [\"/P\", \"/R\", \"/AIRWIKIUPDATE\"]",
            "const WINDOWS_INSTALLER_ARGS: [&str; 2] = [\"/P\", \"/R\"]",
        );

        let error = verify_windows_update_handoff_sources(&template, &unsafe_updater).unwrap_err();

        assert!(error.to_string().contains("/P /R /AIRWIKIUPDATE"));
    }

    #[test]
    fn windows_update_handoff_rejects_missing_inherited_package_guards() {
        let (template, updater) = windows_update_handoff_sources();
        let unsafe_updater = updater.replace(
            "PROC_THREAD_ATTRIBUTE_HANDLE_LIST",
            "PROC_THREAD_ATTRIBUTE_PARENT_PROCESS",
        );

        let error = verify_windows_update_handoff_sources(&template, &unsafe_updater).unwrap_err();

        assert!(error.to_string().contains("inherited guards"));
    }

    #[test]
    fn windows_update_handoff_rejects_path_only_artifact_validation() {
        let (template, updater) = windows_update_handoff_sources();
        let unsafe_updater = updater.replace(
            "compare_staged_package(&mut installer, package)?",
            "installer.seek(SeekFrom::Start(0))?",
        );

        let error = verify_windows_update_handoff_sources(&template, &unsafe_updater).unwrap_err();

        assert!(error.to_string().contains("revalidate exact bytes"));
    }

    #[test]
    fn windows_update_handoff_rejects_a_reparse_following_final_open() {
        let (template, updater) = windows_update_handoff_sources();
        let unsafe_updater = updater.replace(
            ".custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)",
            ".custom_flags(0)",
        );

        let error = verify_windows_update_handoff_sources(&template, &unsafe_updater).unwrap_err();

        assert!(error.to_string().contains("non-reparse read-only handle"));
    }

    #[test]
    fn windows_update_handoff_rejects_an_unbound_manifest_version() {
        let (template, updater) = windows_update_handoff_sources();
        let unsafe_updater = updater.replace(
            "expected_windows_update_version(&update.version, env!(\"CARGO_PKG_VERSION\"))",
            "expected_windows_update_version(\"999.0.0\", env!(\"CARGO_PKG_VERSION\"))",
        );

        let error = verify_windows_update_handoff_sources(&template, &unsafe_updater).unwrap_err();

        assert!(error.to_string().contains("signed PE versions"));
    }

    #[test]
    fn windows_update_handoff_rejects_single_field_pe_version_checks() {
        let (template, updater) = windows_update_handoff_sources();
        let unsafe_updater = updater.replace(
            "fixed_info.dwProductVersionMS",
            "fixed_info.dwFileVersionMS",
        );

        let error = verify_windows_update_handoff_sources(&template, &unsafe_updater).unwrap_err();

        assert!(error.to_string().contains("signed PE versions"));
    }

    #[test]
    fn windows_update_handoff_rejects_worker_process_termination() {
        let (template, mut updater) = windows_update_handoff_sources();
        updater.push_str("\nprocess::exit(0);\n");

        let error = verify_windows_update_handoff_sources(&template, &updater).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("must not use PowerShell or terminate")
        );
    }

    #[test]
    fn windows_uninstaller_rejects_autostart_cleanup_without_exact_comparison() {
        let (config, template) = windows_uninstaller_sources();
        let unsafe_template = template.replace(
            "StrCmp $R0 $R1 0 autostart_cleanup_done",
            "Goto autostart_cleanup_delete",
        );

        let error = verify_windows_uninstaller_sources(&config, &unsafe_template).unwrap_err();

        assert!(error.to_string().contains("exact comparison"));
    }

    #[test]
    fn windows_uninstaller_rejects_preselected_destructive_choices() {
        let (config, mut template) = windows_uninstaller_sources();
        template.push_str("\nSendMessage $DeleteAppDataCheckbox ${BM_SETCHECK} ${BST_CHECKED} 0\n");

        let error = verify_windows_uninstaller_sources(&config, &template).unwrap_err();

        assert!(error.to_string().contains("unchecked by default"));
    }

    #[test]
    fn windows_uninstaller_rejects_an_additional_appdata_root() {
        let (config, template) = windows_uninstaller_sources();
        let unsafe_config = config.replace(
            "  \"$APPDATA/airwiki/AirWiki\",\n]",
            "  \"$APPDATA/airwiki/AirWiki\",\n  \"$LOCALAPPDATA\",\n]",
        );

        let error = verify_windows_uninstaller_sources(&unsafe_config, &template).unwrap_err();

        assert!(error.to_string().contains("exactly the two managed"));
    }

    #[test]
    fn windows_uninstaller_rejects_a_second_autostart_mutation() {
        let (config, template) = windows_uninstaller_sources();
        let unsafe_template = template.replacen(
            "    DeleteRegValue HKCU \"${AUTOSTARTKEY}\" \"${AUTOSTARTVALUENAME}\"",
            "    DeleteRegValue HKCU \"${AUTOSTARTKEY}\" \"${AUTOSTARTVALUENAME}\"\n    DeleteRegValue HKCU \"${AUTOSTARTKEY}\" \"${AUTOSTARTVALUENAME}\"",
            1,
        );

        let error = verify_windows_uninstaller_sources(&config, &unsafe_template).unwrap_err();

        assert!(error.to_string().contains("exactly one guarded"));
    }

    #[test]
    fn windows_uninstaller_rejects_recursive_deletion_outside_confirmation() {
        let (config, template) = windows_uninstaller_sources();
        let unsafe_template = template.replacen(
            "  ; Delete app data",
            "  RmDir /r \"$LOCALAPPDATA\"\n\n  ; Delete app data",
            1,
        );

        let error = verify_windows_uninstaller_sources(&config, &unsafe_template).unwrap_err();

        assert!(error.to_string().contains("no recursive deletion"));
    }

    #[test]
    fn legal_filename_detection_includes_notices_but_not_source_code() {
        assert!(is_legal_filename(Path::new("crate"), "LICENSE-MIT"));
        assert!(is_legal_filename(
            Path::new("third_party"),
            "THIRD_PARTY_NOTICES.txt"
        ));
        assert!(is_legal_filename(Path::new("fonts"), "Hack-Regular.txt"));
        assert!(!is_legal_filename(Path::new("src"), "copying.rs"));
        assert!(!is_legal_filename(Path::new("fonts"), "Font.ttf"));
    }

    #[test]
    fn legal_text_normalization_is_platform_independent() {
        assert_eq!(normalize_legal_text(b"one\r\ntwo\r\n\r\n"), "one\ntwo\n");
    }

    #[test]
    fn non_cargo_legal_inventory_matches_verified_upstream_texts() {
        validate_non_cargo_legal_inventory(&workspace_root()).unwrap();
    }

    #[test]
    fn workflow_gate_accepts_the_repository_workflows() {
        validate_workflow_action_references(&workspace_root()).unwrap();
    }

    #[test]
    fn workflow_gate_rejects_a_moving_external_tag() {
        let error = validate_workflow_action_references_in(
            Path::new(".github/workflows/example.yml"),
            "jobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n",
        )
        .unwrap_err();

        assert!(error.to_string().contains("full 40-character commit SHA"));
    }

    #[test]
    fn workflow_gate_rejects_an_unaudited_local_action() {
        let error = validate_workflow_action_references_in(
            Path::new(".github/workflows/example.yml"),
            "jobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: ./.github/actions/build\n",
        )
        .unwrap_err();

        assert!(error.to_string().contains("local `uses` is forbidden"));
    }

    #[test]
    fn workflow_gate_rejects_an_unaudited_local_reusable_workflow() {
        let error = validate_workflow_action_references_in(
            Path::new(".github/workflows/example.yml"),
            "jobs:\n  reused:\n    uses: ./.github/workflows/reused.yml\n",
        )
        .unwrap_err();

        assert!(error.to_string().contains("local `uses` is forbidden"));
    }

    #[test]
    fn workflow_gate_accepts_a_quoted_commit_with_a_version_comment() {
        validate_workflow_action_references_in(
            Path::new(".github/workflows/example.yml"),
            "jobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: 'actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5' # v4.3.1\n",
        )
        .unwrap();
    }

    #[test]
    fn workflow_gate_rejects_a_moving_tag_in_a_flow_mapping() {
        let error = validate_workflow_action_references_in(
            Path::new(".github/workflows/example.yml"),
            "jobs: { test: { runs-on: ubuntu-latest, steps: [ { uses: actions/checkout@v4 } ] } }\n",
        )
        .unwrap_err();

        assert!(error.to_string().contains("full 40-character commit SHA"));
    }

    #[test]
    fn workflow_gate_rejects_a_moving_tag_behind_a_quoted_key() {
        let error = validate_workflow_action_references_in(
            Path::new(".github/workflows/example.yml"),
            "jobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - \"uses\": actions/checkout@v4\n",
        )
        .unwrap_err();

        assert!(error.to_string().contains("full 40-character commit SHA"));
    }

    #[test]
    fn workflow_gate_rejects_an_opaque_yaml_merge_in_an_action_step() {
        let error = validate_workflow_action_references_in(
            Path::new(".github/workflows/example.yml"),
            "action: &action\n  uses: actions/checkout@v4\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - <<: *action\n",
        )
        .unwrap_err();

        assert!(
            error.to_string().contains("YAML merge key")
                || error.to_string().contains("full 40-character commit SHA")
        );
    }

    #[test]
    fn workflow_gate_rejects_a_step_without_uses_or_run() {
        let error = validate_workflow_action_references_in(
            Path::new(".github/workflows/example.yml"),
            "jobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - name: Empty\n",
        )
        .unwrap_err();

        assert!(error.to_string().contains("exactly one of `uses` or `run`"));
    }

    #[test]
    fn workflow_gate_rejects_a_step_with_uses_and_run() {
        let error = validate_workflow_action_references_in(
            Path::new(".github/workflows/example.yml"),
            "jobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5\n        run: echo unsafe\n",
        )
        .unwrap_err();

        assert!(error.to_string().contains("exactly one of `uses` or `run`"));
    }

    #[test]
    fn workflow_gate_ignores_uses_text_in_comments() {
        validate_workflow_action_references_in(
            Path::new(".github/workflows/example.yml"),
            "# uses: actions/checkout@v4\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      # uses: actions/upload-artifact@v4\n      - run: echo safe\n",
        )
        .unwrap();
    }

    #[test]
    fn workflow_gate_ignores_uses_text_in_block_scalars() {
        validate_workflow_action_references_in(
            Path::new(".github/workflows/example.yml"),
            "jobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - run: |\n          uses: actions/checkout@v4\n      - run: >-\n          uses: actions/upload-artifact@v4\n",
        )
        .unwrap();
    }

    #[test]
    fn workflow_gate_ignores_uses_text_in_multiline_quoted_scalars() {
        validate_workflow_action_references_in(
            Path::new(".github/workflows/example.yml"),
            "env:\n  DESCRIPTION: \"first line\n    uses: actions/checkout@v4\n    final line\"\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo safe\n",
        )
        .unwrap();
    }

    #[test]
    fn application_id_gate_rejects_the_unlicensed_plugin() {
        let error = validate_application_id_is_not_distributed(
            "ApplicationID::Set \"shortcut.lnk\" \"app-id\"",
        )
        .unwrap_err();

        assert_eq!(error.to_string(), APPLICATION_ID_LICENSE_ERROR);
    }

    #[test]
    fn application_id_gate_rejects_the_unlicensed_toolchain_download() {
        let error = validate_application_id_toolchain_is_inert(
            "Invoke-WebRequest https://example.invalid/NSIS-ApplicationID.zip",
        )
        .unwrap_err();

        assert_eq!(error.to_string(), APPLICATION_ID_LICENSE_ERROR);
    }

    #[test]
    fn markdown_fence_is_longer_than_embedded_backticks() {
        assert_eq!(code_fence("text with ``` inside"), "````");
    }

    #[test]
    fn relevance_fixture_v2_has_the_expected_reviewed_shape() {
        let loaded = load_relevance_fixture().unwrap();
        assert_eq!(
            (
                loaded.fixture.schema_version,
                loaded.fixture.cases.len(),
                loaded.sha256.len()
            ),
            (2, EXPECTED_RELEVANCE_CASE_IDS.len(), 64)
        );
    }

    #[test]
    fn relevance_fixture_rejects_a_reduced_case_set() {
        let mut fixture = load_relevance_fixture().unwrap().fixture;
        fixture.cases.pop();
        assert!(validate_relevance_fixture_data(&fixture).is_err());
    }

    #[test]
    fn relevance_fixture_rejects_overlapping_calibration_and_holdout_domains() {
        let mut fixture = load_relevance_fixture().unwrap().fixture;
        let holdout = fixture
            .cases
            .iter_mut()
            .find(|case| case.split == RelevanceSplit::Holdout)
            .unwrap();
        holdout.domain = "project_operations".to_owned();
        assert!(validate_relevance_fixture_data(&fixture).is_err());
    }

    #[test]
    fn relevance_fixture_rejects_empty_candidate_metadata() {
        let mut fixture = load_relevance_fixture().unwrap().fixture;
        fixture.cases[0].candidates[0].heading.clear();
        assert!(validate_relevance_fixture_data(&fixture).is_err());
    }

    #[test]
    fn relevance_fixture_rejects_missing_security_category() {
        let mut fixture = load_relevance_fixture().unwrap().fixture;
        for case in fixture
            .cases
            .iter_mut()
            .filter(|case| case.split == RelevanceSplit::Holdout)
        {
            case.tags.retain(|tag| *tag != RelevanceTag::Injection);
        }
        assert!(validate_relevance_fixture_data(&fixture).is_err());
    }

    #[test]
    fn core_and_inference_pin_the_same_relevance_revision() {
        assert_eq!(MMARCO_RERANKER_REVISION, MMARCO_REVISION);
    }

    #[test]
    fn packaging_scripts_pin_and_authenticate_each_platform_runtime() {
        let root = workspace_root();
        let macos = fs::read_to_string(root.join("packaging/fetch-llama-macos.sh")).unwrap();
        let windows = fs::read_to_string(root.join("packaging/fetch-llama-windows.ps1")).unwrap();

        assert!(macos.contains(MACOS_LLAMA_SERVER_SHA256));
        assert!(
            windows.contains("llama-windows-build-policy.json")
                && windows.contains("BUILD-MANIFEST.json")
                && windows.contains("Get-ImportedDlls")
                && !windows.contains("llama-b9946-bin-win-cpu-x64.zip")
        );
        validate_windows_llama_runtime_supply_chain(&root).unwrap();
    }

    #[test]
    fn macos_package_matches_fresh_build_by_uuid_and_architecture() {
        let script =
            fs::read_to_string(workspace_root().join("packaging/package-macos.sh")).unwrap();

        assert!(
            script.contains("xcrun lipo -archs")
                && script.contains("xcrun dwarfdump --uuid")
                && script.contains(r#"[ "$RELEASE_ARCH" != "arm64" ]"#)
                && script.contains(r#"[ "$PACKAGED_ARCH" != "arm64" ]"#)
                && script.contains(r#"[ "$RELEASE_UUID" != "$PACKAGED_UUID" ]"#)
                && script.contains("packaging-macos-ready.stamp")
                && script.contains("rm -f -- \"$SOURCE_MCPB\" \"$READY_STAMP\"")
        );
    }

    #[test]
    fn macos_package_does_not_compare_resigned_binary_bytes() {
        let script =
            fs::read_to_string(workspace_root().join("packaging/package-macos.sh")).unwrap();

        assert!(!script.contains("cmp -s \"$RELEASE_BINARY\""));
        assert!(!script.contains("cmp -s \"$RELEASE_BRIDGE\""));
    }

    #[test]
    fn macos_packaging_supports_development_and_release_signing() {
        let root = workspace_root();
        let config = fs::read_to_string(root.join("packaging/macos/Packager.toml")).unwrap();
        let script = fs::read_to_string(root.join("packaging/package-macos.sh")).unwrap();

        assert!(
            !config.contains("signingIdentity")
                && config.contains("./packaging/sign-macos-bridge.sh")
                && script.contains("SIGNING_IDENTITY=${AIRWIKI_SIGNING_IDENTITY:--}")
                && script.contains(
                    "codesign --force --sign \"$SIGNING_IDENTITY\" --options runtime --timestamp"
                )
                && script.contains("Contents/_CodeSignature/CodeResources")
                && script.contains("codesign --verify --deep --strict")
                && script.contains("Signature=adhoc")
                && script.contains("Authority=Developer ID Application:")
                && script.contains("Runtime Version=")
                && script.contains("Sealed Resources version=")
                && !script.contains("codesign --force --deep")
        );
    }

    #[test]
    fn macos_packaging_preserves_runtime_and_converges_on_dmg_verification() {
        let script =
            fs::read_to_string(workspace_root().join("packaging/package-macos.sh")).unwrap();
        assert!(
            script.contains("PACKAGED_RUNTIME_DIR")
                && script.contains("diff -qr")
                && script.contains("find \"$PACKAGED_RUNTIME_DIR\" -type l")
        );
        let runtime_check = script.find("if ! runtime_bytes_match").unwrap();
        let signing = script
            .find("codesign --force --sign \"$SIGNING_IDENTITY\"")
            .unwrap();
        let post_sign_runtime_check = script.rfind("if ! runtime_bytes_match").unwrap();
        let app_verification = script.find("codesign --verify --deep --strict").unwrap();
        let dmg_creation = script.find("\"$CREATE_DMG\" \\").unwrap();
        let dmg_verification = script.find("hdiutil verify").unwrap();

        assert!(
            runtime_check < signing
                && signing < post_sign_runtime_check
                && post_sign_runtime_check < app_verification
                && app_verification < dmg_creation
                && dmg_creation < dmg_verification
        );
    }

    #[test]
    fn packaging_includes_platform_bridge_and_validated_mcpb() {
        let root = workspace_root();
        let macos = fs::read_to_string(root.join("packaging/macos/Packager.toml")).unwrap();
        let windows = fs::read_to_string(root.join("packaging/windows/Packager.toml")).unwrap();
        let windows_wrapper =
            fs::read_to_string(root.join("packaging/package-windows.ps1")).unwrap();
        for config in [&macos, &windows] {
            assert!(config.contains("airwiki-mcp-bridge"));
            assert!(config.contains("airwiki-claude.mcpb"));
            assert!(config.contains("resources/licenses"));
        }
        assert!(windows.contains("airwiki-windows-firewall-helper.exe"));
        assert!(macos.contains("mcpb build"));
        assert!(!windows.contains("beforePackagingCommand"));
        assert!(windows_wrapper.contains("licenses check"));
        assert!(windows_wrapper.contains("fetch-llama-windows.ps1"));
        assert!(windows_wrapper.contains("cargo build --locked --release"));
        assert!(windows_wrapper.contains("mcpb build"));
        let mcpb_build = windows_wrapper.find("mcpb build").unwrap();
        let packaging = windows_wrapper
            .find("& $CargoPackager --config packaging/windows/Packager.toml")
            .unwrap();
        assert!(mcpb_build < packaging);
    }

    #[test]
    fn windows_wrapper_rejects_stale_or_non_x64_payloads() {
        let script =
            fs::read_to_string(workspace_root().join("packaging/package-windows.ps1")).unwrap();
        assert!(script.contains("windows-safe-staging.ps1"));
        assert!(script.contains("Remove-AirWikiWindowsStagingPath"));
        assert!(script.contains("-AllowedRoot (Join-Path $Root \"target\")"));
        assert!(!script.contains("Remove-Item -LiteralPath $OutDir -Recurse -Force"));
        assert!(script.contains("$Bytes[$Offset + 4] -ne 0x64"));
        assert!(script.contains("mcpb verify"));
        assert!(script.contains("$Installers.Count -ne 1"));
        assert!(script.contains("LastWriteTimeUtc -lt $Started"));
        assert!(script.contains("prepare-verified-7zip.ps1"));
        assert!(script.contains("target\\verified-tools\\7zip-26.02"));
        assert!(!script.contains("Get-Command 7z.exe"));
        assert!(script.contains("Get-FileHash -LiteralPath"));
        assert!(
            script.contains(
                "(Join-Path $PayloadRoot \"integrations\\bridge\\airwiki-mcp-bridge.exe\")"
            )
        );
        assert!(
            script.contains("(Join-Path $PayloadRoot \"airwiki-windows-firewall-helper.exe\")")
        );
        assert!(script.contains(
            "(Join-Path $Root \"resources\\licenses\") `\n            (Join-Path $PayloadRoot \"licenses\")"
        ));
        assert!(!script.contains("Get-SinglePayload $ExtractDir \"airwiki-mcp-bridge.exe\""));
    }

    #[test]
    fn windows_runtime_verification_is_bounded_and_compares_the_exact_tree() {
        let root = workspace_root();
        let runtime = fs::read_to_string(root.join("packaging/windows-runtime.ps1")).unwrap();
        let package = fs::read_to_string(root.join("packaging/package-windows.ps1")).unwrap();
        let verify = fs::read_to_string(root.join("packaging/verify-windows-release.ps1")).unwrap();

        assert!(
            runtime.contains("WindowsRuntimeMaxEntries = 4096")
                && runtime.contains("WindowsRuntimeMaxBytes = 2GB")
                && runtime.contains("Queue[IO.DirectoryInfo]")
                && runtime.contains("FileAttributes]::ReparsePoint")
                && runtime.contains("SortedDictionary[string, object]")
                && runtime.contains("Length = $Length")
                && runtime.contains("Sha256 = $Hash")
                && runtime.contains("Test-WindowsOrdinalSequenceEqual")
                && runtime.contains("Set-WindowsAtomicFileReplacement")
                && runtime.contains("Invoke-WindowsToolVersionLine")
                && runtime.contains("function Get-WindowsFileSha256")
                && runtime.contains("[Security.Cryptography.SHA256]::Create()")
                && runtime.contains("[IO.FileShare]::Read")
                && !runtime.contains("Get-FileHash")
                && runtime.contains("$Expected.Directories.Count")
                && runtime.contains("Get-WindowsPackagedRuntimeRoot")
                && !runtime.contains("Get-ChildItem -LiteralPath $RootPath -Recurse")
                && package.contains("Get-WindowsPackagedRuntimeRoot")
                && verify.contains("Get-WindowsPackagedRuntimeRoot")
                && verify.contains("AIRWIKI_XTASK is required in CI")
        );
    }

    #[test]
    fn windows_firewall_helper_manifest_is_verified_after_packaging_and_installation() {
        let root = workspace_root();
        let runtime = fs::read_to_string(root.join("packaging/windows-runtime.ps1")).unwrap();
        let package = fs::read_to_string(root.join("packaging/package-windows.ps1")).unwrap();
        let signed = fs::read_to_string(root.join("packaging/package-signed-windows.ps1")).unwrap();
        let verify = fs::read_to_string(root.join("packaging/verify-windows-release.ps1")).unwrap();
        let smoke = fs::read_to_string(root.join("packaging/smoke-install-windows.ps1")).unwrap();

        assert!(
            runtime.contains("ManifestResourceType = new IntPtr(24)")
                && runtime.contains("PrimaryManifest = new IntPtr(1)")
                && runtime.contains("LoadLibraryAsDataFileExclusive")
                && runtime.contains("EnumResourceLanguagesW")
                && runtime.contains("FindResourceExW")
                && runtime.contains("MaximumLanguageVariants = 16")
                && runtime.contains("Assert-WindowsFirewallHelperManifestVariants")
                && runtime.contains("exactly one application manifest language variant")
                && runtime.contains("DtdProcessing]::Prohibit")
                && runtime.contains("$ExpectedNodes.Count -ne 1")
                && runtime.contains("$AllExecutionLevels.Count -ne 1")
                && runtime.contains("requireAdministrator")
                && runtime.contains("uiAccess=false")
                && package.matches("Assert-WindowsFirewallHelperManifest").count() == 2
                && signed.contains(
                    "Assert-WindowsFirewallHelperManifest $Helper \"signed Windows firewall helper\""
                )
                && verify.matches("Assert-WindowsFirewallHelperManifest").count() == 2
                && smoke.contains("installed Windows firewall helper")
        );
    }

    #[test]
    fn windows_runtime_is_built_before_the_desktop_and_bound_after_build() {
        let script =
            fs::read_to_string(workspace_root().join("packaging/package-windows.ps1")).unwrap();
        let fetch = script.find("fetch-llama-windows.ps1").unwrap();
        let cargo_release = script
            .find("cargo build --locked --release --target x86_64-pc-windows-msvc")
            .unwrap();
        let receipt = script
            .find("Assert-WindowsDesktopEmbedsLlamaRuntimeHash")
            .unwrap();
        let mcpb_build = script.find("& $Xtask mcpb build").unwrap();

        assert!(fetch < cargo_release && cargo_release < receipt && receipt < mcpb_build);
    }

    #[cfg(windows)]
    fn powershell_literal(path: &Path) -> String {
        format!("'{}'", path.display().to_string().replace('\'', "''"))
    }

    #[cfg(windows)]
    fn run_windows_runtime_script(body: &str) -> std::process::Output {
        let runtime = powershell_literal(&workspace_root().join("packaging/windows-runtime.ps1"));
        let command = format!("$ErrorActionPreference='Stop'; . {runtime}; {body}");
        std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &command,
            ])
            .output()
            .unwrap()
    }

    #[cfg(windows)]
    fn run_windows_signing_script(body: &str) -> std::process::Output {
        let signing = powershell_literal(&workspace_root().join("packaging/windows-signing.ps1"));
        let command = format!("$ErrorActionPreference='Stop'; . {signing}; {body}");
        std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &command,
            ])
            .output()
            .unwrap()
    }

    #[cfg(windows)]
    #[test]
    fn windows_signer_identity_accepts_one_canonical_durable_eku() {
        let output = run_windows_signing_script(
            "$Identity='1.3.6.1.4.1.311.97.123.456'; \
             if ((Get-SingleArtifactSigningDurableIdentity @($Identity) 'fixture') -cne $Identity) { throw 'identity changed' }",
        );

        assert!(output.status.success(), "{}", powershell_output(&output));
    }

    #[cfg(windows)]
    #[test]
    fn windows_signer_identity_rejects_a_duplicate_durable_eku() {
        let output = run_windows_signing_script(
            "$Identity='1.3.6.1.4.1.311.97.123.456'; \
             Get-SingleArtifactSigningDurableIdentity @($Identity,$Identity) 'fixture'",
        );

        assert!(
            !output.status.success()
                && powershell_output(&output).contains("exactly one Artifact Signing subscriber")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_signer_identity_rejects_surrounding_whitespace() {
        let output = run_windows_signing_script(
            "Assert-ValidExpectedWindowsSignerIdentity ' artifact-signing-eku:1.3.6.1.4.1.311.97.123.456'",
        );

        assert!(
            !output.status.success()
                && powershell_output(&output).contains("must not contain surrounding whitespace")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_authenticode_der_rejects_nonzero_certificate_padding() {
        let output = run_windows_signing_script(
            "$Bytes=[byte[]](0x30,0x03,0x02,0x01,0x01,0xff); \
             Get-SingleDerObject $Bytes 'fixture'",
        );

        assert!(
            !output.status.success()
                && powershell_output(&output).contains("nonzero certificate padding")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_absolute_path_rejects_drive_relative_input() {
        let output = run_windows_runtime_script(
            "Assert-WindowsAbsolutePath 'C:relative\\payload' 'fixture path'",
        );

        assert!(
            !output.status.success()
                && powershell_output(&output).contains("must be an absolute path")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_manifest_sequence_comparison_rejects_reordered_flags() {
        let output = run_windows_runtime_script(
            "if (Test-WindowsOrdinalSequenceEqual @('/O2','/GL') @('/GL','/O2')) { throw 'reordered flags were accepted' }",
        );

        assert!(output.status.success(), "{}", powershell_output(&output));
    }

    #[cfg(windows)]
    #[test]
    fn windows_tool_version_rejects_nonzero_exit_after_output() {
        let directory = tempfile::tempdir().unwrap();
        let tool = directory.path().join("failing-version.cmd");
        fs::write(&tool, "@echo 1.11.1\r\n@exit /b 7\r\n").unwrap();
        let command = format!(
            "Invoke-WindowsToolVersionLine {} @('--version') 'fixture tool'",
            powershell_literal(&tool)
        );

        let output = run_windows_runtime_script(&command);

        assert!(
            !output.status.success()
                && powershell_output(&output).contains("failed with exit code 7")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_atomic_replacement_recovers_a_committed_backup() {
        let directory = tempfile::tempdir().unwrap();
        let staged = directory.path().join("manifest.tmp");
        let destination = directory.path().join("manifest.json");
        let backup = directory.path().join("manifest.backup");
        fs::write(&staged, b"new receipt").unwrap();
        fs::write(&destination, b"new receipt").unwrap();
        fs::write(&backup, b"old receipt").unwrap();
        let command = format!(
            "Set-WindowsAtomicFileReplacement {} {} {} 'fixture receipt'",
            powershell_literal(&staged),
            powershell_literal(&destination),
            powershell_literal(&backup)
        );

        let output = run_windows_runtime_script(&command);

        assert!(output.status.success(), "{}", powershell_output(&output));
        assert_eq!(fs::read(&destination).unwrap(), b"new receipt");
        assert!(!staged.exists());
        assert!(!backup.exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_atomic_replacement_preserves_ambiguous_recovery_state() {
        let directory = tempfile::tempdir().unwrap();
        let staged = directory.path().join("manifest.tmp");
        let destination = directory.path().join("manifest.json");
        let backup = directory.path().join("manifest.backup");
        fs::write(&staged, b"new receipt").unwrap();
        fs::write(&destination, b"unexpected receipt").unwrap();
        fs::write(&backup, b"old receipt").unwrap();
        let command = format!(
            "Set-WindowsAtomicFileReplacement {} {} {} 'fixture receipt'",
            powershell_literal(&staged),
            powershell_literal(&destination),
            powershell_literal(&backup)
        );

        let output = run_windows_runtime_script(&command);

        assert!(
            !output.status.success()
                && powershell_output(&output).contains("ambiguous interrupted replacement")
        );
        assert_eq!(fs::read(&staged).unwrap(), b"new receipt");
        assert_eq!(fs::read(&destination).unwrap(), b"unexpected receipt");
        assert_eq!(fs::read(&backup).unwrap(), b"old receipt");
    }

    #[cfg(windows)]
    fn assert_windows_manifest_command(xml: &str) -> String {
        let encoded = BASE64_STANDARD.encode(xml.as_bytes());
        format!(
            "$Bytes=[Convert]::FromBase64String('{encoded}'); \
             Assert-WindowsFirewallHelperManifestBytes $Bytes 'fixture helper'"
        )
    }

    #[cfg(windows)]
    const VALID_FIREWALL_HELPER_MANIFEST: &str = r#"
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="requireAdministrator" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#;

    #[cfg(windows)]
    #[test]
    fn windows_manifest_verification_accepts_the_required_elevation_contract() {
        let command = assert_windows_manifest_command(VALID_FIREWALL_HELPER_MANIFEST);

        let output = run_windows_runtime_script(&command);

        assert!(output.status.success(), "{}", powershell_output(&output));
    }

    #[cfg(windows)]
    #[test]
    fn windows_manifest_verification_rejects_a_non_elevated_helper() {
        let manifest = VALID_FIREWALL_HELPER_MANIFEST.replace("requireAdministrator", "asInvoker");
        let command = assert_windows_manifest_command(&manifest);

        let output = run_windows_runtime_script(&command);

        assert!(
            !output.status.success()
                && powershell_output(&output).contains("must request requireAdministrator")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_manifest_verification_rejects_ui_access() {
        let manifest =
            VALID_FIREWALL_HELPER_MANIFEST.replace("uiAccess=\"false\"", "uiAccess=\"true\"");
        let command = assert_windows_manifest_command(&manifest);

        let output = run_windows_runtime_script(&command);

        assert!(
            !output.status.success()
                && powershell_output(&output).contains("must set uiAccess=false")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_manifest_verification_rejects_duplicate_execution_levels() {
        let duplicate = concat!(
            "<requestedExecutionLevel level=\"requireAdministrator\" uiAccess=\"false\" />",
            "<requestedExecutionLevel level=\"requireAdministrator\" uiAccess=\"false\" />"
        );
        let manifest = VALID_FIREWALL_HELPER_MANIFEST.replace(
            "<requestedExecutionLevel level=\"requireAdministrator\" uiAccess=\"false\" />",
            duplicate,
        );
        let command = assert_windows_manifest_command(&manifest);

        let output = run_windows_runtime_script(&command);

        assert!(
            !output.status.success()
                && powershell_output(&output)
                    .contains("must contain exactly one requestedExecutionLevel")
        );
    }

    #[cfg(windows)]
    fn assert_windows_manifest_variants_command(
        first: &str,
        second: &str,
        second_lang: u16,
    ) -> String {
        let first = BASE64_STANDARD.encode(first.as_bytes());
        let second = BASE64_STANDARD.encode(second.as_bytes());
        format!(
            "$First=[pscustomobject]@{{Language=[uint16]0;Bytes=[Convert]::FromBase64String('{first}')}}; \
             $Second=[pscustomobject]@{{Language=[uint16]{second_lang};Bytes=[Convert]::FromBase64String('{second}')}}; \
             Assert-WindowsFirewallHelperManifestVariants @($First,$Second) 'fixture helper'"
        )
    }

    #[cfg(windows)]
    fn assert_windows_single_manifest_variant_command(xml: &str, language: u16) -> String {
        let encoded = BASE64_STANDARD.encode(xml.as_bytes());
        format!(
            "$Variant=[pscustomobject]@{{Language=[uint16]{language};Bytes=[Convert]::FromBase64String('{encoded}')}}; \
             Assert-WindowsFirewallHelperManifestVariants @($Variant) 'fixture helper'"
        )
    }

    #[cfg(windows)]
    #[test]
    fn windows_manifest_verification_accepts_one_neutral_language_variant() {
        let command =
            assert_windows_single_manifest_variant_command(VALID_FIREWALL_HELPER_MANIFEST, 0);

        let output = run_windows_runtime_script(&command);

        assert!(output.status.success(), "{}", powershell_output(&output));
    }

    #[cfg(windows)]
    #[test]
    fn windows_manifest_verification_rejects_multiple_identical_language_variants() {
        let command = assert_windows_manifest_variants_command(
            VALID_FIREWALL_HELPER_MANIFEST,
            VALID_FIREWALL_HELPER_MANIFEST,
            1033,
        );

        let output = run_windows_runtime_script(&command);

        assert!(
            !output.status.success()
                && powershell_output(&output).contains("exactly one application manifest")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_manifest_verification_accepts_one_non_neutral_language() {
        let command =
            assert_windows_single_manifest_variant_command(VALID_FIREWALL_HELPER_MANIFEST, 1033);

        let output = run_windows_runtime_script(&command);

        assert!(output.status.success(), "{}", powershell_output(&output));
    }

    #[cfg(windows)]
    fn write_windows_runtime_fixture(root: &Path, server: &[u8], backend: &[u8]) {
        fs::create_dir_all(root.join("backend")).unwrap();
        fs::write(root.join("llama-server.exe"), server).unwrap();
        fs::write(root.join("backend").join("ggml.dll"), backend).unwrap();
    }

    #[cfg(windows)]
    fn powershell_output(output: &std::process::Output) -> String {
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    }

    #[cfg(windows)]
    #[test]
    fn windows_runtime_verification_accepts_identical_nested_trees() {
        let directory = tempfile::tempdir().unwrap();
        let expected = directory.path().join("expected");
        let actual = directory.path().join("actual");
        write_windows_runtime_fixture(&expected, b"server", b"backend");
        write_windows_runtime_fixture(&actual, b"server", b"backend");
        let command = format!(
            "Assert-WindowsRuntimeTreeMatches {} {}",
            powershell_literal(&expected),
            powershell_literal(&actual)
        );

        let output = run_windows_runtime_script(&command);

        assert!(output.status.success(), "{}", powershell_output(&output));
    }

    #[cfg(windows)]
    #[test]
    fn windows_runtime_verification_rejects_an_extra_file() {
        let directory = tempfile::tempdir().unwrap();
        let expected = directory.path().join("expected");
        let actual = directory.path().join("actual");
        write_windows_runtime_fixture(&expected, b"server", b"backend");
        write_windows_runtime_fixture(&actual, b"server", b"backend");
        fs::write(actual.join("unexpected.dll"), b"unexpected").unwrap();
        let command = format!(
            "Assert-WindowsRuntimeTreeMatches {} {}",
            powershell_literal(&expected),
            powershell_literal(&actual)
        );

        let output = run_windows_runtime_script(&command);

        assert!(
            !output.status.success()
                && powershell_output(&output).contains("file set differs from the verified source")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_runtime_verification_rejects_an_extra_directory() {
        let directory = tempfile::tempdir().unwrap();
        let expected = directory.path().join("expected");
        let actual = directory.path().join("actual");
        write_windows_runtime_fixture(&expected, b"server", b"backend");
        write_windows_runtime_fixture(&actual, b"server", b"backend");
        fs::create_dir(actual.join("unexpected-empty-directory")).unwrap();
        let command = format!(
            "Assert-WindowsRuntimeTreeMatches {} {}",
            powershell_literal(&expected),
            powershell_literal(&actual)
        );

        let output = run_windows_runtime_script(&command);

        assert!(
            !output.status.success()
                && powershell_output(&output).contains("file set differs from the verified source")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_runtime_verification_rejects_same_length_modified_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let expected = directory.path().join("expected");
        let actual = directory.path().join("actual");
        write_windows_runtime_fixture(&expected, b"server-a", b"backend");
        write_windows_runtime_fixture(&actual, b"server-b", b"backend");
        let command = format!(
            "Assert-WindowsRuntimeTreeMatches {} {}",
            powershell_literal(&expected),
            powershell_literal(&actual)
        );

        let output = run_windows_runtime_script(&command);

        assert!(
            !output.status.success()
                && powershell_output(&output)
                    .contains("runtime bytes differ from the verified source")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_runtime_verification_rejects_a_traversal_root() {
        let directory = tempfile::tempdir().unwrap();
        let expected = directory.path().join("expected");
        write_windows_runtime_fixture(&expected, b"server", b"backend");
        let traversing = expected.join("..").join("expected");
        let command = format!(
            "Assert-WindowsRuntimeTreeMatches {} {}",
            powershell_literal(&expected),
            powershell_literal(&traversing)
        );

        let output = run_windows_runtime_script(&command);

        assert!(
            !output.status.success()
                && powershell_output(&output).contains("root contains a traversal segment")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_runtime_verification_enforces_the_entry_limit() {
        let directory = tempfile::tempdir().unwrap();
        let expected = directory.path().join("expected");
        write_windows_runtime_fixture(&expected, b"server", b"backend");
        let command = format!(
            "$script:WindowsRuntimeMaxEntries = 2; Get-VerifiedRuntimeTree {} 'fixture'",
            powershell_literal(&expected)
        );

        let output = run_windows_runtime_script(&command);

        assert!(
            !output.status.success()
                && powershell_output(&output).contains("runtime tree contains too many entries")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_runtime_verification_enforces_the_total_size_limit() {
        let directory = tempfile::tempdir().unwrap();
        let expected = directory.path().join("expected");
        write_windows_runtime_fixture(&expected, b"server", b"backend");
        let command = format!(
            "$script:WindowsRuntimeMaxBytes = 1; Get-VerifiedRuntimeTree {} 'fixture'",
            powershell_literal(&expected)
        );

        let output = run_windows_runtime_script(&command);

        assert!(
            !output.status.success()
                && powershell_output(&output).contains("runtime tree exceeds the size limit")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_runtime_verification_rejects_a_reparse_ancestor() {
        let directory = tempfile::tempdir().unwrap();
        let expected = directory.path().join("expected");
        let target_parent = directory.path().join("target-parent");
        let target_runtime = target_parent.join("runtime");
        let link_parent = directory.path().join("linked-parent");
        write_windows_runtime_fixture(&expected, b"server", b"backend");
        write_windows_runtime_fixture(&target_runtime, b"server", b"backend");
        let command = format!(
            "New-Item -ItemType Junction -Path {} -Target {} | Out-Null; \
             Assert-WindowsRuntimeTreeMatches {} {}",
            powershell_literal(&link_parent),
            powershell_literal(&target_parent),
            powershell_literal(&expected),
            powershell_literal(&link_parent.join("runtime"))
        );

        let output = run_windows_runtime_script(&command);

        assert!(
            !output.status.success()
                && powershell_output(&output).contains("ancestors is a reparse point")
        );
    }

    #[test]
    fn relevance_report_path_is_platform_specific() {
        let filename = relevance_report_path()
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap()
            .to_owned();
        assert_eq!(
            filename,
            format!(
                "relevance-model-{}-{}.json",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        );
    }

    #[test]
    fn relevance_acceptance_rejects_any_false_positive() {
        let counts = ClassificationCounts {
            true_positive: 10,
            false_positive: 1,
            true_negative: 10,
            false_negative: 0,
        };
        assert!(!split_passes(counts));
    }

    #[test]
    fn relevance_acceptance_requires_ninety_percent_recall() {
        let counts = ClassificationCounts {
            true_positive: 8,
            false_positive: 0,
            true_negative: 10,
            false_negative: 2,
        };
        assert!(!split_passes(counts));
    }

    #[test]
    fn relevance_acceptance_allows_exactly_ninety_percent_recall() {
        let counts = ClassificationCounts {
            true_positive: 9,
            false_positive: 0,
            true_negative: 10,
            false_negative: 1,
        };
        assert!(split_passes(counts));
    }

    #[test]
    fn mcpb_manifest_is_v03_binary_and_has_no_remote_endpoint() {
        let bytes = mcpb_manifest(McpbTarget::MacosArm64).unwrap();
        validate_mcpb_manifest(&bytes, McpbTarget::MacosArm64).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\"manifest_version\": \"0.3\""));
        assert!(!text.contains("http://"));
        assert!(!text.contains("https://"));
    }

    #[test]
    fn mcpb_manifest_resolves_the_binary_from_the_bundle_root() {
        let bytes = mcpb_manifest(McpbTarget::MacosArm64).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(
            value["server"]["mcp_config"]["command"],
            "${__dirname}/server/airwiki-mcp-bridge"
        );
    }

    #[test]
    fn mcpb_manifest_rejects_a_cwd_relative_binary_command() {
        let bytes = mcpb_manifest(McpbTarget::MacosArm64).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["server"]["mcp_config"]["command"] =
            serde_json::Value::String(McpbTarget::MacosArm64.binary_entry().to_owned());
        let modified = serde_json::to_vec_pretty(&value).unwrap();

        assert!(validate_mcpb_manifest(&modified, McpbTarget::MacosArm64).is_err());
    }

    #[test]
    fn mcpb_manifest_rejects_static_response_metadata() {
        let bytes = mcpb_manifest(McpbTarget::MacosArm64).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value.as_object_mut().unwrap().insert(
            "_meta".to_owned(),
            serde_json::json!({ "static_responses": {} }),
        );
        let modified = serde_json::to_vec_pretty(&value).unwrap();

        assert!(validate_mcpb_manifest(&modified, McpbTarget::MacosArm64).is_err());
    }

    #[test]
    fn mcpb_build_is_deterministic_and_verifiable() {
        let directory = tempfile::tempdir().unwrap();
        let bridge = directory.path().join("airwiki-mcp-bridge");
        fs::write(&bridge, [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0x00, 0x00, 0x01]).unwrap();
        let first = directory.path().join("first.mcpb");
        let second = directory.path().join("second.mcpb");
        for output in [&first, &second] {
            build_mcpb(&McpbRequest {
                target: McpbTarget::MacosArm64,
                bridge: bridge.clone(),
                output: output.clone(),
            })
            .unwrap();
        }
        assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
        verify_mcpb(&first, McpbTarget::MacosArm64, &bridge).unwrap();
    }

    #[test]
    fn mcpb_contains_the_complete_verified_legal_inventory() {
        let directory = tempfile::tempdir().unwrap();
        let bridge = directory.path().join("airwiki-mcp-bridge");
        fs::write(&bridge, [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0x00, 0x00, 0x01]).unwrap();
        let bundle = directory.path().join("legal.mcpb");
        build_mcpb(&McpbRequest {
            target: McpbTarget::MacosArm64,
            bridge,
            output: bundle.clone(),
        })
        .unwrap();
        let file = File::open(bundle).unwrap();
        let mut archive = ZipArchive::new(file).unwrap();
        let names = (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().to_owned())
            .collect::<BTreeSet<_>>();

        assert!(
            names.contains("licenses/NON_CARGO_COMPONENTS.md")
                && names.contains("licenses/THIRD_PARTY_LICENSES.md")
                && names.contains("licenses/non-cargo/NSIS-3.09-COPYING.txt")
                && names
                    .contains("licenses/non-cargo/nsis-tauri-utils-0.2.1-LICENSE_APACHE-2.0.txt")
                && names.contains("licenses/non-cargo/nsis-tauri-utils-0.2.1-LICENSE_MIT.txt")
        );
    }

    #[test]
    fn mcpb_verification_rejects_path_traversal() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("malicious.mcpb");
        let file = File::create(&path).unwrap();
        let mut archive = ZipWriter::new(file);
        archive
            .start_file("../manifest.json", SimpleFileOptions::default())
            .unwrap();
        archive.write_all(b"{}").unwrap();
        archive.finish().unwrap();
        let bridge = directory.path().join("bridge");
        fs::write(&bridge, [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0x00, 0x00, 0x01]).unwrap();
        assert!(verify_mcpb(&path, McpbTarget::MacosArm64, &bridge).is_err());
    }

    #[test]
    fn mcpb_verification_rejects_a_substituted_bridge() {
        let directory = tempfile::tempdir().unwrap();
        let packaged_bridge = directory.path().join("packaged-bridge");
        fs::write(
            &packaged_bridge,
            [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0x00, 0x00, 0x01, 1],
        )
        .unwrap();
        let bundle = directory.path().join("bundle.mcpb");
        build_mcpb(&McpbRequest {
            target: McpbTarget::MacosArm64,
            bridge: packaged_bridge,
            output: bundle.clone(),
        })
        .unwrap();
        let expected_bridge = directory.path().join("expected-bridge");
        fs::write(
            &expected_bridge,
            [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0x00, 0x00, 0x01, 2],
        )
        .unwrap();

        assert!(verify_mcpb(&bundle, McpbTarget::MacosArm64, &expected_bridge).is_err());
    }

    #[test]
    fn bridge_architecture_is_target_specific() {
        let macho = [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0x00, 0x00, 0x01];
        assert!(validate_bridge_architecture(&macho, McpbTarget::MacosArm64).is_ok());
        assert!(validate_bridge_architecture(&macho, McpbTarget::WindowsX64).is_err());

        let mut pe = vec![0_u8; 0x86];
        pe[..2].copy_from_slice(b"MZ");
        pe[0x3c..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
        pe[0x80..0x84].copy_from_slice(b"PE\0\0");
        pe[0x84..0x86].copy_from_slice(&[0x64, 0x86]);
        assert!(validate_bridge_architecture(&pe, McpbTarget::WindowsX64).is_ok());
        assert!(validate_bridge_architecture(&pe, McpbTarget::MacosArm64).is_err());
    }

    #[test]
    fn mcpb_uses_the_workspace_apache_license() {
        let bytes = mcpb_manifest(McpbTarget::MacosArm64).unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(manifest["license"], "Apache-2.0");

        let workspace_license = fs::read(workspace_root().join("LICENSE")).unwrap();
        assert!(workspace_license.starts_with(b"                                 Apache License"));
    }

    #[test]
    fn windows_signing_scripts_cover_identity_timestamp_uninstaller_and_updater() {
        let root = workspace_root();
        let identity = fs::read_to_string(root.join("packaging/windows-signing.ps1")).unwrap();
        let signer = fs::read_to_string(root.join("packaging/sign-windows-artifact.ps1")).unwrap();
        let package =
            fs::read_to_string(root.join("packaging/package-signed-windows.ps1")).unwrap();
        let verify = fs::read_to_string(root.join("packaging/verify-windows-release.ps1")).unwrap();

        assert!(identity.contains("1.3.6.1.5.5.7.3.3"));
        assert!(identity.contains("1.3.6.1.5.5.7.3.8"));
        assert!(identity.contains("1.3.6.1.4.1.311.97."));
        assert!(identity.contains("1.3.6.1.4.1.311.97.1.0"));
        assert!(identity.contains("2.16.840.1.101.3.4.2.1"));
        assert!(identity.contains("1.2.840.113549.1.9.16.2.14"));
        assert!(identity.contains("Get-SpcIndirectDataDigestOid"));
        assert!(identity.contains("Get-Rfc3161MessageImprintOid"));
        assert!(identity.contains("Get-SingleArtifactSigningDurableIdentity"));
        assert!(identity.contains("Assert-SingleRequiredEku"));
        assert!(identity.contains("verify /pa /tw /ds 0"));
        assert!(identity.contains("verify /pa /tw /ds 1"));
        assert!(identity.contains("must not contain a nested Authenticode signature"));
        assert!(identity.contains("must not contain a legacy Authenticode timestamp"));
        assert!(!identity.contains(".Thumbprint"));
        assert!(signer.contains("Invoke-ArtifactSigning"));
        assert!(signer.contains("Assert-ExpectedWindowsSigner $ResolvedPath"));
        assert!(
            signer.contains("if ($IsPreSignedMainBinary)")
                && signer.contains("$ExpectedMachineLow = 0x64")
                && signer.contains("$ExpectedMachineHigh = 0x86")
                && signer.contains("$ExpectedMachineLow = 0x4c")
                && signer.contains("$ExpectedMachineHigh = 0x01")
                && signer.contains("NSIS 3.09 I386 executable")
        );
        assert!(
            signer.contains("airwiki_0.2.0_x64-setup.exe") && signer.contains("$IsFinalInstaller")
        );
        assert!(
            signer.contains("$IsPreSignedMainBinary")
                && signer.contains(
                    "main executable must already have the expected Authenticode signature"
                )
        );
        assert!(package.contains("signCommand ="));
        assert!(package.contains("sign-windows-artifact.ps1"));
        assert!(package.contains("$env:TEMP = $SigningTemp"));
        assert!(verify.contains("target\\windows-uninstaller\\airwiki-uninstall.exe"));
        assert!(
            verify.contains("airwiki_${ExpectedVersion}_x64-setup.exe")
                && verify.contains(&format!(
                    "$ExpectedVersion = \"{}\"",
                    env!("CARGO_PKG_VERSION")
                ))
        );
        assert!(verify.contains("Expected only the exact AirWiki NSIS installer"));
        assert!(verify.contains("function Assert-ExactWindowsVersion"));
        assert!(verify.contains("Assert-ExactWindowsVersion $Installer \"NSIS installer\""));
        assert!(verify.contains("version metadata does not exactly match"));
        assert!(signer.contains("Save-SignedUninstallerReceipt"));
        assert!(
            verify.contains("\"packaging\", \"verify-updater-signature\"")
                && verify.contains("updater signature failed cryptographic verification")
        );
    }

    #[test]
    fn windows_release_pins_every_downloaded_nsis_toolchain_artifact() {
        let script = fs::read_to_string(
            workspace_root().join("packaging/prepare-verified-nsis-toolchain.ps1"),
        )
        .unwrap();

        assert!(
            script.contains("f5dc52eef1f3884230520199bac6f36b82d643d86b003ce51bd24b05c6ba7c91")
                && script
                    .contains("0eed48313a7f904d7cc1977b70000ab3f11f18cadc8e6a69b807d288ca71f9db")
                && script
                    .contains("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
                && script.contains("Get-FileHash -LiteralPath $Destination -Algorithm SHA256")
                && script.contains("$CompatibilitySentinel")
                && script.contains("[IO.File]::WriteAllBytes($CompatibilitySentinel")
                && script.contains("$SentinelItem.Length -ne 0")
                && script.contains("Assert-RequiredNsisLayout $FinalNsis")
                && !script.contains("NSIS-ApplicationID.zip")
                && !script.contains("nsis-plugins-v0")
                && !script
                    .contains("1c2772b0edfb0f96a7524734d6c8fac1fc011f26221faf88f3ed2c950f0c06c0")
        );
    }

    #[test]
    fn local_windows_packaging_prepares_pinned_tools_before_packaging() {
        let package =
            fs::read_to_string(workspace_root().join("packaging/package-windows.ps1")).unwrap();

        assert!(validate_local_windows_package_tools(&package).is_ok());
    }

    #[test]
    fn local_windows_packaging_uses_the_pinned_7zip_extractor() {
        let root = workspace_root();
        let preparation =
            fs::read_to_string(root.join("packaging/prepare-verified-7zip.ps1")).unwrap();
        let package = fs::read_to_string(root.join("packaging/package-windows.ps1")).unwrap();
        let verify = fs::read_to_string(root.join("packaging/verify-windows-release.ps1")).unwrap();

        assert!(validate_pinned_seven_zip_tool(&root).is_ok());
        assert!(
            preparation.contains("Start-Process")
                && preparation.contains("/a `\"$Msi`\"")
                && preparation.contains("TARGETDIR=`\"$AdministrativeImage`\"")
                && preparation.contains("Assert-X64Pe")
                && preparation.contains("$Entries.Count -ne $PinnedFiles.Count")
        );
        assert!(
            package.contains("$SevenZip = Join-Path $SevenZipToolRoot \"7z.exe\"")
                && verify.contains("AIRWIKI_7ZIP_ROOT or -SevenZipToolRoot is required")
                && !package.contains("Get-Command 7z.exe")
                && !verify.contains("Get-Command 7z.exe")
        );
    }

    #[test]
    fn windows_clean_install_gate_checks_materialized_payload_and_cleanup() {
        let root = workspace_root();
        let smoke = fs::read_to_string(root.join("packaging/smoke-install-windows.ps1")).unwrap();
        let template = fs::read_to_string(root.join("packaging/windows/installer.nsi")).unwrap();
        let signed = fs::read_to_string(root.join("packaging/package-signed-windows.ps1")).unwrap();

        assert!(
            smoke.contains("param([switch] $AuthorizeDestructiveClientInstallerGate)")
                && smoke.contains("$script:UninstallerVerified = $true")
                && smoke.contains("silent uninstaller left managed state")
                && smoke.contains("Assert-WindowsRuntimeTreeMatches")
                && smoke.contains("Assert-SameFile $UninstallerReceipt $InstalledUninstaller")
                && smoke.contains("Add-WindowsPayloadTree $ExpectedPayload \"licenses\"")
                && smoke.contains("(Join-Path $Root \"resources\\licenses\") \"license inventory\"")
                && smoke.contains(
                    "Assert-WindowsPayloadMatches $ExpectedPayload $InstallDir \"Windows installer matrix\""
                )
                && template.contains("ReadRegStr $R0 SHCTX \"${UNINSTKEY}\" \"UninstallString\"")
                && template.contains("StrCmp $R0 $R1 0 uninstall_registry_cleanup_done")
                && template.contains("ReadRegStr $R0 SHCTX \"${MANUPRODUCTKEY}\" \"\"")
                && template.contains("StrCmp $R0 $INSTDIR 0 product_registry_cleanup_done")
                && template.contains("DeleteRegValue SHCTX \"${MANUPRODUCTKEY}\" \"\"")
                && template.contains("DeleteRegKey /ifempty SHCTX \"${MANUPRODUCTKEY}\"")
                && signed.contains("AIRWIKI_USE_PREBUILT_MCPB")
                && signed.contains("Get-VerifiedWindowsRegularFile $Mcpb")
        );
    }
}
