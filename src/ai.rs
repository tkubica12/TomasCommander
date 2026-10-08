use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(windows)]
mod windows;

pub type Result<T> = std::result::Result<T, String>;
const INSTRUCTIONS: &str = "Answer only the user's explicit prompt. No file access, tools, actions, or private data. Be concise.";
pub const MAX_RESPONSE_BYTES: usize = 65_536;
pub const SESSION_FRESHNESS: Duration = Duration::from_secs(60);

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub resource: String,
    pub resource_group: String,
    pub subscription: String,
    pub tenant: String,
    pub identity: String,
    pub deployment: String,
    pub model: String,
    pub stronger_deployment: String,
    pub stronger_model: String,
    pub limits: Limits,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub input_bytes: usize,
    pub input_token_ceiling: usize,
    pub output_bytes: usize,
    pub output_tokens: u32,
    pub requests: u32,
    pub steps: u32,
    pub timeout_seconds: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            input_bytes: 2048,
            input_token_ceiling: 3072,
            output_bytes: 8192,
            output_tokens: 512,
            requests: 5,
            steps: 4,
            timeout_seconds: 60,
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        for (name, value) in [
            ("resource", &self.resource),
            ("resource group", &self.resource_group),
            ("deployment", &self.deployment),
            ("expected model", &self.model),
        ] {
            if !identifier(value, 128) {
                return Err(format!(
                    "Invalid {name}: use ASCII letters, numbers, hyphen, underscore or dot."
                ));
            }
        }
        if self.resource.contains(['_', '.'])
            || self.resource.starts_with('-')
            || self.resource.ends_with('-')
        {
            return Err("Resource must be an Azure resource DNS label.".into());
        }
        if !self.stronger_deployment.is_empty() && !identifier(&self.stronger_deployment, 128) {
            return Err("Invalid stronger deployment.".into());
        }
        if !self.stronger_deployment.is_empty() && !identifier(&self.stronger_model, 128) {
            return Err("Stronger deployment needs a verified expected model.".into());
        }
        if !guid(&self.subscription) || !guid(&self.tenant) {
            return Err("Subscription and tenant must be GUIDs.".into());
        }
        if self.identity.len() > 254
            || !self.identity.contains('@')
            || !self
                .identity
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"@._+-".contains(&b))
        {
            return Err("Expected identity must be an email address.".into());
        }
        let l = self.limits;
        if l.input_bytes == 0
            || l.input_bytes > 2048
            || l.input_token_ceiling == 0
            || l.input_token_ceiling > 3072
            || l.output_bytes == 0
            || l.output_bytes > 8192
            || l.output_tokens == 0
            || l.output_tokens > 512
            || l.requests == 0
            || l.requests > 5
            || l.steps == 0
            || l.steps > 4
            || l.timeout_seconds == 0
            || l.timeout_seconds > 60
        {
            return Err(
                "Pilot limits must be positive and cannot exceed the displayed hard ceilings."
                    .into(),
            );
        }
        Ok(())
    }

    pub fn host(&self) -> Result<String> {
        self.validate()?;
        Ok(format!("{}.cognitiveservices.azure.com", self.resource))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let bytes = read_bounded(path, 8192)?;
        let config: Self = serde_json::from_slice(&bytes)
            .map_err(|_| "Invalid AI configuration; file was preserved.")?;
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        if path.exists() {
            Self::load(path)?;
        }
        let bytes =
            serde_json::to_vec_pretty(self).map_err(|_| "Could not serialize AI configuration.")?;
        persist(path, &bytes)
    }
}

fn identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

fn guid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

pub fn config_path() -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is missing.")?)
            .join("TomasCommander")
            .join("ai-connection.json"),
    )
}

pub fn read_bounded(path: &Path, max: usize) -> Result<Vec<u8>> {
    let file = fs::File::open(path).map_err(|_| "Could not open AI state/configuration.")?;
    let mut bytes = Vec::new();
    file.take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read AI state/configuration.")?;
    if bytes.len() > max {
        return Err("AI state/configuration exceeds its size limit.".into());
    }
    Ok(bytes)
}

fn persist(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or("AI configuration has no parent.")?;
    fs::create_dir_all(parent).map_err(|_| "Could not create AI settings directory.")?;
    let temporary = path.with_extension("pending");
    let mut file = OpenOptions::new().write(true).create_new(true).open(&temporary)
        .map_err(|_| "AI state transaction is busy or interrupted; inspect the pending file before retrying.")?;
    let result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if result.is_err() {
        return Err(
            "AI state write failed; pending file preserved. No request is authorized.".into(),
        );
    }
    fs::rename(&temporary, path).map_err(|_| "AI state commit failed; pending file preserved.")?;
    Ok(())
}

pub enum Input<'a> {
    ExplicitPrompt(&'a str),
    FileMetadata,
    FileContent,
    Credentials,
    Workplace,
}

#[derive(Clone, Copy)]
pub enum Route {
    Primary,
    Stronger,
}

pub fn request(config: &Config, input: Input<'_>, route: Route) -> Result<Vec<u8>> {
    config.validate()?;
    if matches!(route, Route::Stronger) {
        return Err(
            "Stronger route is separately configured but paid use is not approved in AI-01.".into(),
        );
    }
    let Input::ExplicitPrompt(prompt) = input else {
        return Err("Data boundary: only explicitly submitted prompt text is authorized.".into());
    };
    if prompt.trim().is_empty()
        || prompt.len() > config.limits.input_bytes
        || prompt.len() + INSTRUCTIONS.len() > config.limits.input_token_ceiling
    {
        return Err("Prompt is empty or exceeds the input byte/conservative token ceiling.".into());
    }
    if prompt.contains('\0')
        || [
            "-----BEGIN ",
            "Bearer ",
            "accessToken",
            "api-key",
            "api_key",
        ]
        .iter()
        .any(|marker| prompt.contains(marker))
    {
        return Err(
            "Possible credential material is blocked. Use a harmless prompt without secrets."
                .into(),
        );
    }
    serde_json::to_vec(&json!({
        "model": config.deployment, "instructions": INSTRUCTIONS, "input": prompt,
        "max_output_tokens": config.limits.output_tokens,
        "store": false, "stream": false, "background": false,
        "tools": [], "tool_choice": "none", "parallel_tool_calls": false
    }))
    .map_err(|_| "Could not encode request.".into())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reply {
    pub text: String,
    pub request_id: String,
    pub response_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub elapsed_ms: u64,
    pub authentication_ms: u64,
    pub authentication_cached: bool,
    pub authentication_stages: AuthTimings,
    pub provider_ms: u64,
    pub local_us: u64,
    pub attempts: u32,
}

pub fn parse_reply(
    bytes: &[u8],
    limits: Limits,
    request_id: &str,
    model: &str,
    version: &str,
) -> Result<Reply> {
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err("Provider response exceeded byte ceiling.".into());
    }
    let response: Value = serde_json::from_slice(bytes).map_err(|_| "Malformed provider JSON.")?;
    if response["model"]
        .as_str()
        .is_none_or(|actual| actual != model && actual != format!("{model}-{version}"))
    {
        return Err(
            "Response model does not match the verified primary deployment; no fallback was made."
                .into(),
        );
    }
    if response["status"] != "completed" || !response["error"].is_null() {
        return Err("Provider did not complete the response (failed, filtered, or output-token limit reached). No retry was made.".into());
    }
    let output = response["output"]
        .as_array()
        .ok_or("Provider output is missing.")?;
    let mut text = String::new();
    for item in output {
        match item["type"].as_str() {
            Some("reasoning") => {}
            Some("message") if item["role"] == "assistant" => {
                for part in item["content"].as_array().ok_or("Malformed message content.")? {
                    let fragment = match part["type"].as_str() {
                        Some("output_text") => part["text"].as_str().ok_or("Malformed output text.")?,
                        Some("refusal") => return Err("Provider refused this prompt. No retry was made.".into()),
                        _ => return Err("Unexpected provider message content blocked.".into()),
                    };
                    if text.len() + fragment.len() > limits.output_bytes {
                        return Err("Provider output exceeded byte ceiling.".into());
                    }
                    text.push_str(fragment);
                }
            }
            _ => return Err("Unexpected tool/action output blocked; no effects or follow-up requests were executed.".into()),
        }
    }
    if text.trim().is_empty() {
        return Err("Provider returned no answer text.".into());
    }
    let input_tokens = response["usage"]["input_tokens"]
        .as_u64()
        .ok_or("Provider input usage is missing.")?;
    let output_tokens = response["usage"]["output_tokens"]
        .as_u64()
        .ok_or("Provider output usage is missing.")?;
    if input_tokens > limits.input_token_ceiling as u64
        || output_tokens > u64::from(limits.output_tokens)
    {
        return Err("Provider usage exceeded runtime token ceiling. No follow-up was made.".into());
    }
    let response_id = response["id"]
        .as_str()
        .filter(|s| identifier(s, 128))
        .ok_or("Invalid provider response ID.")?;
    if !identifier(request_id, 128) {
        return Err("Missing/invalid provider request ID.".into());
    }
    Ok(Reply {
        text,
        request_id: request_id.into(),
        response_id: response_id.into(),
        input_tokens,
        output_tokens,
        elapsed_ms: 0,
        authentication_ms: 0,
        authentication_cached: false,
        authentication_stages: AuthTimings::default(),
        provider_ms: 0,
        local_us: 0,
        attempts: 1,
    })
}

pub fn http_error(status: u32) -> String {
    let message = match status {
        401 => "Identity token is expired or rejected; sign in using Azure CLI.",
        403 => "Identity lacks inference permission. No role or key was changed.",
        404 => "Deployment or API is unavailable.",
        429 => "Provider quota/rate limit reached.",
        300..=399 => "Provider redirect blocked to protect the identity token.",
        _ => "Provider request failed.",
    };
    format!("{message} HTTP {status}. No automatic retry or fallback.")
}

pub struct ToolBudget {
    steps: u32,
    maximum: u32,
}

impl ToolBudget {
    pub fn new(maximum: u32) -> Self {
        Self {
            steps: 0,
            maximum: maximum.min(4),
        }
    }
    pub fn dispatch(&mut self, name: &str, arguments: &str) -> Result<&'static str> {
        if self.steps >= self.maximum {
            return Err("Tool step budget exhausted.".into());
        }
        self.steps += 1;
        if arguments.len() > 256 {
            return Err("Tool argument byte ceiling exceeded.".into());
        }
        let value: Value =
            serde_json::from_str(arguments).map_err(|_| "Malformed tool arguments.")?;
        if name != "runtime_capabilities" || value.as_object().is_none_or(|o| !o.is_empty()) {
            return Err("Tool is not allowlisted or arguments/effects are unapproved.".into());
        }
        Ok(
            "Explicit text inference only; file, shell, MCP, mutation and stronger-model effects are blocked.",
        )
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub config: Config,
    pub prompt: Option<String>,
    pub config_path: PathBuf,
    #[serde(default)]
    pub recheck: bool,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct AuthTimings {
    pub account_ms: u64,
    pub resource_ms: u64,
    pub deployments_ms: u64,
    pub token_ms: u64,
}

#[derive(Serialize, Deserialize)]
pub struct Connection {
    pub message: String,
    pub authentication_ms: u64,
    pub cached: bool,
    pub elapsed_ms: u64,
    pub authentication_stages: AuthTimings,
}

#[derive(Serialize, Deserialize)]
pub enum Completion {
    Connected(Connection),
    Answer(Reply),
    Error(String),
}

pub fn run(job: Job, cancel: Arc<AtomicBool>) -> Result<Completion> {
    Session::default().run(job, cancel)
}

#[derive(Default)]
pub struct Session {
    worker: Option<Worker>,
}

struct Worker {
    child: std::process::Child,
    input: Arc<Mutex<std::process::ChildStdin>>,
    results: mpsc::Receiver<Result<Vec<u8>>>,
    sender: mpsc::SyncSender<Result<Vec<u8>>>,
    last_used: Instant,
    #[cfg(windows)]
    job: Option<windows::JobHandle>,
}

impl Worker {
    fn spawn(mut command: Command) -> Result<Self> {
        #[cfg(windows)]
        let contained = windows::spawn_contained(&mut command)?;
        #[cfg(windows)]
        let mut child = contained.child;
        #[cfg(not(windows))]
        let mut child = command.spawn().map_err(|_| "Could not start AI worker.")?;
        let input = child.stdin.take().ok_or("Worker input unavailable.")?;
        let stdout = child.stdout.take().ok_or("Worker output unavailable.")?;
        let (sender, results) = mpsc::sync_channel(1);
        let output = sender.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let result = read_frame(&mut reader, MAX_RESPONSE_BYTES)
                    .and_then(|bytes| bytes.ok_or("AI worker closed its result pipe.".into()));
                let failed = result.is_err();
                if output.send(result).is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input: Arc::new(Mutex::new(input)),
            results,
            sender,
            last_used: Instant::now(),
            #[cfg(windows)]
            job: Some(contained.job),
        })
    }

    fn stop(&mut self) -> Result<()> {
        #[cfg(windows)]
        drop(self.job.take());
        match self
            .child
            .try_wait()
            .map_err(|_| "Could not inspect AI worker termination.")?
        {
            Some(_) => Ok(()),
            None => {
                let killed = self.child.kill();
                if killed.is_err()
                    && self
                        .child
                        .try_wait()
                        .map_err(|_| "Could not inspect AI worker termination.")?
                        .is_none()
                {
                    return Err("AI worker termination failed; do not submit more requests.".into());
                }
                self.child
                    .wait()
                    .map_err(|_| "AI worker termination could not be confirmed.")?;
                Ok(())
            }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Explicit request/idle/disconnect paths report stop errors; this is last-resort ownership cleanup.
        let _ = self.stop();
    }
}

impl Session {
    pub fn clear(&mut self) -> Result<()> {
        if let Some(mut worker) = self.worker.take() {
            worker.stop()?;
        }
        Ok(())
    }

    pub fn expire_idle(&mut self) -> Result<bool> {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.last_used.elapsed() >= SESSION_FRESHNESS)
        {
            self.clear()?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn run(&mut self, job: Job, cancel: Arc<AtomicBool>) -> Result<Completion> {
        let preflight = job.config.validate().and_then(|()| {
            job.prompt
                .as_ref()
                .map(|prompt| request(&job.config, Input::ExplicitPrompt(prompt), Route::Primary))
                .transpose()
                .map(|_| ())
        });
        if let Err(error) = preflight {
            self.clear()?;
            return Err(error);
        }
        if cancel.load(Ordering::Acquire) {
            self.clear()?;
            return Err("Cancelled before authentication.".into());
        }
        let executable =
            std::env::current_exe().map_err(|_| "Could not locate native AI worker executable.")?;
        let mut command = Command::new(executable);
        command
            .arg("--ai-worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        self.run_command(job, cancel, command)
    }

    fn run_command(
        &mut self,
        job: Job,
        cancel: Arc<AtomicBool>,
        command: Command,
    ) -> Result<Completion> {
        let started = Instant::now();
        self.expire_idle()?;
        if self.worker.is_none() {
            self.worker = Some(Worker::spawn(command)?);
        }
        let result = self.exchange(&job, cancel, started);
        if result.is_err() || matches!(result, Ok(Completion::Error(_))) {
            self.clear().map_err(|cleanup| {
                format!(
                    "{} {cleanup}",
                    result
                        .as_ref()
                        .err()
                        .map(String::as_str)
                        .unwrap_or("AI session failed.")
                )
            })?;
        }
        result
    }

    fn exchange(
        &mut self,
        job: &Job,
        cancel: Arc<AtomicBool>,
        started: Instant,
    ) -> Result<Completion> {
        let worker = self.worker.as_mut().ok_or("AI worker is unavailable.")?;
        let encoded = serde_json::to_vec(&job).map_err(|_| "Could not encode AI job.")?;
        if encoded.len() > 16_384 {
            return Err("AI job exceeded its byte ceiling.".into());
        }
        let input = worker.input.clone();
        let sender = worker.sender.clone();
        thread::spawn(move || {
            let result = (|| -> Result<()> {
                let mut stdin = input.lock().map_err(|_| "Worker input lock failed.")?;
                stdin
                    .write_all(&encoded)
                    .map_err(|_| "Worker input failed.")?;
                stdin
                    .write_all(b"\n")
                    .and_then(|()| stdin.flush())
                    .map_err(|_| "Worker input frame failed.")?;
                Ok(())
            })();
            if let Err(error) = result {
                let _ = sender.send(Err(error));
            }
        });
        loop {
            let stopped = cancel.load(Ordering::Acquire);
            if stopped
                || started.elapsed() >= Duration::from_secs(job.config.limits.timeout_seconds)
            {
                worker.stop()?;
                return Err(if stopped {
                "Cancelled. Any submitted request may still be billed; no retry was made."
            } else {
                "AI deadline exceeded. Any submitted request has an uncertain billable outcome; no retry was made."
            }.into());
            }
            match worker.results.recv_timeout(Duration::from_millis(25)) {
                Ok(result) => {
                    let bytes = result?;
                    let mut completion: Completion = serde_json::from_slice(&bytes)
                        .map_err(|_| "Malformed AI worker result.")?;
                    if cancel.load(Ordering::Acquire) {
                        worker.stop()?;
                        return Err("Cancelled as the result arrived. A submitted request may have completed; no retry was made.".into());
                    }
                    if let Completion::Answer(reply) = &mut completion {
                        reply.elapsed_ms = started.elapsed().as_millis() as u64;
                    }
                    if let Completion::Connected(connection) = &mut completion {
                        connection.elapsed_ms = started.elapsed().as_millis() as u64;
                    }
                    worker.last_used = Instant::now();
                    return Ok(completion);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    worker.stop()?;
                    return Err("AI worker channel disconnected.".into());
                }
            }
        }
    }
}

fn read_frame(reader: &mut impl BufRead, maximum: usize) -> Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    loop {
        let chunk = reader.fill_buf().map_err(|_| "Worker frame read failed.")?;
        if chunk.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err("Incomplete worker frame.".into())
            };
        }
        let end = chunk.iter().position(|byte| *byte == b'\n');
        let length = end.unwrap_or(chunk.len());
        if bytes.len() + length > maximum {
            return Err("Worker frame byte ceiling exceeded.".into());
        }
        bytes.extend_from_slice(&chunk[..length]);
        reader.consume(length + usize::from(end.is_some()));
        if end.is_some() {
            return Ok(Some(bytes));
        }
    }
}

pub fn worker() -> Result<()> {
    let mut input = BufReader::new(std::io::stdin());
    #[cfg(windows)]
    let mut cache = windows::AuthCache::default();
    while let Some(bytes) = read_frame(&mut input, 16_384)? {
        let job: Job = serde_json::from_slice(&bytes).map_err(|_| "Malformed AI job.")?;
        #[cfg(windows)]
        let completion = match windows::execute(job, &mut cache) {
            Ok(completion) => completion,
            Err(error) => {
                cache.clear();
                Completion::Error(error)
            }
        };
        #[cfg(not(windows))]
        let completion = {
            let _ = job;
            Completion::Error("Foundry adapter requires Windows.".into())
        };
        let output =
            serde_json::to_vec(&completion).map_err(|_| "Worker result encoding failed.")?;
        let mut output_stream = std::io::stdout();
        output_stream
            .write_all(&output)
            .and_then(|()| output_stream.write_all(b"\n"))
            .and_then(|()| output_stream.flush())
            .map_err(|_| "Worker result write failed.")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config {
            resource: "synthetic-foundry".into(),
            resource_group: "synthetic".into(),
            subscription: "00000000-0000-0000-0000-000000000001".into(),
            tenant: "00000000-0000-0000-0000-000000000002".into(),
            identity: "test@example.org".into(),
            deployment: "synthetic-luna".into(),
            model: "synthetic-model-luna".into(),
            stronger_deployment: "synthetic-sol".into(),
            stronger_model: "synthetic-model-sol".into(),
            ..Config::default()
        }
    }
    #[test]
    fn framed_protocol_bounds_depth_and_partial_reads() {
        let mut input = BufReader::with_capacity(2, &b"abc\nsecond\n"[..]);
        assert_eq!(read_frame(&mut input, 3).unwrap().unwrap(), b"abc");
        assert_eq!(read_frame(&mut input, 6).unwrap().unwrap(), b"second");
        assert!(read_frame(&mut input, 6).unwrap().is_none());
        assert!(read_frame(&mut &b"abcd\n"[..], 3).is_err());
        assert!(read_frame(&mut &b"partial"[..], 20).is_err());
        let deep = format!("{}0{}", "[".repeat(200), "]".repeat(200));
        assert!(serde_json::from_str::<Job>(&deep).is_err());
        let mut encoded = serde_json::to_value(Job {
            config: config(),
            prompt: None,
            config_path: PathBuf::from("synthetic"),
            recheck: false,
        })
        .unwrap();
        encoded["unexpected"] = json!(true);
        assert!(serde_json::from_value::<Job>(encoded).is_err());
    }

    #[cfg(windows)]
    fn synthetic_command(program: &str) -> Command {
        let mut command = Command::new(r"C:\Program Files\Microsoft SDKs\Azure\CLI2\python.exe");
        command
            .args(["-I", "-c", program])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command
    }

    #[cfg(windows)]
    #[test]
    fn persistent_worker_reuses_frames_and_clears_on_idle_invalid_input_and_disconnect() {
        let result = serde_json::to_string(&Completion::Connected(Connection {
            message: "Synthetic fixture; no Azure access.".into(),
            authentication_ms: 0,
            cached: false,
            elapsed_ms: 0,
            authentication_stages: AuthTimings::default(),
        }))
        .unwrap();
        let program = format!("import sys\nfor line in sys.stdin:\n print({result:?},flush=True)");
        let job = || Job {
            config: config(),
            prompt: None,
            config_path: PathBuf::from("synthetic"),
            recheck: false,
        };
        let mut session = Session::default();
        for _ in 0..2 {
            assert!(matches!(
                session
                    .run_command(
                        job(),
                        Arc::new(AtomicBool::new(false)),
                        synthetic_command(&program)
                    )
                    .unwrap(),
                Completion::Connected(_)
            ));
        }
        let pid = session.worker.as_ref().unwrap().child.id();
        assert!(!session.expire_idle().unwrap());
        assert_eq!(session.worker.as_ref().unwrap().child.id(), pid);
        session.worker.as_mut().unwrap().last_used = Instant::now() - SESSION_FRESHNESS;
        assert!(session.expire_idle().unwrap());
        assert!(session.worker.is_none());
        session
            .run_command(
                job(),
                Arc::new(AtomicBool::new(false)),
                synthetic_command(&program),
            )
            .unwrap();
        let mut invalid = job();
        invalid.config.identity = "invalid".into();
        assert!(
            session
                .run(invalid, Arc::new(AtomicBool::new(false)))
                .is_err()
        );
        assert!(session.worker.is_none());
        session
            .run_command(
                job(),
                Arc::new(AtomicBool::new(false)),
                synthetic_command(&program),
            )
            .unwrap();
        session.clear().unwrap();
        assert!(session.worker.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn malformed_worker_results_are_redacted_and_never_retried() {
        for payload in [
            "synthetic.secret.token\n".to_string(),
            "x".repeat(MAX_RESPONSE_BYTES + 1),
        ] {
            let program =
                format!("import sys\nsys.stdin.readline()\nprint({payload:?},flush=True)");
            let mut session = Session::default();
            let error = session
                .run_command(
                    Job {
                        config: config(),
                        prompt: None,
                        config_path: PathBuf::from("synthetic"),
                        recheck: false,
                    },
                    Arc::new(AtomicBool::new(false)),
                    synthetic_command(&program),
                )
                .err()
                .unwrap();
            assert!(!error.contains("synthetic.secret.token"));
            assert!(session.worker.is_none());
        }
    }
    #[test]
    fn endpoint_and_argument_injection_blocked() {
        for resource in ["evil.com", "x/evil", "x:443", "x@evil", "x&cmd", "x\r\n"] {
            let mut c = config();
            c.resource = resource.into();
            assert!(c.host().is_err());
        }
        assert_eq!(
            config().host().unwrap(),
            "synthetic-foundry.cognitiveservices.azure.com"
        );
    }
    #[test]
    fn input_boundaries_and_routes() {
        let c = config();
        for input in [
            Input::FileMetadata,
            Input::FileContent,
            Input::Credentials,
            Input::Workplace,
        ] {
            assert!(request(&c, input, Route::Primary).is_err());
        }
        assert!(request(&c, Input::ExplicitPrompt("hello"), Route::Stronger).is_err());
        assert!(request(&c, Input::ExplicitPrompt(&"a".repeat(2049)), Route::Primary).is_err());
        assert!(request(&c, Input::ExplicitPrompt("Bearer secret"), Route::Primary).is_err());
        let body: Value = serde_json::from_slice(
            &request(&c, Input::ExplicitPrompt("hello"), Route::Primary).unwrap(),
        )
        .unwrap();
        assert_eq!(body["store"], false);
        assert_eq!(body["tools"], json!([]));
        assert!(body.get("previous_response_id").is_none());
    }
    #[test]
    fn typed_dispatch_blocks_malformed_effects_and_exhaustion() {
        let mut budget = ToolBudget::new(4);
        assert!(budget.dispatch("runtime_capabilities", "not json").is_err());
        assert!(budget.dispatch("shell", "{}").is_err());
        assert!(
            budget
                .dispatch("runtime_capabilities", r#"{"path":"private"}"#)
                .is_err()
        );
        assert!(budget.dispatch("runtime_capabilities", "{}").is_ok());
        assert!(budget.dispatch("runtime_capabilities", "{}").is_err());
    }
    #[test]
    fn caps_cannot_expand_and_token_ceiling_is_conservative() {
        let mut c = config();
        c.limits.requests = 6;
        assert!(c.validate().is_err());
        c = config();
        c.limits.input_token_ceiling = 100;
        assert!(request(&c, Input::ExplicitPrompt("hello"), Route::Primary).is_err());
    }
    #[test]
    fn parse_success_and_adversarial_outputs() {
        let c = config();
        let valid = json!({"id":"resp_synthetic","model":c.model,"status":"completed","error":null,
            "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Hello"}]}],
            "usage":{"input_tokens":30,"output_tokens":5}});
        let parse = |v: &Value| {
            parse_reply(
                &serde_json::to_vec(v).unwrap(),
                c.limits,
                "synthetic-id",
                &c.model,
                "synthetic-version",
            )
        };
        assert_eq!(parse(&valid).unwrap().text, "Hello");
        let mut value = valid.clone();
        value["model"] = json!("unapproved-stronger");
        assert!(parse(&value).is_err());
        value = valid.clone();
        value["model"] = json!(format!("{}-synthetic-version", c.model));
        assert!(parse(&value).is_ok());
        value["model"] = json!(format!("{}-different-version", c.model));
        assert!(parse(&value).is_err());
        value = valid.clone();
        value["output"][0]["type"] = json!("function_call");
        assert!(parse(&value).is_err());
        value = valid.clone();
        value["status"] = json!("incomplete");
        assert!(parse(&value).is_err());
        value = valid.clone();
        value["usage"]["output_tokens"] = json!(513);
        assert!(parse(&value).is_err());
        value = valid;
        value["output"][0]["content"][0]["text"] = json!("x".repeat(8193));
        assert!(parse(&value).is_err());
        assert!(parse_reply(b"broken", c.limits, "id", &c.model, "synthetic-version").is_err());
        assert!(
            parse_reply(
                &vec![0; MAX_RESPONSE_BYTES + 1],
                c.limits,
                "id",
                &c.model,
                "synthetic-version"
            )
            .is_err()
        );
    }
    #[test]
    fn provider_errors_are_safe_and_visible() {
        for status in [401, 403, 404, 429, 302, 500] {
            assert!(http_error(status).contains(&status.to_string()));
            assert!(http_error(status).contains("No automatic retry"));
        }
    }

    #[test]
    fn configuration_roundtrip_preserves_invalid_existing_state() {
        let root = std::env::temp_dir().join(format!(
            "tc-ai-config-owned-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("ai-connection.json");
        let c = config();
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap().model, c.model);
        fs::write(&path, br#"{"unexpected":"preserve"}"#).unwrap();
        assert!(c.save(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), br#"{"unexpected":"preserve"}"#);
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn deadline_and_cancellation_terminate_the_contained_worker() {
        fn sleeping_worker() -> Command {
            let mut command =
                Command::new(r"C:\Program Files\Microsoft SDKs\Azure\CLI2\python.exe");
            command
                .args([
                    "-I",
                    "-c",
                    "import sys,time; sys.stdin.buffer.read(); time.sleep(30)",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            command
        }
        for cancelled in [false, true] {
            let mut config = config();
            config.limits.timeout_seconds = 1;
            let flag = Arc::new(AtomicBool::new(false));
            if cancelled {
                let signal = flag.clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(100));
                    signal.store(true, Ordering::Release);
                });
            }
            let started = Instant::now();
            let result = Session::default().run_command(
                Job {
                    config,
                    prompt: None,
                    config_path: PathBuf::from("unused-synthetic-config"),
                    recheck: false,
                },
                flag,
                sleeping_worker(),
            );
            let error = result.err().expect("worker must stop");
            assert!(
                error.contains(if cancelled { "Cancelled" } else { "deadline" }),
                "{error}"
            );
            assert!(started.elapsed() < Duration::from_secs(5));
        }
    }

    #[cfg(windows)]
    #[test]
    fn cancellation_terminates_actual_descendants_without_waiting_for_blocked_io() {
        use ::windows::Win32::{
            Foundation::CloseHandle,
            System::Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        let path =
            std::env::temp_dir().join(format!("tc-ai-descendant-owned-{}.txt", std::process::id()));
        assert!(!path.exists(), "owned fixture path must be fresh");
        let mut command = synthetic_command(
            "import sys,time,subprocess\np=subprocess.Popen([sys.executable,'-I','-c','import time;time.sleep(30)'],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\nwith open(sys.argv[1],'w') as f: f.write(str(p.pid))\nsys.stdin.readline()\ntime.sleep(30)",
        );
        command.arg(&path);
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let controller = thread::spawn(move || {
            let mut c = config();
            c.limits.timeout_seconds = 5;
            Session::default().run_command(
                Job {
                    config: c,
                    prompt: None,
                    config_path: PathBuf::from("synthetic"),
                    recheck: false,
                },
                cancel,
                command,
            )
        });
        let started = Instant::now();
        while !path.is_file() && started.elapsed() < Duration::from_secs(3) {
            thread::sleep(Duration::from_millis(10));
        }
        let mut pid = None;
        while pid.is_none() && started.elapsed() < Duration::from_secs(3) {
            pid = fs::read_to_string(&path)
                .ok()
                .and_then(|value| value.parse::<u32>().ok());
            thread::sleep(Duration::from_millis(10));
        }
        signal.store(true, Ordering::Release);
        let cancelled_at = Instant::now();
        let error = controller.join().unwrap().err().unwrap();
        assert!(error.contains("Cancelled"));
        assert!(cancelled_at.elapsed() < Duration::from_millis(500));
        let pid = pid.expect("descendant must have started");
        unsafe {
            match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(handle) => {
                    let mut code = 0;
                    let result = GetExitCodeProcess(handle, &mut code);
                    CloseHandle(handle).unwrap();
                    result.unwrap();
                    assert_ne!(code, 259, "descendant must not remain active");
                }
                Err(error) => assert_eq!(
                    error.code().0 as u32,
                    0x80070057,
                    "only a vanished PID is acceptable"
                ),
            }
        }
        fs::remove_file(path).unwrap();
    }
}
