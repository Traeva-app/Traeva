//! Tests for model.rs, all offline: a small HTTP server on 127.0.0.1 plays
//! OpenAI. Synthetic data only; keys stay in memory. The classification
//! tests (#15) use the same server.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::*;
use crate::contracts::{ActionStatus, FileView, NextStep};
use crate::settings::DEFAULT_MODEL;
use crate::settings::tests::{TEST_KEY, openai};
use crate::test_fixture::Fixture;

/// Stands for file text: it may go to the provider, never into a record.
const CONTENT_CANARY: &str = "CONTENTCANARY-7f3a";

// ---------------------------------------------------------------------------
// The test server
// ---------------------------------------------------------------------------

/// What the test server answers, every time.
#[derive(Clone)]
pub(crate) struct Planned {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    delay: Duration,
    /// Hold the answer until the test calls `release` (or 10 s pass).
    gated: bool,
}

impl Planned {
    pub(crate) fn answer(status: u16, body: Vec<u8>) -> Planned {
        Planned {
            status,
            headers: vec![("x-request-id".to_string(), "req_test123".to_string())],
            body,
            delay: Duration::ZERO,
            gated: false,
        }
    }

    pub(crate) fn gated(body: Vec<u8>) -> Planned {
        Planned {
            gated: true,
            ..Planned::answer(200, body)
        }
    }
}

pub(crate) struct Received {
    request_line: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Received {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub(crate) fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

pub(crate) struct TestServer {
    pub(crate) url: String,
    pub(crate) received: Arc<Mutex<Vec<Received>>>,
    gate: Sender<()>,
}

impl TestServer {
    pub(crate) fn start(planned: Planned) -> TestServer {
        TestServer::start_sequence(vec![planned])
    }

    /// Answers the requests in order with `answers`; the last one repeats.
    pub(crate) fn start_sequence(answers: Vec<Planned>) -> TestServer {
        assert!(!answers.is_empty());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/responses", listener.local_addr().unwrap());
        let received = Arc::new(Mutex::new(Vec::new()));
        let (gate, gate_rx) = mpsc::channel::<()>();
        let gate_rx = Arc::new(Mutex::new(gate_rx));
        let log = Arc::clone(&received);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let Some(request) = read_request(&stream) else {
                    continue;
                };
                let planned = {
                    let mut log = log.lock().unwrap();
                    log.push(request);
                    answers[(log.len() - 1).min(answers.len() - 1)].clone()
                };
                // Each request is answered on its own thread, so a held
                // (gated) answer never delays the next request's.
                let gate_rx = Arc::clone(&gate_rx);
                thread::spawn(move || {
                    if planned.gated {
                        let _ = gate_rx
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(10));
                    }
                    thread::sleep(planned.delay);
                    let mut head = format!("HTTP/1.1 {} Test\r\n", planned.status);
                    for (name, value) in &planned.headers {
                        head.push_str(&format!("{name}: {value}\r\n"));
                    }
                    head.push_str(&format!(
                        "Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        planned.body.len()
                    ));
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(&planned.body);
                });
            }
        });
        TestServer {
            url,
            received,
            gate,
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.received.lock().unwrap().len()
    }

    pub(crate) fn release(&self) {
        let _ = self.gate.send(());
    }

    pub(crate) fn last<T>(&self, look: impl FnOnce(&Received) -> T) -> T {
        look(self.received.lock().unwrap().last().unwrap())
    }
}

fn read_request(stream: &TcpStream) -> Option<Received> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':')?;
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(Received {
        request_line: request_line.trim_end().to_string(),
        headers,
        body,
    })
}

/// A finished Responses API answer holding one structured output.
pub(crate) fn completed(output: &Value) -> Vec<u8> {
    json!({
        "id": "resp_test",
        "object": "response",
        "status": "completed",
        "model": "gpt-6-luna",
        "output": [
            { "type": "reasoning", "id": "rs_1", "summary": [] },
            {
                "type": "message",
                "id": "msg_1",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": output.to_string(), "annotations": [] }]
            }
        ],
        "usage": {
            "input_tokens": 120,
            "input_tokens_details": { "cached_tokens": 0 },
            "output_tokens": 30,
            "output_tokens_details": { "reasoning_tokens": 10 },
            "total_tokens": 150
        }
    })
    .to_string()
    .into_bytes()
}

fn choice() -> Value {
    json!({ "folder": "c1" })
}

fn wait_until(done: impl Fn() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(5), "waited too long");
        thread::sleep(Duration::from_millis(10));
    }
}

// ---------------------------------------------------------------------------
// The service under test
// ---------------------------------------------------------------------------

struct Setup {
    f: Fixture,
    file: FileView,
}

/// Settings, key and consent saved; requests go to `server`.
fn configured(server: &TestServer) -> Setup {
    let mut f = Fixture::new();
    f.service_mut().model.endpoint = server.url.clone();
    f.service()
        .save_model_settings(&openai(DEFAULT_MODEL, Some(TEST_KEY)))
        .unwrap();
    f.service().set_analysis_consent(true).unwrap();
    let file = f.register("todo.txt");
    Setup { f, file }
}

fn request(file: &FileView) -> ModelRequest {
    ModelRequest {
        request_id: uuid::Uuid::new_v4().to_string(),
        file_id: file.file_id.clone(),
        file_revision: file.file_revision,
        prompt_version: "test-prompt-1".to_string(),
        output_schema_version: "test-schema-1".to_string(),
        schema_name: "sample_choice".to_string(),
        output_schema: json!({
            "type": "object",
            "properties": { "folder": { "type": "string" } },
            "required": ["folder"],
            "additionalProperties": false
        }),
        instructions: "Synthetic test: choose one candidate folder for the note.".to_string(),
        input: format!("File name: todo.txt\nText: {CONTENT_CANARY} buy milk"),
        context: json!({ "candidate_ids": ["c1", "c2"], "truncated": false, "excerpt_chars": 9 }),
        images: Vec::new(),
    }
}

/// One `analyses` row, read directly from the database.
#[derive(Debug)]
struct Row {
    status: String,
    error_code: Option<String>,
    provider: String,
    model: String,
    file_sha256: String,
    file_revision: i64,
    prompt_version: String,
    output_schema_version: String,
    extractor_version: String,
    app_version: String,
    request_meta: Value,
    latency_ms: Option<i64>,
    usage: Option<Value>,
    provider_request_id: Option<String>,
    result: Option<Value>,
}

fn row(f: &Fixture, request_id: &str) -> Row {
    let json = |text: Option<String>| text.map(|t| serde_json::from_str::<Value>(&t).unwrap());
    f.db()
        .query_row(
            "SELECT status, error_code, provider, model, file_sha256, file_revision,
                    prompt_version, output_schema_version, extractor_version, app_version,
                    request_meta_json, latency_ms, usage_json, provider_request_id, result_json
             FROM analyses WHERE request_id = ?1",
            [request_id],
            |r| {
                Ok(Row {
                    status: r.get(0)?,
                    error_code: r.get(1)?,
                    provider: r.get(2)?,
                    model: r.get(3)?,
                    file_sha256: r.get(4)?,
                    file_revision: r.get(5)?,
                    prompt_version: r.get(6)?,
                    output_schema_version: r.get(7)?,
                    extractor_version: r.get(8)?,
                    app_version: r.get(9)?,
                    request_meta: serde_json::from_str(&r.get::<_, String>(10)?).unwrap(),
                    latency_ms: r.get(11)?,
                    usage: json(r.get(12)?),
                    provider_request_id: r.get(13)?,
                    result: json(r.get(14)?),
                })
            },
        )
        .unwrap()
}

/// Every file under `dir`, the database included, searched for `needle`.
pub(crate) fn files_containing(dir: &Path, needle: &str) -> Vec<String> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(files_containing(&path, needle));
        } else {
            let bytes = fs::read(&path).unwrap();
            if bytes.windows(needle.len()).any(|w| w == needle.as_bytes()) {
                found.push(path.display().to_string());
            }
        }
    }
    found
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn the_app_talks_only_to_openai_over_https_with_the_spec_limits() {
    let app = ModelSetup::for_app();
    assert_eq!(app.endpoint, "https://api.openai.com/v1/responses");
    assert_eq!(app.timeout, Duration::from_secs(60));
    assert!(app.https_only);
    assert!(matches!(app.keys, KeyStore::Keychain));
    assert_eq!(MAX_REQUEST_BYTES, 64 * 1024);
    assert_eq!(MAX_RESPONSE_BYTES, 64 * 1024);

    // With https_only on, an http:// address is refused before anything is sent.
    let server = TestServer::start(Planned::answer(200, completed(&choice())));
    let mut s = configured(&server);
    s.f.service_mut().model.https_only = true;
    let err = s.f.service().request_model(&request(&s.file)).unwrap_err();
    assert_eq!(err.code, ErrorCode::ModelServiceError);
    assert_eq!(server.count(), 0);
}

#[test]
fn nothing_is_sent_without_settings_consent_and_a_key() {
    let server = TestServer::start(Planned::answer(200, completed(&choice())));
    let mut f = Fixture::new();
    f.service_mut().model.endpoint = server.url.clone();
    let file = f.register("todo.txt");
    let service = f.service();
    let code = |service: &OrganizeService| service.request_model(&request(&file)).unwrap_err().code;

    assert_eq!(code(service), ErrorCode::ModelNotConfigured, "no settings");
    service
        .save_model_settings(&openai(DEFAULT_MODEL, Some(TEST_KEY)))
        .unwrap();
    assert_eq!(code(service), ErrorCode::ConsentRequired, "no consent");
    service.set_analysis_consent(true).unwrap();
    service
        .save_model_settings(&openai("gpt-5.6-luna", None))
        .unwrap();
    assert_eq!(code(service), ErrorCode::ConsentRequired, "service changed");
    service.set_analysis_consent(true).unwrap();
    service.remove_api_key().unwrap();
    assert_eq!(code(service), ErrorCode::ModelNotConfigured, "no key");

    assert_eq!(server.count(), 0, "nothing was sent");
    assert_eq!(f.count("analyses"), 0, "nothing was saved");
}

#[test]
fn an_answer_comes_back_and_only_metadata_is_saved() {
    let server = TestServer::start(Planned::answer(200, completed(&choice())));
    let s = configured(&server);
    let req = request(&s.file);
    let answer = s.f.service().request_model(&req).unwrap();
    assert_eq!(answer.request_id, req.request_id);
    assert_eq!(answer.output, choice());
    assert_eq!(
        answer.usage,
        Some(Usage {
            input_tokens: 120,
            output_tokens: 30,
            total_tokens: 150
        })
    );

    // Exactly one request, to the configured address, as the spec asks.
    assert_eq!(server.count(), 1);
    server.last(|sent| {
        assert_eq!(sent.request_line, "POST /v1/responses HTTP/1.1");
        assert_eq!(
            sent.header("authorization"),
            Some(format!("Bearer {TEST_KEY}").as_str())
        );
        assert_eq!(sent.header("content-type"), Some("application/json"));
        let body = sent.json();
        assert_eq!(body["model"], "gpt-6-luna");
        assert_eq!(body["store"], false);
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert_eq!(body["max_output_tokens"], 4000);
        assert_eq!(body["text"]["format"]["type"], "json_schema");
        assert_eq!(body["text"]["format"]["name"], "sample_choice");
        assert_eq!(body["text"]["format"]["strict"], true);
        assert_eq!(body["text"]["format"]["schema"], req.output_schema);
        assert_eq!(body["instructions"], req.instructions.as_str());
        assert_eq!(body["input"], req.input.as_str());
        assert!(body.get("temperature").is_none());
    });

    let row = row(&s.f, &req.request_id);
    assert_eq!(row.status, "answered");
    assert_eq!(row.error_code, None);
    assert_eq!(
        (row.provider.as_str(), row.model.as_str()),
        ("openai", "gpt-6-luna")
    );
    assert_eq!(row.file_sha256, s.file.sha256);
    assert_eq!(row.file_revision, s.file.file_revision);
    assert_eq!(row.prompt_version, "test-prompt-1");
    assert_eq!(row.output_schema_version, "test-schema-1");
    assert_eq!(row.extractor_version, crate::observe::EXTRACTOR_VERSION);
    assert_eq!(row.app_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(row.request_meta["context"], req.context);
    assert_eq!(row.request_meta["params"]["store"], false);
    assert_eq!(row.request_meta["params"]["reasoning_effort"], "medium");
    assert_eq!(
        row.request_meta["params"]["structured_output"]["name"],
        "sample_choice"
    );
    assert!(row.latency_ms.is_some());
    assert_eq!(
        row.usage,
        Some(json!({ "input_tokens": 120, "output_tokens": 30, "total_tokens": 150 }))
    );
    assert_eq!(row.provider_request_id.as_deref(), Some("req_test123"));
    assert_eq!(row.result, None, "an unchecked answer is never saved");
}

#[test]
fn bad_requests_are_refused_before_anything_is_saved_or_sent() {
    let server = TestServer::start(Planned::answer(200, completed(&choice())));
    let s = configured(&server);
    let service = s.f.service();
    let refused = |change: &dyn Fn(&mut ModelRequest), code: ErrorCode| {
        let mut req = request(&s.file);
        change(&mut req);
        assert_eq!(service.request_model(&req).unwrap_err().code, code);
    };
    refused(
        &|r| r.request_id = "not-an-id".into(),
        ErrorCode::InvalidInput,
    );
    refused(
        &|r| r.prompt_version = "test prompt".into(),
        ErrorCode::InvalidInput,
    );
    refused(
        &|r| r.output_schema_version = String::new(),
        ErrorCode::InvalidInput,
    );
    refused(
        &|r| r.schema_name = "sample choice".into(),
        ErrorCode::InvalidInput,
    );
    refused(
        &|r| r.output_schema = json!(["not", "an", "object"]),
        ErrorCode::InvalidInput,
    );
    refused(
        &|r| r.context = json!(["not an object"]),
        ErrorCode::InvalidInput,
    );
    refused(
        &|r| r.context = json!({ "big": "x".repeat(9000) }),
        ErrorCode::InvalidInput,
    );
    refused(
        &|r| r.file_id = uuid::Uuid::new_v4().to_string(),
        ErrorCode::UnknownFile,
    );
    refused(&|r| r.file_revision += 1, ErrorCode::FileChanged);
    assert_eq!(server.count(), 0);
    assert_eq!(s.f.count("analyses"), 0);

    // A request ID is used once.
    let req = request(&s.file);
    service.request_model(&req).unwrap();
    assert_eq!(
        service.request_model(&req).unwrap_err().code,
        ErrorCode::InvalidInput
    );
    assert_eq!(server.count(), 1);
}

#[test]
fn a_request_over_64_kib_is_not_sent() {
    let server = TestServer::start(Planned::answer(200, completed(&choice())));
    let s = configured(&server);
    let mut req = request(&s.file);
    req.input = "x".repeat(MAX_REQUEST_BYTES);
    let err = s.f.service().request_model(&req).unwrap_err();
    assert_eq!(err.code, ErrorCode::ModelRequestTooLarge);
    assert!(!err.retryable);
    assert_eq!(server.count(), 0);
    let row = row(&s.f, &req.request_id);
    assert_eq!(row.status, "failed");
    assert_eq!(row.error_code.as_deref(), Some("model_request_too_large"));
    assert_eq!(row.latency_ms, None, "never sent");
}

#[test]
fn an_answer_over_64_kib_is_dropped() {
    let big = json!({ "folder": "x".repeat(70 * 1024) });
    let server = TestServer::start(Planned::answer(200, completed(&big)));
    let s = configured(&server);
    let req = request(&s.file);
    let err = s.f.service().request_model(&req).unwrap_err();
    assert_eq!(err.code, ErrorCode::ModelResponseTooLarge);
    assert_eq!(row(&s.f, &req.request_id).status, "failed");
}

#[test]
fn a_slow_answer_times_out_and_is_not_retried() {
    let mut planned = Planned::answer(200, completed(&choice()));
    planned.delay = Duration::from_secs(3);
    let server = TestServer::start(planned);
    let mut s = configured(&server);
    s.f.service_mut().model.timeout = Duration::from_secs(1);
    let req = request(&s.file);
    let started = Instant::now();
    let err = s.f.service().request_model(&req).unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(err.code, ErrorCode::ModelTimeout);
    assert!(err.retryable);
    assert_eq!(
        err.allowed_next,
        vec![NextStep::TryAgain, NextStep::ChooseFolder]
    );
    assert_eq!(server.count(), 1, "no automatic retry");
    let row = row(&s.f, &req.request_id);
    assert_eq!(row.error_code.as_deref(), Some("model_timeout"));
}

#[test]
fn an_unreachable_provider_fails_and_is_not_retried() {
    let server = TestServer::start(Planned::answer(200, completed(&choice())));
    let mut s = configured(&server);
    // A port that was free a moment ago and has no listener now.
    let closed = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    s.f.service_mut().model.endpoint = format!("http://{closed}/v1/responses");
    let req = request(&s.file);
    let err = s.f.service().request_model(&req).unwrap_err();
    assert_eq!(err.code, ErrorCode::ModelUnreachable);
    assert!(err.retryable);
    assert_eq!(
        row(&s.f, &req.request_id).error_code.as_deref(),
        Some("model_unreachable")
    );
}

#[test]
fn http_errors_get_traevas_own_messages_and_are_not_retried() {
    // The provider's text quotes the key; it must not appear anywhere.
    let quoting = json!({ "error": {
        "message": format!("Incorrect API key provided: {TEST_KEY}. {CONTENT_CANARY}"),
        "type": "invalid_request_error",
        "code": "invalid_api_key"
    }})
    .to_string()
    .into_bytes();
    for (status, code, retryable) in [
        (401, ErrorCode::ModelKeyRejected, false),
        (403, ErrorCode::ModelKeyRejected, false),
        (404, ErrorCode::ModelServiceError, false),
        (400, ErrorCode::ModelServiceError, false),
        (429, ErrorCode::ModelServiceError, true),
        (500, ErrorCode::ModelServiceError, true),
        (503, ErrorCode::ModelServiceError, true),
    ] {
        let server = TestServer::start(Planned::answer(status, quoting.clone()));
        let s = configured(&server);
        let req = request(&s.file);
        let err = s.f.service().request_model(&req).unwrap_err();
        assert_eq!(
            (err.code, err.retryable),
            (code, retryable),
            "HTTP {status}"
        );
        assert!(!err.message.contains("KEYCANARY") && !err.message.contains(CONTENT_CANARY));
        assert_eq!(server.count(), 1, "HTTP {status}: no automatic retry");
        assert_eq!(row(&s.f, &req.request_id).status, "failed");
    }
}

#[test]
fn a_redirect_is_not_followed() {
    for status in [301, 302, 303, 307, 308] {
        let elsewhere = TestServer::start(Planned::answer(200, completed(&choice())));
        let mut planned = Planned::answer(status, Vec::new());
        planned
            .headers
            .push(("location".to_string(), elsewhere.url.clone()));
        let server = TestServer::start(planned);
        let s = configured(&server);
        let err = s.f.service().request_model(&request(&s.file)).unwrap_err();
        assert_eq!(err.code, ErrorCode::ModelServiceError, "HTTP {status}");
        assert_eq!(server.count(), 1);
        assert_eq!(
            elsewhere.count(),
            0,
            "HTTP {status}: nothing went anywhere else"
        );
    }
}

#[test]
fn answers_without_one_valid_structured_output_are_invalid() {
    let message = |content: Value| {
        json!({ "status": "completed", "output": [{ "type": "message", "content": content }] })
            .to_string()
            .into_bytes()
    };
    let text = |t: &str| json!({ "type": "output_text", "text": t });
    let looks_fine = text("{\"folder\":\"c1\"}");
    for (name, body, code) in [
        (
            // Cut off, even though what arrived happens to parse.
            "cut off",
            json!({
                "status": "incomplete",
                "incomplete_details": { "reason": "max_output_tokens" },
                "output": [{ "type": "message", "content": [looks_fine.clone()] }]
            })
            .to_string()
            .into_bytes(),
            ErrorCode::InvalidModelOutput,
        ),
        (
            // A refusal never counts, even next to an output.
            "refused",
            message(json!([{ "type": "refusal", "refusal": "no" }, looks_fine.clone()])),
            ErrorCode::InvalidModelOutput,
        ),
        (
            "not json",
            b"<html>".to_vec(),
            ErrorCode::InvalidModelOutput,
        ),
        (
            "output not json",
            message(json!([text("folder: c1")])),
            ErrorCode::InvalidModelOutput,
        ),
        (
            "output not an object",
            message(json!([text("[\"c1\"]")])),
            ErrorCode::InvalidModelOutput,
        ),
        (
            "two outputs",
            message(json!([text("{}"), text("{}")])),
            ErrorCode::InvalidModelOutput,
        ),
        (
            "no output",
            message(json!([])),
            ErrorCode::InvalidModelOutput,
        ),
        (
            "failed",
            json!({ "status": "failed", "error": { "code": "server_error" } })
                .to_string()
                .into_bytes(),
            ErrorCode::ModelServiceError,
        ),
    ] {
        let server = TestServer::start(Planned::answer(200, body));
        let s = configured(&server);
        let req = request(&s.file);
        let err = s.f.service().request_model(&req).unwrap_err();
        assert_eq!(err.code, code, "{name}");
        let row = row(&s.f, &req.request_id);
        assert_eq!(row.status, "failed", "{name}");
        assert_eq!(row.result, None, "{name}");
    }

    // A missing usage report is saved as unknown (null), not as zero.
    let body = message(json!([text("{\"folder\":\"c1\"}")]));
    let server = TestServer::start(Planned::answer(200, body));
    let s = configured(&server);
    let req = request(&s.file);
    let answer = s.f.service().request_model(&req).unwrap();
    assert_eq!(answer.usage, None);
    assert_eq!(row(&s.f, &req.request_id).usage, None);
}

#[test]
fn a_cancelled_request_drops_its_late_answer() {
    let server = TestServer::start(Planned::gated(completed(&choice())));
    let s = configured(&server);
    let service = s.f.service();
    let req = request(&s.file);
    thread::scope(|scope| {
        let waiting = scope.spawn(|| service.request_model(&req));
        wait_until(|| server.count() == 1);
        assert!(service.cancel_model_request(&req.request_id).unwrap());
        server.release();
        let err = waiting.join().unwrap().unwrap_err();
        assert_eq!(err.code, ErrorCode::AnalysisCancelled);
    });
    let row = row(&s.f, &req.request_id);
    assert_eq!(row.status, "cancelled");
    assert_eq!(row.usage, None, "the late answer was not saved");
    assert!(
        !service.cancel_model_request(&req.request_id).unwrap(),
        "already ended"
    );
    assert_eq!(
        service
            .cancel_model_request(&uuid::Uuid::new_v4().to_string())
            .unwrap_err()
            .code,
        ErrorCode::UnknownAnalysis
    );
    assert_eq!(
        service
            .record_model_result(&req.request_id, &choice())
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput,
        "a cancelled request never gets a result"
    );
}

#[test]
fn a_service_change_while_waiting_drops_the_answer() {
    type Change = fn(&OrganizeService);
    let changes: [(&str, Change); 3] = [
        ("other model", |s| {
            s.save_model_settings(&openai("gpt-5.6-luna", None))
                .unwrap();
        }),
        ("consent withdrawn", |s| {
            s.set_analysis_consent(false).unwrap();
        }),
        ("key removed", |s| {
            s.remove_api_key().unwrap();
        }),
    ];
    for (name, change) in changes {
        let server = TestServer::start(Planned::gated(completed(&choice())));
        let s = configured(&server);
        let service = s.f.service();
        let req = request(&s.file);
        thread::scope(|scope| {
            let waiting = scope.spawn(|| service.request_model(&req));
            wait_until(|| server.count() == 1);
            change(service);
            server.release();
            let err = waiting.join().unwrap().unwrap_err();
            assert_eq!(err.code, ErrorCode::AnalysisCancelled, "{name}");
        });
        assert_eq!(row(&s.f, &req.request_id).status, "cancelled", "{name}");
    }
}

#[test]
fn no_database_or_execution_lock_is_held_while_waiting() {
    let server = TestServer::start(Planned::gated(completed(&choice())));
    let s = configured(&server);
    let service = s.f.service();
    let other = s.f.register("Notes/2026/plan.md");
    let plan = s.f.plan(&other, "Taxes", &[]);
    let req = request(&s.file);
    thread::scope(|scope| {
        let waiting = scope.spawn(|| service.request_model(&req));
        wait_until(|| server.count() == 1);
        // The server holds its answer for up to 10 s. Everything below would
        // wait for that if the request held a lock.
        let started = Instant::now();
        service.get_app_state().unwrap();
        service.model_settings().unwrap();
        let action = s.f.apply(&plan.plan_id).unwrap();
        assert_eq!(action.status, ActionStatus::Committed);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "nothing waited for the model"
        );
        server.release();
        assert!(waiting.join().unwrap().is_ok());
    });
}

#[test]
fn a_request_still_waiting_when_traeva_quits_is_interrupted_at_the_next_start() {
    let server = TestServer::start(Planned::answer(200, completed(&choice())));
    let mut s = configured(&server);
    let req = request(&s.file);
    // As if Traeva quit while the request was waiting.
    s.f.service()
        .store()
        .unwrap()
        .insert_analysis(&AnalysisRow {
            request_id: &req.request_id,
            file_id: &s.file.file_id,
            file_sha256: &s.file.sha256,
            file_revision: s.file.file_revision,
            provider: "openai",
            model: "gpt-6-luna",
            requested_at_ms: 1,
            prompt_version: "test-prompt-1",
            output_schema_version: "test-schema-1",
            extractor_version: crate::observe::EXTRACTOR_VERSION,
            app_version: "0.1.0",
            request_meta: &json!({}),
            status: AnalysisStatus::Analyzing,
            error_code: None,
        })
        .unwrap();
    s.f.restart();
    assert_eq!(row(&s.f, &req.request_id).status, "interrupted");
}

#[test]
fn only_an_answered_request_gets_a_checked_result_or_a_rejection() {
    let server = TestServer::start(Planned::answer(200, completed(&choice())));
    let s = configured(&server);
    let service = s.f.service();

    let first = request(&s.file);
    service.request_model(&first).unwrap();
    let refused = |result: &Value| {
        service
            .record_model_result(&first.request_id, result)
            .unwrap_err()
            .code
    };
    assert_eq!(
        refused(&json!(["c1"])),
        ErrorCode::InvalidInput,
        "not an object"
    );
    assert_eq!(
        refused(&json!({ "reason": "x".repeat(MAX_RESULT_BYTES) })),
        ErrorCode::InvalidInput,
        "over 16 KiB"
    );
    service
        .record_model_result(&first.request_id, &choice())
        .unwrap();
    let saved = row(&s.f, &first.request_id);
    assert_eq!(saved.status, "validated");
    assert_eq!(saved.result, Some(choice()));
    assert_eq!(refused(&choice()), ErrorCode::InvalidInput, "only once");
    assert_eq!(
        service
            .reject_model_output(&first.request_id)
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput,
        "a checked result is not rejected afterwards"
    );

    let second = request(&s.file);
    service.request_model(&second).unwrap();
    service.reject_model_output(&second.request_id).unwrap();
    let rejected = row(&s.f, &second.request_id);
    assert_eq!(rejected.status, "failed");
    assert_eq!(rejected.error_code.as_deref(), Some("invalid_model_output"));
    assert_eq!(rejected.result, None);

    let unknown = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        service
            .record_model_result(&unknown, &choice())
            .unwrap_err()
            .code,
        ErrorCode::UnknownAnalysis
    );
    assert_eq!(
        service.reject_model_output("not-an-id").unwrap_err().code,
        ErrorCode::InvalidInput
    );
}

#[test]
fn no_file_text_or_key_reaches_the_database_or_app_data() {
    let key_quoting =
        json!({ "error": { "message": format!("Incorrect API key provided: {TEST_KEY}") } })
            .to_string()
            .into_bytes();
    let app_data = |s: &Setup| s.f.app.path().join("Traeva");

    // A good answer, a refused key and a request that is too large.
    let good = TestServer::start(Planned::answer(200, completed(&choice())));
    let s = configured(&good);
    let req = request(&s.file);
    s.f.service().request_model(&req).unwrap();
    s.f.service()
        .record_model_result(&req.request_id, &choice())
        .unwrap();
    let mut large = request(&s.file);
    large.input = format!("{CONTENT_CANARY}{}", "x".repeat(MAX_REQUEST_BYTES));
    s.f.service().request_model(&large).unwrap_err();
    assert!(good.last(|sent| {
        sent.json()["input"]
            .as_str()
            .unwrap()
            .contains(CONTENT_CANARY)
    }));
    assert_eq!(
        files_containing(&app_data(&s), CONTENT_CANARY),
        Vec::<String>::new()
    );
    assert_eq!(
        files_containing(&app_data(&s), "KEYCANARY"),
        Vec::<String>::new()
    );

    let refusing = TestServer::start(Planned::answer(401, key_quoting.clone()));
    let mut s = configured(&refusing);
    s.f.service().request_model(&request(&s.file)).unwrap_err();
    assert_eq!(
        files_containing(&app_data(&s), CONTENT_CANARY),
        Vec::<String>::new()
    );
    assert_eq!(
        files_containing(&app_data(&s), "KEYCANARY"),
        Vec::<String>::new()
    );

    // Diagnostics on: the body sent is recorded (that is their purpose), the
    // key never is, even where the provider quoted it. Clearing removes all.
    s.f.service().set_model_diagnostics(true).unwrap();
    s.f.service().request_model(&request(&s.file)).unwrap_err();
    let diagnostics = files_containing(&app_data(&s), CONTENT_CANARY);
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].contains("/diagnostics/"));
    assert_eq!(
        files_containing(&app_data(&s), "KEYCANARY"),
        Vec::<String>::new()
    );
    assert_eq!(s.f.service().clear_model_diagnostics().unwrap(), 1);
    assert_eq!(
        files_containing(&app_data(&s), CONTENT_CANARY),
        Vec::<String>::new()
    );

    // After a restart the answer is still metadata only.
    s.f.restart();
    assert_eq!(
        files_containing(&app_data(&s), CONTENT_CANARY),
        Vec::<String>::new()
    );
}

#[test]
fn redaction_removes_the_key_and_key_like_text() {
    let key = "sk-test-SECRET-123";
    assert_eq!(
        redact("use sk-test-SECRET-123, or sk-proj-****abcd. ok", key),
        "use [key removed], or [key removed]. ok"
    );
    assert_eq!(redact("nothing here", key), "nothing here");
}

#[test]
fn a_blocked_storage_sends_nothing() {
    let server = TestServer::start(Planned::answer(200, completed(&choice())));
    let s = configured(&server);
    let mut second = crate::OrganizeService::start(s.f.app.path().join("Traeva"));
    second.model.endpoint = server.url.clone();
    let err = second.request_model(&request(&s.file)).unwrap_err();
    assert_eq!(err.code, ErrorCode::AnotherInstanceRunning);
    assert_eq!(server.count(), 0);
}

#[test]
fn a_setup_change_while_the_request_waits_means_nothing_is_sent() {
    // #70: the request's row waits for the database (another connection
    // holds the write lock); the setup changes meanwhile and the change
    // returns. Once the row is saved, nothing may go out.
    for change in ["consent withdrawn", "key replaced"] {
        let server = TestServer::start(Planned::answer(200, completed(&choice())));
        let s = configured(&server);
        let service = s.f.service();
        let blocker = s.f.db();
        blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
        let req = request(&s.file);
        let outcome = std::thread::scope(|scope| {
            let waiting = scope.spawn(|| service.request_model(&req));
            // Time to read the setup and reach the blocked row.
            std::thread::sleep(Duration::from_millis(500));
            assert!(!waiting.is_finished(), "{change}: the row must be waiting");
            if change == "consent withdrawn" {
                service.set_analysis_consent(false).unwrap();
            } else {
                // Still allowed to send, but not the setup the request was
                // read with.
                let new_key = format!("{TEST_KEY}2");
                service
                    .save_model_settings(&openai(DEFAULT_MODEL, Some(&new_key)))
                    .unwrap();
            }
            assert_eq!(server.count(), 0, "{change}: still waiting");
            blocker.execute_batch("COMMIT").unwrap();
            waiting.join().unwrap()
        });
        let expected = if change == "consent withdrawn" {
            ErrorCode::ConsentRequired
        } else {
            ErrorCode::AnalysisCancelled
        };
        assert_eq!(outcome.unwrap_err().code, expected, "{change}");
        assert_eq!(server.count(), 0, "{change}: nothing reached the provider");
        let status: String =
            s.f.db()
                .query_row("SELECT status FROM analyses", [], |r| r.get(0))
                .unwrap();
        assert_eq!(status, "cancelled", "{change}");
    }
}

#[test]
fn finds_agreement_changed_while_waiting_keeps_the_answer() {
    // Find's agreement is not part of Organize's setup: turning searching
    // with the model on or off while Organize waits does not drop the answer.
    for agree in [true, false] {
        let server = TestServer::start(Planned::gated(completed(&choice())));
        let s = configured(&server);
        let service = s.f.service();
        if !agree {
            service.set_find_consent(true).unwrap();
        }
        let req = request(&s.file);
        thread::scope(|scope| {
            let waiting = scope.spawn(|| service.request_model(&req));
            wait_until(|| server.count() == 1);
            service.set_find_consent(agree).unwrap();
            server.release();
            assert!(waiting.join().unwrap().is_ok(), "agree {agree}");
        });
        assert_eq!(
            row(&s.f, &req.request_id).status,
            "answered",
            "agree {agree}"
        );
    }
}

#[test]
fn image_sending_changed_after_the_setup_was_read_sends_nothing() {
    // §14.10: Organize chooses the images with the setup it sends with. Image
    // sending turned off (or back to asking) after that read stops the
    // request at its last check, so the copies chosen earlier never go out.
    use crate::settings::ImageSending;
    for mode in [ImageSending::Off, ImageSending::AskEachTime] {
        let server = TestServer::start(Planned::answer(200, completed(&choice())));
        let s = configured(&server);
        let service = s.f.service();
        service
            .set_image_sending(ImageSending::WithoutAsking)
            .unwrap();
        let setup = service.sending_setup(Some(&s.file.file_id)).unwrap();
        assert_eq!(setup.image_sending, ImageSending::WithoutAsking);
        service.set_image_sending(mode).unwrap();
        let mut req = request(&s.file);
        req.images = vec![b"synthetic picture".to_vec()];
        let err = service
            .request_model_using(&setup, &req, &mut || true)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::AnalysisCancelled, "{mode:?}");
        assert_eq!(server.count(), 0, "{mode:?}: nothing reached the provider");
        assert_eq!(row(&s.f, &req.request_id).status, "cancelled", "{mode:?}");
    }
}

// ---------------------------------------------------------------------------
// The send path on its own (shared-contract.md §4.6, for #33)
// ---------------------------------------------------------------------------

const MEMORY_INSTRUCTIONS: &str = "Synthetic test: describe what changed.";

fn sample_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "folder": { "type": "string" } },
        "required": ["folder"],
        "additionalProperties": false
    })
}

fn outgoing<'a>(schema: &'a Value, input: &'a str) -> OutgoingRequest<'a> {
    OutgoingRequest {
        schema_name: "sample_choice",
        output_schema: schema,
        instructions: MEMORY_INSTRUCTIONS,
        input,
    }
}

/// Provider, model and key saved, but Organize's agreement to send is not.
fn set_up_without_organize_agreement(server: &TestServer) -> Fixture {
    let mut f = Fixture::new();
    f.service_mut().model.endpoint = server.url.clone();
    f.service()
        .save_model_settings(&openai(DEFAULT_MODEL, Some(TEST_KEY)))
        .unwrap();
    f
}

fn the_recipient() -> Recipient {
    Recipient {
        provider: crate::settings::PROVIDER_OPENAI.to_string(),
        model: DEFAULT_MODEL.to_string(),
    }
}

#[test]
fn send_only_needs_no_organize_agreement_and_records_nothing() {
    let server = TestServer::start(Planned::answer(200, completed(&choice())));
    let f = set_up_without_organize_agreement(&server);
    // Even with Organize's diagnostics on, this path writes none.
    f.service().set_model_diagnostics(true).unwrap();
    let schema = sample_schema();
    let input = format!("{CONTENT_CANARY} the citation changed");
    let mut asked = Vec::new();
    let answer = f
        .service()
        .send_only(&outgoing(&schema, &input), &mut |recipient| {
            asked.push(recipient.clone());
            true
        })
        .unwrap()
        .expect("sent");
    assert_eq!(answer.output, choice());
    assert_eq!(answer.provider_request_id.as_deref(), Some("req_test123"));
    assert_eq!(
        answer.usage,
        Some(Usage {
            input_tokens: 120,
            output_tokens: 30,
            total_tokens: 150
        })
    );
    assert_eq!(asked, [the_recipient()], "asked once, about the recipient");
    assert_eq!(answer.recipient, the_recipient());
    assert_eq!(server.count(), 1);

    assert_eq!(f.count("analyses"), 0, "no Organize record");
    let diagnostics = crate::settings::diagnostics_dir(f.service().store().unwrap().app_data_dir());
    let written = fs::read_dir(&diagnostics).map_or(0, |entries| entries.count());
    assert_eq!(written, 0, "no diagnostic record");
    assert!(
        !format!("{answer:?}").contains("folder"),
        "Debug hides the output"
    );
}

#[test]
fn send_only_sends_the_same_form_with_the_same_limits() {
    let server = TestServer::start(Planned::answer(200, completed(&choice())));
    let f = set_up_without_organize_agreement(&server);
    let schema = sample_schema();
    f.service()
        .send_only(&outgoing(&schema, "Synthetic input"), &mut |_| true)
        .unwrap()
        .expect("sent");
    server.last(|sent| {
        assert_eq!(sent.request_line, "POST /v1/responses HTTP/1.1");
        assert_eq!(
            sent.header("authorization"),
            Some(format!("Bearer {TEST_KEY}").as_str())
        );
        assert_eq!(
            sent.json(),
            json!({
                "model": DEFAULT_MODEL,
                "instructions": MEMORY_INSTRUCTIONS,
                "input": "Synthetic input",
                "text": {
                    "format": {
                        "type": "json_schema",
                        "name": "sample_choice",
                        "schema": schema,
                        "strict": true,
                    }
                },
                "reasoning": { "effort": "low" },
                "max_output_tokens": 4000,
                "store": false,
            })
        );
    });

    // Over 64 KiB: refused before anything is sent.
    let big = "x".repeat(MAX_REQUEST_BYTES);
    let err = f
        .service()
        .send_only(&outgoing(&schema, &big), &mut |_| true)
        .unwrap_err();
    assert_eq!(
        err,
        SendFailure {
            code: ErrorCode::ModelRequestTooLarge,
            retryable: false
        }
    );
    assert_eq!(server.count(), 1);

    // https_only refuses an http:// address before anything is sent.
    let mut f = f;
    f.service_mut().model.https_only = true;
    let err = f
        .service()
        .send_only(&outgoing(&schema, "Synthetic input"), &mut |_| true)
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ModelServiceError);
    assert_eq!(server.count(), 1);
}

#[test]
fn when_still_allowed_says_no_nothing_is_sent() {
    let server = TestServer::start(Planned::answer(200, completed(&choice())));
    let f = set_up_without_organize_agreement(&server);
    let schema = sample_schema();
    let sent = f
        .service()
        .send_only(&outgoing(&schema, "Synthetic input"), &mut |_| false)
        .unwrap();
    assert!(sent.is_none());
    assert_eq!(server.count(), 0);
}

#[test]
fn still_allowed_is_asked_with_the_settings_lock_held() {
    let server = TestServer::start(Planned::answer(200, completed(&choice())));
    let f = set_up_without_organize_agreement(&server);
    let schema = sample_schema();
    let service = f.service();
    let mut held = None;
    service
        .send_only(&outgoing(&schema, "Synthetic input"), &mut |_| {
            held = Some(service.model.settings_lock.try_lock().is_err());
            true
        })
        .unwrap()
        .expect("sent");
    assert_eq!(held, Some(true));
}

#[test]
fn a_setup_changed_since_it_was_read_sends_nothing() {
    for change in ["key replaced", "key removed"] {
        let server = TestServer::start(Planned::answer(200, completed(&choice())));
        let f = set_up_without_organize_agreement(&server);
        let schema = sample_schema();
        let setup = f.service().send_only_setup().unwrap();
        let expected = if change == "key replaced" {
            // Still a usable setup, but not the one the request was read
            // with: trying again can work.
            let new_key = format!("{TEST_KEY}2");
            f.service()
                .save_model_settings(&openai(DEFAULT_MODEL, Some(&new_key)))
                .unwrap();
            SendFailure {
                code: ErrorCode::AnalysisCancelled,
                retryable: true,
            }
        } else {
            // The setup refuses now: its own reason comes back.
            f.service().remove_api_key().unwrap();
            SendFailure {
                code: ErrorCode::ModelNotConfigured,
                retryable: false,
            }
        };
        let mut asked = false;
        let err = f
            .service()
            .send_only_with(&setup, &outgoing(&schema, "Synthetic input"), &mut |_| {
                asked = true;
                true
            })
            .unwrap_err();
        assert_eq!(err, expected, "{change}");
        assert!(!asked, "{change}: the setup check comes first");
        assert_eq!(server.count(), 0, "{change}");
    }
}

#[test]
fn a_blocked_storage_fails_like_request_model_and_sends_nothing() {
    let server = TestServer::start(Planned::answer(200, completed(&choice())));
    let s = configured(&server);
    // A second service on the same app data finds it in use.
    let mut second = crate::OrganizeService::start(s.f.app.path().join("Traeva"));
    second.model.endpoint = server.url.clone();
    let organize = second.request_model(&request(&s.file)).unwrap_err();
    assert_eq!(organize.code, ErrorCode::AnotherInstanceRunning);
    let schema = sample_schema();
    let memory = second
        .send_only(&outgoing(&schema, "Synthetic input"), &mut |_| true)
        .unwrap_err();
    assert_eq!(
        memory,
        SendFailure {
            code: organize.code,
            retryable: organize.retryable
        }
    );
    assert_eq!(server.count(), 0);
}

#[test]
fn a_bad_output_schema_is_refused_before_anything_is_sent() {
    let server = TestServer::start(Planned::answer(200, completed(&choice())));
    let f = set_up_without_organize_agreement(&server);
    let schema = sample_schema();
    let not_an_object = json!(["folder"]);
    let long_name = "n".repeat(65);
    let cases = [
        ("memory.changes", &schema),
        ("", &schema),
        (long_name.as_str(), &schema),
        ("sample_choice", &not_an_object),
    ];
    for (name, schema) in cases {
        let request = OutgoingRequest {
            schema_name: name,
            output_schema: schema,
            instructions: MEMORY_INSTRUCTIONS,
            input: "Synthetic input",
        };
        let err = f.service().send_only(&request, &mut |_| true).unwrap_err();
        assert_eq!(
            err,
            SendFailure {
                code: ErrorCode::InvalidInput,
                retryable: false
            },
            "{name}"
        );
    }
    assert_eq!(server.count(), 0);
}

#[test]
fn send_only_holds_no_lock_while_waiting_and_keeps_a_late_answer() {
    let server = TestServer::start(Planned::gated(completed(&choice())));
    let f = set_up_without_organize_agreement(&server);
    let schema = sample_schema();
    let service = f.service();
    let sent = thread::scope(|scope| {
        let waiting =
            scope.spawn(|| service.send_only(&outgoing(&schema, "Synthetic input"), &mut |_| true));
        wait_until(|| server.count() == 1);
        // Nothing is held while the answer is awaited.
        assert!(service.execution.try_lock().is_ok());
        f.db().execute_batch("BEGIN IMMEDIATE; COMMIT").unwrap();
        // A settings change meanwhile does not drop the answer: the caller
        // checks its own agreement again against the recipient.
        let new_key = format!("{TEST_KEY}2");
        service
            .save_model_settings(&openai(DEFAULT_MODEL, Some(&new_key)))
            .unwrap();
        server.release();
        waiting.join().unwrap()
    });
    let answer = sent.unwrap().expect("sent");
    assert_eq!(answer.output, choice());
    assert_eq!(answer.recipient, the_recipient());
}

#[test]
fn a_missing_setup_or_key_sends_nothing() {
    let server = TestServer::start(Planned::answer(200, completed(&choice())));
    let mut f = Fixture::new();
    f.service_mut().model.endpoint = server.url.clone();
    let schema = sample_schema();
    let send = |f: &Fixture| {
        f.service()
            .send_only(&outgoing(&schema, "Synthetic input"), &mut |_| true)
            .unwrap_err()
    };
    let not_configured = SendFailure {
        code: ErrorCode::ModelNotConfigured,
        retryable: false,
    };
    assert_eq!(send(&f), not_configured, "no settings");
    f.service()
        .save_model_settings(&openai(DEFAULT_MODEL, Some(TEST_KEY)))
        .unwrap();
    f.service().remove_api_key().unwrap();
    assert_eq!(send(&f), not_configured, "no key");
    assert_eq!(server.count(), 0);
}

#[test]
fn send_only_fails_with_the_same_codes_as_request_model() {
    let schema = sample_schema();
    let mut slow = Planned::answer(200, completed(&choice()));
    slow.delay = Duration::from_secs(3);
    let cases = [
        Planned::answer(401, b"{}".to_vec()),
        Planned::answer(404, b"{}".to_vec()),
        Planned::answer(429, b"{}".to_vec()),
        Planned::answer(500, b"{}".to_vec()),
        Planned::answer(400, b"{}".to_vec()),
        Planned::answer(200, b"not json".to_vec()),
        slow,
    ];
    for (case, planned) in cases.into_iter().enumerate() {
        let server = TestServer::start(planned);
        let mut s = configured(&server);
        s.f.service_mut().model.timeout = Duration::from_secs(1);
        let organize = s.f.service().request_model(&request(&s.file)).unwrap_err();
        let memory =
            s.f.service()
                .send_only(&outgoing(&schema, "Synthetic input"), &mut |_| true)
                .unwrap_err();
        assert_eq!(
            memory,
            SendFailure {
                code: organize.code,
                retryable: organize.retryable
            },
            "case {case}"
        );
        assert_eq!(server.count(), 2, "case {case}: one each, no retry");
    }
}
