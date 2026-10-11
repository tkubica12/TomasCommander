use crate::files::{self, FileResult, Scope};
use std::{
    fs::File,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

pub const MAX_TEXT_BYTES: usize = 1_048_576;
pub const MAX_IMAGE_BYTES: usize = 16_777_216;
pub const MAX_IMAGE_EDGE: u32 = 4096;
pub const MAX_CSV_ROWS: usize = 10_000;
pub const MAX_CSV_COLUMNS: usize = 64;
pub const MAX_CSV_CELLS: usize = 50_000;

pub struct Preview {
    pub path: PathBuf,
    pub content: Content,
    pub notice: &'static str,
}

pub enum Content {
    Editable(crate::changes::Editor),
    Pdf {
        page: u32,
        count: u32,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    Markdown {
        text: String,
        links: Vec<String>,
    },
    Archive(crate::archives::Inventory),
    Text(Vec<String>),
    Table {
        rows: Vec<Vec<String>>,
        columns: usize,
    },
    Image {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
}

pub fn load(scope: &Scope, path: &Path, cancelled: impl Fn() -> bool) -> FileResult<Preview> {
    load_page(scope, path, 0, cancelled)
}

pub fn load_archive_entry(
    scope: &Scope,
    archive: &Path,
    name: &str,
    page: u32,
    rotation: u8,
    cancelled: impl Fn() -> bool,
) -> FileResult<Preview> {
    let bytes = crate::archive_changes::read_entry(scope, archive, name, &cancelled)?;
    decode_snapshot(PathBuf::from(name), bytes, page, rotation, cancelled)
}

pub fn load_page(
    scope: &Scope,
    path: &Path,
    page: u32,
    cancelled: impl Fn() -> bool,
) -> FileResult<Preview> {
    load_transformed(scope, path, page, 0, cancelled)
}

pub fn load_transformed(
    scope: &Scope,
    path: &Path,
    page: u32,
    quarter_turns: u8,
    cancelled: impl Fn() -> bool,
) -> FileResult<Preview> {
    if quarter_turns > 3 {
        return Err("Image rotation must be zero to three quarter turns.".into());
    }
    if cancelled() {
        return Err("Preview cancelled.".into());
    }
    let path = scope.resolve(path)?;
    let before = files::fingerprint(&path)?;
    if before.directory {
        return Err("Choose a regular file for internal preview.".into());
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if extension == "zip" {
        let inventory = crate::archives::inventory(scope, &path, cancelled)?;
        return Ok(Preview {
            path,
            content: Content::Archive(inventory),
            notice: "ZIP inventory / content and CRC verified on member preview or exact effect planning",
        });
    }
    let image = matches!(extension.as_str(), "jpg" | "jpeg" | "png" | "bmp");
    let pdf = extension == "pdf";
    if quarter_turns != 0 && !image && !pdf {
        return Err("Rotation is available only for images and PDF pages.".into());
    }
    let limit = if image || pdf {
        MAX_IMAGE_BYTES
    } else {
        MAX_TEXT_BYTES
    };
    if before.size > limit as u64 {
        return Err(format!(
            "Preview input exceeds the {} MiB limit.",
            limit / 1_048_576
        ));
    }
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !image
        && !pdf
        && !matches!(
            extension.as_str(),
            "txt"
                | "log"
                | "env"
                | "yaml"
                | "yml"
                | "json"
                | "csv"
                | "toml"
                | "ini"
                | "xml"
                | "rs"
                | "py"
                | "ps1"
                | "sh"
                | "js"
                | "ts"
                | "css"
                | "md"
                | "markdown"
                | "html"
                | "htm"
        )
        && filename != ".env"
        && !filename.starts_with(".env.")
    {
        return Err("This format has no internal viewer yet. Enter opens it externally.".into());
    }
    let mut bytes = Vec::new();
    File::open(&path)
        .map_err(|error| format!("Open preview: {error}"))?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Read preview: {error}"))?;
    if bytes.len() > limit {
        return Err("File grew past the preview limit.".into());
    }
    if cancelled() {
        return Err("Preview cancelled.".into());
    }
    let preview = decode_snapshot(path.clone(), bytes, page, quarter_turns, &cancelled)?;
    if files::fingerprint(&path)? != before {
        return Err("File changed while being previewed. Open preview again.".into());
    }
    Ok(preview)
}

pub(crate) fn decode_snapshot(
    path: PathBuf,
    bytes: Vec<u8>,
    page: u32,
    quarter_turns: u8,
    cancelled: impl Fn() -> bool,
) -> FileResult<Preview> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let image = matches!(extension.as_str(), "jpg" | "jpeg" | "png" | "bmp");
    let pdf = extension == "pdf";
    if !image && !pdf && !crate::changes::editable(&path) {
        return Err(
            "This entry has no internal viewer; extract it explicitly to open externally.".into(),
        );
    }
    if bytes.len()
        > if image || pdf {
            MAX_IMAGE_BYTES
        } else {
            MAX_TEXT_BYTES
        }
        || quarter_turns > 3
    {
        return Err("Preview data exceeds its format limit or rotation is invalid.".into());
    }
    let (content, notice) = if image || pdf {
        let rendered = if pdf {
            Some(crate::pdf::render(&bytes, page, &cancelled)?)
        } else {
            None
        };
        let image_bytes = rendered
            .as_ref()
            .map_or(bytes.as_slice(), |page| page.png.as_slice());
        let mut reader = image::ImageReader::new(Cursor::new(image_bytes))
            .with_guessed_format()
            .map_err(|error| format!("Identify image: {error}"))?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(MAX_IMAGE_EDGE);
        limits.max_image_height = Some(MAX_IMAGE_EDGE);
        limits.max_alloc = Some(64 * 1_048_576);
        reader.limits(limits);
        use image::ImageDecoder;
        let mut decoder = reader
            .into_decoder()
            .map_err(|error| format!("Create image decoder: {error}"))?;
        let orientation = decoder
            .orientation()
            .map_err(|error| format!("Read image orientation: {error}"))?;
        let mut decoded = image::DynamicImage::from_decoder(decoder)
            .map_err(|error| format!("Decode image: {error}"))?;
        decoded.apply_orientation(orientation);
        decoded = match quarter_turns {
            0 => decoded,
            1 => decoded.rotate90(),
            2 => decoded.rotate180(),
            3 => decoded.rotate270(),
            _ => unreachable!("validated quarter turns"),
        };
        let rgba = decoded.to_rgba8();
        let content = if let Some(rendered) = rendered {
            Content::Pdf {
                page: rendered.index,
                count: rendered.count,
                width: rgba.width(),
                height: rgba.height(),
                rgba: rgba.into_raw(),
            }
        } else {
            Content::Image {
                width: rgba.width(),
                height: rgba.height(),
                rgba: rgba.into_raw(),
            }
        };
        (
            content,
            if pdf {
                "Windows-native PDF / read-only / page bitmap / no browser engine"
            } else {
                "Local image / read-only / no upload"
            },
        )
    } else {
        let text = String::from_utf8(bytes)
            .map_err(|_| "Text preview requires UTF-8; the original file is unchanged.")?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        if text.contains('\0') {
            return Err("Binary/NUL content cannot be shown as text.".into());
        }
        match extension.as_str() {
            "csv" => {
                let rows = parse_csv(text)?;
                let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
                (
                    Content::Table { rows, columns },
                    "CSV / comma delimiter / quoted fields / read-only",
                )
            }
            "json" => {
                let parsed: serde_json::Value =
                    serde_json::from_str(text).map_err(|error| format!("Invalid JSON: {error}"))?;
                let formatted = serde_json::to_string_pretty(&parsed)
                    .map_err(|error| format!("Format JSON: {error}"))?;
                (
                    Content::Text(text_lines(&formatted)?),
                    "Formatted JSON / read-only / original bytes unchanged",
                )
            }
            "md" | "markdown" => (
                prepare_markdown(text, &cancelled)?,
                "Native Markdown / read-only / document images omitted / links inspected locally, never automatically opened",
            ),
            "html" | "htm" => (
                prepare_markdown(&crate::html::markdown(text)?, &cancelled)?,
                "Native semantic HTML / headings, emphasis, lists, tables, code / CSS layout, scripts, images and embedded content not activated",
            ),
            _ => (
                Content::Text(text_lines(text)?),
                "UTF-8 text/source / read-only / no upload",
            ),
        }
    };
    if cancelled() {
        return Err("Preview cancelled.".into());
    }
    Ok(Preview {
        path,
        content,
        notice,
    })
}

fn prepare_markdown(text: &str, cancelled: impl Fn() -> bool) -> FileResult<Content> {
    use pulldown_cmark::{Event, Options, Parser, Tag};
    if text.len() > 65_536 {
        return Err("Rendered Markdown exceeds the 64 KiB limit.".into());
    }
    text_lines(text)?;
    // Keep options aligned with the renderer so every rendered link is intercepted.
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_DEFINITION_LIST;
    let mut links = std::collections::BTreeSet::new();
    let mut images = Vec::new();
    let mut depth = 0_usize;
    for (count, (event, range)) in Parser::new_ext(text, options)
        .into_offset_iter()
        .enumerate()
    {
        if cancelled() {
            return Err("Markdown preview cancelled.".into());
        }
        if count >= 4000 {
            return Err("Markdown exceeds the 4000-event render limit.".into());
        }
        match event {
            Event::Start(tag) => {
                depth += 1;
                if depth > 32 {
                    return Err("Markdown nesting exceeds 32 levels.".into());
                }
                match tag {
                    Tag::Link { dest_url, .. } => {
                        links.insert(dest_url.to_string());
                        if links.len() > 128 {
                            return Err("Markdown exceeds the 128-link limit.".into());
                        }
                    }
                    Tag::Image { .. } => images.push(range),
                    _ => (),
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => (),
        }
    }
    let mut rendered = text.to_owned();
    images.sort_by_key(|range| range.start);
    let mut previous = 0;
    for range in &images {
        if range.start < previous {
            return Err("Nested Markdown image ranges are unsupported.".into());
        }
        previous = range.end;
    }
    for range in images.into_iter().rev() {
        rendered.replace_range(range, "[image omitted]");
    }
    // Image removal can change inline parsing; collect hooks from the actual render input.
    for (count, event) in Parser::new_ext(&rendered, options).enumerate() {
        if cancelled() {
            return Err("Markdown preview cancelled.".into());
        }
        if count >= 4000 {
            return Err("Markdown exceeds render limits after asset removal.".into());
        }
        match event {
            Event::Start(Tag::Link { dest_url, .. }) => {
                links.insert(dest_url.to_string());
            }
            Event::Start(Tag::Image { .. }) => {
                return Err(
                    "Markdown asset removal produced another image reference; preview stopped."
                        .into(),
                );
            }
            _ => (),
        }
    }
    if links.len() > 128 {
        return Err("Markdown exceeds the 128-link limit after asset removal.".into());
    }
    Ok(Content::Markdown {
        text: rendered,
        links: links.into_iter().collect(),
    })
}

pub(crate) fn validate_text(text: &str) -> FileResult<()> {
    if text.len() > MAX_TEXT_BYTES || text.lines().count() > 20_000 {
        return Err("Text presentation exceeds the preview size/line limit.".into());
    }
    if text.lines().any(|line| line.len() > 16_384) {
        return Err("A text line exceeds the 16 KiB presentation limit.".into());
    }
    Ok(())
}

fn text_lines(text: &str) -> FileResult<Vec<String>> {
    validate_text(text)?;
    Ok(text.lines().map(str::to_owned).collect())
}

pub fn parse_csv(text: &str) -> FileResult<Vec<Vec<String>>> {
    if text.len() > MAX_TEXT_BYTES {
        return Err("CSV exceeds the input limit.".into());
    }
    let mut characters = text.chars().peekable();
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut closed_quote = false;
    let mut cells = 0;
    let mut pending = false;
    while let Some(character) = characters.next() {
        pending = true;
        if quoted {
            if character == '"' {
                if characters.peek() == Some(&'"') {
                    characters.next();
                    field.push('"');
                } else {
                    quoted = false;
                    closed_quote = true;
                }
            } else {
                field.push(character);
            }
            continue;
        }
        match character {
            '"' if field.is_empty() && !closed_quote => quoted = true,
            ',' | '\r' | '\n' => {
                push_csv_field(&mut row, &mut field, &mut cells)?;
                closed_quote = false;
                if character != ',' {
                    if character == '\r' && characters.peek() == Some(&'\n') {
                        characters.next();
                    }
                    if rows.len() == MAX_CSV_ROWS {
                        return Err("CSV exceeds the row limit.".into());
                    }
                    rows.push(std::mem::take(&mut row));
                    pending = false;
                }
            }
            '"' => return Err("CSV quote must start a field.".into()),
            _ if closed_quote => return Err("CSV has characters after a closing quote.".into()),
            _ => field.push(character),
        }
    }
    if quoted {
        return Err("CSV has an unterminated quoted field.".into());
    }
    if pending {
        push_csv_field(&mut row, &mut field, &mut cells)?;
        if rows.len() == MAX_CSV_ROWS {
            return Err("CSV exceeds the row limit.".into());
        }
        rows.push(row);
    }
    Ok(rows)
}

fn push_csv_field(row: &mut Vec<String>, field: &mut String, cells: &mut usize) -> FileResult<()> {
    if row.len() == MAX_CSV_COLUMNS || *cells == MAX_CSV_CELLS || field.len() > 16_384 {
        return Err("CSV exceeds the column/cell limit.".into());
    }
    *cells += 1;
    row.push(std::mem::take(field));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_is_bounded_and_removes_assets_while_intercepting_all_link_schemes() {
        let source = "# Příliš\n\n- one\n- **two**\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\nlet x = 1;\n```\n\n![private](file:///secret.env)\n![remote](https://example.org/image.png)\n[web](https://example.org) $[unsafe](javascript:alert)${}\n";
        let Content::Markdown { text, links } = prepare_markdown(source, || false).unwrap() else {
            panic!("Expected rendered Markdown");
        };
        assert!(text.contains("| a | b |") && text.contains("```rust"));
        assert!(!text.contains("secret.env") && !text.contains("image.png"));
        assert!(links.contains(&"https://example.org".to_owned()));
        assert!(links.contains(&"javascript:alert".to_owned()));
        assert!(prepare_markdown(source, || true).is_err());
        assert!(prepare_markdown(&"x".repeat(65_537), || false).is_err());
        assert!(prepare_markdown(&"- item\n".repeat(2000), || false).is_err());
    }

    #[test]
    fn csv_preserves_unicode_quotes_newlines_empty_fields_and_crlf() {
        assert_eq!(
            parse_csv("name,value\r\n\"Příliš, žluťoučký\",\"a\"\"b\"\r\n\"multi\nline\",\r\n")
                .unwrap(),
            vec![
                vec!["name", "value"],
                vec!["Příliš, žluťoučký", "a\"b"],
                vec!["multi\nline", ""]
            ]
        );
        assert!(parse_csv("\"unclosed").is_err());
        assert!(parse_csv("unquoted\"quote").is_err());
        assert!(parse_csv("\"closed\"extra").is_err());
        assert!(parse_csv("").unwrap().is_empty());
        assert_eq!(parse_csv("a,").unwrap(), vec![vec!["a", ""]]);
    }

    #[test]
    fn viewer_limits_reject_unbounded_text_and_csv() {
        assert!(text_lines(&"x".repeat(16_385)).is_err());
        assert!(text_lines(&"\n".repeat(20_001)).is_err());
        assert!(parse_csv(&",".repeat(MAX_CSV_COLUMNS)).is_err());
        assert!(parse_csv(&"\n".repeat(MAX_CSV_ROWS + 1)).is_err());
        assert!(parse_csv(&"x".repeat(16_385)).is_err());
        assert!(parse_csv(&"x".repeat(MAX_TEXT_BYTES + 1)).is_err());
    }

    #[test]
    fn local_preview_checks_content_scope_cancel_change_and_image_pixels() {
        use std::{
            cell::Cell,
            fs,
            sync::atomic::{AtomicU64, Ordering},
        };
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tomas-viewer-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let scope = Scope::fixture(&root).unwrap();
        let text = root.join("sample.txt");
        fs::write(&text, "\u{feff}Příliš\r\nnext").unwrap();
        let preview = load(&scope, &text, || false).unwrap();
        assert_eq!(preview.path, scope.resolve(&text).unwrap());
        let Content::Text(lines) = preview.content else {
            panic!("Expected text");
        };
        assert_eq!(lines, ["Příliš", "next"]);
        assert!(load(&scope, &text, || true).is_err());
        assert!(load(&scope, &root, || false).is_err());
        assert!(load(&scope, &root.join("missing.txt"), || false).is_err());
        let calls = Cell::new(0);
        assert!(
            load(&scope, &text, || {
                let call = calls.get();
                calls.set(call + 1);
                if call == 2 {
                    fs::write(&text, "Changed to a different byte count").unwrap();
                }
                false
            })
            .err()
            .unwrap()
            .contains("changed")
        );
        fs::write(&text, [0xff]).unwrap();
        assert!(load(&scope, &text, || false).is_err());
        fs::write(&text, "binary\0").unwrap();
        assert!(load(&scope, &text, || false).is_err());
        let json = root.join("sample.json");
        fs::write(&json, "{\"name\":\"Příliš\"}").unwrap();
        assert!(matches!(
            load(&scope, &json, || false).unwrap().content,
            Content::Text(_)
        ));
        assert_eq!(fs::read_to_string(&json).unwrap(), "{\"name\":\"Příliš\"}");
        fs::write(&json, "{bad").unwrap();
        assert!(load(&scope, &json, || false).is_err());
        let csv = root.join("sample.csv");
        fs::write(&csv, "a,b\r\n\"x,y\",z").unwrap();
        let Content::Table { rows, columns } = load(&scope, &csv, || false).unwrap().content else {
            panic!("Expected table");
        };
        assert_eq!(columns, 2);
        assert_eq!(rows[1], ["x,y", "z"]);
        let mut image = image::RgbaImage::from_pixel(2, 3, image::Rgba([10, 20, 30, 255]));
        image.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        for extension in ["png", "bmp", "jpeg"] {
            let path = root.join(format!("sample.{extension}"));
            image::DynamicImage::ImageRgba8(image.clone())
                .to_rgb8()
                .save(&path)
                .unwrap();
            let Content::Image {
                width,
                height,
                rgba,
            } = load(&scope, &path, || false).unwrap().content
            else {
                panic!("Expected image");
            };
            assert_eq!((width, height, rgba.len()), (2, 3, 24));
            if extension != "jpeg" {
                assert_eq!(&rgba[..4], &[255, 0, 0, 255]);
                let before = fs::read(&path).unwrap();
                let Content::Image {
                    width,
                    height,
                    rgba,
                } = load_transformed(&scope, &path, 0, 1, || false)
                    .unwrap()
                    .content
                else {
                    panic!("Expected rotated image");
                };
                assert_eq!((width, height), (3, 2));
                assert_eq!(&rgba[8..12], &[255, 0, 0, 255]);
                assert_eq!(fs::read(&path).unwrap(), before);
            } else {
                let original = fs::read(&path).unwrap();
                let exif =
                    b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
                let mut tagged = original[..2].to_vec();
                tagged.extend_from_slice(&[0xff, 0xe1]);
                tagged.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
                tagged.extend_from_slice(exif);
                tagged.extend_from_slice(&original[2..]);
                fs::write(&path, &tagged).unwrap();
                let Content::Image { width, height, .. } =
                    load(&scope, &path, || false).unwrap().content
                else {
                    panic!("Expected EXIF-oriented JPEG");
                };
                assert_eq!((width, height), (3, 2));
                assert_eq!(fs::read(&path).unwrap(), tagged);
            }
            fs::remove_file(path).unwrap();
        }
        let large = root.join("large.txt");
        File::create(&large)
            .unwrap()
            .set_len(MAX_TEXT_BYTES as u64 + 1)
            .unwrap();
        assert!(load(&scope, &large, || false).is_err());
        for path in [text, json, csv, large] {
            fs::remove_file(path).unwrap();
        }
        fs::remove_dir(root).unwrap();
    }
}
