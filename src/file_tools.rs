use crate::{ai, files, search};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub const MAX_TOOL_BYTES: usize = 32_768;
const MAX_TEXT_BYTES: usize = 32 * 1024 * 1024;
const MAX_ITEMS: usize = 100_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Root {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub roots: Vec<Root>,
    pub connection: ai::Config,
    pub allow_content: bool,
    pub allow_cloud_download: bool,
}

impl Settings {
    pub fn path() -> ai::Result<PathBuf> {
        Ok(ai::config_path()?.with_file_name("ai-file-access.json"))
    }

    pub fn validate(&self) -> ai::Result<()> {
        self.connection.validate()?;
        if self.version != 1 || self.roots.is_empty() || self.roots.len() > 16 {
            return Err("AI file access needs 1-16 explicitly approved roots.".into());
        }
        let mut paths = BTreeSet::new();
        for root in &self.roots {
            if root.name.trim().is_empty()
                || root.name.len() > 128
                || !root.path.is_absolute()
                || !paths.insert(root.path.clone())
            {
                return Err("Invalid or duplicate AI root.".into());
            }
            validate_path_syntax(&root.path)?;
        }
        Ok(())
    }

    pub fn load(path: &Path) -> ai::Result<Self> {
        let settings: Self = serde_json::from_slice(&ai::read_bounded(path, 32_768)?)
            .map_err(|_| "Missing or malformed AI file-access settings; access is blocked.")?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn save(&self, path: &Path) -> ai::Result<()> {
        self.validate()?;
        if path.exists() {
            Self::load(path)?;
        }
        ai::persist(
            path,
            &serde_json::to_vec_pretty(self).map_err(|_| "Encode AI roots failed.")?,
        )
    }
}

fn validate_path_syntax(path: &Path) -> ai::Result<()> {
    let text = path.to_str().ok_or("AI paths must be Unicode.")?;
    if !path.is_absolute()
        || text.len() > 2048
        || text.contains(['\0', '\n', '\r'])
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        || text.get(3..).is_some_and(|rest| rest.contains(':')) && !text.starts_with(r"\\?\")
        || text.starts_with(r"\\.\")
    {
        return Err(
            "AI paths must be absolute filesystem paths without traversal or streams.".into(),
        );
    }
    #[cfg(windows)]
    {
        let normal = files::display_path(path);
        if normal.as_bytes().get(1) != Some(&b':')
            || normal.get(2..).is_some_and(|s| s.contains(':'))
        {
            return Err(
                "Only approved local drive paths are supported; devices/streams are blocked."
                    .into(),
            );
        }
    }
    Ok(())
}

fn protected(path: &Path) -> bool {
    path.components().any(|part| {
        let name = part.as_os_str().to_string_lossy().to_ascii_lowercase();
        matches!(
            name.as_str(),
            ".ssh"
                | ".azure"
                | ".aws"
                | ".kube"
                | ".gnupg"
                | ".copilot"
                | "credentials"
                | "credentials.json"
                | "secrets.json"
                | ".npmrc"
                | ".pypirc"
                | ".netrc"
                | "id_rsa"
                | "id_ed25519"
        ) || name == ".env"
            || name.starts_with(".env.")
            || [".pem", ".pfx", ".p12", ".key", ".kdbx"]
                .iter()
                .any(|suffix| name.ends_with(suffix))
    })
}

pub fn contains_secret(text: &str) -> bool {
    let folded = text.to_ascii_lowercase();
    [
        "-----begin ",
        "bearer ",
        "accesstoken",
        "refresh_token",
        "client_secret",
        "api_key",
        "api-key",
        "password=",
        "password:",
        "ghp_",
        "github_pat_",
        "accountkey=",
        "sharedaccesssignature=",
    ]
    .iter()
    .any(|marker| folded.contains(marker))
}

fn metadata_handle(path: &Path) -> ai::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Metadata-only access avoids hydration; deny replacement while inspecting.
        options
            .access_mode(0)
            .share_mode(3)
            .custom_flags(0x0220_0000);
    }
    options
        .open(path)
        .map_err(|e| format!("Inspect {}: {e}", files::display_path(path)))
}

#[cfg(windows)]
fn inspect_handle(file: &File) -> ai::Result<(PathBuf, bool)> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{
            FILE_ATTRIBUTE_TAG_INFO, FileAttributeTagInfo, GETFINALPATHNAMEBYHANDLE_FLAGS,
            GetFileInformationByHandleEx, GetFinalPathNameByHandleW,
        },
    };
    let handle = HANDLE(file.as_raw_handle());
    let mut attributes = FILE_ATTRIBUTE_TAG_INFO::default();
    unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileAttributeTagInfo,
            (&mut attributes as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of_val(&attributes) as u32,
        )
        .map_err(|_| "Cannot verify filesystem reparse tag; access blocked.")?;
    }
    let tag = attributes.ReparseTag;
    let cloud = tag & 0xffff_0fff == 0x9000_001a;
    if attributes.FileAttributes & 0x400 != 0 && !cloud {
        return Err("Links, junctions and unknown reparse points are blocked for AI.".into());
    }
    let mut buffer = vec![0u16; 32_768];
    let count = unsafe {
        GetFinalPathNameByHandleW(handle, &mut buffer, GETFINALPATHNAMEBYHANDLE_FLAGS(0))
    } as usize;
    if count == 0 || count >= buffer.len() {
        return Err("Cannot verify final handle path; access blocked.".into());
    }
    let path = String::from_utf16(&buffer[..count]).map_err(|_| "Invalid final handle path.")?;
    Ok((PathBuf::from(path), cloud))
}

#[cfg(not(windows))]
fn inspect_handle(_file: &File) -> ai::Result<(PathBuf, bool)> {
    Err("AI file tools require Windows handle-bound path verification.".into())
}

pub struct Tools {
    settings: Settings,
    pub observed: Vec<Observed>,
    pub errors: Vec<String>,
    pub examined: usize,
    pub skipped: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observed {
    pub path: PathBuf,
    pub root: PathBuf,
    pub directory: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub identity: (u32, u64),
    pub reason: String,
}

impl Tools {
    pub fn new(settings: Settings) -> ai::Result<Self> {
        settings.validate()?;
        Ok(Self {
            settings,
            observed: vec![],
            errors: vec![],
            examined: 0,
            skipped: 0,
            truncated: false,
        })
    }

    fn resolve(&self, path: &Path) -> ai::Result<(PathBuf, File, bool, Vec<File>)> {
        validate_path_syntax(path)?;
        if protected(path) {
            return Err(
                "Credential/sensitive configuration paths are excluded from AI reads.".into(),
            );
        }
        let display = files::display_path(path);
        let lexical = PathBuf::from(display);
        let root = self
            .settings
            .roots
            .iter()
            .filter(|root| lexical.starts_with(PathBuf::from(files::display_path(&root.path))))
            .max_by_key(|root| root.path.components().count())
            .ok_or("Path is outside the owner-approved AI roots.")?;
        let root_final =
            fs::canonicalize(&root.path).map_err(|e| format!("Approved root unavailable: {e}"))?;
        // Verify every named component, not just the final canonical destination.
        let mut prefix = PathBuf::new();
        let mut guards = Vec::new();
        for component in lexical.components() {
            prefix.push(component);
            if !matches!(component, Component::Normal(_)) {
                continue;
            }
            let guard = metadata_handle(&prefix)?;
            let (_, cloud) = inspect_handle(&guard)?;
            if cloud && !self.settings.allow_cloud_download {
                return Err("Cloud file access requires approved download permission.".into());
            }
            guards.push(guard);
        }
        let handle = metadata_handle(&lexical)?;
        let (final_path, cloud) = inspect_handle(&handle)?;
        if !final_path.starts_with(&root_final) {
            return Err("Final filesystem path escaped the approved AI root.".into());
        }
        if protected(&final_path) {
            return Err("Protected path cannot be read by AI.".into());
        }
        Ok((root_final, handle, cloud, guards))
    }

    pub fn observe(&mut self, path: &Path, reason: &str) -> ai::Result<Value> {
        let (root, handle, cloud, _parents) = self.resolve(path)?;
        let (path, _) = inspect_handle(&handle)?;
        let metadata = handle
            .metadata()
            .map_err(|e| format!("Read metadata: {e}"))?;
        #[cfg(windows)]
        let identity = crate::platform::file_identity(&handle)?;
        #[cfg(not(windows))]
        let identity = (0, 0);
        let modified = metadata
            .modified()
            .map_err(|e| format!("Read modification time: {e}"))?;
        let modified_unix_seconds = modified
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "Modification time is before Unix epoch.")?
            .as_secs();
        let record = Observed {
            path: path.clone(),
            root,
            directory: metadata.is_dir(),
            size: metadata.len(),
            modified: Some(modified),
            identity,
            reason: reason.into(),
        };
        let index = if let Some(index) = self.observed.iter().position(|old| {
            old.path == path
                && old.root == record.root
                && old.identity == record.identity
                && old.directory == record.directory
                && old.size == record.size
                && old.modified == record.modified
        }) {
            self.observed[index] = record;
            index
        } else {
            if self.observed.len() >= 1000 {
                return Err("Observed-result capacity reached; narrow the search.".into());
            }
            self.observed.push(record);
            self.observed.len() - 1
        };
        Ok(
            json!({"id":index+1,"path": files::display_path(&path), "directory": metadata.is_dir(),
            "size": metadata.len(), "modified_unix_seconds": modified_unix_seconds,
            "cloud_placeholder": cloud}),
        )
    }

    pub fn text(&self, path: &Path, stopped: &impl Fn() -> bool) -> ai::Result<String> {
        self.bound_text(path, stopped).map(|(text, _guards)| text)
    }

    fn bound_text(
        &self,
        path: &Path,
        stopped: &impl Fn() -> bool,
    ) -> ai::Result<(String, Vec<File>)> {
        if !self.settings.allow_content {
            return Err("Content reads are not approved.".into());
        }
        let (_, metadata, _, mut guards) = self.resolve(path)?;
        let (resolved, _) = inspect_handle(&metadata)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        if stopped() {
            return Err("Cancelled or time limit reached.".into());
        }
        let mut file = options
            .open(&resolved)
            .map_err(|e| format!("Open content (cloud download may be needed): {e}"))?;
        let (bound, _) = inspect_handle(&file)?;
        if bound != resolved {
            return Err("File path changed before reading.".into());
        }
        let before = file.metadata().map_err(|e| format!("Read metadata: {e}"))?;
        if !before.is_file() || before.len() > MAX_TEXT_BYTES as u64 {
            return Err("Content read supports regular files up to 32 MiB; oversized input is not silently truncated.".into());
        }
        let extension = resolved
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let text = if matches!(extension.as_str(), "docx" | "pptx") {
            office_text(&mut file, &extension, stopped)?
        } else {
            if !matches!(
                extension.as_str(),
                "txt"
                    | "md"
                    | "markdown"
                    | "csv"
                    | "json"
                    | "xml"
                    | "html"
                    | "htm"
                    | "rs"
                    | "py"
                    | "js"
                    | "ts"
                    | "tsx"
                    | "jsx"
                    | "css"
                    | "toml"
                    | "yaml"
                    | "yml"
                    | "log"
                    | "ini"
                    | "cfg"
                    | "sql"
                    | "ps1"
                    | "sh"
                    | "go"
                    | "c"
                    | "cpp"
                    | "h"
                    | "cs"
                    | "java"
                    | "r"
                    | "bat"
            ) {
                return Err(format!(
                    "Unsupported content format .{extension}; PDF/OCR, images and legacy Office are not text extraction."
                ));
            }
            let mut bytes = Vec::new();
            let mut block = [0u8; 16_384];
            loop {
                if stopped() {
                    return Err("Cancelled or time limit reached during read.".into());
                }
                let n = file
                    .read(&mut block)
                    .map_err(|e| format!("Read content: {e}"))?;
                if n == 0 {
                    break;
                }
                if bytes.len() + n > MAX_TEXT_BYTES {
                    return Err("Content exceeds 32 MiB.".into());
                }
                bytes.extend_from_slice(&block[..n]);
            }
            decode_text(&bytes)?
        };
        let after = file
            .metadata()
            .map_err(|e| format!("Recheck read metadata: {e}"))?;
        if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
            return Err("File changed during reading; content discarded.".into());
        }
        if contains_secret(&text) {
            return Err("Possible credential material blocked; no content sent.".into());
        }
        guards.push(metadata);
        guards.push(file);
        Ok((text, guards))
    }

    pub fn dispatch(
        &mut self,
        name: &str,
        arguments: &str,
        stopped: impl Fn() -> bool,
    ) -> ai::Result<Value> {
        if arguments.len() > 4096 {
            return Err("Tool arguments exceed 4096 bytes.".into());
        }
        if stopped() {
            return Err("Cancelled or time limit reached.".into());
        }
        match name {
            "list_directory" | "glob" | "grep" => {
                let args: WalkArgs = serde_json::from_str(arguments).map_err(|_| "Invalid traversal arguments.")?;
                args.validate()?;
                if name == "grep" && args.query.trim().is_empty() { return Err("Grep requires nonempty literal query.".into()); }
                let patterns = glob_patterns(&args.pattern.replace('\\', "/"))?;
                let query = search::fold(&args.query);
                let roots: Vec<PathBuf> = match &args.path {
                    Some(path) => vec![PathBuf::from(path)],
                    None => self.settings.roots.iter().map(|root| root.path.clone()).collect(),
                };
                let mut stack: Vec<_> = roots.into_iter().map(|root| (root.clone(), root)).collect();
                let mut values = Vec::new();
                let mut visited = 0usize;
                let mut truncated = false;
                let started = Instant::now();
                while let Some((directory, root)) = stack.pop() {
                    if stopped() || started.elapsed().as_secs() >= 15 { truncated = true; break; }
                    let (_, _guard, _, _parents) = match self.resolve(&directory) {
                        Ok(value) => value,
                        Err(error) => { self.record_error(error); continue; }
                    };
                    let entries = match fs::read_dir(&directory) {
                        Ok(entries) => entries,
                        Err(error) => { self.record_error(format!("List {}: {error}", files::display_path(&directory))); continue; }
                    };
                    for entry in entries {
                        if stopped() || visited >= MAX_ITEMS || values.len() >= args.limit
                            || started.elapsed().as_secs() >= 15 { truncated = true; break; }
                        visited += 1;
                        let entry = match entry {
                            Ok(entry) => entry,
                            Err(error) => { self.record_error(format!("Directory item: {error}")); continue; }
                        };
                        let path = entry.path();
                        if protected(&path) { self.skipped += 1; continue; }
                        let candidate = if args.pattern.contains(['/', '\\']) {
                            path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().replace('\\', "/")
                        } else {
                            path.file_name().ok_or("Missing filename.")?.to_string_lossy().into_owned()
                        };
                        let matches = (name != "glob" || query.is_empty()
                            || search::fold(&path.to_string_lossy()).contains(&query))
                            && patterns.iter().any(|pattern| glob_single_matches(pattern, &candidate));
                        if !matches {
                            if args.recursive {
                                match entry.file_type() {
                                    Ok(kind) if kind.is_dir() => stack.push((path, root.clone())),
                                    Ok(_) => (),
                                    Err(error) => self.record_error(format!("Directory item type: {error}")),
                                }
                            }
                            continue;
                        }
                        let previous_count = self.observed.len();
                        let metadata = match self.observe(&path, name) {
                            Ok(metadata) => metadata,
                            Err(error) => { self.skipped += 1; self.record_error(error); continue; }
                        };
                        let is_directory = metadata["directory"] == true;
                        if is_directory && args.recursive { stack.push((path.clone(), root.clone())); }
                        if name == "grep" {
                            if is_directory { self.observed.truncate(previous_count); continue; }
                            let before_matches = values.len();
                            match self.bound_text(&path, &stopped) {
                                Ok((text, _guards)) => {
                                    if let Some((line, matched)) = text.lines().enumerate()
                                        .find(|(_, line)| search::fold(line).contains(&search::fold(&args.query)))
                                    {
                                        let metadata = self.observe(&path, "content match")?;
                                        values.push(json!({"file":metadata,"line":line+1,
                                            "excerpt":matched.chars().take(400).collect::<String>()}));
                                    }
                                }
                                Err(error) => self.record_error(format!("{}: {error}", files::display_path(&path))),
                            }
                            if values.len() == before_matches { self.observed.truncate(previous_count); }
                        } else { values.push(metadata); }
                        if serde_json::to_vec(&values).map_err(|_| "Tool result encoding failed.")?.len() > MAX_TOOL_BYTES - 4096 {
                            values.pop();
                            self.observed.truncate(previous_count);
                            truncated = true;
                            break;
                        }
                    }
                    if truncated { break; }
                    if !args.recursive { break; }
                }
                self.examined += visited;
                self.truncated |= truncated;
                Ok(json!({"items":values,"examined":visited,"truncated":truncated,
                    "errors":self.errors.iter().rev().take(8).collect::<Vec<_>>(),"skipped":self.skipped}))
            }
            "read_file" => {
                let args: ReadArgs = serde_json::from_str(arguments).map_err(|_| "Invalid read_file arguments.")?;
                if args.length == 0 || args.length > 6000 { return Err("Read length must be 1-6000 characters.".into()); }
                if args.offset > 0 && args.version_id.is_none() {
                    return Err("Paged reads require the file.id returned by the first page as version_id.".into());
                }
                let (text, _guards) = self.bound_text(Path::new(&args.path), &stopped)?;
                let total = text.chars().count();
                if args.offset > total { return Err("Read offset exceeds document length.".into()); }
                let fragment: String = text.chars().skip(args.offset).take(args.length).collect();
                let next = args.offset + fragment.chars().count();
                let metadata = self.observe(Path::new(&args.path), "content read")?;
                if args.version_id.is_some_and(|id| metadata["id"].as_u64() != Some(id as u64)) {
                    return Err("File changed since the requested version; restart reading at offset 0.".into());
                }
                Ok(json!({"file":metadata,"text":fragment,"offset":args.offset,"next_offset":next,
                    "total_characters":total,"eof":next==total}))
            }
            _ => Err("Unknown tool blocked; only list_directory, glob, grep and read_file are available.".into()),
        }
    }

    fn record_error(&mut self, error: String) {
        if self.errors.len() < 32 && !self.errors.contains(&error) {
            self.errors.push(error);
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WalkArgs {
    #[serde(deserialize_with = "Option::deserialize")]
    path: Option<String>,
    pattern: String,
    query: String,
    recursive: bool,
    limit: usize,
}
impl WalkArgs {
    fn validate(&self) -> ai::Result<()> {
        if self.pattern.is_empty()
            || self.pattern.len() > 256
            || self.query.len() > 256
            || self.limit == 0
            || self.limit > 100
        {
            return Err("Traversal needs a bounded pattern/query and 1-100 results.".into());
        }
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    path: String,
    offset: usize,
    length: usize,
    version_id: Option<usize>,
}

fn decode_text(bytes: &[u8]) -> ai::Result<String> {
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        if !bytes.len().is_multiple_of(2) {
            return Err("Invalid UTF-16 byte length.".into());
        }
        let little = bytes[0] == 0xff;
        let units: Vec<_> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                if little {
                    u16::from_le_bytes([pair[0], pair[1]])
                } else {
                    u16::from_be_bytes([pair[0], pair[1]])
                }
            })
            .collect();
        String::from_utf16(&units).map_err(|_| "Invalid UTF-16 document.".into())
    } else {
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        let text =
            std::str::from_utf8(bytes).map_err(|_| "Text must be UTF-8 or BOM-marked UTF-16.")?;
        if text.contains('\0') {
            return Err("Binary content is not readable text.".into());
        }
        Ok(text.into())
    }
}

fn office_text(
    file: &mut File,
    extension: &str,
    stopped: &impl Fn() -> bool,
) -> ai::Result<String> {
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Read Office package: {e}"))?;
    if archive.len() > 10_000 {
        return Err("Office package exceeds entry limit.".into());
    }
    let mut members = Vec::new();
    for index in 0..archive.len() {
        let member = archive
            .by_index(index)
            .map_err(|e| format!("Inspect Office member: {e}"))?;
        let name = member.name();
        if extension == "docx" && name == "word/document.xml" {
            members.push((0, index));
        } else if extension == "pptx"
            && name.starts_with("ppt/slides/slide")
            && name.ends_with(".xml")
            && let Ok(number) = name["ppt/slides/slide".len()..name.len() - 4].parse::<u32>()
        {
            members.push((number, index));
        }
    }
    members.sort();
    if members.is_empty() {
        return Err("Office package has no supported document text parts.".into());
    }
    let mut text = String::new();
    for (page, index) in members {
        if stopped() {
            return Err("Cancelled during Office extraction.".into());
        }
        let mut member = archive
            .by_index(index)
            .map_err(|e| format!("Read Office member: {e}"))?;
        if member.size() > 4 * 1024 * 1024 {
            return Err("Office XML part exceeds 4 MiB.".into());
        }
        let mut bytes = Vec::new();
        (&mut member)
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("Extract Office XML: {e}"))?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("Office XML exceeds limit.".into());
        }
        let xml = decode_text(&bytes)?;
        if xml.contains("<!DOCTYPE") || xml.contains("<!ENTITY") {
            return Err("Office XML entities are not supported.".into());
        }
        let document = scraper::Html::parse_fragment(&xml);
        let selector = scraper::Selector::parse("w\\:t, a\\:t")
            .map_err(|_| "Internal Office selector failed.")?;
        text.push_str(&format!("\n[part {page}]\n"));
        for element in document.select(&selector) {
            for fragment in element.text() {
                text.push_str(fragment);
            }
            text.push('\n');
        }
        if text.len() > MAX_TEXT_BYTES {
            return Err("Extracted Office text exceeds 32 MiB.".into());
        }
    }
    Ok(text)
}

pub fn glob_matches(pattern: &str, text: &str) -> bool {
    glob_patterns(pattern).is_ok_and(|patterns| {
        patterns
            .iter()
            .any(|pattern| glob_single_matches(pattern, text))
    })
}

fn glob_patterns(pattern: &str) -> ai::Result<Vec<String>> {
    let Some(start) = pattern.find('{') else {
        if pattern.contains('}') {
            return Err("Unmatched glob brace.".into());
        }
        return Ok(vec![pattern.to_owned()]);
    };
    let end = pattern[start..]
        .find('}')
        .map(|end| start + end)
        .ok_or("Unmatched glob brace.")?;
    let options: Vec<_> = pattern[start + 1..end].split(',').collect();
    if pattern[..start].contains('}')
        || pattern[end + 1..].contains(['{', '}'])
        || options.len() > 16
        || options
            .iter()
            .any(|option| option.is_empty() || option.contains('{'))
    {
        return Err("Glob supports one brace group with 1-16 nonempty alternatives.".into());
    }
    Ok(options
        .into_iter()
        .map(|option| format!("{}{option}{}", &pattern[..start], &pattern[end + 1..]))
        .collect())
}

fn glob_single_matches(pattern: &str, text: &str) -> bool {
    let pattern: Vec<_> = search::fold(pattern).chars().collect();
    let text: Vec<_> = search::fold(text).chars().collect();
    let mut previous = vec![false; text.len() + 1];
    previous[0] = true;
    let mut directory_skip = None;
    for (index, p) in pattern.iter().enumerate() {
        if *p == '*'
            && (index == 0 || pattern[index - 1] == '/')
            && pattern.get(index + 1) == Some(&'*')
        {
            let end = index
                + pattern[index..]
                    .iter()
                    .take_while(|character| **character == '*')
                    .count();
            if pattern.get(end) == Some(&'/') {
                directory_skip = Some((end, previous.clone()));
            }
        }
        let mut next = vec![false; text.len() + 1];
        if *p == '*' {
            next[0] = previous[0];
        }
        let cross = *p == '*'
            && (pattern.get(index.wrapping_sub(1)) == Some(&'*')
                || pattern.get(index + 1) == Some(&'*'));
        for j in 1..=text.len() {
            next[j] = if *p == '*' {
                previous[j] || next[j - 1] && (cross || text[j - 1] != '/')
            } else {
                previous[j - 1] && (*p == text[j - 1] || *p == '?' && text[j - 1] != '/')
            };
        }
        if directory_skip
            .as_ref()
            .is_some_and(|(separator, _)| *separator == index)
        {
            // A whole **/ component also matches zero directories.
            let (_, skip) = directory_skip.take().expect("Checked directory component");
            for (matched, skipped) in next.iter_mut().zip(skip) {
                *matched |= skipped;
            }
        }
        previous = next;
    }
    previous[text.len()]
}

pub fn tool_definitions() -> Vec<Value> {
    let walk = json!({"type":"object","properties":{
        "path":{"type":["string","null"],"description":"Absolute directory inside approved roots, or null to search all approved roots in one call"},
        "pattern":{"type":"string","description":"Case/diacritic-insensitive glob. * or *.pptx matches basenames; ** crosses directories. One brace group batches types, e.g. *.{pptx,pdf,html,htm}"},
        "query":{"type":"string","description":"For grep: literal content query. For glob: optional literal full-path/name filter, e.g. an organization or project. Empty for no filter or listing"},
        "recursive":{"type":"boolean"},"limit":{"type":"integer","minimum":1,"maximum":100}},
        "required":["path","pattern","query","recursive","limit"],"additionalProperties":false});
    ["list_directory","glob","grep"].into_iter().map(|name| json!({"type":"function","name":name,
        "description":match name { "list_directory"=>"List real files and folders. Use recursive=false to inspect a directory.",
            "glob"=>"Find actual paths matching a glob recursively.",_=>"Search literal text in supported documents; returns matching excerpt and real path." },
        "parameters":walk,"strict":true})).chain(std::iter::once(json!({"type":"function","name":"read_file",
        "description":"Read UTF-8/UTF-16 text or extracted DOCX/PPTX text in pages of characters. Use version_id=null at offset 0, then returned file.id as version_id with next_offset for later pages. Changed files require restart; other binary formats explicitly fail.",
        "parameters":{"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"integer","minimum":0},
            "length":{"type":"integer","minimum":1,"maximum":6000},
            "version_id":{"type":["integer","null"],"minimum":1,"maximum":1000}},"required":["path","offset","length","version_id"],
            "additionalProperties":false},"strict":true}))).collect()
}

#[cfg(all(test, windows))]
pub(crate) mod tests {
    use super::*;
    use std::{
        io::Write,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    pub(crate) struct Fixture {
        pub root: PathBuf,
        pub settings: Settings,
    }
    impl Fixture {
        pub fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "tc-file-agent-owned-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            let root = fs::canonicalize(root).unwrap();
            let connection = ai::Config {
                resource: "synthetic-foundry".into(),
                resource_group: "synthetic".into(),
                subscription: "00000000-0000-0000-0000-000000000001".into(),
                tenant: "00000000-0000-0000-0000-000000000002".into(),
                identity: "test@example.org".into(),
                deployment: "synthetic-luna".into(),
                model: "synthetic-luna".into(),
                ..Default::default()
            };
            let settings = Settings {
                version: 1,
                roots: vec![Root {
                    name: "Synthetic only".into(),
                    path: root.clone(),
                }],
                connection,
                allow_content: true,
                allow_cloud_download: true,
            };
            Self { root, settings }
        }
        pub fn tools(&self) -> Tools {
            Tools::new(self.settings.clone()).unwrap()
        }
        pub fn write(&self, name: &str, text: &str) -> PathBuf {
            let path = self.root.join(name);
            fs::write(&path, text).unwrap();
            path
        }
        pub fn office(&self, name: &str, member: &str, xml: &str) -> PathBuf {
            let path = self.root.join(name);
            let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
            writer
                .start_file(member, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(xml.as_bytes()).unwrap();
            writer.finish().unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    fn persisted_roots_are_separate_and_provider_bound() {
        let fixture = Fixture::new();
        let path = fixture.root.join("access.json");
        fixture.settings.save(&path).unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert!(loaded == fixture.settings);
        assert_eq!(loaded.roots.len(), 1);
        fs::write(
            fixture.root.join("preferences.txt"),
            "favorite=C:\\unapproved",
        )
        .unwrap();
        assert!(Settings::load(&path).unwrap() == fixture.settings);
        let mut invalid = loaded;
        invalid.roots.clear();
        assert!(invalid.save(&path).is_err());
        assert!(Settings::load(&path).unwrap() == fixture.settings);
    }

    #[test]
    fn unsupported_metadata_does_not_poison_partial_result_serialization() {
        let fixture = Fixture::new();
        let path = fixture.write("historic.txt", "Synthetic historical timestamp.");
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(UNIX_EPOCH - std::time::Duration::from_secs(86400)),
            )
            .unwrap();
        let mut tools = fixture.tools();
        assert!(
            tools
                .observe(&path, "synthetic")
                .unwrap_err()
                .contains("epoch")
        );
        assert!(tools.observed.is_empty());
        assert!(serde_json::to_vec(&tools.observed).is_ok());
    }

    #[test]
    fn bound_content_keeps_file_and_ancestors_stable_through_observation() {
        let fixture = Fixture::new();
        let directory = fixture.root.join("nested");
        fs::create_dir(&directory).unwrap();
        let path = fixture.write("nested\\bound.txt", "Synthetic bound contents.");
        let mut tools = fixture.tools();
        let (text, guards) = tools.bound_text(&path, &|| false).unwrap();
        assert!(fs::write(&path, "Changed").is_err());
        assert!(fs::rename(&path, directory.join("renamed.txt")).is_err());
        assert!(fs::rename(&directory, fixture.root.join("renamed")).is_err());
        let metadata = tools.observe(&path, "bound read").unwrap();
        assert_eq!(text, "Synthetic bound contents.");
        assert_eq!(metadata["size"], text.len());
        assert!(crate::file_agent::locate(&fixture.settings, &tools.observed[0]).is_ok());
        drop(guards);
        fs::write(&path, "Changed").unwrap();
        assert!(crate::file_agent::locate(&fixture.settings, &tools.observed[0]).is_err());
    }

    #[test]
    fn changed_observations_keep_old_ids_and_paged_reads_reject_mixed_versions() {
        let fixture = Fixture::new();
        let path = fixture.write("paged.txt", "Original document content.");
        let mut tools = fixture.tools();
        let first = tools
            .dispatch(
                "read_file",
                &json!({"path":path,"offset":0,"length":5,"version_id":null}).to_string(),
                || false,
            )
            .unwrap();
        let original = tools.observed[0].clone();
        fs::write(&path, "Changed document with a different size.").unwrap();
        let error = tools
            .dispatch(
                "read_file",
                &json!({"path":path,"offset":5,"length":5,"version_id":first["file"]["id"]})
                    .to_string(),
                || false,
            )
            .unwrap_err();
        assert!(error.contains("changed"));
        assert_eq!(tools.observed.len(), 2);
        assert_eq!(tools.observed[0].size, original.size);
        assert_eq!(tools.observed[0].modified, original.modified);
        assert!(crate::file_agent::locate(&fixture.settings, &original).is_err());
        let restarted = tools
            .dispatch(
                "read_file",
                &json!({"path":path,"offset":0,"length":5,"version_id":null}).to_string(),
                || false,
            )
            .unwrap();
        assert_ne!(first["file"]["id"], restarted["file"]["id"]);
        assert_eq!(tools.observed.len(), 2);
        assert_eq!(restarted["text"], "Chang");
        let next = tools
            .dispatch(
                "read_file",
                &json!({"path":path,"offset":5,"length":5,"version_id":restarted["file"]["id"]})
                    .to_string(),
                || false,
            )
            .unwrap();
        assert_eq!(next["text"], "ed do");
        assert!(
            tools
                .dispatch(
                    "read_file",
                    &json!({"path":path,"offset":5,"length":5}).to_string(),
                    || false
                )
                .unwrap_err()
                .contains("version_id")
        );
    }

    #[test]
    fn scoped_tools_discover_grep_and_read_full_unicode_in_pages() {
        let fixture = Fixture::new();
        fixture.write(
            "Penta-hackathon.txt",
            "Dovolena v Srbsku. Penta hackathon pro Kosik.",
        );
        fixture.write("unrelated.txt", "Nothing here.");
        let mut tools = fixture.tools();
        let args =
            json!({"path":fixture.root,"pattern":"*penta*","query":"","recursive":true,"limit":20})
                .to_string();
        let result = tools.dispatch("glob", &args, || false).unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), 1);
        assert_eq!(tools.observed.len(), 1);
        let args=json!({"path":fixture.root,"pattern":"*.txt","query":"dovolena","recursive":true,"limit":20}).to_string();
        let result = tools.dispatch("grep", &args, || false).unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), 1);
        assert!(
            result["items"][0]["excerpt"]
                .as_str()
                .unwrap()
                .contains("Srbs")
        );
        let text = "abc\u{10d}".repeat(5000);
        let path = fixture.write("large.txt", &text);
        let mut offset = 0;
        let mut read = String::new();
        let mut version_id = None;
        loop {
            let result = tools
                .dispatch(
                    "read_file",
                    &json!({"path":path,"offset":offset,"length":6000,"version_id":version_id})
                        .to_string(),
                    || false,
                )
                .unwrap();
            read.push_str(result["text"].as_str().unwrap());
            version_id = result["file"]["id"].as_u64();
            offset = result["next_offset"].as_u64().unwrap() as usize;
            if result["eof"] == true {
                break;
            }
        }
        assert_eq!(read, text);
        assert_eq!(offset, 20_000);
    }

    #[test]
    fn candidate_glob_batches_roots_types_and_paths_before_opening_unrelated_files() {
        let fixture = Fixture::new();
        let second = Fixture::new();
        fs::create_dir(fixture.root.join("PPF")).unwrap();
        let first = fixture.write("PPF\\intro.html", "<h1>Hackathon introduction</h1>");
        fs::create_dir(second.root.join("PPF")).unwrap();
        let other = second.write("PPF\\overview.pptx", "Filename-only fixture");
        let unrelated = fixture.write("ignored.txt", "Do not inspect this metadata.");
        File::options()
            .write(true)
            .open(&unrelated)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(UNIX_EPOCH - std::time::Duration::from_secs(86400)),
            )
            .unwrap();
        let mut settings = fixture.settings.clone();
        settings.roots.push(second.settings.roots[0].clone());
        let mut tools = Tools::new(settings).unwrap();
        let result = tools.dispatch("glob",
            &json!({"path":null,"pattern":"*.{html,pdf,pptx}","query":"ppf","recursive":true,"limit":20}).to_string(),
            || false).unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), 2);
        assert!(tools.errors.is_empty());
        assert_eq!(tools.skipped, 0);
        assert_eq!(tools.observed.len(), 2);
        assert!(tools.observed.iter().any(|record| record.path == first));
        assert!(tools.observed.iter().any(|record| record.path == other));
        assert!(tools.dispatch("glob",
            &json!({"path":null,"pattern":"*.{html,}","query":"","recursive":true,"limit":20}).to_string(),
            || false).unwrap_err().contains("brace"));
        assert!(
            tools
                .dispatch(
                    "glob",
                    &json!({"pattern":"*","query":"","recursive":true,"limit":20}).to_string(),
                    || false
                )
                .unwrap_err()
                .contains("arguments")
        );
    }

    #[test]
    fn content_boundaries_reject_secrets_unknown_arguments_and_unsupported_formats() {
        let fixture = Fixture::new();
        let secret = fixture.write("secret.txt", "password=synthetic-do-not-send");
        let env = fixture.write(".env", "synthetic");
        let pdf = fixture.write("test.pdf", "Not an actual PDF");
        let mut tools = fixture.tools();
        for path in [secret, env, pdf] {
            let result = tools.dispatch(
                "read_file",
                &json!({"path":path,"offset":0,"length":100}).to_string(),
                || false,
            );
            assert!(result.is_err());
        }
        assert!(
            tools
                .dispatch("shell", r#"{"command":"delete"}"#, || false)
                .is_err()
        );
        assert!(
            tools
                .dispatch(
                    "read_file",
                    &json!({"path":fixture.root,"offset":0,"length":100,"extra":"effect"})
                        .to_string(),
                    || false
                )
                .is_err()
        );
        assert!(tools.dispatch("glob",&json!({"path":fixture.root,"pattern":"*","query":"","recursive":true,"limit":100}).to_string(),||true).is_err());
        assert!(
            tools
                .observe(&fixture.root.join("..").join("outside"), "bad")
                .is_err()
        );
    }

    #[test]
    fn actual_junction_cannot_escape_the_allowlist_and_activation_detects_changes() {
        let fixture = Fixture::new();
        let outside = Fixture::new();
        let path = outside.write("outside.txt", "PRIVATE_SYNTHETIC_MARKER");
        let link = fixture.root.join("escape");
        let status = std::process::Command::new(std::env::var_os("ComSpec").unwrap())
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&outside.root)
            .output()
            .unwrap();
        assert!(status.status.success());
        assert!(
            fixture
                .tools()
                .text(&link.join("outside.txt"), &|| false)
                .is_err()
        );
        assert!(fixture.tools().observe(&path, "outside").is_err());
        fs::remove_dir(&link).unwrap();
        let local = fixture.write("local.txt", "old");
        let mut tools = fixture.tools();
        tools.observe(&local, "found").unwrap();
        let record = tools.observed[0].clone();
        assert!(crate::file_agent::locate(&fixture.settings, &record).is_ok());
        fs::write(&local, "changed content").unwrap();
        assert!(crate::file_agent::locate(&fixture.settings, &record).is_err());
    }

    #[test]
    fn office_text_is_extracted_without_external_relationships_or_execution() {
        let fixture = Fixture::new();
        let doc=fixture.office("presentation.docx","word/document.xml",
            r#"<w:document xmlns:w="urn:test"><w:p><w:r><w:t>Penta &amp; Kosik</w:t></w:r></w:p></w:document>"#);
        let ppt=fixture.office("presentation.pptx","ppt/slides/slide1.xml",
            r#"<p:sld xmlns:p="urn:p" xmlns:a="urn:a"><a:p><a:r><a:t>Srbsko hackathon</a:t></a:r></a:p></p:sld>"#);
        let tools = fixture.tools();
        assert!(
            tools
                .text(&doc, &|| false)
                .unwrap()
                .contains("Penta & Kosik")
        );
        assert!(
            tools
                .text(&ppt, &|| false)
                .unwrap()
                .contains("Srbsko hackathon")
        );
    }

    #[test]
    fn globs_are_case_diacritic_aware_and_distinguish_directory_separators() {
        assert!(glob_matches("*dovolena*", "DOVOLEN\u{e1}.txt"));
        assert!(glob_matches("**/*.txt", "one/two/file.txt"));
        assert!(glob_matches("**/*.txt", "file.txt"));
        assert!(glob_matches("one/**/file.txt", "one/file.txt"));
        assert!(glob_matches("one/**/file.txt", "one/two/file.txt"));
        assert!(glob_matches("**/one/**/file.txt", "one/file.txt"));
        assert!(!glob_matches("**a**/file.txt", "file.txt"));
        assert!(!glob_matches("*.txt", "one/file.txt"));
        assert!(glob_matches("a?.txt", "ab.txt"));
        assert!(glob_matches("**/*.{pptx,pdf,html}", "PPF/intro.HTML"));
        assert!(glob_matches("*.{pptx,pdf,html}", "intro.pptx"));
        assert!(!glob_matches("*.{pptx,pdf,html}", "notes.txt"));
    }
}
