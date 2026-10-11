use super::*;
use ::windows::{
    Win32::Security::Credentials::{
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredFree, CredReadW, CredWriteW,
    },
    core::{PCWSTR, PWSTR},
};

pub(super) fn save_key(key: &Credential) -> Result<()> {
    let mut target: Vec<u16> = Credential::VAULT_TARGET
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut user: Vec<u16> = "TomasCommander WebIQ"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let value = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_mut_ptr()),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: PWSTR(user.as_mut_ptr()),
        CredentialBlobSize: key.0.len() as u32,
        CredentialBlob: key.0.as_ptr().cast_mut(),
        ..CREDENTIALW::default()
    };
    unsafe { CredWriteW(&value, 0) }.map_err(|_| {
        "Windows could not save the app-specific WebIQ credential; raw errors suppressed.".into()
    })
}

struct VaultRecord(*mut CREDENTIALW);

impl Drop for VaultRecord {
    fn drop(&mut self) {
        unsafe {
            let record = &*self.0;
            if !record.CredentialBlob.is_null() && record.CredentialBlobSize <= 4096 {
                for byte in std::slice::from_raw_parts_mut(
                    record.CredentialBlob,
                    record.CredentialBlobSize as usize,
                ) {
                    std::ptr::write_volatile(byte, 0);
                }
            }
            CredFree(self.0.cast());
        }
    }
}

pub(super) fn load_key() -> Result<Credential> {
    let target: Vec<u16> = Credential::VAULT_TARGET
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut record = std::ptr::null_mut();
    unsafe { CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut record) }
        .map_err(|_| "No accessible app-specific WebIQ credential. Enter/save your existing key locally; no host credential store was inspected.".to_string())?;
    if record.is_null() {
        return Err("Windows returned no credential record.".into());
    }
    let record = VaultRecord(record);
    let value = unsafe { &*record.0 };
    if value.Type != CRED_TYPE_GENERIC
        || value.CredentialBlob.is_null()
        || value.CredentialBlobSize == 0
        || value.CredentialBlobSize > 4096
    {
        return Err("Invalid app-specific vault credential shape; record preserved.".into());
    }
    let bytes = unsafe {
        std::slice::from_raw_parts(value.CredentialBlob, value.CredentialBlobSize as usize)
    };
    let key =
        std::str::from_utf8(bytes).map_err(|_| "Invalid app-specific credential encoding.")?;
    Credential::api_key(key.to_owned())
}

struct WebTransport {
    credential: Credential,
    state: PathBuf,
    cumulative: u32,
    started: Instant,
    network_us: u64,
}

impl Transport for WebTransport {
    fn post(
        &mut self,
        message: &Value,
        session: Option<&str>,
        initialized: bool,
    ) -> Result<WireResponse> {
        if self.started.elapsed() >= Duration::from_secs(30) {
            return Err("Source overall deadline exceeded before next step.".into());
        }
        self.credential.validate()?;
        let bytes = serde_json::to_vec(message).map_err(|_| "MCP request encoding failed.")?;
        let (count, lease) = ai::reserve_counter(
            self.state.parent().ok_or("Source state parent missing.")?,
            "answer-budget.json",
            "answer-budget.lock",
            |_| Ok(MAX_LIFETIME_POSTS),
        )?;
        self.cumulative = count;
        let started = Instant::now();
        let response = ai::windows::post_web_mcp(&self.credential.0, &bytes, session, initialized)?;
        self.network_us += started.elapsed().as_micros() as u64;
        lease.release()?;
        Ok(response)
    }
}

pub(super) fn execute(job: Job) -> Result<Retrieved> {
    job.config.validate()?;
    validate_question(&job.question)?;
    job.credential.validate()?;
    let ai_path = job.state.with_file_name("ai-connection.json");
    if ai::Config::load(&ai_path)? != job.ai_config {
        return Err(
            "Foundry configuration changed after ANSWER confirmation; no source read.".into(),
        );
    }
    ai::validate_answer_authority(&job.ai_config, &ai_path, &job.question)?;
    let (submission_count, submission) = ai::reserve_counter(
        job.state
            .parent()
            .ok_or("Source state directory missing.")?,
        "answer-submissions.json",
        "answer-submissions.lock",
        |count| {
            if count >= 2 || PILOT_QUESTIONS[count as usize] != job.question {
                return Err(
                    "Only the next once-only approved source question is allowed; no retry.".into(),
                );
            }
            Ok(2)
        },
    )?;
    let started = Instant::now();
    let mut client = Client::new(WebTransport {
        credential: job.credential,
        state: job.state,
        cumulative: 0,
        started,
        network_us: 0,
    });
    let passages = client.web(&job.question)?;
    let result = Retrieved {
        source: Source::PublicWeb,
        passages,
        posts: client.posts,
        cumulative_posts: client.transport.cumulative,
        source_ms: client.transport.network_us / 1000,
        local_us: (started.elapsed().as_micros() as u64)
            .saturating_sub(client.transport.network_us),
    };
    result.validate()?;
    let source_path = client
        .transport
        .state
        .with_file_name(format!("answer-source-{submission_count}.json"));
    if source_path.exists() {
        return Err("Existing source receipt preserved; no repeated retrieval.".into());
    }
    let context = PublicContext {
        question: job.question,
        retrieved: result,
    };
    context.validate()?;
    ai::persist(
        &source_path,
        &serde_json::to_vec_pretty(&context)
            .map_err(|_| "Public source receipt encoding failed.")?,
    )?;
    submission.release()?;
    Ok(context.retrieved)
}
