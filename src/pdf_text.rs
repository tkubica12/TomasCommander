//! Read-only PDF text extraction for the AI file harness.
//!
//! The text layer is parsed with `lopdf` on a contained helper thread
//! (panic-contained, time-bounded, decompression-bounded). Pages without a
//! usable text layer may be recognized by a bounded OCR seam. Nothing is
//! executed: JavaScript, forms, annotations and attachments are never read.

use lopdf::{Dictionary, Document, Encoding, LoadOptions, Object, ObjectId, content::Content};
use std::{
    collections::HashMap,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

mod cmap;
use cmap::Decoded;

type Result<T> = std::result::Result<T, String>;

pub(crate) const MAX_PAGES: usize = 500;
const STREAM_LIMIT: usize = 16 * 1024 * 1024;
const PAGE_CONTENT_LIMIT: usize = 32 * 1024 * 1024;
const MAX_OPERATIONS: usize = 2_000_000;
const MAX_FORM_DEPTH: usize = 4;
const MAX_FORMS: usize = 1_000;
/// Distinct font dictionaries decoded per document; further fonts are undecodable.
const MAX_FONTS: usize = 4_096;
/// ToUnicode mapping entries kept across all CMaps of one document.
const MAX_DOCUMENT_CMAP_ENTRIES: usize = 1 << 20;
const MIN_TEXT_CHARACTERS: usize = 16;
pub(crate) const ENCRYPTED: &str =
    "Encrypted PDF is not readable without its password; only names and metadata are available.";
const CANCELLED: &str = "Cancelled or time limit reached during PDF extraction.";
const TOO_LARGE: &str = "Extracted PDF text exceeds 32 MiB.";

#[derive(Clone, Debug)]
pub(crate) struct Limits {
    pub max_text: usize,
    pub parse: Duration,
    pub ocr_pages: usize,
    pub ocr_page: Duration,
    pub ocr_total: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_text: 32 * 1024 * 1024,
            parse: Duration::from_secs(20),
            ocr_pages: 10,
            ocr_page: Duration::from_secs(2),
            ocr_total: Duration::from_secs(20),
        }
    }
}

#[derive(Debug)]
pub(crate) enum OcrError {
    /// OCR cannot work for this document (no language pack, renderer refused it).
    Unavailable(String),
    /// Only this page failed (time budget, rendering error).
    Failed(String),
}

/// Recognizes rendered PDF pages. `prepare` runs once before the first page.
pub(crate) trait PageOcr {
    fn prepare(
        &mut self,
        deadline: Instant,
        stopped: &dyn Fn() -> bool,
    ) -> std::result::Result<(), OcrError>;
    fn recognize(
        &mut self,
        page_index: u32,
        deadline: Instant,
        stopped: &dyn Fn() -> bool,
    ) -> std::result::Result<String, OcrError>;
}

pub(crate) fn system_ocr(bytes: Arc<[u8]>) -> Box<dyn PageOcr> {
    #[cfg(windows)]
    {
        Box::new(crate::pdf::NativeOcr::new(bytes))
    }
    #[cfg(not(windows))]
    {
        struct NoOcr;
        impl PageOcr for NoOcr {
            fn prepare(
                &mut self,
                _: Instant,
                _: &dyn Fn() -> bool,
            ) -> std::result::Result<(), OcrError> {
                Err(OcrError::Unavailable("OCR requires Windows".into()))
            }
            fn recognize(
                &mut self,
                _: u32,
                _: Instant,
                _: &dyn Fn() -> bool,
            ) -> std::result::Result<String, OcrError> {
                Err(OcrError::Unavailable("OCR requires Windows".into()))
            }
        }
        let _ = bytes;
        Box::new(NoOcr)
    }
}

#[derive(Debug)]
pub(crate) struct Extracted {
    pub text: String,
    /// False when OCR-eligible pages were skipped because OCR was not requested.
    pub complete: bool,
}

#[derive(Debug, Default)]
pub(crate) struct PageText {
    pub text: String,
    pub needs_ocr: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Layer {
    pub pages: Vec<PageText>,
    pub total: usize,
    pub incomplete: Option<String>,
}

/// Extracts `[page N]` text; OCR is attempted only when `ocr` is provided.
pub(crate) fn extract(
    bytes: Arc<[u8]>,
    ocr: Option<&mut dyn PageOcr>,
    limits: &Limits,
    stopped: &dyn Fn() -> bool,
) -> Result<Extracted> {
    let layer = contained_text_layer(bytes, limits, stopped)?;
    assemble(layer, ocr, limits, stopped)
}

fn contained_text_layer(
    bytes: Arc<[u8]>,
    limits: &Limits,
    stopped: &dyn Fn() -> bool,
) -> Result<Layer> {
    let max_text = limits.max_text;
    contained(limits.parse, stopped, move |cancel, deadline| {
        text_layer(&bytes, cancel, deadline, max_text)
    })
}

/// Runs parser work on its own thread so panics are contained and wall-clock
/// time stays bounded even inside the parser; a runaway parse is abandoned and
/// ends with the contained worker process.
fn contained<T: Send + 'static>(
    budget: Duration,
    stopped: &dyn Fn() -> bool,
    work: impl FnOnce(&AtomicBool, Instant) -> Result<T> + Send + 'static,
) -> Result<T> {
    let cancel = Arc::new(AtomicBool::new(false));
    let deadline = Instant::now() + budget;
    let (sender, receiver) = mpsc::sync_channel(1);
    let flag = cancel.clone();
    std::thread::Builder::new()
        .name("pdf-text".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let result =
                catch_unwind(AssertUnwindSafe(|| work(&flag, deadline))).unwrap_or_else(|_| {
                    Err("PDF could not be parsed (contained parser failure).".into())
                });
            let _ = sender.send(result);
        })
        .map_err(|_| "Could not start contained PDF extraction.")?;
    let grace = deadline + Duration::from_secs(1);
    loop {
        if stopped() {
            cancel.store(true, Ordering::Release);
            return Err(CANCELLED.into());
        }
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() >= grace => {
                cancel.store(true, Ordering::Release);
                return Err("PDF parsing exceeded its time budget.".into());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => (),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("PDF could not be parsed.".into());
            }
        }
    }
}

fn load_error(bytes: &[u8], error: &lopdf::Error) -> String {
    if bytes.windows(8).any(|window| window == b"/Encrypt") {
        return ENCRYPTED.into();
    }
    let detail: String = error.to_string().chars().take(160).collect();
    format!("PDF could not be parsed: {detail}")
}

pub(crate) fn text_layer(
    bytes: &[u8],
    cancel: &AtomicBool,
    deadline: Instant,
    max_text: usize,
) -> Result<Layer> {
    if !bytes.starts_with(b"%PDF-")
        && !bytes[..bytes.len().min(1024)]
            .windows(5)
            .any(|w| w == b"%PDF-")
    {
        return Err("PDF could not be parsed: missing PDF header.".into());
    }
    let options = LoadOptions {
        max_decompressed_size: Some(STREAM_LIMIT),
        ..LoadOptions::default()
    };
    let document = Document::load_mem_with_options(bytes, options)
        .map_err(|error| load_error(bytes, &error))?;
    if document.is_encrypted() {
        return Err(ENCRYPTED.into());
    }
    let pages = document.get_pages();
    if pages.is_empty() {
        return Err("PDF could not be parsed: no pages found.".into());
    }
    let mut layer = Layer {
        total: pages.len(),
        ..Layer::default()
    };
    let mut size = 0usize;
    let mut walker = Walker::new(&document, cancel, deadline);
    for page_id in pages.values().take(MAX_PAGES) {
        if walker.expired() {
            layer.incomplete = Some("PDF time budget reached".into());
            break;
        }
        // Each page may only use what is left of the text budget.
        let page = walker.page(*page_id, max_text - size);
        size += page.text.len();
        if walker.exhausted || size > max_text {
            return Err(TOO_LARGE.into());
        }
        layer.pages.push(page);
        if walker.timed_out {
            layer.incomplete = Some("PDF time budget reached".into());
            break;
        }
    }
    Ok(layer)
}

/// Assembles page text and applies the bounded OCR decision for weak pages.
pub(crate) fn assemble(
    layer: Layer,
    mut ocr: Option<&mut dyn PageOcr>,
    limits: &Limits,
    stopped: &dyn Fn() -> bool,
) -> Result<Extracted> {
    let total_deadline = Instant::now() + limits.ocr_total;
    let mut text = String::new();
    let mut complete = true;
    let mut used = 0usize;
    let mut prepared = false;
    let mut unavailable: Option<String> = None;
    let processed = layer.pages.len();
    for (index, page) in layer.pages.into_iter().enumerate() {
        if stopped() {
            return Err(CANCELLED.into());
        }
        let number = index + 1;
        if !text.is_empty() {
            text.push('\n');
        }
        if !page.needs_ocr {
            text.push_str(&format!("[page {number}]\n"));
            push_block(&mut text, &page.text);
            continue;
        }
        let note = if let Some(engine) = ocr.as_deref_mut() {
            if let Some(reason) = &unavailable {
                format!("no text layer; {reason}")
            } else if used >= limits.ocr_pages {
                format!(
                    "no text layer; OCR limit of {} pages reached",
                    limits.ocr_pages
                )
            } else if Instant::now() >= total_deadline {
                "no text layer; OCR time budget exhausted".into()
            } else {
                used += 1;
                let ready = if prepared {
                    Ok(())
                } else {
                    prepared = true;
                    engine.prepare(
                        (Instant::now() + limits.ocr_page).min(total_deadline),
                        stopped,
                    )
                };
                let deadline = (Instant::now() + limits.ocr_page).min(total_deadline);
                let result = ready.and_then(|()| engine.recognize(index as u32, deadline, stopped));
                if stopped() {
                    return Err(CANCELLED.into());
                }
                match result {
                    Ok(recognized) => {
                        text.push_str(&format!("[page {number}, OCR]\n"));
                        let recognized = normalize(&recognized);
                        if recognized.is_empty() {
                            text.push_str("(no text recognized)\n");
                        } else {
                            push_block(&mut text, &recognized);
                        }
                        continue;
                    }
                    Err(OcrError::Unavailable(reason)) => {
                        let note = format!("no text layer; {reason}");
                        unavailable = Some(reason);
                        note
                    }
                    Err(OcrError::Failed(reason)) => {
                        format!("no text layer; OCR failed: {reason}")
                    }
                }
            }
        } else {
            complete = false;
            "no text layer; OCR runs only when read_file reads this PDF".into()
        };
        text.push_str(&format!("[page {number}]\n({note})\n"));
        push_block(&mut text, &page.text);
    }
    if layer.total > processed {
        let reason = layer
            .incomplete
            .unwrap_or_else(|| format!("{MAX_PAGES}-page limit"));
        text.push_str(&format!(
            "\n[pages {}-{} not processed: {reason}]\n",
            processed + 1,
            layer.total
        ));
    } else if let Some(reason) = layer.incomplete {
        text.push_str(&format!("\n[text extraction stopped early: {reason}]\n"));
    }
    if text.len() > limits.max_text {
        return Err(TOO_LARGE.into());
    }
    Ok(Extracted { text, complete })
}

fn push_block(text: &mut String, block: &str) {
    if !block.is_empty() {
        text.push_str(block);
        text.push('\n');
    }
}

/// Collapses whitespace runs, trims lines and keeps at most one blank line.
pub(crate) fn normalize(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut blank = false;
    for line in text.split(['\n', '\r']) {
        let mut words = line
            .split(|c: char| c.is_whitespace() || c.is_control())
            .filter(|word| !word.is_empty());
        let Some(first) = words.next() else {
            if !output.is_empty() && !blank {
                output.push('\n');
                blank = true;
            }
            continue;
        };
        blank = false;
        output.push_str(first);
        for word in words {
            output.push(' ');
            output.push_str(word);
        }
        output.push('\n');
    }
    output.truncate(output.trim_end().len());
    output
}

#[derive(Clone, Copy, Debug)]
struct Matrix([f64; 6]);

impl Matrix {
    const IDENTITY: Self = Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    fn translate(x: f64, y: f64) -> Self {
        Self([1.0, 0.0, 0.0, 1.0, x, y])
    }

    /// Row-vector PDF convention: `self` is applied first, then `next`.
    fn then(self, next: Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = next.0;
        Self([
            a * a2 + b * c2,
            a * b2 + b * d2,
            c * a2 + d * c2,
            c * b2 + d * d2,
            e * a2 + f * c2 + e2,
            e * b2 + f * d2 + f2,
        ])
    }
}

fn number(object: &Object) -> Option<f64> {
    object
        .as_float()
        .ok()
        .map(f64::from)
        .filter(|value| value.is_finite())
}

fn matrix(operands: &[Object]) -> Option<Matrix> {
    if operands.len() < 6 {
        return None;
    }
    let mut values = [0.0; 6];
    for (value, operand) in values.iter_mut().zip(operands) {
        *value = number(operand)?;
    }
    Some(Matrix(values))
}

/// `(first code, last code, width)`, sorted by first code.
type WidthRanges = Rc<Vec<(u32, u32, f64)>>;

enum Widths {
    Simple {
        first: u32,
        widths: Vec<f64>,
        missing: f64,
    },
    Composite {
        default: f64,
        /// Shared by every font whose descendant uses the same `/W` array.
        ranges: WidthRanges,
    },
}

impl Widths {
    fn get(&self, code: u32) -> f64 {
        match self {
            Self::Simple {
                first,
                widths,
                missing,
            } => code
                .checked_sub(*first)
                .and_then(|index| widths.get(index as usize))
                .copied()
                .unwrap_or(*missing),
            Self::Composite { default, ranges } => {
                let index = ranges.partition_point(|range| range.0 <= code);
                index
                    .checked_sub(1)
                    .map(|index| ranges[index])
                    .filter(|range| range.1 >= code)
                    .map_or(*default, |range| range.2)
            }
        }
    }
}

fn composite_widths(document: &Document, list: &[Object]) -> Vec<(u32, u32, f64)> {
    let mut ranges = Vec::new();
    let mut index = 0;
    while index + 1 < list.len() && ranges.len() < 65_536 {
        let Some(first) = number(&list[index]).filter(|value| *value >= 0.0) else {
            break;
        };
        let first = first as u32;
        match resolve(document, &list[index + 1]) {
            Some(Object::Array(values)) => {
                for (offset, value) in values.iter().take(65_536).enumerate() {
                    if let Some(width) = resolve(document, value).and_then(number) {
                        let code = first.saturating_add(offset as u32);
                        ranges.push((code, code, width));
                    }
                }
                index += 2;
            }
            Some(last) => {
                let (Some(last), Some(width)) = (
                    number(last),
                    list.get(index + 2)
                        .and_then(|object| resolve(document, object))
                        .and_then(number),
                ) else {
                    break;
                };
                ranges.push((first, last.max(0.0) as u32, width));
                index += 3;
            }
            None => break,
        }
    }
    ranges.sort_by_key(|range| range.0);
    ranges
}

struct Font<'a> {
    decoder: Option<Decoder<'a>>,
    two_byte: bool,
    widths: Widths,
    glyph_scale: f64,
}

enum Decoder<'a> {
    /// lopdf's single-byte encodings; lopdf never sees a ToUnicode stream.
    Lopdf(Encoding<'a>),
    /// Byte-to-text table (lenient `/Differences` or StandardEncoding).
    Table(Rc<[String]>),
    /// ToUnicode through the bounded CMap parser.
    Unicode(Rc<cmap::CMap>),
    /// UniGB-UCS2-H / UniGB-UTF16-H codes are UTF-16BE.
    Utf16,
}

impl Decoder<'_> {
    /// Appends decoded text to `output`, stopping once it exceeds `limit`.
    /// Single-byte decoders work in small chunks, so one call can overshoot
    /// the limit by at most one chunk's text.
    fn decode(
        &self,
        bytes: &[u8],
        output: &mut String,
        limit: usize,
        expired: &dyn Fn() -> bool,
        replaced: &mut usize,
    ) -> Decoded {
        match self {
            Self::Unicode(cmap) => cmap.decode(bytes, output, limit, expired, replaced),
            Self::Utf16 => {
                let mut sink = cmap::Utf16::new(output, replaced);
                for (index, pair) in bytes.chunks(2).enumerate() {
                    if index % 8_192 == 8_191 && expired() {
                        return Decoded::Expired;
                    }
                    match *pair {
                        [high, low] => sink.push(u16::from_be_bytes([high, low])),
                        _ => sink.replace(),
                    }
                    if sink.output.len() > limit {
                        return Decoded::Full;
                    }
                }
                sink.finish();
                Decoded::Done
            }
            Self::Lopdf(encoding) => chunked(bytes, output, limit, expired, |chunk, output| {
                encoding.write_to_string(chunk, output).is_ok()
            }),
            Self::Table(table) => chunked(bytes, output, limit, expired, |chunk, output| {
                for byte in chunk {
                    output.push_str(&table[usize::from(*byte)]);
                }
                true
            }),
        }
    }
}

fn chunked(
    bytes: &[u8],
    output: &mut String,
    limit: usize,
    expired: &dyn Fn() -> bool,
    mut write: impl FnMut(&[u8], &mut String) -> bool,
) -> Decoded {
    for (index, chunk) in bytes.chunks(4_096).enumerate() {
        if index % 4 == 3 && expired() {
            return Decoded::Expired;
        }
        if !write(chunk, output) {
            return Decoded::Failed;
        }
        if output.len() > limit {
            return Decoded::Full;
        }
    }
    Decoded::Done
}

/// Decodes one code through lopdf's own tables using a synthetic font dictionary.
fn probe(document: &Document, encoding: Object, code: u8) -> Option<String> {
    let mut font = Dictionary::new();
    font.set("Type", Object::Name(b"Font".to_vec()));
    font.set("Encoding", encoding);
    let encoding = font
        .get_font_encoding_with_limit(document, STREAM_LIMIT)
        .ok()?;
    let mut output = String::new();
    encoding.write_to_string(&[code], &mut output).ok()?;
    Some(output)
}

/// lopdf rejects a whole `/Differences` array when one glyph name is not in the
/// Adobe Glyph List (common in Czech CE fonts, e.g. `/controlSTX`) and falls back
/// to StandardEncoding. This keeps every known name and drops only unknown ones.
fn lenient_differences(document: &Document, font: &Dictionary) -> Option<Vec<String>> {
    let encoding = resolve(document, font.get(b"Encoding").ok()?)?
        .as_dict()
        .ok()?;
    let differences = resolve(document, encoding.get(b"Differences").ok()?)?
        .as_array()
        .ok()?;
    let base = encoding
        .get(b"BaseEncoding")
        .and_then(Object::as_name)
        .unwrap_or(b"StandardEncoding")
        .to_vec();
    let mut table: Vec<String> = (0..=255u8)
        .map(|code| probe(document, Object::Name(base.clone()), code).unwrap_or_default())
        .collect();
    let mut code: Option<usize> = None;
    for entry in differences.iter().take(1024) {
        match resolve(document, entry) {
            Some(Object::Integer(value)) => code = usize::try_from(*value).ok(),
            Some(Object::Name(name)) => {
                if let Some(current) = code.filter(|current| *current < 256) {
                    let mut single = Dictionary::new();
                    single.set(
                        "Differences",
                        Object::Array(vec![
                            Object::Integer(current as i64),
                            Object::Name(name.clone()),
                        ]),
                    );
                    table[current] = probe(document, Object::Dictionary(single), current as u8)
                        .unwrap_or_default();
                }
                code = code.map(|current| current + 1);
            }
            _ => code = None,
        }
    }
    Some(table)
}

fn resolve<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Object> {
    document.dereference(object).ok().map(|(_, object)| object)
}

#[derive(Clone)]
struct State {
    ctm: Matrix,
    font: Option<usize>,
    size: f64,
    character_spacing: f64,
    word_spacing: f64,
    horizontal_scale: f64,
    leading: f64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            ctm: Matrix::IDENTITY,
            font: None,
            size: 0.0,
            character_spacing: 0.0,
            word_spacing: 0.0,
            horizontal_scale: 1.0,
            leading: 0.0,
        }
    }
}

struct Last {
    end: (f64, f64),
    direction: (f64, f64),
    size: f64,
}

struct Walker<'a> {
    document: &'a Document,
    cancel: &'a AtomicBool,
    deadline: Instant,
    fonts: HashMap<usize, Rc<Font<'a>>>,
    /// Parsed ToUnicode CMaps by stream object, so a shared stream is parsed once.
    cmaps: HashMap<usize, Option<Rc<cmap::CMap>>>,
    /// Composite widths by `/W` array object.
    widths: HashMap<usize, WidthRanges>,
    standard: Option<Rc<[String]>>,
    cmap_entries: usize,
    output: String,
    /// Raw page output allowed before the document text budget is exhausted.
    limit: usize,
    last: Option<Last>,
    new_line: bool,
    operations: usize,
    shown: usize,
    content_bytes: usize,
    forms: usize,
    saw_image: bool,
    undecodable: usize,
    truncated: bool,
    timed_out: bool,
    exhausted: bool,
}

impl<'a> Walker<'a> {
    fn new(document: &'a Document, cancel: &'a AtomicBool, deadline: Instant) -> Self {
        Self {
            document,
            cancel,
            deadline,
            fonts: HashMap::new(),
            cmaps: HashMap::new(),
            widths: HashMap::new(),
            standard: None,
            cmap_entries: MAX_DOCUMENT_CMAP_ENTRIES,
            output: String::new(),
            limit: 0,
            last: None,
            new_line: false,
            operations: 0,
            shown: 0,
            content_bytes: 0,
            forms: 0,
            saw_image: false,
            undecodable: 0,
            truncated: false,
            timed_out: false,
            exhausted: false,
        }
    }

    fn expired(&self) -> bool {
        self.cancel.load(Ordering::Acquire) || Instant::now() >= self.deadline
    }

    fn page(&mut self, page: ObjectId, limit: usize) -> PageText {
        self.output.clear();
        self.limit = limit;
        self.last = None;
        self.new_line = false;
        self.operations = 0;
        self.shown = 0;
        self.content_bytes = 0;
        self.forms = 0;
        self.saw_image = false;
        self.undecodable = 0;
        self.truncated = false;
        self.exhausted = false;
        let mut resources = Vec::new();
        if let Ok((direct, inherited)) = self.document.get_page_resources(page) {
            resources.extend(direct);
            resources.extend(
                inherited
                    .into_iter()
                    .filter_map(|id| self.document.get_dictionary(id).ok()),
            );
        }
        match self
            .document
            .get_page_content_with_limit(page, PAGE_CONTENT_LIMIT)
        {
            Ok(content) => {
                self.content_bytes = content.len();
                self.run(&content, &resources, State::default(), 0, &mut Vec::new());
            }
            Err(_) => self.truncated = true,
        }
        let mut text = normalize(&self.output);
        let characters = text.chars().filter(|c| c.is_alphanumeric()).count();
        let needs_ocr =
            characters < MIN_TEXT_CHARACTERS && (self.saw_image || self.undecodable > 0);
        if self.truncated && !self.timed_out {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str("(page content exceeds extraction limits; text truncated)");
        }
        PageText { text, needs_ocr }
    }

    fn font_key(&mut self, dictionary: &'a Dictionary) -> Option<usize> {
        let key = dictionary as *const Dictionary as usize;
        if !self.fonts.contains_key(&key) {
            if self.fonts.len() >= MAX_FONTS {
                return None;
            }
            let font = self.font(dictionary);
            self.fonts.insert(key, Rc::new(font));
        }
        Some(key)
    }

    fn font(&mut self, dictionary: &'a Dictionary) -> Font<'a> {
        let document = self.document;
        let subtype = dictionary
            .get(b"Subtype")
            .and_then(Object::as_name)
            .unwrap_or(b"");
        let composite = subtype == b"Type0";
        // ToUnicode is the most faithful extraction mapping (ISO 32000 9.10.2).
        let decoder = match dictionary
            .get(b"ToUnicode")
            .ok()
            .and_then(|object| self.unicode_map(object))
        {
            Some(cmap) => Some(Decoder::Unicode(cmap)),
            None => self.fallback_decoder(dictionary, composite),
        };
        let mut glyph_scale = 0.001;
        let widths = if composite {
            let descendant = dictionary
                .get(b"DescendantFonts")
                .ok()
                .and_then(|object| resolve(document, object))
                .and_then(|object| object.as_array().ok())
                .and_then(|array| array.first())
                .and_then(|object| resolve(document, object))
                .and_then(|object| object.as_dict().ok());
            let default = descendant
                .and_then(|font| font.get(b"DW").ok())
                .and_then(number)
                .unwrap_or(1000.0);
            let ranges = descendant
                .and_then(|font| font.get(b"W").ok())
                .and_then(|object| resolve(document, object))
                .and_then(|object| object.as_array().ok())
                .map(|list| {
                    self.widths
                        .entry(list as *const Vec<Object> as usize)
                        .or_insert_with(|| Rc::new(composite_widths(document, list)))
                        .clone()
                })
                .unwrap_or_default();
            Widths::Composite { default, ranges }
        } else {
            if subtype == b"Type3"
                && let Some(scale) = dictionary
                    .get(b"FontMatrix")
                    .ok()
                    .and_then(|object| resolve(document, object))
                    .and_then(|object| object.as_array().ok())
                    .and_then(|array| array.first())
                    .and_then(number)
            {
                glyph_scale = scale;
            }
            let first = dictionary
                .get(b"FirstChar")
                .ok()
                .and_then(number)
                .unwrap_or(0.0)
                .clamp(0.0, 255.0) as u32;
            let widths: Vec<f64> = dictionary
                .get(b"Widths")
                .ok()
                .and_then(|object| resolve(document, object))
                .and_then(|object| object.as_array().ok())
                .map(|array| {
                    array
                        .iter()
                        .take(256)
                        .map(|value| resolve(document, value).and_then(number).unwrap_or(0.0))
                        .collect()
                })
                .unwrap_or_default();
            let declared = dictionary
                .get(b"FontDescriptor")
                .ok()
                .and_then(|object| resolve(document, object))
                .and_then(|object| object.as_dict().ok())
                .and_then(|descriptor| descriptor.get(b"MissingWidth").ok())
                .and_then(number);
            let monospace = dictionary
                .get(b"BaseFont")
                .and_then(Object::as_name)
                .is_ok_and(|name| name.windows(7).any(|part| part == b"Courier"));
            let missing = match (declared, widths.is_empty()) {
                (Some(width), _) => width,
                (None, false) => 0.0,
                // Unembedded standard fonts have no widths; estimate an average advance.
                (None, true) if monospace => 600.0,
                (None, true) => 500.0,
            };
            Widths::Simple {
                first,
                widths,
                missing,
            }
        };
        Font {
            decoder,
            two_byte: composite,
            widths,
            glyph_scale,
        }
    }

    /// Parses a ToUnicode stream once per document with the bounded parser;
    /// lopdf's parser would expand every code of every range.
    fn unicode_map(&mut self, object: &'a Object) -> Option<Rc<cmap::CMap>> {
        let stream = resolve(self.document, object)?;
        let key = stream as *const Object as usize;
        if let Some(cached) = self.cmaps.get(&key) {
            return cached.clone();
        }
        let (cancel, deadline) = (self.cancel, self.deadline);
        let expired = || cancel.load(Ordering::Acquire) || Instant::now() >= deadline;
        let parsed = stream
            .as_stream()
            .ok()
            .and_then(|stream| stream.get_plain_content_with_limit(STREAM_LIMIT).ok())
            .and_then(|data| cmap::parse(&data, self.cmap_entries, &expired))
            .map(Rc::new);
        if let Some(cmap) = &parsed {
            self.cmap_entries -= cmap.entries;
        }
        self.cmaps.insert(key, parsed.clone());
        parsed
    }

    /// Encodings for fonts without a usable ToUnicode. lopdf reads ToUnicode
    /// itself for Identity encodings (and fonts without `/Encoding`), so those
    /// cases never reach it; like lopdf, they fall back to StandardEncoding.
    fn fallback_decoder(
        &mut self,
        dictionary: &'a Dictionary,
        composite: bool,
    ) -> Option<Decoder<'a>> {
        let document = self.document;
        let encoding = dictionary
            .get(b"Encoding")
            .ok()
            .map(|object| resolve(document, object));
        if composite {
            // Composite fonts without ToUnicode would decode CIDs as single bytes.
            return matches!(
                encoding.flatten().and_then(|object| object.as_name().ok()),
                Some(b"UniGB-UCS2-H" | b"UniGB-UTF16-H")
            )
            .then_some(Decoder::Utf16);
        }
        let identity = |name: &[u8]| matches!(name, b"Identity-H" | b"Identity-V");
        let standard = match encoding {
            None | Some(None) => true,
            Some(Some(Object::Name(name))) => identity(name),
            Some(Some(Object::Dictionary(encoding))) => encoding
                .get(b"BaseEncoding")
                .and_then(Object::as_name)
                .is_ok_and(identity),
            Some(Some(_)) => false,
        };
        if standard {
            let table = self.standard.get_or_insert_with(|| {
                (0..=255u8)
                    .map(|code| {
                        probe(document, Object::Name(b"StandardEncoding".to_vec()), code)
                            .unwrap_or_default()
                    })
                    .collect()
            });
            return Some(Decoder::Table(table.clone()));
        }
        let encoding = dictionary
            .get_font_encoding_with_limit(document, STREAM_LIMIT)
            .ok();
        if !matches!(encoding, Some(Encoding::Differences(_)))
            && let Some(table) = lenient_differences(document, dictionary)
        {
            return Some(Decoder::Table(table.into()));
        }
        encoding.map(Decoder::Lopdf)
    }

    fn lookup(
        &self,
        resources: &[&'a Dictionary],
        category: &[u8],
        name: &[u8],
    ) -> Option<&'a Object> {
        resources.iter().find_map(|dictionary| {
            let group = resolve(self.document, dictionary.get(category).ok()?)?;
            resolve(self.document, group.as_dict().ok()?.get(name).ok()?)
        })
    }

    fn run(
        &mut self,
        content: &[u8],
        resources: &[&'a Dictionary],
        initial: State,
        depth: usize,
        active: &mut Vec<ObjectId>,
    ) {
        let mut state = initial;
        let mut saved: Vec<State> = Vec::new();
        let mut text_matrix = Matrix::IDENTITY;
        let mut line_matrix = Matrix::IDENTITY;
        for operation in Content::decode_iter(content) {
            let Ok(operation) = operation else {
                break;
            };
            self.operations += 1;
            if self.operations > MAX_OPERATIONS {
                self.truncated = true;
                return;
            }
            if self.operations.is_multiple_of(4096) && self.expired() {
                self.truncated = true;
                self.timed_out = true;
                return;
            }
            let operands = &operation.operands;
            let first = operands.first().and_then(number);
            let second = operands.get(1).and_then(number);
            match operation.operator.as_str() {
                "q" if saved.len() < 64 => saved.push(state.clone()),
                "Q" => {
                    if let Some(previous) = saved.pop() {
                        state = previous;
                    }
                }
                "cm" => {
                    if let Some(matrix) = matrix(operands) {
                        state.ctm = matrix.then(state.ctm);
                    }
                }
                "BT" => {
                    text_matrix = Matrix::IDENTITY;
                    line_matrix = Matrix::IDENTITY;
                }
                "Tf" => {
                    if let Some(size) = second {
                        state.size = size;
                    }
                    state.font = operands
                        .first()
                        .and_then(|name| name.as_name().ok())
                        .and_then(|name| self.lookup(resources, b"Font", name))
                        .and_then(|object| object.as_dict().ok())
                        .and_then(|dictionary| self.font_key(dictionary));
                }
                "Tc" => state.character_spacing = first.unwrap_or(0.0),
                "Tw" => state.word_spacing = first.unwrap_or(0.0),
                "Tz" => state.horizontal_scale = first.unwrap_or(100.0) / 100.0,
                "TL" => state.leading = first.unwrap_or(0.0),
                "Td" | "TD" => {
                    if let (Some(x), Some(y)) = (first, second) {
                        if operation.operator == "TD" {
                            state.leading = -y;
                        }
                        line_matrix = Matrix::translate(x, y).then(line_matrix);
                        text_matrix = line_matrix;
                    }
                }
                "Tm" => {
                    if let Some(matrix) = matrix(operands) {
                        line_matrix = matrix;
                        text_matrix = matrix;
                    }
                }
                "T*" => {
                    line_matrix = Matrix::translate(0.0, -state.leading).then(line_matrix);
                    text_matrix = line_matrix;
                    self.new_line = true;
                }
                "Tj" => {
                    if let Some(Object::String(bytes, _)) = operands.first() {
                        self.show(bytes, &state, &mut text_matrix);
                    }
                }
                "'" | "\"" => {
                    if operation.operator == "\"" {
                        state.word_spacing = first.unwrap_or(state.word_spacing);
                        state.character_spacing = second.unwrap_or(state.character_spacing);
                    }
                    line_matrix = Matrix::translate(0.0, -state.leading).then(line_matrix);
                    text_matrix = line_matrix;
                    self.new_line = true;
                    if let Some(Object::String(bytes, _)) = operands.last() {
                        self.show(bytes, &state, &mut text_matrix);
                    }
                }
                "TJ" => {
                    if let Some(Object::Array(items)) = operands.first() {
                        for item in items {
                            match item {
                                Object::String(bytes, _) => {
                                    self.show(bytes, &state, &mut text_matrix)
                                }
                                other => {
                                    if let Some(adjustment) = number(other) {
                                        let shift = -adjustment / 1000.0
                                            * state.size
                                            * state.horizontal_scale;
                                        text_matrix =
                                            Matrix::translate(shift, 0.0).then(text_matrix);
                                    }
                                }
                            }
                            if self.truncated {
                                break;
                            }
                        }
                    }
                }
                "BI" => self.saw_image = true,
                "Do" => {
                    if let Some(name) = operands.first().and_then(|name| name.as_name().ok()) {
                        self.draw(name, resources, &state, depth, active);
                    }
                }
                _ => (),
            }
            if self.truncated {
                return;
            }
        }
    }

    fn draw(
        &mut self,
        name: &[u8],
        resources: &[&'a Dictionary],
        state: &State,
        depth: usize,
        active: &mut Vec<ObjectId>,
    ) {
        let Some(reference) = resources.iter().find_map(|dictionary| {
            let group = resolve(self.document, dictionary.get(b"XObject").ok()?)?;
            group.as_dict().ok()?.get(name).ok()
        }) else {
            return;
        };
        let Ok((id, object)) = self.document.dereference(reference) else {
            return;
        };
        let Ok(stream) = object.as_stream() else {
            return;
        };
        match stream.dict.get(b"Subtype").and_then(Object::as_name) {
            Ok(b"Image") => self.saw_image = true,
            Ok(b"Form") => {
                if depth >= MAX_FORM_DEPTH
                    || self.forms >= MAX_FORMS
                    || id.is_some_and(|id| active.contains(&id))
                {
                    return;
                }
                self.forms += 1;
                let remaining = PAGE_CONTENT_LIMIT.saturating_sub(self.content_bytes);
                let content = match stream.decompressed_content_with_limit(remaining) {
                    Ok(content) => content,
                    Err(lopdf::Error::Decompress(
                        lopdf::DecompressError::MemoryLimitExceeded { .. },
                    )) => {
                        self.truncated = true;
                        return;
                    }
                    Err(_) if stream.content.len() <= remaining => stream.content.clone(),
                    Err(_) => {
                        self.truncated = true;
                        return;
                    }
                };
                self.content_bytes += content.len();
                let form_resources: Vec<&'a Dictionary> = match stream
                    .dict
                    .get(b"Resources")
                    .ok()
                    .and_then(|object| resolve(self.document, object))
                    .and_then(|object| object.as_dict().ok())
                {
                    Some(dictionary) => vec![dictionary],
                    None => resources.to_vec(),
                };
                let mut inner = state.clone();
                let form_matrix = stream
                    .dict
                    .get(b"Matrix")
                    .ok()
                    .and_then(|object| resolve(self.document, object))
                    .and_then(|object| object.as_array().ok())
                    .and_then(|values| matrix(values))
                    .unwrap_or(Matrix::IDENTITY);
                inner.ctm = form_matrix.then(state.ctm);
                if let Some(id) = id {
                    active.push(id);
                }
                self.run(&content, &form_resources, inner, depth + 1, active);
                if id.is_some() {
                    active.pop();
                }
            }
            _ => (),
        }
    }

    fn show(&mut self, bytes: &[u8], state: &State, text_matrix: &mut Matrix) {
        self.shown += 1;
        if self.shown.is_multiple_of(4096) && self.expired() {
            self.truncated = true;
            self.timed_out = true;
            return;
        }
        let font = state.font.and_then(|key| self.fonts.get(&key)).cloned();
        // Each string may only use what is left of the text budget, however
        // many characters a single code expands to.
        let room = self.limit.saturating_sub(self.output.len());
        let (cancel, deadline) = (self.cancel, self.deadline);
        let expired = || cancel.load(Ordering::Acquire) || Instant::now() >= deadline;
        let mut piece = String::new();
        let mut replaced = 0;
        match font.as_ref().and_then(|font| font.decoder.as_ref()) {
            Some(decoder) => match decoder.decode(bytes, &mut piece, room, &expired, &mut replaced)
            {
                Decoded::Done => (),
                Decoded::Failed => {
                    piece.clear();
                    self.undecodable += bytes.len();
                }
                Decoded::Full => {
                    self.truncated = true;
                    self.exhausted = true;
                    return;
                }
                Decoded::Expired => {
                    self.truncated = true;
                    self.timed_out = true;
                    return;
                }
            },
            None => self.undecodable += bytes.len(),
        }
        self.undecodable += replaced;
        if piece.contains('\u{fffd}') {
            self.undecodable += piece.matches('\u{fffd}').count();
            piece.retain(|c| c != '\u{fffd}');
        }
        let mut advance = 0.0;
        let mut step = |code: u32, space: bool| {
            let (width, scale) = font.as_ref().map_or((500.0, 0.001), |font| {
                (font.widths.get(code), font.glyph_scale)
            });
            advance += (width * scale * state.size
                + state.character_spacing
                + if space { state.word_spacing } else { 0.0 })
                * state.horizontal_scale;
        };
        match font.as_ref() {
            Some(font) if font.two_byte => {
                for pair in bytes.chunks(2) {
                    step(
                        pair.iter()
                            .fold(0u32, |code, byte| code << 8 | *byte as u32),
                        false,
                    );
                }
            }
            _ => {
                for byte in bytes {
                    step(*byte as u32, *byte == 32);
                }
            }
        }
        let rendering = text_matrix.then(state.ctm);
        let [a, b, c, d, e, f] = rendering.0;
        let size = (state.size * c.hypot(d)).abs();
        if !piece.is_empty() {
            self.separate(&piece, (e, f), size);
            self.output.push_str(&piece);
        }
        let length = a.hypot(b);
        let direction = if length > 1e-9 {
            (a / length, b / length)
        } else {
            (1.0, 0.0)
        };
        if !piece.is_empty() {
            self.last = Some(Last {
                end: (e + advance * a, f + advance * b),
                direction,
                size,
            });
        }
        *text_matrix = Matrix::translate(advance, 0.0).then(*text_matrix);
    }

    fn separate(&mut self, piece: &str, start: (f64, f64), size: f64) {
        if self.output.is_empty() {
            self.new_line = false;
            return;
        }
        let ends_with_space = self.output.ends_with(char::is_whitespace);
        let starts_with_space = piece.starts_with(char::is_whitespace);
        let mut line = std::mem::take(&mut self.new_line);
        let mut space = false;
        if let Some(last) = &self.last {
            let (dx, dy) = (start.0 - last.end.0, start.1 - last.end.1);
            let along = dx * last.direction.0 + dy * last.direction.1;
            let across = dy * last.direction.0 - dx * last.direction.1;
            let larger = last.size.max(size).max(0.01);
            let smaller = last.size.min(size).max(0.01);
            if !across.is_finite() || across.abs() > 0.5 * larger {
                line = true;
            } else if along > 0.15 * smaller || along < -0.5 * larger {
                space = true;
            }
        }
        if line {
            if !self.output.ends_with('\n') {
                self.output.push('\n');
            }
        } else if space && !ends_with_space && !starts_with_space {
            self.output.push(' ');
        }
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Small PDFs written in-test; no binary fixtures are committed.

    pub(crate) fn stream(dictionary: &str, data: &[u8]) -> Vec<u8> {
        let mut bytes = format!("<< {dictionary} /Length {} >>\nstream\n", data.len()).into_bytes();
        bytes.extend_from_slice(data);
        bytes.extend_from_slice(b"\nendstream");
        bytes
    }

    pub(crate) fn helvetica() -> Vec<u8> {
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec()
    }

    pub(crate) fn gray_pixel() -> Vec<u8> {
        stream(
            "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8",
            b"\x80",
        )
    }

    /// Objects: 1 catalog, 2 page tree, 3 font /F1, 4 image /Im1, then a page
    /// and its content per entry, then `extra` objects numbered afterwards.
    pub(crate) fn build(
        font: &[u8],
        image: &[u8],
        contents: &[Vec<u8>],
        extra: &[Vec<u8>],
    ) -> Vec<u8> {
        let kids: Vec<String> = (0..contents.len())
            .map(|index| format!("{} 0 R", 5 + 2 * index))
            .collect();
        let mut objects = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {} >>",
                kids.join(" "),
                contents.len()
            )
            .into_bytes(),
            font.to_vec(),
            image.to_vec(),
        ];
        for (index, content) in contents.iter().enumerate() {
            objects.push(
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> /XObject << /Im1 4 0 R >> >> /Contents {} 0 R >>",
                    6 + 2 * index
                )
                .into_bytes(),
            );
            objects.push(stream("", content));
        }
        objects.extend(extra.iter().cloned());
        document(&objects)
    }

    /// Serializes objects numbered from 1; object 1 is the catalog.
    pub(crate) fn document(objects: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            bytes.extend_from_slice(object);
            bytes.extend_from_slice(b"\nendobj\n");
        }
        let xref = bytes.len();
        bytes.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        bytes
    }

    pub(crate) fn text_page(lines: &[&str]) -> Vec<u8> {
        let mut content = String::from("BT /F1 12 Tf 72 720 Td ");
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                content.push_str("0 -16 Td ");
            }
            content.push_str(&format!("({line}) Tj "));
        }
        content.push_str("ET");
        content.into_bytes()
    }

    pub(crate) fn image_page() -> Vec<u8> {
        b"q 200 0 0 200 100 400 cm /Im1 Do Q".to_vec()
    }

    pub(crate) fn simple(pages: &[&[&str]]) -> Vec<u8> {
        let contents: Vec<_> = pages.iter().map(|lines| text_page(lines)).collect();
        build(&helvetica(), &gray_pixel(), &contents, &[])
    }

    pub(crate) const CZECH: &str = "Příliš žluťoučký kůň úpěl ďábelské ódy";

    /// A Type0/Identity-H font whose two-byte codes map to Unicode only via ToUnicode.
    pub(crate) fn czech() -> Vec<u8> {
        let mut alphabet: Vec<char> = CZECH.chars().collect();
        alphabet.sort();
        alphabet.dedup();
        let code = |c: char| alphabet.iter().position(|known| *known == c).unwrap() + 1;
        let mut cmap = String::from(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
        );
        cmap.push_str(&format!("{} beginbfchar\n", alphabet.len()));
        for c in &alphabet {
            cmap.push_str(&format!("<{:04X}> <{:04X}>\n", code(*c), *c as u32));
        }
        cmap.push_str(
            "endbfchar\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n",
        );
        let hex: String = CZECH.chars().map(|c| format!("{:04X}", code(c))).collect();
        let content = format!("BT /F1 14 Tf 72 700 Td <{hex}> Tj ET").into_bytes();
        let descendant = 7;
        let font = format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /SyntheticCzech /Encoding /Identity-H /DescendantFonts [{descendant} 0 R] /ToUnicode {} 0 R >>",
            descendant + 1
        );
        build(
            font.as_bytes(),
            &gray_pixel(),
            &[content],
            &[
                b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /SyntheticCzech /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /DW 600 >>".to_vec(),
                stream("", cmap.as_bytes()),
            ],
        )
    }

    /// Encrypts `simple` with RC4-128 using an owner password and `user` password.
    pub(crate) fn encrypted(user: &str) -> Vec<u8> {
        let mut document =
            lopdf::Document::load_mem(&simple(&[&["Confidential synthetic page"]])).unwrap();
        let id = lopdf::Object::string_literal(b"0123456789abcdef".to_vec());
        document
            .trailer
            .set("ID", lopdf::Object::Array(vec![id.clone(), id]));
        let state = lopdf::EncryptionState::try_from(lopdf::EncryptionVersion::V2 {
            document: &document,
            owner_password: "synthetic-owner",
            user_password: user,
            key_length: 128,
            permissions: lopdf::Permissions::default(),
        })
        .unwrap();
        document.encrypt(&state).unwrap();
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::{fixtures::*, *};
    use std::cell::Cell;

    fn read(bytes: Vec<u8>, ocr: Option<&mut dyn PageOcr>) -> Result<Extracted> {
        extract(bytes.into(), ocr, &Limits::default(), &|| false)
    }

    #[test]
    fn pages_keep_order_words_lines_and_kerning() {
        let mut contents = vec![
            b"BT /F1 12 Tf 72 720 Td [(Hel) -20 (lo) -300 (world)] TJ 0 -16 Td (Second   line) Tj ET"
                .to_vec(),
        ];
        contents.push(text_page(&["Middle page"]));
        contents.push(text_page(&["Last page", "Ends here"]));
        let text = read(build(&helvetica(), &gray_pixel(), &contents, &[]), None)
            .unwrap()
            .text;
        assert_eq!(
            text,
            "[page 1]\nHello world\nSecond line\n\n[page 2]\nMiddle page\n\n[page 3]\nLast page\nEnds here\n"
        );
    }

    #[test]
    fn rejected_differences_keep_known_central_european_glyphs() {
        // lopdf rejects the whole array because `.notdef`/`g17` are not glyph-list names.
        let font = b"<< /Type /Font /Subtype /Type1 /BaseFont /HelveticaCE /Encoding << /Type /Encoding /Differences [0 /.notdef /g17 225 /aacute 232 /ccaron 248 /rcaron /uring /uacute 253 /yacute] >> >>";
        let content =
            b"BT /F1 12 Tf 72 720 Td (Z\\341kaznick\\375 \\372\\350et \\370 k\\371\\001) Tj ET"
                .to_vec();
        let text = read(build(font, &gray_pixel(), &[content], &[]), None)
            .unwrap()
            .text;
        assert_eq!(text, "[page 1]\nZákaznický účet ř ků\n");
    }

    #[test]
    fn czech_to_unicode_identity_h_text_is_exact() {
        let extracted = read(czech(), None).unwrap();
        assert!(extracted.complete);
        assert_eq!(extracted.text, format!("[page 1]\n{CZECH}\n"));
    }

    fn cmap_program(body: &str) -> Vec<u8> {
        stream(
            "",
            format!("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CMapName /Hostile def\n/CMapType 2 def\n{body}\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n").as_bytes(),
        )
    }

    fn walk(bytes: &[u8], limit: usize, check: impl FnOnce(&Walker, PageText)) {
        let parsed = Document::load_mem(bytes).unwrap();
        let cancel = AtomicBool::new(false);
        let mut walker = Walker::new(&parsed, &cancel, Instant::now() + Duration::from_secs(20));
        let page = walker.page(*parsed.get_pages().values().next().unwrap(), limit);
        check(&walker, page);
    }

    #[test]
    fn expanding_to_unicode_targets_stay_within_the_text_budget() {
        let codes = "01".repeat(64 * 1024);
        let pdf = |target: String| {
            build(
                b"<< /Type /Font /Subtype /Type1 /BaseFont /Hostile /ToUnicode 7 0 R >>",
                &gray_pixel(),
                &[format!("BT /F1 12 Tf 72 700 Td <{codes}> Tj ET").into_bytes()],
                &[cmap_program(&format!(
                    "1 begincodespacerange <00> <FF> endcodespacerange\n2 beginbfchar <01> <{target}> <02> <0042> endbfchar"
                ))],
            )
        };
        // A 256-unit target is skipped: its codes stay undecodable instead of expanding.
        let started = Instant::now();
        let skipped = read(pdf("0041".repeat(256)), None).unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(
            skipped.text,
            "[page 1]\n(no text layer; OCR runs only when read_file reads this PDF)\n"
        );
        // The longest kept target still cannot outgrow the remaining text budget.
        let bytes = pdf("0041".repeat(cmap::MAX_TARGET_UNITS));
        let limits = Limits {
            max_text: 256 * 1024,
            ..Limits::default()
        };
        let started = Instant::now();
        assert_eq!(
            extract(bytes.clone().into(), None, &limits, &|| false).unwrap_err(),
            TOO_LARGE
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        walk(&bytes, 256 * 1024, |walker, page| {
            assert!(walker.truncated && walker.exhausted);
            assert!(walker.output.len() <= 256 * 1024 + 64);
            assert!(page.text.len() <= 256 * 1024 + 128);
        });
    }

    #[test]
    fn hostile_to_unicode_ranges_are_parsed_once_without_expansion() {
        const FONTS: usize = 300;
        // 1 catalog, 2 pages, 3 page, 4 content, 5 descendant, 6 shared hostile
        // ToUnicode, 7 unusable ToUnicode, then the font dictionaries.
        let descendant = b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Hostile /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /DW 600 >>";
        let mut objects = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            Vec::new(),
            Vec::new(),
            descendant.to_vec(),
            cmap_program(
                "1 begincodespacerange <00000000> <FFFFFFFF> endcodespacerange\n2 beginbfrange <00000000> <FFFFFFFF> <0041> <000000> <FFFFFF> <0061> endbfrange",
            ),
            // Only an over-long target: unusable here, and never handed to lopdf.
            cmap_program(&format!(
                "1 beginbfrange <0000> <FFFF> <{}> endbfrange",
                "0041".repeat(cmap::MAX_TARGET_UNITS + 1)
            )),
        ];
        let mut names = String::new();
        let mut content = String::from("BT 72 700 Td ");
        for index in 0..FONTS {
            objects.push(
                b"<< /Type /Font /Subtype /Type0 /BaseFont /Hostile /Encoding /Identity-H /DescendantFonts [5 0 R] /ToUnicode 6 0 R >>"
                    .to_vec(),
            );
            names.push_str(&format!("/H{index} {} 0 R ", 8 + index));
            content.push_str(&format!("/H{index} 12 Tf <000000{:02X}> Tj ", index % 26));
        }
        objects.push(
            b"<< /Type /Font /Subtype /Type0 /BaseFont /Unusable /Encoding /Identity-H /DescendantFonts [5 0 R] /ToUnicode 7 0 R >>"
                .to_vec(),
        );
        names.push_str(&format!("/U {} 0 R ", 8 + FONTS));
        content.push_str("0 -20 Td /U 12 Tf <00410042> Tj ET");
        objects[2] = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << {names}>> >> /Contents 4 0 R >>"
        )
        .into_bytes();
        objects[3] = stream("", content.as_bytes());
        let bytes = document(&objects);
        let started = Instant::now();
        let text = read(bytes.clone(), None).unwrap().text;
        assert!(started.elapsed() < Duration::from_secs(2));
        let alphabet: String = ('A'..='Z').collect();
        assert!(
            text.starts_with(&format!("[page 1]\n{alphabet}{alphabet}")),
            "{text}"
        );
        assert_eq!(
            text.matches(|c: char| c.is_ascii_uppercase()).count(),
            FONTS
        );
        walk(&bytes, usize::MAX, |walker, _| {
            assert_eq!(walker.fonts.len(), FONTS + 1);
            assert_eq!(walker.cmaps.len(), 2);
            assert_eq!(walker.cmap_entries, MAX_DOCUMENT_CMAP_ENTRIES - 2);
            assert_eq!(walker.undecodable, 4);
        });
    }

    #[test]
    fn form_xobjects_contribute_text_with_their_own_resources() {
        let form = stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F9 3 0 R >> >>",
            b"BT /F9 12 Tf 72 500 Td (Inside form) Tj ET",
        );
        let content = b"BT /F1 12 Tf 72 720 Td (Outside) Tj ET /Fm1 Do".to_vec();
        let mut bytes = build(&helvetica(), &form, &[content], &[]);
        let text = String::from_utf8_lossy(&bytes).replace("/Im1 4 0 R", "/Fm1 4 0 R");
        bytes = text.into_bytes();
        let extracted = read(bytes, None).unwrap();
        assert_eq!(extracted.text, "[page 1]\nOutside\nInside form\n");
    }

    #[test]
    fn encrypted_pdfs_need_passwords_but_owner_only_protection_is_readable() {
        assert_eq!(
            read(encrypted("synthetic-user"), None).err().unwrap(),
            ENCRYPTED
        );
        let text = read(encrypted(""), None).unwrap().text;
        assert!(text.contains("Confidential synthetic page"), "{text}");
    }

    #[test]
    fn malformed_and_truncated_pdfs_fail_without_panicking() {
        assert!(
            read(b"Not an actual PDF".to_vec(), None)
                .unwrap_err()
                .contains("header")
        );
        assert!(read(b"%PDF-1.7\n1 0 obj << /Type /Catalog".to_vec(), None).is_err());
        let complete = simple(&[&["Alpha page"], &["Beta page"]]);
        for cut in [
            16,
            complete.len() / 3,
            complete.len() / 2,
            complete.len() - 30,
        ] {
            let result = read(complete[..cut].to_vec(), None);
            if let Ok(extracted) = result {
                assert!(extracted.text.starts_with("[page 1]"));
            }
        }
        let mut noise: Vec<u8> = b"%PDF-1.4\n".to_vec();
        noise.extend((0..20_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8));
        assert!(read(noise, None).is_err());
    }

    #[test]
    fn parser_panics_and_runaway_parses_are_contained() {
        let panicked = contained(Duration::from_secs(5), &|| false, |_, _| -> Result<()> {
            panic!("synthetic parser bug")
        });
        assert_eq!(
            panicked.unwrap_err(),
            "PDF could not be parsed (contained parser failure)."
        );
        let started = Instant::now();
        let runaway = contained(Duration::from_millis(50), &|| false, |_, _| -> Result<()> {
            std::thread::sleep(Duration::from_secs(30));
            Ok(())
        });
        assert!(runaway.unwrap_err().contains("time budget"));
        assert!(started.elapsed() < Duration::from_secs(5));
        let cooperative = contained(Duration::from_millis(50), &|| false, |cancel, deadline| {
            while !cancel.load(Ordering::Acquire) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok("partial")
        });
        assert_eq!(cooperative.unwrap(), "partial");
    }

    #[test]
    fn cancellation_stops_text_layer_and_ocr() {
        let bytes = simple(&[&["Text"]]);
        assert_eq!(
            extract(bytes.into(), None, &Limits::default(), &|| true)
                .err()
                .unwrap(),
            CANCELLED
        );
        let stop = Cell::new(false);
        let mut ocr = Fake::new(Duration::from_secs(2));
        ocr.stop_after = Some((1, &stop));
        let result = extract(
            build(
                &helvetica(),
                &gray_pixel(),
                &[image_page(), image_page(), image_page()],
                &[],
            )
            .into(),
            Some(&mut ocr),
            &Limits::default(),
            &|| stop.get(),
        );
        assert_eq!(result.err().unwrap(), CANCELLED);
        assert_eq!(ocr.pages, vec![0]);
    }

    struct Fake<'a> {
        budget: Duration,
        prepared: usize,
        pages: Vec<u32>,
        unavailable: bool,
        stop_after: Option<(usize, &'a Cell<bool>)>,
    }

    impl Fake<'_> {
        fn new(budget: Duration) -> Self {
            Self {
                budget,
                prepared: 0,
                pages: Vec::new(),
                unavailable: false,
                stop_after: None,
            }
        }
    }

    impl PageOcr for Fake<'_> {
        fn prepare(
            &mut self,
            deadline: Instant,
            _: &dyn Fn() -> bool,
        ) -> std::result::Result<(), OcrError> {
            assert!(deadline <= Instant::now() + self.budget);
            self.prepared += 1;
            if self.unavailable {
                return Err(OcrError::Unavailable(
                    "synthetic OCR language missing".into(),
                ));
            }
            Ok(())
        }

        fn recognize(
            &mut self,
            page: u32,
            deadline: Instant,
            _: &dyn Fn() -> bool,
        ) -> std::result::Result<String, OcrError> {
            assert!(deadline <= Instant::now() + self.budget);
            self.pages.push(page);
            if let Some((after, stop)) = self.stop_after
                && self.pages.len() >= after
            {
                stop.set(true);
            }
            Ok(format!("Recognized   scan\n\n\n\npage {}", page + 1))
        }
    }

    fn scanned(images: usize) -> Vec<u8> {
        let mut contents = vec![text_page(&["Typed cover page with a real text layer"])];
        contents.extend((0..images).map(|_| image_page()));
        contents.push(b"BT /F1 12 Tf 72 720 Td (7) Tj ET".to_vec());
        build(&helvetica(), &gray_pixel(), &contents, &[])
    }

    #[test]
    fn weak_text_pages_choose_bounded_ocr_with_labels() {
        let limits = Limits::default();
        let mut ocr = Fake::new(limits.ocr_page);
        let extracted = read(scanned(12), Some(&mut ocr)).unwrap();
        let text = extracted.text;
        assert!(extracted.complete);
        assert_eq!(ocr.prepared, 1);
        assert_eq!(ocr.pages, (1..=10).collect::<Vec<u32>>());
        assert!(text.starts_with("[page 1]\nTyped cover page with a real text layer\n"));
        assert!(text.contains("[page 2, OCR]\nRecognized scan\n\npage 2\n"));
        assert!(text.contains("[page 11, OCR]\nRecognized scan\n\npage 11\n"));
        assert!(text.contains("[page 12]\n(no text layer; OCR limit of 10 pages reached)\n"));
        assert!(text.contains("[page 13]\n(no text layer; OCR limit of 10 pages reached)\n"));
        // A short text page without images is a real (if brief) text layer, not a scan.
        assert!(text.ends_with("[page 14]\n7\n"), "{text}");
    }

    #[test]
    fn ocr_is_skipped_for_grep_unavailable_engines_and_exhausted_budgets() {
        let skipped = read(scanned(2), None).unwrap();
        assert!(!skipped.complete);
        assert!(
            skipped.text.contains(
                "[page 2]\n(no text layer; OCR runs only when read_file reads this PDF)\n"
            )
        );
        let mut missing = Fake::new(Duration::from_secs(2));
        missing.unavailable = true;
        let text = read(scanned(3), Some(&mut missing)).unwrap().text;
        assert_eq!(missing.prepared, 1);
        assert!(missing.pages.is_empty());
        assert!(text.contains("Typed cover page"));
        assert_eq!(
            text.matches("(no text layer; synthetic OCR language missing)")
                .count(),
            3
        );
        let limits = Limits {
            ocr_total: Duration::ZERO,
            ..Limits::default()
        };
        let mut idle = Fake::new(Duration::from_secs(2));
        let text = extract(scanned(2).into(), Some(&mut idle), &limits, &|| false)
            .unwrap()
            .text;
        assert_eq!(idle.prepared, 0);
        assert_eq!(text.matches("OCR time budget exhausted").count(), 2);
    }

    #[test]
    fn page_limit_and_whitespace_normalization_are_explicit() {
        assert_eq!(normalize(" a \t b \r\n\n\n\nc\u{0007}d \n"), "a b\n\nc d");
        let layer = Layer {
            pages: vec![PageText {
                text: "only".into(),
                needs_ocr: false,
            }],
            total: MAX_PAGES + 7,
            incomplete: None,
        };
        let text = assemble(layer, None, &Limits::default(), &|| false)
            .unwrap()
            .text;
        assert!(text.ends_with(&format!(
            "[pages 2-{} not processed: 500-page limit]\n",
            MAX_PAGES + 7
        )));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "opt-in: requires an installed Windows OCR language"]
    fn windows_ocr_reads_an_image_only_scanned_page() {
        let typed = build(
            &helvetica(),
            &gray_pixel(),
            &[
                b"BT /F1 54 Tf 60 600 Td (INVOICE 2026) Tj 0 -80 Td (Tomas Commander) Tj ET"
                    .to_vec(),
            ],
            &[],
        );
        let page = crate::pdf::render(&typed, 0, || false).unwrap();
        let image = image::load_from_memory(&page.png).unwrap().to_rgb8();
        let (width, height) = image.dimensions();
        let scan = stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceRGB /BitsPerComponent 8"
            ),
            &image.into_raw(),
        );
        let bytes: Arc<[u8]> = build(
            &helvetica(),
            &scan,
            &[b"q 612 0 0 792 0 0 cm /Im1 Do Q".to_vec()],
            &[],
        )
        .into();
        let mut ocr = system_ocr(bytes.clone());
        let started = Instant::now();
        let text = extract(bytes, Some(ocr.as_mut()), &Limits::default(), &|| false)
            .unwrap()
            .text;
        assert!(text.starts_with("[page 1, OCR]\n"), "{text}");
        let folded = crate::search::fold(&text);
        assert!(
            folded.contains("invoice") && folded.contains("commander"),
            "{text}"
        );
        assert!(started.elapsed() < Duration::from_secs(25));
    }
}
