//! Asking the model once (organize-spec.md §5; #14). One provider: OpenAI's
//! Responses API over HTTPS, with fixed limits:
//!
//! - the request body is at most 64 KiB, the answer at most 64 KiB;
//! - the whole call (connect, send, wait, read) ends after 60 seconds;
//! - no automatic retry, no other model, no redirect to another address.
//!
//! One request goes like this:
//!
//!   1. Check the setup (settings, consent, key; settings.rs). A refused
//!      request sends nothing and saves nothing.
//!   2. Save a row in `analyses` with status `analyzing` and only non-content
//!      metadata: provider, model, versions, file hash, parameters, context.
//!   3. Send it. No database lock and no execution lock is held while waiting.
//!   4. Save the outcome on the same row, but only if the row is still
//!      `analyzing` and the setup did not change meanwhile. Otherwise the
//!      answer is dropped: the request was cancelled, or it came back late.
//!
//! The text sent (instructions and input) stays in memory. The database gets
//! no file text, no prompt, no raw request or answer, and never the key.
//! The answer returned is the provider's structured output, not checked yet:
//! the caller checks it (#15) and saves the checked, limited result with
//! `record_model_result`, or rejects it with `reject_model_output`.
//!
//! That is Organize's request (`request_model`). Memory's analysis (#33) uses
//! `send_only` instead (shared-contract.md §4.6): the same call and limits,
//! but no Organize agreement, no `analyses` row and no diagnostics. Memory
//! checks its own agreement at the last moment and keeps its own records.

use std::fs;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::contracts::{AppError, ErrorCode};
use crate::settings::{self, KeyStore, SendingSetup};
use crate::store::{AnalysisEnd, AnalysisRow, AnalysisStatus};
use crate::{OrganizeService, is_canonical_uuid, observe};

/// The provider's address. The app never sends anywhere else.
pub(crate) const OPENAI_RESPONSES_URL: &str = "https://api.openai.com/v1/responses";
pub(crate) const MAX_REQUEST_BYTES: usize = 64 * 1024;
pub(crate) const MAX_RESPONSE_BYTES: u64 = 64 * 1024;
pub(crate) const TIMEOUT: Duration = Duration::from_secs(60);
/// Sorting a file into a folder (Organize). "medium" weighs the folders
/// that compete before it recommends one; with "low" the model more often
/// stopped at a parent folder or skipped a needed question (#22 regression,
/// 2026-10-10).
const CLASSIFY_REASONING_EFFORT: &str = "medium";
/// Memory's requests through `send_only` (#33) keep "low".
const SEND_ONLY_REASONING_EFFORT: &str = "low";
/// Includes reasoning tokens. A structured result stays far below this.
const MAX_OUTPUT_TOKENS: u32 = 4000;
/// The caller's non-content context and the checked result have small limits.
const MAX_CONTEXT_BYTES: usize = 8 * 1024;
pub(crate) const MAX_RESULT_BYTES: usize = 16 * 1024;

/// How this service reaches the model. Fixed in the app; in the crate's own
/// tests, requests go to a local test server and keys stay in memory, so no
/// test can reach OpenAI or the real Keychain.
pub(crate) struct ModelSetup {
    pub endpoint: String,
    pub timeout: Duration,
    /// Only `https://` addresses. Off only for the local test server.
    pub https_only: bool,
    pub keys: KeyStore,
    /// Added to the provider to make the Keychain account: empty for the
    /// normal app-data folder, `@test-…` for a test folder (#65).
    pub key_account_suffix: String,
    /// Settings changes happen one at a time (read, change, write).
    pub settings_lock: std::sync::Mutex<()>,
}

impl ModelSetup {
    /// The app: OpenAI over HTTPS, 60 seconds, keys in the Keychain.
    pub(crate) fn for_app() -> ModelSetup {
        ModelSetup {
            endpoint: OPENAI_RESPONSES_URL.to_string(),
            timeout: TIMEOUT,
            https_only: true,
            keys: KeyStore::Keychain,
            key_account_suffix: String::new(),
            settings_lock: std::sync::Mutex::new(()),
        }
    }

    /// The Keychain account of the provider's API key.
    pub(crate) fn key_account(&self, provider: &str) -> String {
        format!("{provider}{}", self.key_account_suffix)
    }

    #[cfg(not(test))]
    pub(crate) fn new() -> ModelSetup {
        ModelSetup::for_app()
    }

    /// The crate's own tests. Port 9 on this Mac is closed: a test that
    /// forgets to start its own server gets "unreachable", never an answer.
    #[cfg(test)]
    pub(crate) fn new() -> ModelSetup {
        ModelSetup {
            endpoint: "http://127.0.0.1:9/v1/responses".to_string(),
            timeout: Duration::from_secs(5),
            https_only: false,
            keys: KeyStore::memory(),
            key_account_suffix: String::new(),
            settings_lock: std::sync::Mutex::new(()),
        }
    }
}

/// One request, built by the caller (classification, #15). It is not
/// serializable and its Debug output hides the text, so the text cannot end
/// up in a record or a log by accident.
pub struct ModelRequest {
    /// A new UUID chosen by the caller, so it can cancel the request while
    /// it runs.
    pub request_id: String,
    pub file_id: String,
    /// The file revision the text came from; an older one is refused.
    pub file_revision: i64,
    pub prompt_version: String,
    pub output_schema_version: String,
    /// Name and JSON schema of the structured output (strict mode).
    pub schema_name: String,
    pub output_schema: Value,
    /// The fixed rules for the model. Never file text.
    pub instructions: String,
    /// File name, excerpt, folder names and the user's answer. Memory only.
    pub input: String,
    /// Non-content facts worth keeping: candidate IDs, truncation, character
    /// counts, generation, answer state. A JSON object of at most 8 KiB.
    pub context: Value,
    /// Smaller copies of the file's image or scanned pages, as JPEG bytes
    /// without metadata, made in the OCR child (organize-spec.md §14.10).
    /// Empty unless the user turned sending images on.
    pub images: Vec<Vec<u8>>,
}

impl std::fmt::Debug for ModelRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelRequest")
            .field("request_id", &self.request_id)
            .field("file_id", &self.file_id)
            .field("instructions", &"<hidden>")
            .field("input", &"<hidden>")
            .field("images", &self.images.len())
            .finish_non_exhaustive()
    }
}

/// The provider's structured output, not checked yet. Memory only.
pub struct ModelAnswer {
    pub request_id: String,
    /// Always a JSON object.
    pub output: Value,
    pub latency_ms: i64,
    /// Null when the provider did not report it.
    pub usage: Option<Usage>,
}

/// The output can echo file text, so Debug shows only its size.
impl std::fmt::Debug for ModelAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelAnswer")
            .field("request_id", &self.request_id)
            .field(
                "output",
                &format!("<{} bytes>", self.output.to_string().len()),
            )
            .field("latency_ms", &self.latency_ms)
            .field("usage", &self.usage)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

/// Why a request ended without a usable answer.
#[derive(Debug)]
struct Failure {
    code: ErrorCode,
    message: String,
    retryable: bool,
}

impl Failure {
    fn new(code: ErrorCode, message: impl Into<String>, retryable: bool) -> Failure {
        Failure {
            code,
            message: message.into(),
            retryable,
        }
    }
}

/// What came back over the network, before it is read.
struct Reply {
    status: u16,
    provider_request_id: Option<String>,
    body: Vec<u8>,
}

impl OrganizeService {
    /// Sends one request and waits for the answer (up to 60 seconds). Run it
    /// in the background: it holds no lock while waiting. The answer belongs
    /// to `file_revision`; a plan built from it checks the file, its folders
    /// and the authorization again (plan.rs).
    pub fn request_model(&self, request: &ModelRequest) -> Result<ModelAnswer, AppError> {
        self.request_model_with(request, &mut || true)
    }

    /// `request_model`, asking `still_wanted` at the last moment before the
    /// request goes out: after the setup was read and the request's row was
    /// saved, together with a check that the setup did not change (#70).
    /// When either says no, nothing is sent and the row is marked cancelled,
    /// so a cancel or a settings change at any earlier moment is never lost
    /// (#16).
    pub(crate) fn request_model_with(
        &self,
        request: &ModelRequest,
        still_wanted: &mut dyn FnMut() -> bool,
    ) -> Result<ModelAnswer, AppError> {
        check_request(request)?;
        let setup = self.sending_setup(Some(request.file_id.as_str()))?;
        self.request_model_using(&setup, request, still_wanted)
    }

    /// `request_model_with` with a setup read earlier. The last check below
    /// stops the request when the setup changed since `setup` was read:
    /// Organize reads it before it chooses the images, so turning image
    /// sending off meanwhile stops the request too (§14.10).
    pub(crate) fn request_model_using(
        &self,
        setup: &SendingSetup,
        request: &ModelRequest,
        still_wanted: &mut dyn FnMut() -> bool,
    ) -> Result<ModelAnswer, AppError> {
        let store = self.store()?;
        let file_id = Some(request.file_id.as_str());
        check_request(request)?;
        let Some((file, present)) = store.get_file(&request.file_id)? else {
            return Err(AppError::analysis(
                ErrorCode::UnknownFile,
                "No registered file has this ID.",
                false,
                file_id,
            ));
        };
        if !present || file.record_revision != request.file_revision {
            return Err(AppError::analysis(
                ErrorCode::FileChanged,
                "The file changed since it was read. Choose it again.",
                false,
                file_id,
            ));
        }

        let body = request_body(&setup.model, request);
        let bytes = serde_json::to_vec(&body).map_err(|_| {
            AppError::analysis(
                ErrorCode::InvalidInput,
                "The request cannot be built.",
                false,
                file_id,
            )
        })?;
        // Images (§14.10) come on top of the text request's own limit.
        let image_room: usize = request
            .images
            .iter()
            .map(|image| image.len().div_ceil(3) * 4 + IMAGE_PART_OVERHEAD)
            .sum();
        let too_large = bytes.len() > MAX_REQUEST_BYTES + image_room;
        let meta = json!({
            "params": {
                "reasoning_effort": CLASSIFY_REASONING_EFFORT,
                "max_output_tokens": MAX_OUTPUT_TOKENS,
                "store": false,
                "structured_output": { "name": request.schema_name, "strict": true },
                "timeout_seconds": self.model.timeout.as_secs(),
                "request_bytes": bytes.len(),
            },
            "context": request.context,
        });
        store.insert_analysis(&AnalysisRow {
            request_id: &request.request_id,
            file_id: &request.file_id,
            file_sha256: &file.sha256,
            file_revision: file.record_revision,
            provider: &setup.provider,
            model: &setup.model,
            requested_at_ms: self.now(),
            prompt_version: &request.prompt_version,
            output_schema_version: &request.output_schema_version,
            extractor_version: observe::EXTRACTOR_VERSION,
            app_version: env!("CARGO_PKG_VERSION"),
            request_meta: &meta,
            status: if too_large {
                AnalysisStatus::Failed
            } else {
                AnalysisStatus::Analyzing
            },
            error_code: too_large.then_some(ErrorCode::ModelRequestTooLarge),
        })?;
        if too_large {
            return Err(AppError::analysis(
                ErrorCode::ModelRequestTooLarge,
                "The request would be larger than 64 KiB, so it was not sent.",
                false,
                file_id,
            ));
        }

        // The last check before anything goes out (#70). The setup must
        // still be the one this request was read with: a change of consent,
        // provider, model or key that finished meanwhile (for example while
        // the row above waited to be saved) stops the request, and so does a
        // cancel. Every settings change takes the settings lock, so the check
        // and the decision to send are one step against them. The lock is
        // released before the network wait (organize-plan.md §5), so a
        // change that starts after this point can still finish while the
        // request is being sent (connecting, writing): that request was
        // already decided, and its late answer is dropped below.
        let ready = {
            let _settings = settings::lock(&self.model.settings_lock);
            if self.settings_generation().unwrap_or(None) != Some(setup.generation) {
                // Refused now (consent withdrawn, key removed, ...), or still
                // allowed but no longer the setup this request was built for.
                Err(self.sending_setup(file_id).err().unwrap_or_else(|| {
                    cancelled(
                        "The model settings changed before the request was sent, so nothing \
                         was sent. Try again.",
                        file_id,
                    )
                }))
            } else if !still_wanted() {
                Err(cancelled(
                    "The analysis was cancelled before anything was sent.",
                    file_id,
                ))
            } else {
                Ok(())
            }
        };
        if let Err(error) = ready {
            store.finish_analysis(&request.request_id, &AnalysisEnd::Cancelled)?;
            return Err(error);
        }

        // The network wait: no lock of any kind is held here.
        let started = Instant::now();
        let sent = send(&self.model, &setup.api_key, &bytes);
        let latency_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let outcome = sent.as_ref().map_err(clone_failure).and_then(read_answer);
        if setup.diagnostics {
            // The images themselves are never kept, only how many there were.
            write_diagnostic(
                self,
                request,
                setup,
                &without_images(&body),
                &sent,
                &outcome,
            );
        }
        let provider_request_id = sent
            .as_ref()
            .ok()
            .and_then(|r| r.provider_request_id.clone());

        // A late answer: the setup changed (or cannot be read any more)
        // while this request was waiting.
        if self.settings_generation().unwrap_or(None) != Some(setup.generation) {
            store.finish_analysis(&request.request_id, &AnalysisEnd::Cancelled)?;
            return Err(cancelled(
                "The model setup changed while Traeva was waiting, so the answer was dropped.",
                file_id,
            ));
        }
        let end = match &outcome {
            Ok((_, usage)) => AnalysisEnd::Answered {
                latency_ms,
                usage: usage.map(|u| json!(u)),
                provider_request_id,
            },
            Err(failure) => AnalysisEnd::Failed {
                code: failure.code,
                latency_ms: Some(latency_ms),
                provider_request_id,
            },
        };
        if !store.finish_analysis(&request.request_id, &end)? {
            return Err(cancelled(
                "The request was cancelled, so its answer was dropped.",
                file_id,
            ));
        }
        match outcome {
            Ok((output, usage)) => Ok(ModelAnswer {
                request_id: request.request_id.clone(),
                output,
                latency_ms,
                usage,
            }),
            Err(failure) => Err(AppError::analysis(
                failure.code,
                failure.message,
                failure.retryable,
                file_id,
            )),
        }
    }

    /// Cancels a request that is still waiting: whatever answer comes later
    /// is dropped. Returns false when the request had already ended; then
    /// nothing changes.
    pub fn cancel_model_request(&self, request_id: &str) -> Result<bool, AppError> {
        let store = self.store()?;
        check_request_id(request_id)?;
        if store.finish_analysis(request_id, &AnalysisEnd::Cancelled)? {
            return Ok(true);
        }
        match store.analysis_status(request_id)? {
            Some(_) => Ok(false),
            None => Err(unknown_analysis()),
        }
    }

    /// Saves the caller's checked, limited result (#15) for an answered
    /// request. The result must be a JSON object of at most 16 KiB.
    pub fn record_model_result(&self, request_id: &str, result: &Value) -> Result<(), AppError> {
        let store = self.store()?;
        check_request_id(request_id)?;
        let text = serde_json::to_string(result).unwrap_or_default();
        if !result.is_object() || text.len() > MAX_RESULT_BYTES {
            return Err(AppError::analysis(
                ErrorCode::InvalidInput,
                "A checked result must be a JSON object of at most 16 KiB.",
                false,
                None,
            ));
        }
        self.end_answered(store.validate_analysis(request_id, &text)?, request_id)
    }

    /// Marks an answered request as `invalid_model_output` (#15): its answer
    /// did not pass the checks. Nothing is chosen or moved because of it.
    pub fn reject_model_output(&self, request_id: &str) -> Result<(), AppError> {
        let store = self.store()?;
        check_request_id(request_id)?;
        self.end_answered(store.reject_analysis(request_id)?, request_id)
    }

    fn end_answered(&self, changed: bool, request_id: &str) -> Result<(), AppError> {
        if changed {
            return Ok(());
        }
        match self.store()?.analysis_status(request_id)? {
            None => Err(unknown_analysis()),
            Some(_) => Err(AppError::analysis(
                ErrorCode::InvalidInput,
                "This request has no answer waiting to be checked.",
                false,
                None,
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// The send path on its own (shared-contract.md §4.6), for Memory (#33)
// ---------------------------------------------------------------------------

/// One request for `send_only`. Its Debug output hides the text.
pub(crate) struct OutgoingRequest<'a> {
    /// Name and JSON schema of the structured output (strict mode).
    pub schema_name: &'a str,
    pub output_schema: &'a Value,
    pub instructions: &'a str,
    pub input: &'a str,
}

impl std::fmt::Debug for OutgoingRequest<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutgoingRequest")
            .field("schema_name", &self.schema_name)
            .field("instructions", &"<hidden>")
            .field("input", &"<hidden>")
            .finish_non_exhaustive()
    }
}

/// Who a request is about to go to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Recipient {
    pub provider: String,
    pub model: String,
}

/// The provider's structured output, not checked yet.
pub(crate) struct SentAnswer {
    /// Always a JSON object.
    pub output: Value,
    pub latency_ms: i64,
    /// Null when the provider did not report it.
    pub usage: Option<Usage>,
    pub provider_request_id: Option<String>,
    /// Where it was sent.
    pub recipient: Recipient,
}

/// The output can echo document text, so Debug shows only its size.
impl std::fmt::Debug for SentAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SentAnswer")
            .field(
                "output",
                &format!("<{} bytes>", self.output.to_string().len()),
            )
            .field("latency_ms", &self.latency_ms)
            .field("usage", &self.usage)
            .field("recipient", &self.recipient)
            .finish_non_exhaustive()
    }
}

/// Why `send_only` ended without an answer: only the code and whether
/// trying again can help. The caller gives it its own message and next step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SendFailure {
    pub code: ErrorCode,
    pub retryable: bool,
}

impl OrganizeService {
    /// Sends one request and waits for the answer (up to 60 seconds), for a
    /// feature that keeps its own agreement and its own records: Memory's
    /// analysis (#33). Run it in the background.
    ///
    /// It reads the provider, model and key and builds the request. Then,
    /// holding the settings lock, it checks that the setup did not change
    /// since it was read, and asks `still_allowed` about the recipient: the
    /// caller's own agreement, checked at the last moment (as in #70).
    ///
    /// - `still_allowed` says no: nothing is sent, `Ok(None)`. It runs with
    ///   the settings lock held, so it must not change the model settings.
    /// - The setup changed since it was read: nothing is sent. The failure is
    ///   the setup's refusal now (no key any more, ...), or
    ///   `analysis_cancelled`, retryable, when it would still allow sending.
    ///
    /// The limits are `request_model`'s: 64 KiB each way, 60 seconds, no
    /// retry, no redirect, the same rule for the output schema. Unlike it,
    /// this checks no agreement of Organize's, writes no `analyses` row and
    /// no diagnostics, and keeps an answer even when the settings change
    /// while it waits: the caller checks its own agreement again against
    /// `SentAnswer::recipient`. A failure keeps the code and the retryable
    /// flag `request_model` gives for the same problem.
    pub(crate) fn send_only(
        &self,
        request: &OutgoingRequest,
        still_allowed: &mut dyn FnMut(&Recipient) -> bool,
    ) -> Result<Option<SentAnswer>, SendFailure> {
        let setup = self.send_only_setup().map_err(refused)?;
        self.send_only_with(&setup, request, still_allowed)
    }

    /// `send_only` with a setup read earlier (tests pass an outdated one).
    fn send_only_with(
        &self,
        setup: &SendingSetup,
        request: &OutgoingRequest,
        still_allowed: &mut dyn FnMut(&Recipient) -> bool,
    ) -> Result<Option<SentAnswer>, SendFailure> {
        if !schema_ok(request.schema_name, request.output_schema) {
            return Err(SendFailure {
                code: ErrorCode::InvalidInput,
                retryable: false,
            });
        }
        let body = structured_body(
            &setup.model,
            SEND_ONLY_REASONING_EFFORT,
            request.instructions,
            request.input,
            request.schema_name,
            request.output_schema,
        );
        let bytes = serde_json::to_vec(&body).map_err(|_| SendFailure {
            code: ErrorCode::InvalidInput,
            retryable: false,
        })?;
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(SendFailure {
                code: ErrorCode::ModelRequestTooLarge,
                retryable: false,
            });
        }
        let recipient = Recipient {
            provider: setup.provider.clone(),
            model: setup.model.clone(),
        };
        // The last check before anything goes out, as in `request_model_with`:
        // every settings change takes this lock, so the check and the
        // decision to send are one step against them.
        {
            let _settings = settings::lock(&self.model.settings_lock);
            if self.settings_generation().unwrap_or(None) != Some(setup.generation) {
                return Err(match self.send_only_setup() {
                    Err(refusal) => refused(refusal),
                    Ok(_) => SendFailure {
                        code: ErrorCode::AnalysisCancelled,
                        retryable: true,
                    },
                });
            }
            if !still_allowed(&recipient) {
                return Ok(None);
            }
        }

        // The network wait: no lock of any kind is held here.
        let started = Instant::now();
        let reply = send(&self.model, &setup.api_key, &bytes).map_err(neutral)?;
        let latency_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let (output, usage) = read_answer(&reply).map_err(neutral)?;
        Ok(Some(SentAnswer {
            output,
            latency_ms,
            usage,
            provider_request_id: reply.provider_request_id,
            recipient,
        }))
    }
}

fn neutral(failure: Failure) -> SendFailure {
    SendFailure {
        code: failure.code,
        retryable: failure.retryable,
    }
}

/// A refusal by the setup (settings, key, storage), keeping its code and
/// whether trying again can help.
fn refused(error: AppError) -> SendFailure {
    SendFailure {
        code: error.code,
        retryable: error.retryable,
    }
}

/// The JSON body sent to the Responses API. `store: false` asks OpenAI not
/// to keep the answer for later retrieval.
pub(crate) fn request_body(model: &str, request: &ModelRequest) -> Value {
    let mut body = structured_body(
        model,
        CLASSIFY_REASONING_EFFORT,
        &request.instructions,
        &request.input,
        &request.schema_name,
        &request.output_schema,
    );
    if !request.images.is_empty() {
        // The text and the images as one user message (§14.10).
        let mut content = vec![json!({ "type": "input_text", "text": request.input })];
        for image in &request.images {
            content.push(json!({
                "type": "input_image",
                "image_url": format!("data:image/jpeg;base64,{}", crate::pdf_ocr::base64_encode(image)),
            }));
        }
        body["input"] = json!([{ "role": "user", "content": content }]);
    }
    body
}

/// Room each image takes in the request besides its base64 text.
const IMAGE_PART_OVERHEAD: usize = 128;

/// The body as a diagnostic record keeps it: every image replaced by a
/// short note, so no picture is ever written to disk (§14.10).
fn without_images(body: &Value) -> Value {
    let mut body = body.clone();
    if let Some(messages) = body["input"].as_array_mut() {
        for message in messages {
            if let Some(parts) = message["content"].as_array_mut() {
                for part in parts {
                    if part["type"] == "input_image" {
                        part["image_url"] = json!("[image not kept]");
                    }
                }
            }
        }
    }
    body
}

/// The body of one structured-output request; both send paths build it here.
fn structured_body(
    model: &str,
    effort: &str,
    instructions: &str,
    input: &str,
    schema_name: &str,
    output_schema: &Value,
) -> Value {
    json!({
        "model": model,
        "instructions": instructions,
        "input": input,
        "text": {
            "format": {
                "type": "json_schema",
                "name": schema_name,
                "schema": output_schema,
                "strict": true,
            }
        },
        "reasoning": { "effort": effort },
        "max_output_tokens": MAX_OUTPUT_TOKENS,
        "store": false,
    })
}

/// One HTTP call. No retry and no redirect: a redirect comes back as a
/// failure, so the key is never sent to another address.
fn send(model: &ModelSetup, api_key: &str, body: &[u8]) -> Result<Reply, Failure> {
    // Marked sensitive, so even a debug log of the request would hide it.
    let mut authorization = ureq::http::HeaderValue::from_str(&format!("Bearer {api_key}"))
        .map_err(|_| {
            Failure::new(
                ErrorCode::KeyUnavailable,
                "The saved API key contains characters that cannot be sent. Save the key again.",
                false,
            )
        })?;
    authorization.set_sensitive(true);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(model.timeout))
        .max_redirects(0)
        .http_status_as_error(false)
        .https_only(model.https_only)
        .proxy(None)
        .build()
        .into();
    let mut response = agent
        .post(&model.endpoint)
        .header("Authorization", authorization)
        .header("Content-Type", "application/json")
        .send(body)
        .map_err(network_failure)?;
    let provider_request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|id| is_short_token(id))
        .map(str::to_string);
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_vec()
        .map_err(network_failure)?;
    Ok(Reply {
        status,
        provider_request_id,
        body,
    })
}

fn network_failure(err: ureq::Error) -> Failure {
    match err {
        ureq::Error::Timeout(_) => Failure::new(
            ErrorCode::ModelTimeout,
            "OpenAI did not answer completely within 60 seconds.",
            true,
        ),
        ureq::Error::BodyExceedsLimit(_) => Failure::new(
            ErrorCode::ModelResponseTooLarge,
            "The answer was larger than 64 KiB, so it was dropped.",
            false,
        ),
        ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        | ureq::Error::Io(_)
        | ureq::Error::Tls(_)
        | ureq::Error::Rustls(_) => Failure::new(
            ErrorCode::ModelUnreachable,
            "Traeva could not reach OpenAI. Check the internet connection and try again.",
            true,
        ),
        _ => Failure::new(
            ErrorCode::ModelServiceError,
            "The request to OpenAI failed.",
            false,
        ),
    }
}

/// Reads the reply: an HTTP error, or the one structured output. Messages
/// are Traeva's own; the provider's text (which can quote part of the key)
/// is never shown or saved.
fn read_answer(reply: &Reply) -> Result<(Value, Option<Usage>), Failure> {
    let service =
        |message: &str, retryable| Failure::new(ErrorCode::ModelServiceError, message, retryable);
    match reply.status {
        200..=299 => {}
        401 | 403 => {
            return Err(Failure::new(
                ErrorCode::ModelKeyRejected,
                "OpenAI did not accept the API key. Check the key in the settings.",
                false,
            ));
        }
        404 => {
            return Err(service(
                "OpenAI does not offer this model to this key.",
                false,
            ));
        }
        408 | 409 | 429 => {
            return Err(service(
                "OpenAI refused the request for now (too many requests or no credit left). Try \
                 again later.",
                true,
            ));
        }
        500..=599 => return Err(service("OpenAI had a problem. Try again later.", true)),
        300..=399 => {
            return Err(service(
                "OpenAI answered with a redirect, which Traeva does not follow.",
                false,
            ));
        }
        _ => return Err(service("OpenAI refused the request.", false)),
    }
    let invalid = |message: &str| Failure::new(ErrorCode::InvalidModelOutput, message, true);
    let reply: Value =
        serde_json::from_slice(&reply.body).map_err(|_| invalid("The answer is not JSON."))?;
    match reply.get("status").and_then(Value::as_str) {
        Some("completed") => {}
        Some("incomplete") => {
            return Err(invalid("The answer was cut off before it was complete."));
        }
        Some("failed") => return Err(service("OpenAI could not finish the request.", true)),
        _ => return Err(invalid("The answer has no final status.")),
    }
    let mut texts = Vec::new();
    for item in reply["output"].as_array().into_iter().flatten() {
        if item["type"] != "message" {
            continue;
        }
        for part in item["content"].as_array().into_iter().flatten() {
            match part["type"].as_str() {
                Some("output_text") => texts.push(part["text"].as_str().unwrap_or_default()),
                Some("refusal") => return Err(invalid("The model refused to answer.")),
                _ => {}
            }
        }
    }
    let [text] = texts.as_slice() else {
        return Err(invalid(
            "The answer does not hold exactly one structured output.",
        ));
    };
    let output: Value = serde_json::from_str(text)
        .map_err(|_| invalid("The structured output is not valid JSON."))?;
    if !output.is_object() {
        return Err(invalid("The structured output is not a JSON object."));
    }
    let usage = serde_json::from_value::<UsageIn>(reply["usage"].clone())
        .ok()
        .map(|u| Usage {
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            total_tokens: u.total_tokens,
        });
    Ok((output, usage))
}

/// The three counts Traeva keeps; the provider sends more.
#[derive(Deserialize)]
struct UsageIn {
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
}

/// Only when diagnostics are on: the body sent and the reply, in
/// `diagnostics/<request_id>.json` in app data. Never the key or any header.
/// A record that cannot be written is skipped; it never changes the outcome.
fn write_diagnostic(
    service: &OrganizeService,
    request: &ModelRequest,
    setup: &SendingSetup,
    body: &Value,
    sent: &Result<Reply, Failure>,
    outcome: &Result<(Value, Option<Usage>), Failure>,
) {
    let Ok(store) = service.store() else { return };
    let dir = settings::diagnostics_dir(store.app_data_dir());
    let (http_status, reply_text) = match sent {
        Ok(reply) => (
            Some(reply.status),
            redact(&String::from_utf8_lossy(&reply.body), &setup.api_key),
        ),
        Err(_) => (None, String::new()),
    };
    let record = json!({
        "request_id": request.request_id,
        "recorded_at_ms": service.now(),
        "model": setup.model,
        "request_body": body,
        "http_status": http_status,
        "reply_body": reply_text,
        "error_code": outcome.as_ref().err().map(|failure| failure.code),
    });
    let Ok(text) = serde_json::to_vec_pretty(&record) else {
        return;
    };
    if fs::create_dir_all(&dir).is_ok() {
        let _ = fs::write(dir.join(format!("{}.json", request.request_id)), text);
    }
}

/// Removes the key and anything that looks like a key (`sk-...`, possibly
/// masked with `*`) from text the provider sent back.
fn redact(text: &str, api_key: &str) -> String {
    let text = text.replace(api_key, "[key removed]");
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(start) = rest.find("sk-") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 3..];
        let end = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || "-_*".contains(c)))
            .unwrap_or(after.len());
        out.push_str("[key removed]");
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

fn check_request(request: &ModelRequest) -> Result<(), AppError> {
    check_request_id(&request.request_id)?;
    let bad = |message: &str| {
        AppError::analysis(
            ErrorCode::InvalidInput,
            message,
            false,
            Some(&request.file_id),
        )
    };
    if !is_canonical_uuid(&request.file_id) {
        return Err(bad("The file ID is not a valid ID."));
    }
    if !is_short_token(&request.prompt_version) || !is_short_token(&request.output_schema_version) {
        return Err(bad(
            "Versions must be short names (letters, digits, '.', '-', '_').",
        ));
    }
    if !schema_ok(&request.schema_name, &request.output_schema) {
        return Err(bad(
            "The output schema needs a short name and must be a JSON object.",
        ));
    }
    let context_size = serde_json::to_string(&request.context).map_or(usize::MAX, |t| t.len());
    if !request.context.is_object() || context_size > MAX_CONTEXT_BYTES {
        return Err(bad("The context must be a JSON object of at most 8 KiB."));
    }
    Ok(())
}

/// OpenAI's rule for structured outputs: a short name of letters, digits,
/// '_' and '-', and a schema that is a JSON object.
fn schema_ok(name: &str, schema: &Value) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && schema.is_object()
}

fn check_request_id(request_id: &str) -> Result<(), AppError> {
    if is_canonical_uuid(request_id) {
        Ok(())
    } else {
        Err(AppError::analysis(
            ErrorCode::InvalidInput,
            "The request ID is not a valid ID.",
            false,
            None,
        ))
    }
}

/// Letters, digits, `.`, `-` and `_`, 1 to 128 characters.
fn is_short_token(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 128
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

fn clone_failure(failure: &Failure) -> Failure {
    Failure::new(failure.code, failure.message.clone(), failure.retryable)
}

fn cancelled(message: &str, file_id: Option<&str>) -> AppError {
    AppError::analysis(ErrorCode::AnalysisCancelled, message, true, file_id)
}

fn unknown_analysis() -> AppError {
    AppError::analysis(
        ErrorCode::UnknownAnalysis,
        "No model request has this ID.",
        false,
        None,
    )
}

#[cfg(test)]
#[path = "model_tests.rs"]
pub(crate) mod tests;
