use super::*;
use ::windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        Networking::WinHttp::*,
        System::JobObjects::*,
    },
    core::{PCWSTR, w},
};
use std::{
    ffi::c_void,
    os::windows::{io::AsRawHandle, process::CommandExt},
};

pub(super) struct JobHandle(HANDLE);
impl Drop for JobHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
pub(super) struct Contained {
    pub child: std::process::Child,
    pub job: JobHandle,
}

pub(super) fn spawn_contained(command: &mut Command) -> Result<Contained> {
    unsafe {
        let handle = CreateJobObjectW(None, PCWSTR::null())
            .map_err(|_| "Could not create AI worker containment.")?;
        let job = JobHandle(handle);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const c_void,
            std::mem::size_of_val(&limits) as u32,
        )
        .map_err(|_| "Could not bind AI worker termination policy.")?;
        // The worker cannot start auth/network work until its parent writes stdin after assignment.
        let mut child = command
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|_| "Could not launch contained AI worker.")?;
        if AssignProcessToJobObject(handle, HANDLE(child.as_raw_handle())).is_err() {
            let _ = child.kill();
            child
                .wait()
                .map_err(|_| "Uncontained AI worker termination could not be confirmed.")?;
            return Err(
                "Windows could not contain AI worker and descendants; request blocked.".into(),
            );
        }
        Ok(Contained { child, job })
    }
}

struct Secret(Vec<u8>);
impl Drop for Secret {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            unsafe {
                std::ptr::write_volatile(byte, 0);
            }
        }
    }
}
struct SecretHeader(Vec<u16>);
impl Drop for SecretHeader {
    fn drop(&mut self) {
        for unit in &mut self.0 {
            unsafe {
                std::ptr::write_volatile(unit, 0);
            }
        }
    }
}

fn cli(arguments: &[&str]) -> Result<Secret> {
    let python = Path::new(r"C:\Program Files\Microsoft SDKs\Azure\CLI2\python.exe");
    if !python.is_file() {
        return Err("Azure CLI MSI installation is missing. Install/sign in explicitly; the app does not install or log in automatically.".into());
    }
    let mut child = Command::new(python)
        .args(["-IBm", "azure.cli"])
        .args(arguments)
        .args(["--only-show-errors", "--output", "json"])
        .env("AZURE_CORE_COLLECT_TELEMETRY", "false")
        .env("AZURE_EXTENSION_USE_DYNAMIC_INSTALL", "no")
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Azure CLI could not start. No login or cloud change was attempted.")?;
    let mut bytes = Secret(Vec::new());
    child
        .stdout
        .take()
        .ok_or("Azure CLI output unavailable.")?
        .take(32_769)
        .read_to_end(&mut bytes.0)
        .map_err(|_| "Azure CLI output failed.")?;
    if bytes.0.len() > 32_768 {
        let _ = child.kill();
        child
            .wait()
            .map_err(|_| "Azure CLI termination could not be confirmed.")?;
        return Err("Azure CLI result exceeded its bounded size.".into());
    }
    if !child
        .wait()
        .map_err(|_| "Azure CLI exit could not be confirmed.")?
        .success()
    {
        return Err("Azure CLI identity/resource check failed. Sign in with the configured identity and tenant, then verify subscription/deployment access. No credentials or raw CLI errors were logged.".into());
    }
    Ok(bytes)
}

fn cli_json(arguments: &[&str]) -> Result<Value> {
    let bytes = cli(arguments)?;
    serde_json::from_slice(&bytes.0)
        .map_err(|_| "Malformed Azure CLI result; raw output suppressed.".into())
}

fn validate_account(c: &Config, value: &Value) -> Result<()> {
    if value["id"].as_str() != Some(&c.subscription)
        || value["tenantId"].as_str() != Some(&c.tenant)
        || value["user"]["name"]
            .as_str()
            .is_none_or(|s| !s.eq_ignore_ascii_case(&c.identity))
        || value["user"]["type"] != "user"
        || value["environmentName"] != "AzureCloud"
        || value["state"] != "Enabled"
    {
        return Err("Azure CLI account does not match the configured identity, tenant, enabled subscription and Azure public cloud. Request blocked.".into());
    }
    Ok(())
}

fn validate_resource(c: &Config, value: &Value) -> Result<()> {
    let expected = format!("https://{}/", c.host()?);
    if value["properties"]["endpoint"].as_str() != Some(&expected) || value["kind"] != "AIServices"
    {
        return Err("Foundry resource endpoint/kind does not match the bound Azure host. Token transmission blocked.".into());
    }
    Ok(())
}

fn validate_deployment(value: &Value, deployment: &str, model: &str) -> Result<()> {
    if value["name"].as_str() != Some(deployment)
        || value["properties"]["provisioningState"] != "Succeeded"
        || value["properties"]["model"]["format"] != "OpenAI"
        || value["properties"]["model"]["name"].as_str() != Some(model)
    {
        return Err(
            "Configured OpenAI deployment is unavailable or not successfully provisioned.".into(),
        );
    }
    Ok(())
}

fn verify(c: &Config) -> Result<(AuthTimings, String)> {
    c.validate()?;
    let started = Instant::now();
    let account = cli_json(&[
        "account",
        "show",
        "--subscription",
        &c.subscription,
        "--query",
        "{id:id,tenantId:tenantId,user:user,environmentName:environmentName,state:state}",
    ])
    .map_err(|error| format!("Account check: {error}"))?;
    validate_account(c, &account)?;
    let account_ms = started.elapsed().as_millis() as u64;
    let started = Instant::now();
    let resource = cli_json(&[
        "cognitiveservices",
        "account",
        "show",
        "--subscription",
        &c.subscription,
        "--resource-group",
        &c.resource_group,
        "--name",
        &c.resource,
        "--query",
        "{kind:kind,properties:{endpoint:properties.endpoint}}",
    ])
    .map_err(|error| format!("Resource check: {error}"))?;
    validate_resource(c, &resource)?;
    let resource_ms = started.elapsed().as_millis() as u64;
    let started = Instant::now();
    let query = format!(
        "[?name=='{}' || name=='{}'].{{name:name,properties:{{provisioningState:properties.provisioningState,model:{{format:properties.model.format,name:properties.model.name,version:properties.model.version}}}}}}",
        c.deployment, c.stronger_deployment
    );
    let values = cli_json(&[
        "cognitiveservices",
        "account",
        "deployment",
        "list",
        "--subscription",
        &c.subscription,
        "--resource-group",
        &c.resource_group,
        "--name",
        &c.resource,
        "--query",
        &query,
    ])
    .map_err(|error| format!("Deployment check: {error}"))?;
    let values = values
        .as_array()
        .ok_or("Malformed deployment inventory; request blocked.")?;
    let mut primary_version = None;
    for (deployment, model) in [
        (&c.deployment, &c.model),
        (&c.stronger_deployment, &c.stronger_model),
    ] {
        if deployment.is_empty() {
            continue;
        }
        let mut matches = values
            .iter()
            .filter(|value| value["name"].as_str() == Some(deployment));
        let value = matches
            .next()
            .ok_or("Configured deployment is missing; request blocked.")?;
        if matches.next().is_some() {
            return Err("Ambiguous deployment inventory; request blocked.".into());
        }
        validate_deployment(value, deployment, model)?;
        if deployment == &c.deployment {
            primary_version = value["properties"]["model"]["version"]
                .as_str()
                .filter(|version| identifier(version, 128))
                .map(str::to_owned);
        }
    }
    Ok((
        AuthTimings {
            account_ms,
            resource_ms,
            deployments_ms: started.elapsed().as_millis() as u64,
            token_ms: 0,
        },
        primary_version
            .ok_or("Primary deployment model version is missing/invalid; request blocked.")?,
    ))
}

struct Token {
    secret: Secret,
    expires: u64,
}

fn token(c: &Config) -> Result<Token> {
    let bytes = cli(&[
        "account",
        "get-access-token",
        "--subscription",
        &c.subscription,
        "--resource",
        "https://cognitiveservices.azure.com/",
    ])
    .map_err(|error| format!("Token acquisition: {error}"))?;
    parse_token(&bytes.0, c)
}

fn parse_token(bytes: &[u8], c: &Config) -> Result<Token> {
    let mut value: Value = serde_json::from_slice(bytes)
        .map_err(|_| "Malformed authentication result; raw output suppressed.")?;
    let access = match value["accessToken"].take() {
        Value::String(access) => access,
        _ => return Err("Authentication returned no token.".into()),
    };
    let secret = Secret(access.into_bytes());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "System clock is invalid.")?
        .as_secs();
    let expires = value["expires_on"]
        .as_u64()
        .or_else(|| value["expires_on"].as_str().and_then(|s| s.parse().ok()))
        .ok_or("Authentication expiry is unavailable.")?;
    if value["tenant"].as_str() != Some(&c.tenant)
        || value["subscription"].as_str() != Some(&c.subscription)
        || expires <= now + c.limits.timeout_seconds + 60
    {
        return Err("Identity token is expired, near expiry, or for a different tenant/subscription. Sign in again; no retry was made.".into());
    }
    if secret.0.is_empty()
        || secret.0.len() > 16_384
        || !secret
            .0
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(b))
    {
        return Err("Invalid authentication token; raw output suppressed.".into());
    }
    Ok(Token { secret, expires })
}

#[derive(Default)]
pub(super) struct AuthCache(Option<CachedAuth>);

struct CachedAuth {
    config: Config,
    path: PathBuf,
    verified: Instant,
    token: Token,
    version: String,
}

fn unix_seconds() -> Result<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| "System clock is invalid.".into())
}

impl AuthCache {
    pub(super) fn clear(&mut self) {
        self.0 = None;
    }

    fn usable(&self, job: &Job, now: u64) -> bool {
        self.0.as_ref().is_some_and(|entry| {
            !job.recheck
                && entry.config == job.config
                && entry.path == job.config_path
                && entry.verified.elapsed() < SESSION_FRESHNESS
                && entry.token.expires > now.saturating_add(job.config.limits.timeout_seconds + 60)
        })
    }

    fn prepare(
        &mut self,
        job: &Job,
        load: impl FnOnce() -> Result<(Token, String)>,
    ) -> Result<bool> {
        if self.usable(job, unix_seconds()?) {
            return Ok(true);
        }
        self.clear();
        let (token, version) = load()?;
        self.0 = Some(CachedAuth {
            config: job.config.clone(),
            path: job.config_path.clone(),
            verified: Instant::now(),
            token,
            version,
        });
        Ok(false)
    }
}

struct Lease(PathBuf);
impl Lease {
    fn release(self) -> Result<()> {
        fs::remove_file(self.0)
            .map_err(|_| "AI spending lease cleanup failed; further requests are blocked.".into())
    }
}

fn reserve(path: &Path, maximum: u32) -> Result<(u32, Lease)> {
    let budget = path.with_file_name("ai-budget.json");
    let lock = path.with_file_name("ai-budget.lock");
    fs::create_dir_all(path.parent().ok_or("AI state path has no parent.")?)
        .map_err(|_| "Could not create AI state directory.")?;
    let lease_file = OpenOptions::new().create_new(true).write(true).open(&lock)
        .map_err(|_| "AI spending lease is busy or interrupted. Inspect ai-budget.lock; no request was sent.")?;
    drop(lease_file);
    let lease = Lease(lock);
    let result = (|| {
        let count = if budget.exists() {
            let bytes = read_bounded(&budget, 256)?;
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|_| "AI budget record is malformed; request blocked.")?;
            u32::try_from(
                value["attempts"]
                    .as_u64()
                    .ok_or("AI budget counter is missing.")?,
            )
            .map_err(|_| "AI budget counter is out of range.")?
        } else {
            0
        };
        if count >= maximum {
            return Err("Durable pilot request budget exhausted. Additional spending needs explicit owner approval; no automatic reset.".into());
        }
        let count = count + 1;
        persist(
            &budget,
            &serde_json::to_vec(&json!({"attempts":count}))
                .map_err(|_| "Budget encoding failed.")?,
        )?;
        Ok(count)
    })();
    match result {
        Ok(count) => Ok((count, lease)),
        Err(error) => {
            lease
                .release()
                .map_err(|cleanup| format!("{error} {cleanup}"))?;
            Err(error)
        }
    }
}

struct Internet(*mut c_void);
impl Internet {
    fn new(handle: *mut c_void) -> Result<Self> {
        if handle.is_null() {
            Err("WinHTTP initialization failed; raw errors suppressed.".into())
        } else {
            Ok(Self(handle))
        }
    }
}
impl Drop for Internet {
    fn drop(&mut self) {
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn protect_request(request: &Internet, response_timeout_ms: u32) -> Result<()> {
    unsafe {
        let never = WINHTTP_OPTION_REDIRECT_POLICY_NEVER;
        WinHttpSetOption(
            Some(request.0),
            WINHTTP_OPTION_REDIRECT_POLICY,
            Some(&never.to_ne_bytes()),
        )
        .map_err(|_| "WinHTTP redirect protection failed.")?;
        let disable = WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_AUTHENTICATION;
        WinHttpSetOption(
            Some(request.0),
            WINHTTP_OPTION_DISABLE_FEATURE,
            Some(&disable.to_ne_bytes()),
        )
        .map_err(|_| "WinHTTP ambient authentication/cookie protection failed.")?;
        WinHttpSetOption(
            Some(request.0),
            WINHTTP_OPTION_RECEIVE_RESPONSE_TIMEOUT,
            Some(&response_timeout_ms.to_ne_bytes()),
        )
        .map_err(|_| "WinHTTP response-header timeout binding failed.")?;
    }
    Ok(())
}

fn post(c: &Config, token: &Secret, body: &[u8]) -> Result<(Vec<u8>, String)> {
    unsafe {
        let session = Internet::new(WinHttpOpen(
            w!("TomasCommander/AI-01"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        let tls = WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3;
        WinHttpSetOption(
            Some(session.0),
            WINHTTP_OPTION_SECURE_PROTOCOLS,
            Some(&tls.to_ne_bytes()),
        )
        .map_err(|_| "WinHTTP TLS 1.2/1.3 policy binding failed.")?;
        WinHttpSetTimeouts(
            session.0,
            5000,
            5000,
            5000,
            (c.limits.timeout_seconds * 1000) as i32,
        )
        .map_err(|_| "WinHTTP timeout binding failed.")?;
        let host = wide(&c.host()?);
        let connection = Internet::new(WinHttpConnect(session.0, PCWSTR(host.as_ptr()), 443, 0))?;
        let request = Internet::new(WinHttpOpenRequest(
            connection.0,
            w!("POST"),
            w!("/openai/v1/responses"),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        protect_request(&request, (c.limits.timeout_seconds * 1000) as u32)?;
        let mut header = SecretHeader(
            "Content-Type: application/json\r\nAuthorization: Bearer "
                .encode_utf16()
                .collect(),
        );
        header.0.extend(token.0.iter().map(|b| u16::from(*b)));
        header.0.extend("\r\n".encode_utf16());
        WinHttpSendRequest(
            request.0,
            Some(&header.0),
            Some(body.as_ptr().cast()),
            body.len() as u32,
            body.len() as u32,
            0,
        )
        .map_err(|_| "WinHTTP send failed; outcome may be billable. No retry was made.")?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(
            |_| "WinHTTP response failed or timed out; outcome may be billable. No retry was made.",
        )?;
        let mut status: u32 = 0;
        let mut length = 4;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut length,
            std::ptr::null_mut(),
        )
        .map_err(|_| "Provider HTTP status unavailable.")?;
        if status != 200 {
            return Err(http_error(status));
        }
        let mut id = [0u16; 256];
        let mut id_length = std::mem::size_of_val(&id) as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_CUSTOM,
            w!("apim-request-id"),
            Some(id.as_mut_ptr().cast()),
            &mut id_length,
            std::ptr::null_mut(),
        )
        .map_err(|_| "Provider request ID unavailable; no retry was made.")?;
        let request_id =
            String::from_utf16(&id[..id.iter().position(|b| *b == 0).unwrap_or(id.len())])
                .map_err(|_| "Malformed provider request ID.")?;
        if !identifier(&request_id, 128) {
            return Err("Invalid provider request ID.".into());
        }
        let mut bytes = Vec::new();
        loop {
            let mut chunk = [0u8; 4096];
            let mut count = 0;
            WinHttpReadData(
                request.0,
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                &mut count,
            )
            .map_err(|_| "Provider response read failed; no retry was made.")?;
            if count == 0 {
                break;
            }
            if bytes.len() + count as usize > MAX_RESPONSE_BYTES {
                return Err("Provider response byte ceiling exceeded.".into());
            }
            bytes.extend_from_slice(&chunk[..count as usize]);
        }
        Ok((bytes, request_id))
    }
}

pub(super) fn execute(job: Job, cache: &mut AuthCache) -> Result<Completion> {
    let local_started = Instant::now();
    let body = job
        .prompt
        .as_ref()
        .map(|prompt| request(&job.config, Input::ExplicitPrompt(prompt), Route::Primary))
        .transpose()?;
    let prepared_us = local_started.elapsed().as_micros() as u64;
    let auth_started = Instant::now();
    job.config.validate()?;
    let mut stages = AuthTimings::default();
    let cached = cache.prepare(&job, || {
        let (verified, version) = verify(&job.config)?;
        stages = verified;
        let started = Instant::now();
        let token = token(&job.config)?;
        stages.token_ms = started.elapsed().as_millis() as u64;
        Ok((token, version))
    })?;
    let authentication_ms = auth_started.elapsed().as_millis() as u64;
    if let Some(body) = body {
        let (attempts, lease) = reserve(&job.config_path, job.config.limits.requests)?;
        let provider_started = Instant::now();
        let token = &cache
            .0
            .as_ref()
            .ok_or("Verified identity cache unavailable.")?
            .token
            .secret;
        let (bytes, id) = post(&job.config, token, &body).map_err(|error|
            format!("{error} Spending lease preserved; inspect ai-budget.lock before any further spending."))?;
        let provider_ms = provider_started.elapsed().as_millis() as u64;
        let local_started = Instant::now();
        let version = &cache
            .0
            .as_ref()
            .ok_or("Verified deployment version unavailable.")?
            .version;
        let parsed = parse_reply(&bytes, job.config.limits, &id, &job.config.model, version);
        let local_us = prepared_us + local_started.elapsed().as_micros() as u64;
        lease.release()?;
        let mut reply = parsed?;
        reply.attempts = attempts;
        reply.authentication_ms = authentication_ms;
        reply.authentication_cached = cached;
        reply.authentication_stages = stages;
        reply.provider_ms = provider_ms;
        reply.local_us = local_us;
        Ok(Completion::Answer(reply))
    } else {
        Ok(Completion::Connected(Connection {
            message: "Configured identity, tenant, subscription, endpoint and deployments verified. Inference permission is confirmed only by Send. Stronger route remains disabled.".into(),
            authentication_ms, cached, elapsed_ms: 0, authentication_stages: stages,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_winhttp_rejects_invalid_tls_redirect_and_delayed_local_response() {
        use std::net::TcpListener;
        for scenario in 0..3 {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = [0u8; 2048];
                let count = socket.read(&mut bytes).unwrap();
                if scenario == 0 {
                    assert!(count > 0);
                    assert_eq!(bytes[0], 0x16);
                }
                if scenario == 2 {
                    thread::sleep(Duration::from_secs(4));
                }
                let response = if scenario == 1 {
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
                };
                if let Err(error) = socket.write_all(response.as_bytes()) {
                    assert_eq!(scenario, 2, "unexpected fixture write failure: {error}");
                }
            });
            unsafe {
                let session = Internet::new(WinHttpOpen(
                    w!("TomasCommander/synthetic-test"),
                    WINHTTP_ACCESS_TYPE_NO_PROXY,
                    PCWSTR::null(),
                    PCWSTR::null(),
                    0,
                ))
                .unwrap();
                WinHttpSetTimeouts(session.0, 1000, 1000, 1000, 1000).unwrap();
                let connection =
                    Internet::new(WinHttpConnect(session.0, w!("127.0.0.1"), port, 0)).unwrap();
                let request = Internet::new(WinHttpOpenRequest(
                    connection.0,
                    w!("GET"),
                    w!("/synthetic-only"),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    std::ptr::null(),
                    if scenario == 0 {
                        WINHTTP_FLAG_SECURE
                    } else {
                        WINHTTP_OPEN_REQUEST_FLAGS::default()
                    },
                ))
                .unwrap();
                protect_request(&request, 1000).unwrap();
                let sent = WinHttpSendRequest(request.0, None, None, 0, 0, 0);
                if scenario == 0 {
                    assert!(sent.is_err(), "plaintext fixture cannot pass TLS");
                } else {
                    sent.unwrap();
                    let received = WinHttpReceiveResponse(request.0, std::ptr::null_mut());
                    if scenario == 2 {
                        assert!(
                            received.is_err(),
                            "delayed fixture must exceed receive timeout"
                        );
                    } else {
                        received.unwrap();
                        let mut status = 0u32;
                        let mut length = 4;
                        WinHttpQueryHeaders(
                            request.0,
                            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                            PCWSTR::null(),
                            Some((&mut status as *mut u32).cast()),
                            &mut length,
                            std::ptr::null_mut(),
                        )
                        .unwrap();
                        assert_eq!(status, 302, "redirect must not be followed");
                    }
                }
            }
            server.join().unwrap();
        }
    }
    #[test]
    fn cache_is_exact_expiring_rechecked_and_failure_invalidated() {
        let job = Job {
            config: Config::default(),
            config_path: PathBuf::from("synthetic-owned"),
            prompt: None,
            recheck: false,
        };
        let load = || {
            Ok((
                Token {
                    secret: Secret(b"synthetic.secret".to_vec()),
                    expires: unix_seconds()? + 3600,
                },
                "synthetic-version".into(),
            ))
        };
        let mut cache = AuthCache::default();
        assert!(!cache.prepare(&job, load).unwrap());
        assert!(
            cache
                .prepare(&job, || panic!("warm cache must not acquire credentials"))
                .unwrap()
        );
        for field in 0..11 {
            let mut changed = Job {
                config: job.config.clone(),
                config_path: job.config_path.clone(),
                prompt: None,
                recheck: false,
            };
            match field {
                0 => changed.config.identity = "different@example.org".into(),
                1 => changed.config.tenant = "different".into(),
                2 => changed.config.subscription = "different".into(),
                3 => changed.config.resource = "different".into(),
                4 => changed.config.resource_group = "different".into(),
                5 => changed.config.deployment = "different".into(),
                6 => changed.config.model = "different".into(),
                7 => changed.config.stronger_deployment = "different".into(),
                8 => changed.config.stronger_model = "different".into(),
                9 => changed.config.limits.timeout_seconds = 1,
                _ => changed.config_path = PathBuf::from("different"),
            }
            assert!(!cache.usable(&changed, unix_seconds().unwrap()));
        }
        let mut recheck = job;
        recheck.recheck = true;
        assert!(!cache.prepare(&recheck, load).unwrap());
        recheck.recheck = false;
        cache.0.as_mut().unwrap().verified = Instant::now() - SESSION_FRESHNESS;
        assert!(!cache.usable(&recheck, unix_seconds().unwrap()));
        assert!(!cache.prepare(&recheck, load).unwrap());
        cache.0.as_mut().unwrap().token.expires = unix_seconds().unwrap() + 120;
        assert!(!cache.usable(&recheck, unix_seconds().unwrap()));
        assert!(
            cache
                .prepare(&recheck, || Err("Synthetic auth failure.".into()))
                .is_err()
        );
        assert!(cache.0.is_none());
        assert!(!cache.prepare(&recheck, load).unwrap());
        cache.clear();
        assert!(cache.0.is_none());
    }
    #[test]
    fn identity_and_endpoint_mismatch_denied() {
        let c = Config {
            resource: "synthetic-foundry".into(),
            resource_group: "synthetic".into(),
            subscription: "00000000-0000-0000-0000-000000000001".into(),
            tenant: "00000000-0000-0000-0000-000000000002".into(),
            identity: "test@example.org".into(),
            deployment: "synthetic-luna".into(),
            model: "synthetic-model".into(),
            ..Config::default()
        };
        let account = json!({"id":c.subscription,"tenantId":c.tenant,"user":{"name":c.identity,"type":"user"},
            "environmentName":"AzureCloud","state":"Enabled"});
        assert!(validate_account(&c, &account).is_ok());
        for (pointer, wrong) in [
            ("/id", "different"),
            ("/tenantId", "different"),
            ("/user/name", "wrong@example.org"),
            ("/user/type", "servicePrincipal"),
            ("/environmentName", "AzureUSGovernment"),
            ("/state", "Disabled"),
        ] {
            let mut wrong_account = account.clone();
            *wrong_account.pointer_mut(pointer).unwrap() = json!(wrong);
            assert!(validate_account(&c, &wrong_account).is_err());
        }
        assert!(validate_account(&c, &json!({"user":{"name":"attacker@example.org"}})).is_err());
        assert!(
            validate_resource(&c, &json!({"properties":{"endpoint":"https://evil.test/"}}))
                .is_err()
        );
        let resource = json!({"kind":"AIServices","properties":{"endpoint":"https://synthetic-foundry.cognitiveservices.azure.com/"}});
        assert!(validate_resource(&c, &resource).is_ok());
    }
    #[test]
    fn unavailable_deployment_denied() {
        let valid = json!({"name":"luna","properties":{"provisioningState":"Succeeded","model":{"format":"OpenAI","name":"synthetic-model"}}});
        assert!(validate_deployment(&valid, "luna", "synthetic-model").is_ok());
        assert!(validate_deployment(&valid, "luna", "different-model").is_err());
        assert!(
            validate_deployment(
                &json!({"name":"luna","properties":{"provisioningState":"Failed"}}),
                "luna",
                "synthetic-model"
            )
            .is_err()
        );
    }
    #[test]
    fn budget_is_durable_and_exhaustion_blocks() {
        let root = std::env::temp_dir().join(format!(
            "tc-ai-owned-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("ai-connection.json");
        let (count, lease) = reserve(&path, 1).unwrap();
        assert_eq!(count, 1);
        assert!(reserve(&path, 1).is_err());
        lease.release().unwrap();
        assert!(reserve(&path, 1).is_err());
        fs::remove_file(root.join("ai-budget.json")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn interrupted_spending_lease_and_pending_transaction_survive_restart() {
        let root = std::env::temp_dir().join(format!(
            "tc-ai-restart-owned-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("ai-connection.json");
        let (count, lease) = reserve(&path, 5).unwrap();
        assert_eq!(count, 1);
        drop(lease);
        assert!(root.join("ai-budget.lock").is_file());
        assert!(reserve(&path, 5).is_err());
        assert_eq!(
            fs::read(root.join("ai-budget.json")).unwrap(),
            br#"{"attempts":1}"#
        );
        fs::remove_file(root.join("ai-budget.lock")).unwrap();
        fs::write(
            root.join("ai-budget.pending"),
            b"synthetic interrupted transaction",
        )
        .unwrap();
        assert!(reserve(&path, 5).is_err());
        assert_eq!(
            fs::read(root.join("ai-budget.json")).unwrap(),
            br#"{"attempts":1}"#
        );
        fs::remove_file(root.join("ai-budget.pending")).unwrap();
        fs::remove_file(root.join("ai-budget.json")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn expired_wrong_tenant_and_malformed_tokens_are_visible_without_secrets() {
        let c = Config::default();
        let value = json!({"accessToken":"synthetic.secret.token","tenant":"","subscription":"","expires_on":0});
        let error = parse_token(&serde_json::to_vec(&value).unwrap(), &c)
            .err()
            .unwrap();
        assert!(error.contains("expired"));
        assert!(!error.contains("synthetic.secret.token"));
        assert!(parse_token(b"malformed", &c).is_err());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut fresh = value;
        fresh["expires_on"] = json!(now + 3600);
        assert!(parse_token(&serde_json::to_vec(&fresh).unwrap(), &c).is_ok());
        fresh["tenant"] = json!("wrong-tenant");
        assert!(parse_token(&serde_json::to_vec(&fresh).unwrap(), &c).is_err());
        fresh["tenant"] = json!("");
        fresh["subscription"] = json!("wrong-subscription");
        assert!(parse_token(&serde_json::to_vec(&fresh).unwrap(), &c).is_err());
        fresh["subscription"] = json!("");
        for access in ["", "secret\r\nInjected: header"] {
            fresh["accessToken"] = json!(access);
            let error = parse_token(&serde_json::to_vec(&fresh).unwrap(), &c)
                .err()
                .unwrap();
            assert!(!error.contains(access) || access.is_empty());
        }
    }
}
