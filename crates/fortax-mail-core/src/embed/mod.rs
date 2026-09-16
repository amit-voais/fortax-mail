//! Local, offline-first text embeddings for semantic search + RAG.
//!
//! Uses candle (pure Rust, CPU) to run small BERT-family sentence encoders
//! (BGE / MiniLM). No ONNX runtime, no native ML libs - Fortax Mail stays a single
//! static binary. Inference is CPU-heavy, so callers must run [`Embedder::embed`]
//! off the DB writer thread (e.g. under `tokio::task::spawn_blocking`).

#[cfg(feature = "local-embeddings")]
use crate::error::{CoreError, Result};
#[cfg(feature = "local-embeddings")]
use std::path::{Path, PathBuf};
#[cfg(feature = "local-embeddings")]
use std::sync::{Arc, Mutex};

pub mod store;
#[cfg(feature = "local-embeddings")]
pub mod worker;

#[cfg(feature = "local-embeddings")]
use store::VectorIndex;
#[cfg(feature = "local-embeddings")]
use tokio::sync::RwLock;

/// Shared, cloneable semantic-search runtime state held on `Core`: the loaded
/// local model, the in-memory vector index, and the id of the model both are
/// currently scoped to. `active_model` is empty until the first model loads.
#[cfg(feature = "local-embeddings")]
pub struct EmbedState {
    pub index: RwLock<VectorIndex>,
    pub local: RwLock<Option<Arc<LocalCandle>>>,
    pub active_model: RwLock<String>,
    /// Recent query-text -> embedding cache. Typing, backspacing, and re-runs
    /// repeat the same strings; a hit skips a full model forward pass.
    query_cache: RwLock<std::collections::HashMap<String, Vec<f32>>>,
}

/// Evict the whole query cache once it exceeds this; entries are tiny
/// (a few hundred floats) and queries are transient, so LRU bookkeeping
/// isn't worth it.
#[cfg(feature = "local-embeddings")]
const QUERY_CACHE_CAP: usize = 256;
#[cfg(feature = "local-embeddings")]
const MAX_MODEL_CONFIG_BYTES: u64 = 1024 * 1024;
#[cfg(feature = "local-embeddings")]
const MAX_TOKENIZER_BYTES: u64 = 32 * 1024 * 1024;
#[cfg(feature = "local-embeddings")]
static MODEL_INSTALL_LOCK: once_cell::sync::Lazy<tokio::sync::Mutex<()>> =
    once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(()));

#[cfg(feature = "local-embeddings")]
impl EmbedState {
    pub fn new() -> Self {
        EmbedState {
            index: RwLock::new(VectorIndex::new(0, "")),
            local: RwLock::new(None),
            active_model: RwLock::new(String::new()),
            query_cache: RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// The currently loaded local embedder, if any.
    pub async fn embedder(&self) -> Option<Arc<LocalCandle>> {
        self.local.read().await.clone()
    }

    pub async fn cached_query(&self, text: &str) -> Option<Vec<f32>> {
        self.query_cache.read().await.get(text).cloned()
    }

    pub async fn cache_query(&self, text: String, vec: Vec<f32>) {
        let mut cache = self.query_cache.write().await;
        if cache.len() >= QUERY_CACHE_CAP {
            cache.clear();
        }
        cache.insert(text, vec);
    }

    /// Drop cached query embeddings (call when the active model changes).
    pub async fn clear_query_cache(&self) {
        self.query_cache.write().await.clear();
    }
}

#[cfg(feature = "local-embeddings")]
impl Default for EmbedState {
    fn default() -> Self {
        Self::new()
    }
}

/// A supported local embedding model. The registry is the single source of
/// truth for a model's vector dimension and its query-side instruction prefix.
#[derive(Debug, Clone, Copy)]
pub struct ModelSpec {
    /// Stable registry key, also the settings value and on-disk dir name.
    pub key: &'static str,
    /// HuggingFace repo id used to download weights on demand.
    pub hf_repo: &'static str,
    /// Immutable Hub commit. Downloads never follow a mutable branch.
    pub revision: &'static str,
    /// Output vector dimension.
    pub dim: usize,
    /// Max input tokens per chunk.
    pub max_tokens: usize,
    /// Instruction prepended to *query* text (not passages). BGE models are
    /// trained with a retrieval instruction; MiniLM uses none.
    pub query_prefix: &'static str,
    /// Exact size and SHA-256 of every executable model artifact.
    pub artifacts: &'static [ModelArtifact; 3],
}

#[derive(Debug, Clone, Copy)]
pub struct ModelArtifact {
    pub filename: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

const BGE_SMALL_ARTIFACTS: [ModelArtifact; 3] = [
    ModelArtifact {
        filename: "config.json",
        bytes: 743,
        sha256: "094f8e891b932f2000c92cfc663bac4c62069f5d8af5b5278c4306aef3084750",
    },
    ModelArtifact {
        filename: "tokenizer.json",
        bytes: 711_396,
        sha256: "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
    },
    ModelArtifact {
        filename: "model.safetensors",
        bytes: 133_466_304,
        sha256: "3c9f31665447c8911517620762200d2245a2518d6e7208acc78cd9db317e21ad",
    },
];

const MINILM_ARTIFACTS: [ModelArtifact; 3] = [
    ModelArtifact {
        filename: "config.json",
        bytes: 612,
        sha256: "953f9c0d463486b10a6871cc2fd59f223b2c70184f49815e7efbcab5d8908b41",
    },
    ModelArtifact {
        filename: "tokenizer.json",
        bytes: 466_247,
        sha256: "be50c3628f2bf5bb5e3a7f17b1f74611b2561a3a27eeab05e5aa30f411572037",
    },
    ModelArtifact {
        filename: "model.safetensors",
        bytes: 90_868_376,
        sha256: "53aa51172d142c89d9012cce15ae4d6cc0ca6895895114379cacb4fab128d9db",
    },
];

const BGE_BASE_ARTIFACTS: [ModelArtifact; 3] = [
    ModelArtifact {
        filename: "config.json",
        bytes: 777,
        sha256: "bc00af31a4a31b74040d73370aa83b62da34c90b75eb77bfa7db039d90abd591",
    },
    ModelArtifact {
        filename: "tokenizer.json",
        bytes: 711_396,
        sha256: "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
    },
    ModelArtifact {
        filename: "model.safetensors",
        bytes: 437_955_512,
        sha256: "c7c1988aae201f80cf91a5dbbd5866409503b89dcaba877ca6dba7dd0a5167d7",
    },
];

const MULTILINGUAL_MINILM_ARTIFACTS: [ModelArtifact; 3] = [
    ModelArtifact {
        filename: "config.json",
        bytes: 645,
        sha256: "6300193cb75e01cf80c96decef7187dfb33094d97cc1490b7ead6ff134476e4e",
    },
    ModelArtifact {
        filename: "tokenizer.json",
        bytes: 9_081_518,
        sha256: "2c3387be76557bd40970cec13153b3bbf80407865484b209e655e5e4729076b8",
    },
    ModelArtifact {
        filename: "model.safetensors",
        bytes: 470_641_600,
        sha256: "eaa086f0ffee582aeb45b36e34cdd1fe2d6de2bef61f8a559a1bbc9bd955917b",
    },
];

static DEFAULT_SPEC: ModelSpec = ModelSpec {
    key: "bge-small-en-v1.5",
    hf_repo: "BAAI/bge-small-en-v1.5",
    revision: "5c38ec7c405ec4b44b94cc5a9bb96e735b38267a",
    dim: 384,
    max_tokens: 512,
    query_prefix: "Represent this sentence for searching relevant passages: ",
    artifacts: &BGE_SMALL_ARTIFACTS,
};

const REGISTRY: &[ModelSpec] = &[
    DEFAULT_SPEC,
    ModelSpec {
        key: "all-MiniLM-L6-v2",
        hf_repo: "sentence-transformers/all-MiniLM-L6-v2",
        revision: "1110a243fdf4706b3f48f1d95db1a4f5529b4d41",
        dim: 384,
        max_tokens: 256,
        query_prefix: "",
        artifacts: &MINILM_ARTIFACTS,
    },
    ModelSpec {
        key: "bge-base-en-v1.5",
        hf_repo: "BAAI/bge-base-en-v1.5",
        revision: "a5beb1e3e68b9ab74eb54cfd186867f64f240e1a",
        dim: 768,
        max_tokens: 512,
        query_prefix: "Represent this sentence for searching relevant passages: ",
        artifacts: &BGE_BASE_ARTIFACTS,
    },
    // Multilingual sentence encoder (BERT architecture, XLM-R vocabulary) so
    // semantic search works across languages - Vietnamese, Spanish, French,
    // Chinese, and 50+ others - not just English. No retrieval instruction
    // prefix (paraphrase/MiniLM models are trained without one).
    ModelSpec {
        key: "paraphrase-multilingual-MiniLM-L12-v2",
        hf_repo: "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2",
        revision: "e8f8c211226b894fcb81acc59f3b34ba3efd5f42",
        dim: 384,
        max_tokens: 256,
        query_prefix: "",
        artifacts: &MULTILINGUAL_MINILM_ARTIFACTS,
    },
];

/// The model used when none is configured. Bundled in the installer so first
/// run is fully offline.
pub const DEFAULT_MODEL: &str = "bge-small-en-v1.5";

pub fn registry() -> &'static [ModelSpec] {
    REGISTRY
}

pub fn spec(key: &str) -> Option<&'static ModelSpec> {
    REGISTRY.iter().find(|m| m.key == key)
}

/// Resolve a settings value to a spec, falling back to the default model.
pub fn spec_or_default(key: &str) -> &'static ModelSpec {
    spec(key).unwrap_or(&DEFAULT_SPEC)
}

/// Anything that turns text into normalized vectors.
#[cfg(feature = "local-embeddings")]
pub trait Embedder: Send + Sync {
    /// Embed a batch of passages. Returned vectors are L2-normalized so cosine
    /// similarity is a plain dot product.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    fn dim(&self) -> usize;
    fn model_id(&self) -> &str;
    /// Embed one query string, applying the model's retrieval-instruction prefix.
    fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        let prefixed = format!("{}{}", self.query_prefix(), query);
        let mut v = self.embed(std::slice::from_ref(&prefixed))?;
        v.pop()
            .ok_or_else(|| CoreError::Other("embedder returned no vector".into()))
    }
    fn query_prefix(&self) -> &str {
        ""
    }
}

/// A candle BERT encoder loaded from a local directory of
/// `{config.json, tokenizer.json, model.safetensors}`.
#[cfg(feature = "local-embeddings")]
pub struct LocalCandle {
    inner: Mutex<Inner>,
    spec: &'static ModelSpec,
}

#[cfg(feature = "local-embeddings")]
struct Inner {
    model: candle_transformers::models::bert::BertModel,
    tokenizer: tokenizers::Tokenizer,
    device: candle_core::Device,
}

#[cfg(feature = "local-embeddings")]
impl LocalCandle {
    /// Load a model from `dir`. `dir` must contain config.json, tokenizer.json
    /// and model.safetensors (see [`ensure_model`]).
    pub fn load(dir: &Path, spec: &'static ModelSpec) -> Result<Self> {
        verify_model_files(dir, spec)?;
        Self::load_preverified(dir, spec)
    }

    /// Load bytes just verified by [`ensure_model`]. Kept crate-private so
    /// external callers cannot accidentally bypass artifact authentication.
    pub(crate) fn load_preverified(dir: &Path, spec: &'static ModelSpec) -> Result<Self> {
        use candle_core::Device;
        use candle_nn::VarBuilder;
        use candle_transformers::models::bert::{BertModel, Config, DTYPE};

        let device = Device::Cpu;
        let config_bytes = read_model_metadata(
            &dir.join("config.json"),
            MAX_MODEL_CONFIG_BYTES,
            "embed config",
        )?;
        let config: Config = serde_json::from_slice(&config_bytes)
            .map_err(|e| CoreError::Other(format!("embed config parse: {e}")))?;

        let tokenizer_bytes = read_model_metadata(
            &dir.join("tokenizer.json"),
            MAX_TOKENIZER_BYTES,
            "embed tokenizer",
        )?;
        let mut tokenizer = tokenizers::Tokenizer::from_bytes(&tokenizer_bytes)
            .map_err(|e| CoreError::Other(format!("embed tokenizer: {e}")))?;
        // Batch inputs to a common length; truncate to the model window.
        tokenizer
            .with_padding(Some(tokenizers::PaddingParams {
                strategy: tokenizers::PaddingStrategy::BatchLongest,
                ..Default::default()
            }))
            .with_truncation(Some(tokenizers::TruncationParams {
                max_length: spec.max_tokens,
                ..Default::default()
            }))
            .map_err(|e| CoreError::Other(format!("embed tokenizer cfg: {e}")))?;

        let weights = dir.join("model.safetensors");
        // SAFETY: this is an app-managed, read-only model artifact under the
        // private cache directory. Fortax never mutates the file in place;
        // model updates are materialized separately before they are selected.
        // Mapping avoids a second full weights buffer in resident memory.
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights], DTYPE, &device)
                .map_err(|e| CoreError::Other(format!("embed weights: {e}")))?
        };
        let model = BertModel::load(vb, &config)
            .map_err(|e| CoreError::Other(format!("embed model load: {e}")))?;

        Ok(LocalCandle {
            inner: Mutex::new(Inner {
                model,
                tokenizer,
                device,
            }),
            spec,
        })
    }
}

#[cfg(feature = "local-embeddings")]
fn read_model_metadata(path: &Path, max_bytes: u64, context: &'static str) -> Result<Vec<u8>> {
    use std::io::Read;

    let file = std::fs::File::open(path)
        .map_err(|error| CoreError::Other(format!("{context}: {error}")))?;
    let mut bytes = Vec::new();
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| CoreError::Other(format!("{context}: {error}")))?;
    if bytes.len() as u64 > max_bytes {
        return Err(CoreError::Other(format!(
            "{context} exceeds the {max_bytes}-byte safety limit"
        )));
    }
    Ok(bytes)
}

#[cfg(feature = "local-embeddings")]
impl Embedder for LocalCandle {
    fn dim(&self) -> usize {
        self.spec.dim
    }
    fn model_id(&self) -> &str {
        self.spec.key
    }
    fn query_prefix(&self) -> &str {
        self.spec.query_prefix
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        use candle_core::Tensor;
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let device = &inner.device;

        let encodings = inner
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| CoreError::Other(format!("tokenize: {e}")))?;

        let map = |e: candle_core::Error| CoreError::Other(format!("embed fwd: {e}"));

        let mut ids = Vec::with_capacity(encodings.len());
        let mut masks = Vec::with_capacity(encodings.len());
        for enc in &encodings {
            ids.push(Tensor::new(enc.get_ids(), device).map_err(map)?);
            masks.push(Tensor::new(enc.get_attention_mask(), device).map_err(map)?);
        }
        let ids = Tensor::stack(&ids, 0).map_err(map)?;
        let mask = Tensor::stack(&masks, 0).map_err(map)?;
        let token_type_ids = ids.zeros_like().map_err(map)?;

        // [batch, seq, hidden]
        let out = inner
            .model
            .forward(&ids, &token_type_ids, Some(&mask))
            .map_err(map)?;

        // Attention-masked mean pooling over the sequence dimension.
        let mask_f = mask
            .to_dtype(out.dtype())
            .map_err(map)?
            .unsqueeze(2)
            .map_err(map)?; // [b, seq, 1]
        let summed = out
            .broadcast_mul(&mask_f)
            .map_err(map)?
            .sum(1)
            .map_err(map)?; // [b, h]
        let counts = mask_f.sum(1).map_err(map)?; // [b, 1]
        let mean = summed.broadcast_div(&counts).map_err(map)?;

        // L2 normalize so cosine == dot product.
        let norm = mean
            .sqr()
            .map_err(map)?
            .sum_keepdim(candle_core::D::Minus1)
            .map_err(map)?
            .sqrt()
            .map_err(map)?;
        let normalized = mean.broadcast_div(&norm).map_err(map)?;

        normalized
            .to_vec2::<f32>()
            .map_err(|e| CoreError::Other(format!("embed collect: {e}")))
    }
}

/// Roughly how many characters map to one token - used for approximate chunk
/// sizing (the tokenizer still truncates each chunk exactly).
#[cfg(feature = "local-embeddings")]
const CHARS_PER_TOKEN: usize = 4;
#[cfg(feature = "local-embeddings")]
const MAX_CHUNKS: usize = 6;

/// Turn a message subject + plaintext body into passage chunks ready to embed.
/// Strips quoted reply history and signatures, prepends the subject to the
/// first chunk, and splits long bodies into overlapping windows.
#[cfg(feature = "local-embeddings")]
pub fn prepare_chunks(subject: &str, body: &str, max_tokens: usize) -> Vec<String> {
    let cleaned = clean_body(body);
    let subject = subject.trim();

    let window = max_tokens.saturating_mul(CHARS_PER_TOKEN).max(256);
    let overlap = window / 7; // ~15% overlap

    // The head of the message carries the most signal; prepend the subject.
    let head = if subject.is_empty() {
        cleaned.clone()
    } else {
        format!("{subject}\n\n{cleaned}")
    };

    let chars: Vec<char> = head.chars().collect();
    if chars.len() <= window {
        let s = head.trim();
        return if s.is_empty() {
            Vec::new()
        } else {
            vec![s.to_string()]
        };
    }

    let mut chunks = Vec::new();
    let mut start = 0;
    while start < chars.len() && chunks.len() < MAX_CHUNKS {
        let end = (start + window).min(chars.len());
        let piece: String = chars[start..end].iter().collect();
        let piece = piece.trim();
        if !piece.is_empty() {
            chunks.push(piece.to_string());
        }
        if end == chars.len() {
            break;
        }
        start += window - overlap;
    }
    chunks
}

/// Remove quoted reply history and trailing signatures from plaintext so we
/// embed original content, not repeated quotes.
#[cfg(feature = "local-embeddings")]
fn clean_body(body: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for line in body.lines() {
        let t = line.trim_start();
        // Quoted history: ">" prefixed lines and the "On <date>, X wrote:" lead-in.
        if t.starts_with('>') {
            continue;
        }
        // Signature delimiter per RFC 3676 ("-- ").
        if line == "-- " || t == "--" {
            break;
        }
        // Common quote lead-ins that precede a fully-quoted tail.
        let lower = t.to_ascii_lowercase();
        if lower.starts_with("on ") && lower.ends_with("wrote:") {
            break;
        }
        out.push(line);
    }
    let joined = out.join("\n");
    joined.trim().to_string()
}

/// The three files a candle BERT encoder needs.
#[cfg(feature = "local-embeddings")]
pub const MODEL_FILES: &[&str] = &["config.json", "tokenizer.json", "model.safetensors"];

/// Directory holding a model's files: `<models_dir>/<key>`.
#[cfg(feature = "local-embeddings")]
pub fn model_dir(models_dir: &Path, key: &str) -> PathBuf {
    models_dir.join(key)
}

/// True if every required file for `key` is present under `models_dir`.
#[cfg(feature = "local-embeddings")]
pub fn model_present(models_dir: &Path, key: &str) -> bool {
    let Some(spec) = spec(key) else { return false };
    let dir = model_dir(models_dir, key);
    spec.artifacts.iter().all(|artifact| {
        std::fs::symlink_metadata(dir.join(artifact.filename)).is_ok_and(|metadata| {
            metadata.file_type().is_file() && metadata.len() == artifact.bytes
        })
    })
}

/// Verify exact bytes before a model is mapped and executed. Size checks catch
/// partial files cheaply; SHA-256 protects both downloaded and installer-
/// bundled artifacts from corruption or upstream replacement.
#[cfg(feature = "local-embeddings")]
pub fn verify_model_files(dir: &Path, spec: &ModelSpec) -> Result<()> {
    for artifact in spec.artifacts {
        verify_model_artifact(&dir.join(artifact.filename), artifact)?;
    }
    Ok(())
}

#[cfg(feature = "local-embeddings")]
fn verify_model_artifact(path: &Path, artifact: &ModelArtifact) -> Result<()> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| CoreError::Other(format!("model {}: {error}", artifact.filename)))?;
    if !metadata.file_type().is_file() || metadata.len() != artifact.bytes {
        return Err(CoreError::Other(format!(
            "model {} failed its size check",
            artifact.filename
        )));
    }
    let mut file = std::io::BufReader::new(
        std::fs::File::open(path)
            .map_err(|error| CoreError::Other(format!("model {}: {error}", artifact.filename)))?,
    );
    let mut hash = Sha256::new();
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|error| CoreError::Other(format!("model {}: {error}", artifact.filename)))?;
        if read == 0 {
            break;
        }
        hash.update(&chunk[..read]);
    }
    let actual = format!("{:x}", hash.finalize());
    if actual != artifact.sha256 {
        return Err(CoreError::Other(format!(
            "model {} failed its SHA-256 check",
            artifact.filename
        )));
    }
    Ok(())
}

/// Download a model's files from HuggingFace into `<models_dir>/<key>` if not
/// already present. Async (hf-hub over reqwest/rustls). Used for models the
/// user picks that aren't bundled; the default model ships in the installer.
#[cfg(feature = "local-embeddings")]
pub async fn ensure_model(models_dir: &Path, spec: &ModelSpec) -> Result<PathBuf> {
    // A model switch and the background worker may converge on the same
    // missing artifact set. Serialize installation so they share one download
    // and cannot race the atomic directory swap.
    let _install_guard = MODEL_INSTALL_LOCK.lock().await;
    let dir = model_dir(models_dir, spec.key);
    let existing_dir = dir.clone();
    let existing_spec = *spec;
    let existing_valid = tokio::task::spawn_blocking(move || {
        verify_model_files(&existing_dir, &existing_spec).is_ok()
    })
    .await
    .map_err(|error| CoreError::Other(format!("model verification task: {error}")))?;
    if existing_valid {
        return Ok(dir);
    }
    tokio::fs::create_dir_all(models_dir)
        .await
        .map_err(|e| CoreError::Other(format!("model dir: {e}")))?;

    let nonce = rand::random::<u64>();
    let staging = models_dir.join(format!(".{}-{nonce:016x}.download", spec.key));
    let backup = models_dir.join(format!(".{}-{nonce:016x}.replaced", spec.key));
    tokio::fs::create_dir(&staging)
        .await
        .map_err(|e| CoreError::Other(format!("model staging dir: {e}")))?;

    let download_result = async {
        let (owner, name) = hf_hub::split_id(spec.hf_repo);
        let client =
            hf_hub::HFClient::new().map_err(|e| CoreError::Other(format!("hf-hub init: {e}")))?;
        let repo = client.model(owner, name);
        for artifact in spec.artifacts {
            repo.download_file()
                .filename(artifact.filename)
                .revision(spec.revision)
                .local_dir(staging.clone())
                .send()
                .await
                .map_err(|e| CoreError::Other(format!("download {}: {e}", artifact.filename)))?;
        }
        // `hf-hub` keeps resumable-download metadata below `.cache`. The
        // verified artifacts are self-contained, so do not install that
        // transient state alongside the runtime model.
        let hub_cache = staging.join(".cache");
        if tokio::fs::symlink_metadata(&hub_cache).await.is_ok() {
            tokio::fs::remove_dir_all(&hub_cache)
                .await
                .map_err(|error| CoreError::Other(format!("model download cache: {error}")))?;
        }
        let verified_dir = staging.clone();
        let verified_spec = *spec;
        tokio::task::spawn_blocking(move || verify_model_files(&verified_dir, &verified_spec))
            .await
            .map_err(|error| CoreError::Other(format!("model verification task: {error}")))??;
        Ok::<(), CoreError>(())
    }
    .await;
    if let Err(error) = download_result {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(error);
    }

    let had_existing = tokio::fs::symlink_metadata(&dir).await.is_ok();
    if had_existing && let Err(error) = tokio::fs::rename(&dir, &backup).await {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(CoreError::Other(format!("replacing old model: {error}")));
    }
    if let Err(error) = tokio::fs::rename(&staging, &dir).await {
        if had_existing {
            let _ = tokio::fs::rename(&backup, &dir).await;
        }
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(CoreError::Other(format!(
            "installing verified model: {error}"
        )));
    }
    if had_existing {
        let _ = tokio::fs::remove_dir_all(backup).await;
    }
    Ok(dir)
}

#[cfg(all(test, feature = "local-embeddings"))]
mod tests {
    use super::*;

    #[test]
    fn model_artifact_verification_rejects_size_and_hash_changes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("artifact.bin");
        std::fs::write(&path, b"model").unwrap();
        let artifact = ModelArtifact {
            filename: "artifact.bin",
            bytes: 5,
            sha256: "9372c470eeadd5ecd9c3c74c2b3cb633f8e2f2fad799250a0f70d652b6b825e4",
        };
        verify_model_artifact(&path, &artifact).unwrap();

        std::fs::write(&path, b"other").unwrap();
        assert!(verify_model_artifact(&path, &artifact).is_err());
        std::fs::write(&path, b"too long").unwrap();
        assert!(verify_model_artifact(&path, &artifact).is_err());
    }

    #[test]
    fn chunks_strip_quotes_and_signature() {
        let body = "Hello there\nplease review the doc\n> old quoted line\n-- \nMy Signature";
        let chunks = prepare_chunks("Project update", body, 512);
        assert_eq!(chunks.len(), 1);
        let c = &chunks[0];
        assert!(c.starts_with("Project update"));
        assert!(c.contains("please review"));
        assert!(!c.contains("old quoted line"));
        assert!(!c.contains("My Signature"));
    }

    #[test]
    fn long_body_splits_into_capped_windows() {
        let body = "word ".repeat(4000);
        let chunks = prepare_chunks("Subj", &body, 128);
        assert!(chunks.len() > 1);
        assert!(chunks.len() <= MAX_CHUNKS);
    }

    // Requires network + ~130MB download; run with `cargo test -p fortax-mail-core
    // -- --ignored embed_smoke`.
    #[tokio::test]
    #[ignore]
    async fn embed_smoke() {
        let tmp = std::env::temp_dir().join("fortax-mail-embed-test");
        std::fs::create_dir_all(&tmp).unwrap();
        let spec = spec(DEFAULT_MODEL).unwrap();
        let dir = ensure_model(&tmp, spec).await.unwrap();
        let emb = LocalCandle::load(&dir, spec).unwrap();
        assert_eq!(emb.dim(), 384);

        let vecs = emb
            .embed(&[
                "a cat sat on the mat".to_string(),
                "kitten on a rug".to_string(),
            ])
            .unwrap();
        assert_eq!(vecs.len(), 2);
        assert_eq!(vecs[0].len(), 384);
        // L2-normalized: magnitude ~1.
        let mag: f32 = vecs[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((mag - 1.0).abs() < 1e-3, "not normalized: {mag}");
        // Similar sentences should score higher than the query prefix baseline.
        let q = emb.embed_query("where did the cat sit").unwrap();
        let dot: f32 = q.iter().zip(&vecs[0]).map(|(a, b)| a * b).sum();
        assert!(dot > 0.3, "expected semantic similarity, got {dot}");
    }
}
